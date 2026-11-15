//! The application shell: a workspace column, one or more panes, and the key
//! handling that drives them.

// Without this the executable links as a console program, and Windows hands it a
// console window before the first frame is drawn — a black box behind the app
// that a double-click should never produce.
//
// Only in release: a debug build keeps its console, because that is where its
// logs and panics go. The shipped binary has no use for either, and the one
// stdout-dependent path (`--uninstall`) reports through the log rather than
// through the console it would otherwise inherit.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod grid_element;
mod install;
mod layout;
mod palette;
mod pty_command;
mod terminal;
mod update;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use grid_element::{GridStyle, ImageCache, TerminalGrid};
use layout::{Axis, Direction, Layout, Rect};
use pty::{Command, Pty};
use terminal::Session;
use vt::Rgb;

/// The shell to start, wrapped for shell integration when it is on.
fn build_command(shell: config::Shell, integration: bool) -> Command {
    let command = pty_command::profile(shell);
    if integration {
        pty_command::with_shell_integration(command)
    } else {
        command
    }
}

/// The split glyphs, drawn inline.
///
/// The bundled `columns-2` and `rows-2` assets paint nothing on this platform,
/// so the shapes come from data instead: a frame with one dividing line, which
/// is what a split does.
const SPLIT_HORIZONTAL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M12 4v16"/></svg>"#;
const SPLIT_VERTICAL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 12h18"/></svg>"#;

/// The three window controls, drawn at the weight Windows uses for them.
///
/// They are drawn rather than taken from the icon set because the set's shapes
/// are 16 px UI glyphs, and these read as stray marks at that stroke width when
/// scaled to the 10 px a title bar button wants.
const WINDOW_MINIMIZE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>"#;
const WINDOW_MAXIMIZE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><rect x="5" y="5" width="14" height="14" rx="1.5"/></svg>"#;

/// Zoom a pane to fill the card; and the reverse.
const ZOOM_IN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V5.5A1.5 1.5 0 0 1 5.5 4H9"/><path d="M15 4h3.5A1.5 1.5 0 0 1 20 5.5V9"/><path d="M20 15v3.5a1.5 1.5 0 0 1-1.5 1.5H15"/><path d="M9 20H5.5A1.5 1.5 0 0 1 4 18.5V15"/></svg>"#;
const ZOOM_OUT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4v3.5A1.5 1.5 0 0 1 7.5 9H4"/><path d="M20 9h-3.5A1.5 1.5 0 0 1 15 7.5V4"/><path d="M15 20v-3.5a1.5 1.5 0 0 1 1.5-1.5H20"/><path d="M4 15h3.5A1.5 1.5 0 0 1 9 16.5V20"/></svg>"#;

const WINDOW_CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>"#;
const WINDOW_RESTORE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><rect x="4" y="8" width="12" height="12" rx="1.5"/><path d="M8 8V5.5A1.5 1.5 0 0 1 9.5 4h9A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H16"/></svg>"#;

/// The plus that opens a workspace. It lives in the column's foot — and in the
/// rail, and in the empty list — because that is where the workspace
/// it creates will appear.
const PLUS_SMALL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>"#;

/// The close cross on a row in the workspace column. Smaller and lighter than the
/// window caption's, because it sits inside an 18-pixel circle rather than a
/// full caption button.
const TAB_CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>"#;

/// The app mark in the title bar: a prompt caret inside a window frame.
///
/// Drawn inline because several icons from the bundled set render blank on this
/// platform, and the one glyph that identifies the app is the worst place to
/// gamble on that.
const APP_MARK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7 9.5l3 2.5-3 2.5"/><path d="M12.5 15h4.5"/></svg>"#;

/// The about control. A circled `i` is what every platform uses for "what is
/// this program", and it is deliberately not the app mark: that glyph already
/// sits at the leading edge as the window's identity, and using it twice in one
/// bar leaves the second copy with no reason to look clickable.
/// The workspace glyph: a frame cut in both directions, which is what a
/// workspace holding more than one shell looks like.
const WORKSPACE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M12 4v16M3 12h18"/></svg>"#;

/// A sun and a moon: the theme row in the palette toggles between them.
const THEME_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4.5"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4"/></svg>"#;

/// The branch pill in the crumbs.
const BRANCH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.4" stroke-linecap="round"><circle cx="7" cy="6" r="2"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="8" r="2"/><path d="M7 8v8M17 10c0 3-3 3-7 4"/></svg>"#;

/// The terminal glyph: the palette's session rows wear it, the same shape the
/// app mark uses.
const TERMINAL_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 9.5l3 2.5-3 2.5"/><path d="M12.5 15h4.5"/></svg>"#;

/// A minus, for the steppers. Drawn rather than typed: a hyphen and a minus
/// sign are different glyphs in most fonts, and only one of them lines up with
/// the plus beside it.
const MINUS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>"#;

/// The two arrows the find strip walks matches with.
const ARROW_UP: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5"/><path d="M5 12l7-7 7 7"/></svg>"#;
const ARROW_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M19 12l-7 7-7-7"/></svg>"#;

/// The search glyph: the command entry in the top bar, and the find field.
const SEARCH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/></svg>"#;

/// Three dots: the app menu. A menu that needs a word would need a wider bar.
const MENU_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.6" stroke-linecap="round"><path d="M6 12h.01M12 12h.01M18 12h.01"/></svg>"#;

/// A chevron pointing at the edge the list folds toward.
const COLLAPSE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6"/></svg>"#;

const INFO_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 11v5.5"/><path d="M12 7.6h.01"/></svg>"#;

/// Three sliders: the settings entry in the title bar.
///
/// Not a gear. A gear at sixteen pixels is a circle with bumps, and the bar
/// already holds a circled `i` two buttons away — at that size the two would be
/// the same grey dot with a different fringe. Sliders keep their shape when they
/// are small, and they name the idea rather than the object.
const SETTINGS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5h-7"/><path d="M10 5H3"/><path d="M21 12h-9"/><path d="M8 12H3"/><path d="M21 19h-5"/><path d="M12 19H3"/><circle cx="12" cy="5" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="14" cy="19" r="2"/></svg>"#;

/// Height of the drawn title bar.
///
/// The bar is not the system caption any more: it carries the workspace's
/// identity and the command entry, so it is a row of the window rather than a
/// strip above one. 44 px is the height that fits a 28 px control with air on
/// both sides.
const TITLE_BAR_HEIGHT: f32 = 44.0;

/// Height of the strip naming a pane.
const PANE_HEADER_HEIGHT: f32 = 30.0;

/// Height of the conditional status bar. It only exists while it has
/// something to say, so this is the cost of speaking, not a permanent edge.
const STATUS_BAR_HEIGHT: f32 = 26.0;

/// Corner radius of a pane card.
const PANE_RADIUS: f32 = 10.0;

/// Corner radius of a small card — a tab, a sidebar row. The pane card is
/// larger and takes its own, but every 28 px-high card in the app wears this
/// one, so the rounded rectangle means the same thing wherever it appears.
const CHIP_RADIUS: f32 = 10.0;

/// Space between two panes. The gap is the divider: nothing is drawn in it
/// at rest, and the pointer is the only thing that shows it can be moved.
const PANE_GAP: f32 = 8.0;

/// The leading inset of a pane's contents. The header's name and the grid's
/// first cell both start here, which is what makes them read as one column.
const PANE_TEXT_INSET: f32 = 11.0;

/// The grid's own padding, inside the card. Wider than the header's inset on
/// the trailing side so a full line of text is not flush against the border,
/// and deeper at the bottom so the last row has somewhere to sit.
const GRID_PAD_X: f32 = 13.0;
const GRID_PAD_TOP: f32 = 10.0;
const GRID_PAD_BOTTOM: f32 = 14.0;

/// One press of the font size control. Half a point, so the sizes people
/// actually want — 12.5, 13.5 — are reachable.
const FONT_STEP: f32 = 0.5;

/// One press of the scrollback control.
const SCROLLBACK_STEP: usize = 1_000;

/// Width of the workspace list.
const SIDEBAR_WIDTH: f32 = 238.0;

/// Narrowest the list gets: a chip of glyphs with room to aim at.
const RAIL_WIDTH: f32 = 56.0;

const MAX_SIDEBAR_WIDTH: f32 = 420.0;

/// Below this the list is too narrow for names, so it draws as a rail.
const RAIL_THRESHOLD: f32 = 118.0;

/// Builds the 256-entry palette the parser should report: the ANSI sixteen
/// followed by the library's defaults for the rest.
fn palette(mode: gpui_kit::component::ThemeMode) -> [Rgb; 256] {
    // The ANSI sixteen are the terminal's own colours; only the shell's accent
    // follows the setting, and that is applied with the rest of the theme.
    let ansi = palette::Palette::for_mode(mode, config::Accent::Lime).ansi();
    let mut palette = [Rgb::default(); 256];
    palette[..ansi.len()].copy_from_slice(&ansi);
    palette
}

/// One terminal pane.
struct Pane {
    session: Session,
    /// The shell this pane was started with, reused when splitting.
    command: Command,
    /// The grid size the pane was last sized to, so a resize is only sent when
    /// the computed cell count actually changes.
    grid: (u16, u16),
    /// The cell size in pixels last sent to the terminal, for the same reason.
    /// Kept beside the count because a font change moves the pixels without
    /// moving the count.
    cell: (u32, u32),
    /// Filled by the grid element during prepaint with the cells that fit.
    dims: Rc<Cell<(u16, u16)>>,
    /// Where the grid landed in window coordinates, filled by the same prepaint.
    /// The IME needs it to place its candidate window and the mouse needs it to
    /// resolve a pointer to a cell.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Whether the last poll saw this shell writing. The transition out of it is
    /// what raises the tab's badge, so a program that finishes while you are
    /// reading another tab leaves a mark instead of disappearing.
    working: bool,
}

impl Pane {
    /// Starts a shell in a new pane.
    ///
    /// The colors come from the caller rather than from a constant here, so the
    /// grid the parser reports and the grid the theme draws are always the same
    /// values.
    fn new(
        command: &Command,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Result<Self, String> {
        let session = Session::spawn(command, cols, rows)?;
        Ok(Self::from_session(session, command, cols, rows, theme))
    }

    /// Builds a pane around a session that is already running.
    ///
    /// The console can then be opened on a background thread and the pane
    /// finished here, which keeps the UI thread off the 45 ms start-up.
    fn from_session(
        mut session: Session,
        command: &Command,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Self {
        apply_colors(&mut session, theme);
        session.set_scrollback_limit(config::Config::load().scrollback);
        Self {
            session,
            command: command.clone(),
            grid: (cols, rows),
            // Unknown until the first frame measures the font; zero means "not
            // yet", which forces the first sync to send a real size.
            cell: (0, 0),
            dims: Rc::new(Cell::new((cols, rows))),
            bounds: Rc::new(Cell::new(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(0.0), px(0.0)),
            ))),
            // The first frame decides this; assuming "idle" means a shell that
            // starts by printing its banner does not count as activity.
            working: false,
        }
    }

    /// Repaints the terminal with a different theme's colors.
    fn retheme(&mut self, theme: &gpui_kit::component::Theme) {
        apply_colors(&mut self.session, theme);
    }
}

/// Installs a theme's foreground, background, caret and palette into a session.
fn apply_colors(session: &mut Session, theme: &gpui_kit::component::Theme) {
    session.set_colors(
        to_rgb(theme.foreground),
        to_rgb(theme.background),
        Some(to_rgb(theme.caret)),
        &palette(theme.mode),
    );
}

/// Converts a theme color to the byte triplet the parser stores.
fn to_rgb(color: Hsla) -> Rgb {
    let rgba: Rgba = color.into();
    Rgb::new(
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
    )
}

/// One tab: a title, the panes it holds, and how they are arranged.
struct Tab {
    /// The name this workspace was given. It is not the shell's title: a
    /// workspace is a place the user made — `myterm`, `日志`, `部署` — and
    /// naming it after whatever shell happens to be focused would rename it
    /// every time a build starts.
    title: String,
    /// Whether the name above is still the one generated at creation. A
    /// generated name is replaced by the first shell's title; a chosen one is
    /// never touched.
    auto_named: bool,
    /// Panes keyed by the id the layout tree refers to. A map rather than a vec
    /// so removing a pane never invalidates the others' identity.
    panes: BTreeMap<u64, Pane>,
    layout: Layout,
    active: u64,
    /// The id the next pane in this tab will take.
    next_id: u64,
    /// Set when a pane in this tab stopped writing while the tab was not the
    /// one on screen. Cleared by selecting it.
    unseen: bool,
}

impl Tab {
    /// Builds a tab around a pseudo console that is already running.
    fn from_pty(
        pty: Pty,
        command: &Command,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Result<Self, String> {
        let session = Session::from_pty(pty, cols, rows)?;
        Ok(Self::from_pane(Pane::from_session(
            session, command, cols, rows, theme,
        )))
    }

    fn from_pane(pane: Pane) -> Self {
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);

        Self {
            title: "终端".to_owned(),
            auto_named: true,
            panes,
            layout: Layout::Leaf(1),
            active: 1,
            next_id: 2,
            unseen: false,
        }
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.active)
    }

    fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.active)
    }

    /// Splits the active pane, moving focus into the new one.
    /// Splits the active pane around a pseudo console that is already running.
    fn split_with_pty(
        &mut self,
        axis: Axis,
        pty: Pty,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Result<(), String> {
        let command = self.command_snapshot();
        let session = Session::from_pty(pty, cols, rows)?;
        let pane = Pane::from_session(session, &command, cols, rows, theme);

        let id = self.next_id;
        self.next_id += 1;
        self.panes.insert(id, pane);

        if !self.layout.split(self.active, id, axis) {
            self.panes.remove(&id);
            return Err("无法拆分窗格".to_owned());
        }

        self.active = id;
        Ok(())
    }

    /// Closes the active pane, or reports that it is the last one.
    fn close_active(&mut self) -> bool {
        if self.panes.len() == 1 {
            return false;
        }
        if !self.layout.remove(self.active) {
            return false;
        }
        self.panes.remove(&self.active);
        self.active = self.layout.leaves().first().copied().unwrap_or(1);
        true
    }

    /// The shell command new panes in this tab should start.
    fn command_snapshot(&self) -> Command {
        // Every pane in a tab runs the same shell, so any live pane can supply
        // it. Falling back keeps split working while a pane is being replaced.
        self.panes
            .values()
            .next()
            .map(|pane| pane.command.clone())
            .unwrap_or_else(pty_command::default_profile)
    }
}

/// A sidebar edge being dragged.
struct SidebarDrag {
    /// Pointer position when the drag began, in window coordinates.
    start: f32,
    /// The width at that moment.
    width: f32,
}

/// A tab being dragged up or down the sidebar.
///
/// Reordering is per tab, not per pane: a split is one thing the user named,
/// and dragging one of its panes would ask whether the whole tab moves. The row
/// that stands for the tab is the row that drags.
struct TabDrag {
    /// Where the tab sits now.
    from: usize,
    /// Where the pointer was when the button went down.
    start: f32,
    /// The tab the pointer is over, once it has actually moved.
    target: Option<usize>,
    /// Whether the pointer has moved far enough to mean a drag rather than a
    /// click. Checked once and then kept, so a gesture that comes back to its
    /// starting row still reads as a drag.
    moved: bool,
}

/// A split being dragged.
struct ResizeDrag {
    /// The panes on either side of the divider, in visual order.
    left: u64,
    right: u64,
    axis: Axis,
    /// Where the divider started, in window coordinates along the drag axis.
    start: f32,
    /// How far the first half spans at the start of the drag.
    span: f32,
    /// The first half's share when the drag began.
    ratio: f32,
}

/// A tab's context menu: which row opened it, and where it opened.
#[derive(Clone, Copy)]
struct TabMenu {
    tab: usize,
    position: Point<Pixels>,
}

/// The find bar over the active pane, and what it found.
///
/// The terminal exposes no search over its scrollback, so this searches the
/// rows the pane is currently showing. That is the honest limit of what the
/// renderer can see; a match above the viewport would need a scrollback index
/// the library does not publish.
struct FindState {
    input: Entity<InputState>,
    /// Matches on visible rows, as the row and the first and last cell it covers.
    /// The search covers the whole terminal; this is the part of it that can be
    /// drawn.
    matches: Vec<(usize, usize, usize)>,
    /// Which of the visible matches is the selected one.
    current: usize,
    /// Matches in the whole terminal, scrollback included.
    total: usize,
    /// Which match is selected, counting from the newest. Zero-based, as the
    /// library reports it.
    selected: Option<usize>,
    _sub: Subscription,
}

/// The command palette.
struct Palette {
    input: Entity<InputState>,
    /// What the input says, mirrored here so filtering does not read an entity
    /// during render.
    query: String,
    selected: usize,
    _sub: Subscription,
}

/// One row of the command palette.
struct PaletteCommand {
    id: &'static str,
    label: &'static str,
    /// The key it already has. The palette is a discovery surface, not a second
    /// place for commands to live: everything here works without it.
    keys: &'static str,
    /// The glyph the row wears. A list of ten commands with no marks is a wall
    /// of text; the glyph is what makes a row findable before it is read.
    icon: &'static [u8],
}

const PALETTE_COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        id: "new-tab",
        label: "新建工作区",
        keys: "Ctrl+Shift+T",
        icon: PLUS_SMALL,
    },
    PaletteCommand {
        id: "close-tab",
        label: "关闭工作区",
        keys: "Ctrl+Shift+Q",
        icon: WINDOW_CLOSE,
    },
    PaletteCommand {
        id: "close-pane",
        label: "关闭窗格",
        keys: "Ctrl+Shift+W",
        icon: WINDOW_CLOSE,
    },
    PaletteCommand {
        id: "split-right",
        label: "向右分屏",
        keys: "Ctrl+Shift+D",
        icon: SPLIT_HORIZONTAL,
    },
    PaletteCommand {
        id: "split-down",
        label: "向下分屏",
        keys: "Ctrl+Shift+E",
        icon: SPLIT_VERTICAL,
    },
    PaletteCommand {
        id: "zoom",
        label: "缩放窗格",
        keys: "Ctrl+Shift+Z",
        icon: ZOOM_IN,
    },
    PaletteCommand {
        id: "find",
        label: "查找",
        keys: "Ctrl+Shift+F",
        icon: SEARCH_ICON,
    },
    PaletteCommand {
        id: "rename-tab",
        label: "重命名工作区",
        keys: "",
        icon: WORKSPACE_ICON,
    },
    PaletteCommand {
        id: "settings",
        label: "设置",
        keys: "Ctrl+,",
        icon: SETTINGS_ICON,
    },
    PaletteCommand {
        id: "theme",
        label: "切换明暗主题",
        keys: "Ctrl+Shift+L",
        icon: THEME_ICON,
    },
    PaletteCommand {
        id: "close-window",
        label: "关闭窗口",
        keys: "Alt+F4",
        icon: WINDOW_CLOSE,
    },
];

/// One row of the command palette after filtering: a command, or a pane to jump
/// to. Both answer "take me there", which is the only question the palette is
/// for.
///
/// A pane rather than a workspace, because a workspace is not somewhere you can
/// land: the keyboard goes to one of its panes, and a workspace row would leave
/// the choice of which one to the app. The row carries the workspace's name as
/// its hint, so the two levels are still readable.
#[derive(Clone, Copy)]
enum PaletteRow {
    Command(usize),
    Pane(usize, u64),
}

impl Workspace {
    /// Every row the palette can offer, in group order, filtered by `query`.
    ///
    /// Two groups, the way the prototype has them: what the window can do, and
    /// where the shells are. A pane is matched on its own name, its directory
    /// and its workspace's name, so all three ways of remembering it work.
    fn palette_rows(&self, query: &str) -> Vec<PaletteRow> {
        let query = query.trim().to_lowercase();
        let hit = |label: &str| query.is_empty() || label.to_lowercase().contains(&query);

        let mut rows = Vec::new();
        for (index, command) in PALETTE_COMMANDS.iter().enumerate() {
            if hit(command.label) {
                rows.push(PaletteRow::Command(index));
            }
        }
        for (index, tab) in self.tabs.iter().enumerate() {
            for (id, pane) in &tab.panes {
                let cwd = pane_cwd(pane).unwrap_or_default();
                if hit(&pane_label(pane)) || hit(&cwd) || hit(&tab.title) {
                    rows.push(PaletteRow::Pane(index, *id));
                }
            }
        }
        rows
    }

    /// What a row says, what it is grouped under, and the hint beside it.
    fn palette_label(&self, row: PaletteRow) -> (String, &'static str, String) {
        match row {
            PaletteRow::Command(index) => {
                let command = &PALETTE_COMMANDS[index];
                (command.label.to_owned(), "命令", command.keys.to_owned())
            }
            PaletteRow::Pane(tab, id) => {
                let Some(tab) = self.tabs.get(tab) else {
                    return (String::new(), "会话", String::new());
                };
                let Some(pane) = tab.panes.get(&id) else {
                    return (String::new(), "会话", String::new());
                };
                // The workspace, then the directory: the same two facts the
                // column shows, in the order a person scans them.
                let cwd = pane_cwd(pane).unwrap_or_default();
                let hint = if cwd.is_empty() {
                    tab.title.clone()
                } else {
                    format!("{} · {cwd}", tab.title)
                };
                (pane_label(pane), "会话", hint)
            }
        }
    }
}

/// Runs the row the palette is pointing at.
fn palette_icon(row: PaletteRow) -> &'static [u8] {
    match row {
        PaletteRow::Pane(..) => TERMINAL_ICON,
        PaletteRow::Command(index) => PALETTE_COMMANDS[index].icon,
    }
}

/// Which page of the settings sheet is open.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    Look,
    Terminal,
    Keys,
}

/// The shortcut sheet, as data: the settings page and the palette both read it,
/// and neither is allowed to invent a key the other does not have.
const KEYMAP: &[(&str, &str)] = &[
    ("Ctrl+Shift+T", "新建工作区"),
    ("Ctrl+Shift+Q", "关闭工作区"),
    ("Ctrl+Shift+W", "关闭窗格"),
    ("Ctrl+Shift+D", "向右分屏"),
    ("Ctrl+Shift+E", "向下分屏"),
    ("Ctrl+Shift+Z", "缩放窗格"),
    ("Ctrl+Shift+B", "收起 / 展开侧栏"),
    ("Ctrl+Shift+F", "查找"),
    ("Ctrl+K", "命令面板"),
    ("Ctrl+,", "设置"),
    ("Ctrl+Shift+L", "切换主题"),
    ("Ctrl+Tab", "切换工作区"),
    ("Ctrl+1..9", "直达工作区"),
    ("Alt+方向", "切换窗格"),
    ("Ctrl+=", "放大字号"),
    ("Ctrl+-", "缩小字号"),
];

/// A labelled block inside a settings page.
fn sgroup(label: &'static str, theme: &gpui_kit::component::Theme) -> AnyElement {
    div()
        .px(px(4.0))
        .pt(px(12.0))
        .pb(px(6.0))
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(label)
        .into_any_element()
}

/// What a close is about to end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseTarget {
    Window,
    /// The workspace's index, valid while the prompt is up.
    Workspace(usize),
    /// The workspace it lives in, and the pane.
    Pane(usize, u64),
}

/// A workspace that was just closed, kept for the undo toast.
struct ClosedTab {
    tab: Tab,
    index: usize,
    at: Instant,
}

/// The rename field and the workspace it will rename.
struct Rename {
    tab: usize,
    input: Entity<InputState>,
    _sub: Subscription,
}

/// The workspace: tabs, panes, and the transient state of a pointer gesture.
pub struct Workspace {
    tabs: Vec<Tab>,
    active_tab: usize,
    command: Command,
    /// Set while a poll tick is scheduled, so only one timer runs at a time.
    ticking: bool,
    /// The most recent failure, shown in place of the grid.
    error: Option<String>,
    /// Cell metrics from the last frame, kept so the IME can anchor its
    /// candidate window without measuring the font again.
    cached_style: GridStyle,
    /// The caret cell the IME was last told about, so a move can push a fresh
    /// position instead of waiting for the next composition to start.
    ime_cursor: (u16, u16),
    /// When the caret last moved. Blinking restarts from here, so the cursor is
    /// solid while typing instead of disappearing mid-keystroke.
    caret_moved: Instant,
    /// Whether the caret is in its visible half of the blink cycle.
    caret_visible: bool,
    /// Where a mouse drag started, as a pane id and a viewport cell. Present
    /// only while the button is held.
    drag_anchor: Option<(u64, (u16, u32))>,
    /// Whether the active pane currently has a selection, kept in step with the
    /// drag so copying does not have to scan every cell of the frame.
    selection: bool,
    /// A pane shown on its own, filling the window. The rest of the split stays
    /// as it was and comes back when the zoom is released.
    zoomed: Option<u64>,
    /// The result of the last update check, once it has one.
    ///
    /// `None` means "not checked yet". The check runs once shortly after start,
    /// on a background thread, so a slow network never delays the first frame.
    update: Option<Result<update::Release, String>>,
    /// Set while a download is in flight, so the button cannot be pressed twice.
    updating: bool,
    /// Download progress as bytes received and the total when known.
    update_progress: Option<(u64, Option<u64>)>,
    /// Progress reports from the background download, drained by the ticker.
    update_rx: Option<std::sync::mpsc::Receiver<(u64, Option<u64>)>>,
    /// Whether an install or uninstall is running.
    installing: bool,
    /// The last install or uninstall failure, shown in the about panel.
    install_error: Option<String>,
    /// Whether the startup reminder that this copy is not installed is on
    /// screen. Dismissing it hides it for the session; installing ends it
    /// everywhere, because the installed copy is what runs from then on.
    show_install_prompt: bool,
    /// How wide the list is. Width is the whole state: below the rail width the
    /// list draws as a strip of capsules, which is how it collapses without a
    /// second control to learn.
    sidebar_width: f32,
    /// The width to come back to when the rail opens. Kept apart from
    /// `sidebar_width` so collapsing does not forget the width the user chose.
    sidebar_expanded: f32,
    /// Present while the list's edge is being dragged.
    sidebar_drag: Option<SidebarDrag>,
    /// A tab being dragged up or down the sidebar.
    tab_drag: Option<TabDrag>,
    /// Set when a drag ended on another row, so the click that follows the
    /// release does not act on the row's old index.
    swallow_tab_click: bool,
    /// Whether Tab has been claimed from the focus system. Registered once.
    tab_claimed: bool,
    /// The open tab context menu, if any.
    menu: Option<TabMenu>,
    /// The find bar, while it is open.
    find: Option<FindState>,
    /// The command palette, while it is open.
    palette: Option<Palette>,
    /// The rename field, while it is open.
    rename: Option<Rename>,
    /// The app menu under the bar's `⋯`, while it is open.
    app_menu: bool,
    /// Whether the about card is on screen.
    about: bool,
    /// What a close would end, while the question is on screen. Nothing is torn
    /// down until it is answered.
    closing: Option<CloseTarget>,
    /// The last closed workspace, held for the six seconds the toast offers it
    /// back. Dropping it is what ends its shells.
    closed: Option<ClosedTab>,
    /// The preferences this window is running with, kept so the settings sheet
    /// can write them back without re-reading the file.
    config: config::Config,
    /// The branch each pane's directory is on, keyed by the directory.
    ///
    /// A map rather than one value because every pane header shows its own
    /// branch, and two panes in the same checkout must not cost two walks.
    /// Refreshed on the ticker rather than per frame: finding a branch means
    /// walking up the tree and reading a file.
    branches: BTreeMap<String, Option<String>>,
    /// When the map above was last rebuilt.
    branches_at: Option<Instant>,
    /// The last title handed to the platform, so the window is only renamed
    /// when the answer changes rather than every frame.
    window_title: String,
    /// Whether the settings sheet is open.
    settings: bool,
    /// Which page of it is showing.
    settings_tab: SettingsTab,
    /// The split being dragged, as the two panes on either side plus the span
    /// the divider travels across. Present only while the divider is held.
    resize_drag: Option<ResizeDrag>,
    focus: FocusHandle,
    /// Uploaded textures for the images the terminals have drawn.
    ///
    /// Held here rather than in the grid elements because those are rebuilt
    /// every frame, and a cache that lived one frame would re-upload every
    /// image on every repaint.
    images: Rc<RefCell<ImageCache>>,
}

impl Workspace {
    /// Builds an empty workspace. The caller installs the first tab, because its
    /// shell is spawned before the event loop exists.
    fn new(window: &mut Window, cx: &mut Context<Self>, config: config::Config) -> Self {
        let _ = window;

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            command: build_command(config.shell, config.shell_integration),
            ticking: false,
            error: None,
            cached_style: GridStyle::default(),
            ime_cursor: (0, 0),
            caret_moved: Instant::now(),
            caret_visible: true,
            drag_anchor: None,
            selection: false,
            zoomed: None,
            update: None,
            updating: false,
            update_progress: None,
            update_rx: None,
            installing: false,
            install_error: None,
            // One stat call at start-up. A copy at the install location means
            // the app has already been through this; a portable download has
            // not, and is the one case worth mentioning.
            show_install_prompt: !install::is_present(),
            // The rail is the default: a terminal should open with as little
            // chrome as it can get away with, and the list is one click away.
            sidebar_width: RAIL_WIDTH,
            sidebar_expanded: SIDEBAR_WIDTH,
            sidebar_drag: None,
            tab_drag: None,
            swallow_tab_click: false,
            tab_claimed: false,
            menu: None,
            find: None,
            palette: None,
            rename: None,
            app_menu: false,
            about: false,
            closing: None,
            closed: None,
            config,
            branches: BTreeMap::new(),
            branches_at: None,
            window_title: String::new(),
            settings: false,
            settings_tab: SettingsTab::default(),
            resize_drag: None,
            focus: cx.focus_handle(),
            images: Rc::new(RefCell::new(ImageCache::default())),
        }
    }

    /// Installs the pane spawned before start-up as the first tab.
    fn adopt(&mut self, pane: Pane) {
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        self.tabs = vec![Tab {
            title: "终端".to_owned(),
            auto_named: true,
            panes,
            layout: Layout::Leaf(1),
            active: 1,
            next_id: 2,
            unseen: false,
        }];
    }

    /// Starts polling the shells, and focuses the grid once it exists.
    fn ensure_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_ticking(cx);
        self.claim_tab_key(cx);
        // An installed copy that has updated itself still carries the version it
        // was installed at in the registry, because the updater only replaces
        // the executable. The running build knows the real number, so it fixes
        // the entry on the way up. A no-op unless the app is installed and the
        // stored version disagrees.
        install::sync_registry_version();

        // Once per run, and not on the first frame: the terminal should be
        // usable before anything asks the network a question.
        self.check_for_update(cx);

        if !self.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
    }

    /// Keeps Tab out of the focus system so the shell receives it.
    ///
    /// `gpui_base` binds Tab to focus traversal at the root, and that binding is
    /// dispatched before `capture_key_down` runs. In a terminal Tab is a
    /// character the shell has to receive — completion is built on it — so the
    /// keystroke has to be claimed before bindings resolve.
    ///
    /// The interceptor is the only hook early enough for that. It is registered
    /// once, and it declines the keystroke whenever the terminal is not the
    /// focused element, so ordinary focus traversal still works in any future
    /// dialog.
    fn claim_tab_key(&mut self, cx: &mut Context<Self>) {
        if self.tab_claimed {
            return;
        }
        self.tab_claimed = true;

        let focus = self.focus.clone();
        let entity = cx.entity();
        cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = &event.keystroke;
            if keystroke.key != "tab" || !focus.is_focused(window) {
                return;
            }
            // Shift+Tab is the shell's reverse completion, so it goes to the
            // pane as well. A control or alt chord is a shortcut, not a Tab.
            if keystroke.modifiers.control || keystroke.modifiers.alt {
                return;
            }

            let keystroke = keystroke.clone();
            entity.update(cx, |workspace, cx| {
                if let Some(input) = key_input(&keystroke)
                    && let Some(pane) = workspace.active_pane_mut()
                {
                    pane.session.send_key(input);
                }
                cx.stop_propagation();
                cx.notify();
            });
        })
        .detach();
    }

    /// Polls the terminals on a timer instead of on every paint.
    ///
    /// A terminal is idle most of the time, so the timer keeps the process quiet
    /// while still repainting as soon as a child writes something.
    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

                // `update` already holds the app borrow, so the poll result comes
                // out of the closure and the repaint is requested in a second,
                // separate update. Calling `notify` inside the first one re-enters
                // the app and drops the frame.
                //
                // The download reports progress on its own thread; taking only
                // the latest value per tick keeps the badge current without
                // repainting once per chunk.
                let changed = match this.update(cx, |workspace, _| {
                    let polled = workspace.poll();
                    let drained = workspace.drain_update_progress();
                    polled || drained
                }) {
                    Ok(changed) => changed,
                    Err(_) => break,
                };

                if changed && this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Advances the caret blink, returning true when it changed state.
    ///
    /// Only a blinking caret needs this. A steady one costs nothing, which is
    /// why the flag is read from the frame rather than assumed: shells and
    /// programs turn blinking on and off per pane.
    fn advance_caret_blink(&mut self) -> bool {
        const PERIOD: Duration = Duration::from_millis(530);

        let blinking = self
            .active_pane()
            .map(|pane| pane.session.frame().cursor)
            .is_some_and(|cursor| cursor.blinking && cursor.visible);

        if !blinking {
            if self.caret_visible {
                return false;
            }
            self.caret_visible = true;
            return true;
        }

        let elapsed = self.caret_moved.elapsed();
        let visible = (elapsed.as_millis() / PERIOD.as_millis()).is_multiple_of(2);
        if visible != self.caret_visible {
            self.caret_visible = visible;
            true
        } else {
            false
        }
    }

    /// Drains every pane. Returns true when a frame changed.
    fn poll(&mut self) -> bool {
        // A search over a lot of scrollback makes progress a little at a time;
        // this is the beat that lets it finish while the bar stays responsive.
        if self.find.is_some()
            && let Some(pane) = self.active_pane_mut()
        {
            let view = pane.session.find_view();
            let changed = self
                .find
                .as_ref()
                .is_some_and(|find| find.total != view.total || find.selected != view.selected);
            if changed {
                let current = view
                    .current
                    .and_then(|hit| view.ranges.iter().position(|range| *range == hit))
                    .unwrap_or(0);
                if let Some(find) = self.find.as_mut() {
                    find.matches = view.ranges;
                    find.total = view.total;
                    find.selected = view.selected;
                    find.current = current;
                }
                return true;
            }
        }

        // The undo offer expires here, which is also what ends the shells of the
        // workspace it was holding.
        if self
            .closed
            .as_ref()
            .is_some_and(|closed| closed.at.elapsed() > Duration::from_secs(6))
        {
            self.closed = None;
            return true;
        }

        // Each branch is a file read and a walk up the tree, so the map is
        // rebuilt on a slow beat rather than on every tick, and only for the
        // directories actually on screen.
        let directories: BTreeMap<String, ()> = self
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.values())
            .filter_map(pane_cwd)
            .map(|cwd| (cwd, ()))
            .collect();
        let stale = self
            .branches_at
            .is_none_or(|at| at.elapsed() > Duration::from_secs(2))
            || directories
                .keys()
                .any(|cwd| !self.branches.contains_key(cwd))
            || self
                .branches
                .keys()
                .any(|cwd| !directories.contains_key(cwd));
        if stale {
            let fresh: BTreeMap<String, Option<String>> = directories
                .into_keys()
                .map(|cwd| {
                    let branch = git_branch(&cwd);
                    (cwd, branch)
                })
                .collect();
            let changed = fresh != self.branches;
            self.branches = fresh;
            self.branches_at = Some(Instant::now());
            if changed {
                return true;
            }
        }

        let mut changed = self.advance_caret_blink();
        let active = self.active_tab;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            // Which tab is on screen decides whether activity is news. The tab
            // being read has its own answer in the header.
            let watched = index == active;

            for pane in tab.panes.values_mut() {
                if pane.session.poll() {
                    changed = true;
                }

                let working = pane.session.exit_code().is_none()
                    && pane.session.idle_for() < Duration::from_millis(1200);
                if working != pane.working {
                    // The edge that matters is run-to-stop, not "is busy":
                    // "the build finished while you were in another tab" is
                    // the thing a badge is for, and a shell that is merely
                    // busy would badge itself on every prompt.
                    if pane.working && !working && !watched && !tab.unseen {
                        tab.unseen = true;
                        changed = true;
                    }
                    pane.working = working;
                }
            }
            // A generated name follows the shell; a chosen one does not. A
            // workspace someone named `日志` should still say `日志` when a
            // build starts in one of its panes.
            if tab.auto_named
                && let Some(pane) = tab.active_pane()
                && let Some(title) = tab_title(pane)
                && title != tab.title
            {
                tab.title = title;
                changed = true;
            }
        }
        changed
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.active_tab()?.active_pane()
    }

    fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.active_tab_mut()?.active_pane_mut()
    }

    /// Sizes every pane to the grid its element reported it can hold.
    fn sync_grid_sizes(&mut self, window: &Window, cx: &App) {
        let style = self.grid_style(window, cx);
        let cell = (
            style.cell_width.as_f32().round() as u32,
            style.cell_height.as_f32().round() as u32,
        );
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                let dims = pane.dims.get();
                // The cell size is part of the check, not just the cell count.
                // A font or zoom change moves the same grid to a different pixel
                // size, and that is what the terminal reports to a program that
                // asks how big it is — a graphics program sizes its image and
                // its row count from that.
                if pane.grid != dims || pane.cell != cell {
                    pane.grid = dims;
                    pane.cell = cell;
                    pane.session.resize(dims.0, dims.1, cell.0, cell.1);
                }
            }
        }
    }

    /// Where the IME candidate window should point, in window coordinates.
    ///
    /// The grid reports where it was painted during prepaint; the caret cell is
    /// added to that. Returning pane-local coordinates would leave the candidate
    /// window pinned near the window's top-left corner instead of following the
    /// text being typed.
    fn ime_anchor(&self) -> Bounds<Pixels> {
        let Some(pane) = self.active_pane() else {
            return Bounds::new(point(px(0.0), px(0.0)), size(px(1.0), px(1.0)));
        };
        let cursor = pane.session.frame().cursor;
        let style = self.cached_style;
        let origin = pane.bounds.get().origin;

        let width = style.cell_width * f32::from(cursor.x);
        let height = style.cell_height * f32::from(cursor.y);

        Bounds::new(
            point(origin.x + width, origin.y + height),
            size(style.cell_width, style.cell_height),
        )
    }

    fn grid_style(&self, window: &Window, cx: &App) -> GridStyle {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let font_size = theme.mono_font_size;

        // A monospace cell is one character wide; measuring it once keeps every
        // later frame cheap and makes the grid independent of the font family.
        let metrics = window.text_system().layout_line(
            "M",
            font_size,
            &[TextRun {
                len: 1,
                font: Font {
                    family: theme.mono_font_family.clone(),
                    ..Default::default()
                },
                color: theme.foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );

        let cell_width = px(metrics.width.as_f32().ceil().max(1.0));
        let cell_height = px((metrics.ascent + metrics.descent).as_f32().ceil().max(1.0) * 1.25);
        let baseline = (cell_height - (metrics.ascent + metrics.descent)) * 0.5 + metrics.ascent;

        GridStyle {
            cell_width,
            cell_height,
            font_size,
            baseline,
            background: theme.background,
            cursor: theme.caret,
            selection: theme.selection,
            muted: theme.muted_foreground,
            selection_alpha: palette::Palette::current(cx).selection_alpha(),
            cursor_visible: true,
            cursor_shape: self.config.cursor.map(|shape| match shape {
                config::Cursor::Bar => vt::CursorStyle::Bar,
                config::Cursor::Block => vt::CursorStyle::Block,
                config::Cursor::Underline => vt::CursorStyle::Underline,
            }),
        }
    }

    /// Opens a new tab. The console is opened off the UI thread for the same
    /// reason a split is.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (cols, rows) = initial_grid(window);
        let command = self.command.clone();
        let entity = cx.entity();
        cx.spawn(async move |_, cx| {
            let pty = cx
                .background_executor()
                .spawn(async move { Pty::spawn(&command, cols, rows) })
                .await;

            entity.update(cx, |workspace, cx| {
                let pty = match pty {
                    Ok(pty) => pty,
                    Err(message) => {
                        workspace.error = Some(message);
                        cx.notify();
                        return;
                    }
                };
                let theme = gpui_kit::component::Theme::global(cx).clone();
                match Tab::from_pty(pty, &workspace.command, cols, rows, &theme) {
                    Ok(tab) => {
                        workspace.tabs.push(tab);
                        workspace.active_tab = workspace.tabs.len() - 1;
                        workspace.error = None;
                    }
                    Err(message) => workspace.error = Some(message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// What a close would end, waiting on an answer.
    ///
    /// Three units, one dialog: a pane, the workspace around it, and the
    /// window. The question is the same in all three cases — "this is still
    /// running, is that what you meant" — so it is asked once, in one card,
    /// with the noun changed.
    fn prompt_close(&mut self, target: CloseTarget, cx: &mut Context<Self>) {
        self.closing = Some(target);
        cx.notify();
    }

    /// Renames a workspace and stops following its shell.
    ///
    /// The name is trimmed and an empty one is refused: a row with no label is
    /// a row you cannot pick out of a list, which is the one thing the column
    /// exists to do.
    fn rename_tab(&mut self, index: usize, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.title = name.to_owned();
            tab.auto_named = false;
            cx.notify();
        }
    }

    /// Closes the workspace, asking first when something in it is still writing.
    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let running = tab
            .panes
            .values()
            .filter(|pane| matches!(pane_status(pane), PaneStatus::Working))
            .count();
        if running > 0 {
            self.prompt_close(CloseTarget::Workspace(index), cx);
            return;
        }
        self.do_close_tab(index, cx);
    }

    /// Removes the workspace and keeps it for a moment, in case it was a slip.
    fn do_close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        // The zoom names a pane id, and ids are only unique within a tab.
        self.zoomed = None;
        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if index < self.active_tab {
            // Closing a workspace to the left of the current one shifts it; the
            // one on screen should stay on screen.
            self.active_tab -= 1;
        }

        // The closed workspace is *moved*, not dropped: its shells keep running
        // for the six seconds the toast offers to undo. After that the sessions
        // go with it, because a terminal that keeps processes alive forever to
        // protect against a misclick is worse than the misclick.
        self.closed = Some(ClosedTab {
            tab,
            index,
            at: Instant::now(),
        });
        cx.notify();
    }

    /// Puts the last closed workspace back, with its shells still running.
    fn reopen_closed(&mut self, cx: &mut Context<Self>) {
        let Some(closed) = self.closed.take() else {
            return;
        };
        let index = closed.index.min(self.tabs.len());
        self.tabs.insert(index, closed.tab);
        self.active_tab = index;
        cx.notify();
    }

    /// Closes the focused pane, or the workspace when it is the last one.
    fn close_pane(&mut self, cx: &mut Context<Self>) {
        let last_in_tab = self
            .active_tab()
            .map(|tab| tab.panes.len() == 1)
            .unwrap_or(true);

        if last_in_tab {
            self.close_tab(self.active_tab, cx);
            return;
        }

        let Some(tab) = self.active_tab() else {
            return;
        };
        let id = tab.active;
        if tab
            .panes
            .get(&id)
            .is_some_and(|pane| matches!(pane_status(pane), PaneStatus::Working))
        {
            self.prompt_close(CloseTarget::Pane(self.active_tab, id), cx);
            return;
        }
        self.do_close_pane(self.active_tab, id, cx);
    }

    /// Removes one pane from a workspace.
    fn do_close_pane(&mut self, tab_index: usize, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(tab_index) else {
            return;
        };
        tab.active = id;
        tab.close_active();
        if self.zoomed == Some(id) {
            self.zoomed = None;
        }
        cx.notify();
    }

    /// Closes the window, asking first when more than one shell would go with it.
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shells: usize = self.tabs.iter().map(|tab| tab.panes.len()).sum();
        if shells > 1 {
            self.prompt_close(CloseTarget::Window, cx);
            return;
        }
        self.finish_window(window);
    }

    /// Switches to `index`, dropping state that belongs to the tab being left.
    ///
    /// Pane ids restart at 1 in every tab, so a zoom left behind would be read
    /// as an id in the new tab and cover the wrong pane.
    fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        // Looking at a tab is what clears its badge, even when it is already the
        // current one.
        if let Some(tab) = self.tabs.get_mut(index)
            && tab.unseen
        {
            tab.unseen = false;
        }
        if index == self.active_tab {
            cx.notify();
            return;
        }
        self.active_tab = index;
        self.zoomed = None;
        cx.notify();
    }

    /// Moves focus to `id`.
    fn focus_pane(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if !tab.panes.contains_key(&id) {
            return;
        }
        tab.active = id;

        // A zoomed pane covers the others, so focusing a different one has to
        // release the zoom: otherwise the sidebar highlights a pane the window is
        // not showing, and typing goes somewhere invisible.
        if self.zoomed.is_some_and(|zoomed| zoomed != id) {
            self.zoomed = None;
        }
        cx.notify();
    }

    /// Switches between the light and dark themes.
    ///
    /// The shells have to be repainted as well as the chrome: the terminal's
    /// default foreground, background and ANSI palette are its own copy, so a
    /// theme change that only reached the window would leave a black grid under a
    /// white title bar.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = match gpui_kit::component::Theme::global(cx).mode {
            gpui_kit::component::ThemeMode::Light => gpui_kit::component::ThemeMode::Dark,
            _ => gpui_kit::component::ThemeMode::Light,
        };
        self.apply_theme(next, cx);
    }

    /// Paints the app in `mode` and remembers the choice.
    ///
    /// Re-applying the terminal font after the palette matters: the library
    /// re-reads its theme file on every mode change, which drops the font this
    /// app set at start-up. Without this the grid silently falls back to the
    /// library's monospace the first time the theme is switched.
    fn apply_theme(&mut self, mode: gpui_kit::component::ThemeMode, cx: &mut Context<Self>) {
        self.config.light = Some(mode == gpui_kit::component::ThemeMode::Light);
        self.config.save();

        palette::install(cx, mode, self.config.accent);
        apply_terminal_font(cx, self.config.font_size);

        let theme = gpui_kit::component::Theme::global(cx).clone();
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                pane.retheme(&theme);
            }
        }
        cx.notify();
    }

    /// Turns the shell's directory reporting on or off.
    ///
    /// It only changes what the *next* pane starts with: a running shell cannot
    /// have its prompt rewritten from outside, and quietly restarting one to
    /// apply a setting would throw away whatever it was doing.
    fn set_shell_integration(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.shell_integration = on;
        self.config.save();
        self.command = build_command(self.config.shell, on);
        cx.notify();
    }

    /// Changes how much history every pane keeps, now and for new ones.
    fn set_scrollback(&mut self, lines: usize, cx: &mut Context<Self>) {
        let lines = lines.clamp(config::MIN_SCROLLBACK, config::MAX_SCROLLBACK);
        self.config.scrollback = lines;
        self.config.save();
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                pane.session.set_scrollback_limit(lines);
            }
        }
        cx.notify();
    }

    /// Repaints the window in another accent and remembers the choice.
    ///
    /// Only the accent moves: the mode stays whatever it was, which is what
    /// makes this a one-row decision rather than a second theme picker.
    fn set_accent(&mut self, accent: config::Accent, cx: &mut Context<Self>) {
        self.config.accent = accent;
        self.config.save();

        let mode = gpui_kit::component::Theme::global(cx).mode;
        palette::install(cx, mode, accent);
        let theme = gpui_kit::component::Theme::global(cx).clone();
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                pane.retheme(&theme);
            }
        }
        cx.notify();
    }

    /// Sets the terminal's font size, within the range the grid can lay out.
    fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = size.clamp(config::MIN_FONT_SIZE, config::MAX_FONT_SIZE);
        self.config.font_size = Some(size);
        self.config.save();
        apply_terminal_font(cx, Some(size));
        cx.notify();
    }

    /// Opens or closes the settings sheet.
    fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings = !self.settings;
        cx.notify();
    }

    fn finish_window(&mut self, window: &mut Window) {
        self.tabs.clear();
        self.closed = None;
        window.remove_window();
    }

    /// Answers the close prompt with "yes".
    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.closing.take() else {
            return;
        };
        match target {
            CloseTarget::Window => self.finish_window(window),
            CloseTarget::Workspace(index) => self.do_close_tab(index, cx),
            CloseTarget::Pane(tab, id) => {
                // Closing the last pane of a workspace is closing the workspace.
                let last = self.tabs.get(tab).is_some_and(|tab| tab.panes.len() == 1);
                if last {
                    self.do_close_tab(tab, cx);
                } else {
                    self.do_close_pane(tab, id, cx);
                }
            }
        }
        cx.notify();
    }

    fn handle_scroll(&mut self, event: &ScrollEvent, style: GridStyle, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => -delta.y,
            ScrollDelta::Pixels(delta) => -(delta.y.as_f32() / style.cell_height.as_f32().max(1.0)),
        };

        if lines == 0.0 {
            return;
        }

        let rows = if lines.abs() < 1.0 {
            lines.signum() as isize
        } else {
            lines.round() as isize
        };

        if let Some(pane) = self.active_pane_mut() {
            pane.session.scroll(rows);
        }
        cx.notify();
    }

    /// Sends a key to the focused terminal.
    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;

        // App shortcuts come first: they must not reach the shell.
        if keystroke.key == "escape" && self.menu.is_some() {
            self.menu = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }

        // While an overlay owns the keyboard, only the keys that mean something
        // to it are taken here; everything else falls through untouched so the
        // field receives it as text. Taking them all would make the find bar
        // unable to accept a single character.
        if self.find.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.close_find(window, cx);
                    cx.stop_propagation();
                }
                "enter" => {
                    // Enter walks the matches; Shift+Enter walks back.
                    self.step_find(if mods.shift { -1 } else { 1 }, cx);
                    cx.stop_propagation();
                }
                "f3" => {
                    self.step_find(if mods.shift { -1 } else { 1 }, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }

        if self.palette.is_some() {
            let query = self
                .palette
                .as_ref()
                .map(|palette| palette.query.clone())
                .unwrap_or_default();
            let rows = self.palette_rows(&query).len();
            match keystroke.key.as_str() {
                "escape" => {
                    self.palette = None;
                    window.focus(&self.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                "enter" => {
                    let selected = self.palette.as_ref().map_or(0, |palette| palette.selected);
                    self.run_command(selected, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "down" | "up" => {
                    if let Some(palette) = self.palette.as_mut()
                        && rows > 0
                    {
                        let step = if keystroke.key == "down" { 1 } else { rows - 1 };
                        palette.selected = (palette.selected + step) % rows;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                _ => return,
            }
        }

        if self.closing.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.closing = None;
                    cx.notify();
                    cx.stop_propagation();
                }
                "enter" => {
                    self.confirm_close(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }

        // The rename card owns the keyboard while it is up. Enter and Escape
        // mean what they mean in every rename field; everything else falls
        // through to the text field.
        if self.rename.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.rename = None;
                    window.focus(&self.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
                "enter" => {
                    if let Some(rename) = self.rename.as_ref() {
                        let index = rename.tab;
                        let name = rename.input.read(cx).value().to_string();
                        self.rename_tab(index, &name, cx);
                    }
                    self.rename = None;
                    window.focus(&self.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }

        // Escape is the universal "undo the last mode", and a zoom is a mode:
        // it covers the split, so the key that reveals what it covers is the one
        // the status bar already prints.
        if keystroke.key == "escape"
            && let Some(id) = self.zoomed
        {
            self.toggle_zoom(id, cx);
            cx.stop_propagation();
            return;
        }
        if keystroke.key == "escape" && self.settings {
            self.settings = false;
            cx.notify();
            cx.stop_propagation();
            return;
        }

        // Font size. `Ctrl+0` returns to the default rather than to whatever the
        // last size was, so the way back is one chord and always the same size.
        if mods.control && !mods.alt {
            let size = gpui_kit::component::Theme::global(cx)
                .mono_font_size
                .as_f32();
            match keystroke.key.as_str() {
                "=" | "+" => {
                    self.set_font_size(size + FONT_STEP, cx);
                    cx.stop_propagation();
                    return;
                }
                "-" => {
                    self.set_font_size(size - FONT_STEP, cx);
                    cx.stop_propagation();
                    return;
                }
                "0" => {
                    self.set_font_size(config::DEFAULT_FONT_SIZE, cx);
                    cx.stop_propagation();
                    return;
                }
                "," if !mods.shift => {
                    self.toggle_settings(cx);
                    cx.stop_propagation();
                    return;
                }
                "k" if !mods.shift => {
                    self.open_palette(window, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        if mods.control && keystroke.key == "tab" {
            let count = self.tabs.len();
            if count > 1 {
                let next = if mods.shift {
                    (self.active_tab + count - 1) % count
                } else {
                    (self.active_tab + 1) % count
                };
                self.select_tab(next, cx);
            }
            cx.stop_propagation();
            return;
        }

        // `Ctrl+1..9` jumps straight to a tab; nine means the last one, which is
        // what every tabbed desktop app does with the ninth slot. These are
        // outside the `Ctrl+Shift` namespace because a shell has no use for them
        // and the bare chords are quicker to reach.
        if mods.control
            && !mods.shift
            && !mods.alt
            && let Ok(number) = keystroke.key.parse::<usize>()
            && (1..=9).contains(&number)
            && !self.tabs.is_empty()
        {
            let index = if number == 9 {
                self.tabs.len() - 1
            } else {
                number - 1
            };
            if index < self.tabs.len() {
                self.select_tab(index, cx);
            }
            cx.stop_propagation();
            return;
        }

        if mods.control && mods.shift {
            match keystroke.key.as_str() {
                // Copy and paste are `Ctrl+Shift+C` / `Ctrl+Shift+V`, not the
                // bare chords. A terminal has to pass `Ctrl+C` and `Ctrl+V`
                // through: `Ctrl+C` interrupts the foreground program, and
                // `Ctrl+V` is quoted-insert in a readline shell. Taking them for
                // the clipboard breaks the shell to serve the app.
                "c" => {
                    self.copy_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                        && let Some(pane) = self.active_pane_mut()
                    {
                        pane.session.write(text.as_bytes());
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                "r" => {
                    self.reopen_closed(cx);
                    cx.stop_propagation();
                    return;
                }
                "t" => {
                    self.new_tab(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "w" => {
                    self.close_pane(cx);
                    cx.stop_propagation();
                    return;
                }
                "q" => {
                    // Quit is `Alt+F4` and the app menu. This pair is the
                    // window's two units of work — a pane and the workspace
                    // around it — and both stay under the same modifier.
                    self.close_tab(self.active_tab, cx);
                    cx.stop_propagation();
                    return;
                }
                "d" => {
                    self.split_active(Axis::Horizontal, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "e" => {
                    self.split_active(Axis::Vertical, window, cx);
                    cx.stop_propagation();
                    return;
                }
                "l" => {
                    self.toggle_theme(cx);
                    cx.stop_propagation();
                    return;
                }
                "f" => {
                    self.open_find(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "b" => {
                    self.toggle_rail(cx);
                    cx.stop_propagation();
                    return;
                }
                "z" => {
                    if let Some(id) = self.active_tab().map(|tab| tab.active) {
                        self.toggle_zoom(id, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }

        // Alt+arrow moves between panes, matching the direction the user sees.
        if mods.alt {
            let direction = match keystroke.key.as_str() {
                "left" => Some(Direction::Left),
                "right" => Some(Direction::Right),
                "up" => Some(Direction::Up),
                "down" => Some(Direction::Down),
                _ => None,
            };
            if let Some(direction) = direction {
                self.focus_neighbour(direction, cx);
                cx.stop_propagation();
                return;
            }
        }

        if let Some(input) = key_input(keystroke) {
            if let Some(pane) = self.active_pane_mut() {
                pane.session.send_key(input);
            }
            cx.notify();
            cx.stop_propagation();
        }
    }

    /// Starts dragging the divider between `left` and `right`.
    ///
    /// `span` is the extent the divider travels, measured by the caller from the
    /// union of both halves. Measuring between the two panes touching the seam
    /// would be wrong in a nested split, where those panes are only part of their
    /// halves.
    ///
    /// The gap itself is excluded from the share. Flex hands out the space left
    /// after the gaps, so a ratio computed over the gap included sits about ten
    /// pixels off — which is what made the divider trail the pointer instead of
    /// following it.
    fn begin_resize(
        &mut self,
        left: u64,
        right: u64,
        axis: Axis,
        position: Point<Pixels>,
        span: (Bounds<Pixels>, Bounds<Pixels>),
        cx: &mut Context<Self>,
    ) {
        let (first, second) = span;
        let gap = PANE_GAP;

        let (start, extent, origin) = match axis {
            Axis::Horizontal => (
                position.x.as_f32(),
                second.right().as_f32() - first.left().as_f32() - gap,
                first.left().as_f32(),
            ),
            Axis::Vertical => (
                position.y.as_f32(),
                second.bottom().as_f32() - first.top().as_f32() - gap,
                first.top().as_f32(),
            ),
        };
        if extent <= 0.0 {
            return;
        }

        // The pointer sits in the middle of the gap, so half of it belongs to each
        // side: subtracting it puts the divider under the pointer.
        let ratio = (start - origin - gap / 2.0) / extent;

        self.resize_drag = Some(ResizeDrag {
            left,
            right,
            axis,
            start,
            span: extent,
            ratio: ratio.clamp(0.05, 0.95),
        });
        cx.notify();
    }

    /// Moves the divider with the pointer.
    fn extend_resize(&mut self, axis: Axis, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.resize_drag else {
            return;
        };
        if drag.axis != axis {
            return;
        }

        let now = match axis {
            Axis::Horizontal => position.x.as_f32(),
            Axis::Vertical => position.y.as_f32(),
        };
        // A share of the whole span rather than an absolute offset, so the same
        // drag code works at any window size and inside a nested split.
        let ratio = (drag.ratio + (now - drag.start) / drag.span).clamp(0.05, 0.95);
        let (left, right) = (drag.left, drag.right);

        if let Some(tab) = self.active_tab_mut() {
            tab.layout.resize_between(left, right, ratio);
        }
        cx.notify();
    }

    /// Converts a pointer position to a viewport cell in `pane`.
    ///
    /// The pointer arrives in window coordinates and the grid records where it
    /// was painted, so subtracting the origin gives a position inside the grid.
    /// The result is clamped to the pane rather than rejected: dragging past the
    /// edge of the text is how you select to the end of a line.
    fn cell_at(&self, pane: &Pane, position: Point<Pixels>, style: GridStyle) -> (u16, u32) {
        let origin = pane.bounds.get().origin;
        let cols = pane.dims.get().0.max(1);
        let rows = pane.dims.get().1.max(1);

        let x = ((position.x - origin.x).as_f32() / style.cell_width.as_f32()).floor();
        let y = ((position.y - origin.y).as_f32() / style.cell_height.as_f32()).floor();

        (
            (x.max(0.0) as u16).min(cols.saturating_sub(1)),
            (y.max(0.0) as u32).min(u32::from(rows.saturating_sub(1))),
        )
    }

    /// Starts a selection at the pointer.
    fn begin_selection(
        &mut self,
        id: u64,
        position: Point<Pixels>,
        style: GridStyle,
        cx: &mut Context<Self>,
    ) {
        // The cell is resolved before taking a mutable borrow of the pane, so
        // the two borrows never overlap.
        let cell = {
            let Some(tab) = self.active_tab() else {
                return;
            };
            let Some(pane) = tab.panes.get(&id) else {
                return;
            };
            self.cell_at(pane, position, style)
        };

        self.drag_anchor = Some((id, cell));
        self.selection = false;

        if let Some(tab) = self.active_tab_mut() {
            tab.active = id;
            if let Some(pane) = tab.panes.get_mut(&id) {
                pane.session.clear_selection();
            }
        }
        cx.notify();
    }

    /// Extends the selection to the pointer while the button is held.
    fn extend_selection(
        &mut self,
        id: u64,
        position: Point<Pixels>,
        style: GridStyle,
        cx: &mut Context<Self>,
    ) {
        let Some((anchor_pane, anchor)) = self.drag_anchor else {
            return;
        };
        // A selection belongs to one pane; dragging into another would leave the
        // two ends in different grids, which the parser cannot represent.
        if anchor_pane != id {
            return;
        }

        let cell = {
            let Some(tab) = self.active_tab() else {
                return;
            };
            let Some(pane) = tab.panes.get(&id) else {
                return;
            };
            self.cell_at(pane, position, style)
        };

        if let Some(tab) = self.active_tab_mut()
            && let Some(pane) = tab.panes.get_mut(&id)
        {
            pane.session.set_selection(anchor, cell);
        }
        self.selection = anchor != cell;
        cx.notify();
    }

    /// Ends a drag.
    fn end_selection(&mut self) {
        self.drag_anchor = None;
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        // Only a real selection is copied. Copying the whole screen when nothing
        // is selected looks like a convenience and is not one: a terminal's
        // screen is mostly the prompt and output from commands the user has
        // already dealt with, and pasting that somewhere is rarely what they
        // meant. Selecting nothing and getting nothing is the honest answer.
        if !self.selection {
            return;
        }
        if let Some(text) = self
            .active_pane_mut()
            .and_then(|pane| pane.session.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Shows `id` on its own, or returns to the whole split if it already is.
    fn toggle_zoom(&mut self, id: u64, cx: &mut Context<Self>) {
        self.zoomed = if self.zoomed == Some(id) {
            None
        } else {
            Some(id)
        };
        self.focus_pane(id, cx);
    }

    /// Closes `id`, which is the pane the header's close button belongs to.
    fn close_pane_at(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_tab_mut() {
            tab.active = id;
        }
        // The zoom belongs to a pane, so it goes with it.
        if self.zoomed == Some(id) {
            self.zoomed = None;
        }
        self.close_pane(cx);
    }

    /// Splits `id`, focusing it first if it was not already focused.
    ///
    /// The pane overlay uses this so a split can be started from the pane it
    /// applies to instead of from the title bar, which is far away once the
    /// window is divided.
    fn split_pane(&mut self, id: u64, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(id, cx);
        self.split_active(axis, window, cx);
    }

    /// Splits the focused pane and moves focus into the new one.
    ///
    /// Starting a pseudo console takes tens of milliseconds, which is long
    /// enough for the frame timer to fire while the input handler still holds the
    /// app borrow. The work is therefore queued with `defer`, so it runs after
    /// the current event is fully dispatched.
    fn split_active(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let (cols, rows) = self.grid_dims(window, cx);

        // A split halves the pane, so the new shell starts at half the grid and
        // is corrected by the next prepaint rather than starting oversized.
        let cols = match axis {
            Axis::Horizontal => (cols / 2).max(2),
            Axis::Vertical => cols,
        };
        let rows = match axis {
            Axis::Vertical => (rows / 2).max(2),
            Axis::Horizontal => rows,
        };

        // Opening a pseudo console takes around 45 ms. Doing that on the UI thread
        // blocks the frame, and any frame requested during the wait re-enters the
        // app borrow the update holds. The console is therefore opened on a
        // background thread and only the cheap session wrapping happens here.
        let command = self.command.clone();
        let entity = cx.entity();
        cx.spawn(async move |_, cx| {
            let pty = cx
                .background_executor()
                .spawn(async move { Pty::spawn(&command, cols, rows) })
                .await;

            entity.update(cx, |workspace, cx| {
                let pty = match pty {
                    Ok(pty) => pty,
                    Err(message) => {
                        workspace.error = Some(message);
                        cx.notify();
                        return;
                    }
                };
                let theme = gpui_kit::component::Theme::global(cx).clone();
                let Some(tab) = workspace.active_tab_mut() else {
                    return;
                };
                match tab.split_with_pty(axis, pty, cols, rows, &theme) {
                    Ok(()) => workspace.error = None,
                    Err(message) => workspace.error = Some(message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Moves focus to the pane in `direction`.
    fn focus_neighbour(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let area = Rect {
            x: 0,
            y: 0,
            cols: tab
                .panes
                .values()
                .map(|pane| pane.grid.0)
                .max()
                .unwrap_or(80),
            rows: tab
                .panes
                .values()
                .map(|pane| pane.grid.1)
                .max()
                .unwrap_or(24),
        };

        if let Some(next) = tab.layout.neighbour(tab.active, direction, area) {
            tab.active = next;
            cx.notify();
        }
    }

    /// The cell count a freshly split pane should start with.
    fn grid_dims(&self, window: &Window, _cx: &App) -> (u16, u16) {
        self.active_pane()
            .map(|pane| pane.grid)
            .unwrap_or_else(|| initial_grid(window))
    }
}

/// Wires the terminal into the platform's text input stack.
///
/// A terminal has no text buffer to edit, so most of this trait has nothing to
/// report. What matters is `replace_text_in_range`: it is the call a Chinese,
/// Japanese or Korean IME makes when the user commits a composition, and it is
/// the only way composed text reaches the shell. Without it the IME opens, draws
/// its candidate window and then drops whatever was chosen.
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // A terminal has no document to select in, but the platform still asks
        // for the caret before it will place an IME candidate window: the
        // Windows backend reads this range, feeds it to `bounds_for_range` and
        // only then moves the window. Answering `None` here left the candidate
        // list pinned in one spot no matter where the prompt was.
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            return;
        }
        if let Some(pane) = self.active_pane_mut() {
            pane.session.write(text.as_bytes());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // The composition is still being edited. Nothing is sent until it is
        // committed, which arrives through `replace_text_in_range`.
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Anchors the candidate window to the cursor cell rather than the corner
        // of the screen.
        Some(self.ime_anchor())
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Before anything is painted, so the textures this frame asks for are
        // known before the ones it does not are dropped.
        self.images.borrow_mut().begin_frame();
        self.ensure_started(window, cx);
        self.sync_grid_sizes(window, cx);

        // The terminal is the only focusable thing in the window, so it takes the
        // keyboard back whenever something else (a tab click, a menu) stole it.
        if !self.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
        let style = self.grid_style(window, cx);

        // The platform only asks for the caret position when a composition
        // starts, which is too late to help: by then the candidate window is
        // already placed. Watching the caret here means a prompt that moves to a
        // new line pushes a fresh position before the user types anything.
        //
        // The same check restarts the blink, so the caret is always solid right
        // after a keystroke rather than vanishing halfway through one.
        let cursor = self
            .active_pane()
            .map(|pane| pane.session.frame().cursor)
            .map(|cursor| (cursor.x, cursor.y))
            .unwrap_or((0, 0));
        if cursor != self.ime_cursor {
            self.ime_cursor = cursor;
            self.caret_moved = Instant::now();
            self.caret_visible = true;
            window.invalidate_character_coordinates();
        }
        let style = GridStyle {
            cursor_visible: self.caret_visible,
            ..style
        };
        self.cached_style = style;

        let theme = gpui_kit::component::Theme::global(cx).clone();

        // Taskbar, Alt+Tab and the window's own thumbnail all read this one
        // string, and none of them can see the bar. The bar's name is the
        // readable copy; this is the one the operating system has.
        let title = match self.active_tab() {
            Some(tab) => {
                let pane = tab
                    .panes
                    .get(&tab.active)
                    .or_else(|| tab.panes.values().next());
                match pane.and_then(pane_cwd) {
                    Some(cwd) => format!("{} · {cwd}", tab.title),
                    None => tab.title.clone(),
                }
            }
            None => "myterm".to_owned(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        // The window is two columns, not two rows: the workspace column runs the
        // full height, so its own controls sit beside the top bar rather than
        // under a strip that spans the whole window. That is what lets the list
        // own the top-left corner — the corner it opens onto.
        let mut body = div()
            .size_full()
            .relative()
            .flex()
            .flex_row()
            // One chrome surface, painted here so it continues under the title
            // bar and the sidebar without a seam.
            //
            // This used to be left unpainted on the theory that Mica would show
            // through it. Mica is not available on this backend — the window is
            // an opaque DirectComposition surface — so the fill only ever hid
            // the window background, which is the same white the pane cards
            // paint. That left the cards floating on nothing, held apart by a
            // hairline that was doing all the work.
            //
            // Painting it the tone the bar and the sidebar already wear makes
            // the three regions one surface, and the cards separate from it by
            // tone alone. Three hairlines go away and none is missed.
            .bg(theme.title_bar)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::handle_key))
            // The install button dispatches rather than calling, so the about
            // card can be built by a plain function; the handler lands here.
            // A divider drag has to keep tracking once the pointer leaves the
            // few pixels of the grip, so the move and release are handled at the
            // root, which is under the pointer for the whole gesture.
            .on_mouse_move(cx.listener(|workspace, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if let Some(drag) = &workspace.resize_drag {
                    let axis = drag.axis;
                    workspace.extend_resize(axis, event.position, cx);
                }
                workspace.extend_sidebar_resize(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _, _| {
                    workspace.resize_drag = None;
                    workspace.sidebar_drag = None;
                }),
            );

        body = body.child(self.render_sidebar(&theme, cx));

        let mut content = div()
            .relative()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_top_bar(window, cx));

        // An uninstalled copy says so once, in a band that can be dismissed. It
        // is not a dialog: installing is an offer, and a terminal that blocks
        // its own grid until you accept one is not a terminal.
        if self.show_install_prompt {
            content = content.child(self.render_install_prompt(&theme, cx));
        }

        if let Some(error) = &self.error {
            // A failure is the one moment the grid is not there, so it gets the
            // same card treatment as a pane rather than a bare line of red text
            // floating in an empty window.
            content = content.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.0))
                    .bg(theme.title_bar)
                    .child(
                        div()
                            .max_w(px(460.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .p(px(20.0))
                            .rounded(px(12.0))
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.danger)
                                    .child("无法启动终端"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(error.clone()),
                            ),
                    ),
            );
            return body.child(content);
        }

        let Some(tab) = self.active_tab() else {
            content = content.child(self.render_empty(&theme, cx));
            return body.child(content);
        };

        // The pane area is chrome, not grid: the space between panes shows this
        // tone, so the panes read as cards laid on a surface rather than as
        // rectangles that happen to stop.
        //
        // The find bar sits above the cards and inside the same column, so
        // opening it shortens the grid instead of covering the output the
        // search is about.
        content = content.child(
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .bg(theme.title_bar)
                // The leading and top edges have no gutter: the cards meet the
                // top bar and the workspace column directly, so the chrome reads
                // as an L wrapped around the work instead of a frame floating
                // ten pixels away from it. Only the trailing and bottom edges
                // keep the gap, where the window edge would otherwise cut the
                // card.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .pr(px(PANE_GAP))
                        .pb(px(PANE_GAP))
                        .child(self.render_panes(tab, style, cx)),
                ),
        );

        // The status bar is not furniture. It appears only while it has a line
        // to say — a zoom to release, an update to take — and costs nothing the
        // rest of the time. A permanent 26 px strip repeating what the bar above
        // already says is a row of chrome bought for an occasional glance.
        if let Some(hint) = self.status_hint() {
            content = content.child(self.render_status_bar(&hint, &theme, cx));
        }

        // The settings sheet is a child of `content`, not of the window body:
        // hung from the body it would start at the window's top edge and cover
        // the title bar — including the controls that close it. Anchored here it
        // opens under the bar, beside the sidebar, over the work it is tuning.
        if self.settings {
            content = content.child(self.render_settings(&theme, cx));
        }

        body = body.child(content);

        if self.app_menu {
            body = body.child(self.render_app_menu(&theme, cx));
        }

        if self.about {
            body = body.child(self.render_about(&theme, cx));
        }

        if let Some(menu) = self.menu {
            let size = window.bounds().size;
            body = body.child(self.render_tab_menu(
                menu,
                Point {
                    x: size.width,
                    y: size.height,
                },
                &theme,
                cx,
            ));
        }

        if self.rename.is_some() {
            body = body.child(self.render_rename(&theme, cx));
        }

        if self.closed.is_some() {
            body = body.child(self.render_toast(&theme, cx));
        }

        if self.palette.is_some() {
            body = body.child(self.render_palette(&theme, cx));
        }

        if self.closing.is_some() {
            body = body.child(self.render_close_prompt(&theme, cx));
        }

        body
    }
}

impl Workspace {
    /// Lays out and draws every pane of the active tab.
    ///
    /// Each pane is a `flex` cell sized by the split ratio, so the layout engine
    /// does the tiling and the grid element only has to fill its own box. The
    /// active pane gets a one-pixel border, which is the only chrome a split
    /// needs to show where typing will go.
    fn render_panes(&self, tab: &Tab, style: GridStyle, cx: &mut Context<Self>) -> AnyElement {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let multi = tab.panes.len() > 1;
        let active = tab.active;

        // A zoomed pane covers the split rather than replacing it, so releasing
        // the zoom puts every pane back exactly where it was. Which pane that is
        // comes from the same place the sidebar reads it.
        let active = self.effective_active().map(|(_, id)| id).unwrap_or(active);
        if self.zoomed.is_some_and(|id| tab.panes.contains_key(&id)) {
            return self
                .render_node(&Layout::Leaf(active), tab, style, &theme, multi, active, cx)
                .into_any_element();
        }

        self.render_node(&tab.layout, tab, style, &theme, multi, active, cx)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &self,
        layout: &Layout,
        tab: &Tab,
        style: GridStyle,
        theme: &gpui_kit::component::Theme,
        multi: bool,
        active: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match layout {
            Layout::Leaf(id) => {
                let Some(pane) = tab.panes.get(id) else {
                    return div().into_any_element();
                };
                let focused = *id == active;
                let id = *id;
                let frame = pane.session.frame().clone();
                let dims = pane.dims.clone();
                let bounds_cell = pane.bounds.clone();
                let exit = pane.session.exit_code();
                let name = pane_label(pane);
                let meta = pane_meta(pane, &self.branches);
                // Only the pane the bar is over shows its matches. Tinting every
                // pane would answer a question the bar did not ask.
                let find = if focused {
                    self.find.as_ref().map(|find| grid_element::FindRanges {
                        rows: find.matches.clone(),
                        current: find.current,
                    })
                } else {
                    None
                };
                let zoomed = self.zoomed == Some(id);
                let show_header = multi || zoomed;
                // The same number the rail chip wears, on the pane itself: with
                // three panes open, "3" has to be findable in the work area and
                // not only in the column. Numbering follows the tab's key order,
                // which is the order the panes were opened.
                let ordinal = multi.then(|| {
                    tab.panes
                        .keys()
                        .position(|key| *key == id)
                        .map_or(0, |index| index + 1)
                });

                let mut card = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .group("pane")
                    .rounded(px(PANE_RADIUS))
                    // One hairline, always. The card and the chrome are close
                    // in tone on purpose, and tone alone cannot carry the edge
                    // at 1.2:1 — the line is what makes the gap read as a gap.
                    //
                    // Focus is a second step of the same neutral, not the
                    // accent: the accent means "this is current" in the column
                    // and in the bar, and a card outline in the same colour made
                    // three places claim it at once. The pane that is not
                    // holding the keyboard steps back by a sixteenth instead.
                    .bg(style.background)
                    // One pane fills the whole work area, so an edge around it
                    // would be drawing a box around the view itself. A split
                    // needs one: those cards sit next to each other and the gap
                    // between them is not enough on its own.
                    .when(multi, |this| {
                        this.border_1().border_color(if focused {
                            palette::Palette::current(cx).stroke_strong
                        } else {
                            theme.border
                        })
                    })
                    .when(multi && !focused, |this| this.opacity(0.84))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                            workspace.focus_pane(id, cx);
                            workspace.begin_selection(id, event.position, style, cx);
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(move |workspace, event: &MouseMoveEvent, _, cx| {
                            if event.pressed_button == Some(MouseButton::Left) {
                                workspace.extend_selection(id, event.position, style, cx);
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|workspace, _: &MouseUpEvent, _, _| {
                            workspace.end_selection();
                        }),
                    )
                    .on_scroll(cx.listener(move |workspace, event: &ScrollEvent, _, cx| {
                        workspace.handle_scroll(event, style, cx);
                    }));

                if show_header {
                    // While a divider is being dragged the header lends its
                    // trailing space to the numbers that are changing, because
                    // that is the question the drag is asking.
                    let live_size = self.resize_drag.is_some().then(|| dims.get());
                    card = card.child(pane_header(
                        id,
                        ordinal,
                        name,
                        meta,
                        if exit.is_some() {
                            PaneStatus::Exited
                        } else {
                            pane_status(pane)
                        },
                        focused,
                        zoomed,
                        live_size,
                        theme,
                        cx,
                    ));
                }

                if focused && self.find.is_some() {
                    card = card.child(self.render_find_bar(theme, cx));
                }

                // The terminal takes the rest of the card. Its padding is its
                // own: the header's name starts at 11 and the first cell at 13,
                // which is what keeps a line of output off the card's edge
                // without dragging the header out with it.
                card = card.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .px(px(GRID_PAD_X))
                        .pb(px(GRID_PAD_BOTTOM))
                        // With a header above it the grid keeps the header's own
                        // inset; without one the top padding is all it has.
                        .when(show_header, |this| this.pt(px(0.0)))
                        .when(!show_header, |this| this.pt(px(GRID_PAD_TOP)))
                        .child(TerminalGrid::new(
                            ElementId::Name(format!("grid-{id}").into()),
                            frame,
                            style,
                            focused,
                            dims,
                            bounds_cell,
                            cx.entity(),
                            self.focus.clone(),
                            self.images.clone(),
                            id,
                            find,
                        )),
                );

                card.when_some(exit, |this, code| {
                    this.child(
                        div()
                            .absolute()
                            .bottom(px(6.0))
                            .left(px(6.0))
                            .right(px(6.0))
                            .h(px(22.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .bg(theme.muted)
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("进程已退出（代码 {code}）")),
                    )
                })
                .into_any_element()
            }
            Layout::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let mut container = div().flex_1().min_w_0().min_h_0().flex();
                container = match axis {
                    Axis::Horizontal => container.flex_row(),
                    Axis::Vertical => container.flex_col(),
                };

                let first_el = self
                    .render_node(first, tab, style, theme, multi, active, cx)
                    .into_any_element();
                let second_el = self
                    .render_node(second, tab, style, theme, multi, active, cx)
                    .into_any_element();

                // The divider is a real flex child rather than a gap: a gap
                // cannot be grabbed, and splitting a window by eye is the one
                // adjustment a terminal user makes constantly. It stays one
                // pixel wide, but a wider transparent area around it gives the
                // pointer something to hit.
                let dragging = self.resize_drag.is_some();
                let divider = divider_element(*axis, first, second, tab, dragging, theme, cx);

                // The wrapper carries the basis, and it is itself a flex container so
                // the pane inside can fill it. Without that the pane has no box to
                // stretch into and collapses to nothing.
                // The basis is zero and the ratio is the *growth* factor, not a
                // width. Two 50% bases plus a ten-pixel divider add up to more
                // than the container, and the surplus is taken out of whatever
                // the split sits nearest — here, the ten-pixel gutter the window
                // keeps on its trailing and bottom edges. Dividing the space
                // that is actually left after the divider is the only arithmetic
                // that survives a split of a split.
                container
                    .child(
                        div()
                            .flex()
                            .flex_grow(ratio.clamp(0.05, 0.95))
                            .flex_shrink(1.0)
                            .flex_basis(relative(0.0))
                            .min_w_0()
                            .min_h_0()
                            .child(first_el),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex()
                            .flex_grow(1.0 - ratio.clamp(0.05, 0.95))
                            .flex_shrink(1.0)
                            .flex_basis(relative(0.0))
                            .min_w_0()
                            .min_h_0()
                            .child(second_el),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Workspace {
    /// Opens the tab menu for `tab` at the pointer.
    ///
    /// Right click does not select the row it lands on: the menu acts on that
    /// row, and silently switching the focused terminal because a menu was
    /// opened somewhere else would be a click with two meanings.
    fn open_tab_menu(&mut self, tab: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        if tab >= self.tabs.len() {
            return;
        }
        self.menu = Some(TabMenu { tab, position });
        cx.notify();
    }

    /// The tab menu, as one card over a click-away layer.
    fn render_tab_menu(
        &self,
        menu: TabMenu,
        max: Point<Pixels>,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        /// Menu width, also the amount kept inside the window on the right.
        const WIDTH: f32 = 196.0;
        /// Menu height for the clamp: three rows, a separator and padding.
        const HEIGHT: f32 = 116.0;

        let theme = theme.clone();
        let tab = menu.tab;
        let title = self
            .tabs
            .get(tab)
            .map(|tab| tab.title.clone())
            .unwrap_or_default();
        let closable = self.tabs.len() > 1;

        // A menu opened near an edge flips instead of being clipped by it.
        let x = menu.position.x.as_f32();
        let y = menu.position.y.as_f32();
        let left = if x + WIDTH > max.x.as_f32() {
            (x - WIDTH).max(0.0)
        } else {
            x
        };
        let top = if y + HEIGHT > max.y.as_f32() {
            (y - HEIGHT).max(0.0)
        } else {
            y
        };

        let close = menu_row("menu-close", "关闭工作区", closable, &theme).on_click(cx.listener(
            move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                workspace.menu = None;
                workspace.close_tab(tab, cx);
            },
        ));
        let close_others = menu_row(
            "menu-close-others",
            "关闭其他工作区",
            self.tabs.len() > 1,
            &theme,
        )
        .on_click(
            cx.listener(move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                workspace.menu = None;
                workspace.close_others(tab, cx);
            }),
        );
        let rename = menu_row("menu-rename", "重命名工作区", true, &theme).on_click(cx.listener(
            move |workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                workspace.menu = None;
                workspace.open_rename(tab, window, cx);
            },
        ));
        let copy = menu_row("menu-copy", "复制工作区标题", true, &theme).on_click(cx.listener(
            move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(title.clone()));
                workspace.menu = None;
                cx.notify();
            },
        ));

        div()
            .absolute()
            .inset_0()
            // Clicking anywhere else dismisses. The card below stops its own
            // presses, so a menu row is never mistaken for the dismissal.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace: &mut Workspace, _: &MouseDownEvent, _, cx| {
                    workspace.menu = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|workspace: &mut Workspace, _: &MouseDownEvent, _, cx| {
                    workspace.menu = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(WIDTH))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .p(px(4.0))
                    .rounded(px(10.0))
                    .bg(theme.popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(rename)
                    .child(copy)
                    .child(div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(theme.border))
                    .child(close)
                    .child(close_others),
            )
            .into_any_element()
    }

    /// The find bar: a field, a count, and a way out.
    /// Opens the rename field for a workspace.
    fn open_rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("工作区名字")
                .default_value(tab.title.clone())
        });
        let sub = cx.subscribe(&input, |_: &mut Workspace, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        input.update(cx, |state, cx| state.focus(window, cx));

        self.rename = Some(Rename {
            tab: index,
            input,
            _sub: sub,
        });
        cx.notify();
    }

    /// The rename card: one field, one button, and Enter.
    fn render_rename(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(rename) = self.rename.as_ref() else {
            return div().into_any_element();
        };
        let index = rename.tab;
        let input = rename.input.clone();
        let muted = theme.muted_foreground;

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseDownEvent, _, cx| {
                    workspace.rename = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(320.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(18.0))
                    .rounded(px(14.0))
                    .bg(theme.popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("重命名工作区"),
                    )
                    .child(Input::new(&input).small())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("rename-cancel")
                                    .flex()
                                    .items_center()
                                    .h(px(28.0))
                                    .px(px(14.0))
                                    .rounded(px(7.0))
                                    .bg(theme.muted)
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(muted)
                                    .hover(move |this| this.bg(theme.secondary_hover))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                                            workspace.rename = None;
                                            window.focus(&workspace.focus, cx);
                                            cx.notify();
                                        },
                                    ))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("rename-save")
                                    .flex()
                                    .items_center()
                                    .h(px(28.0))
                                    .px(px(14.0))
                                    .rounded(px(7.0))
                                    .bg(theme.ring.opacity(0.13))
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(theme.ring)
                                    .hover(move |this| this.bg(theme.ring.opacity(0.2)))
                                    .on_click(cx.listener(
                                        move |workspace: &mut Workspace,
                                              _: &ClickEvent,
                                              window,
                                              cx| {
                                            let name = workspace
                                                .rename
                                                .as_ref()
                                                .map(|rename| {
                                                    rename.input.read(cx).value().to_string()
                                                })
                                                .unwrap_or_default();
                                            workspace.rename_tab(index, &name, cx);
                                            workspace.rename = None;
                                            window.focus(&workspace.focus, cx);
                                            cx.notify();
                                        },
                                    ))
                                    .child("保存"),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The find strip, inside the pane it searches.
    ///
    /// It takes its own row above the grid rather than floating over the
    /// output: the matches are in the output, and a bar laid on top of them
    /// hides the answer it is taking the user to. Being in the card also says
    /// which pane is being searched without a label spelling it out.
    fn render_find_bar(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(find) = self.find.as_ref() else {
            return div().into_any_element();
        };

        let total = find.total;
        let counter = if total == 0 {
            "0/0".to_owned()
        } else {
            format!("{}/{}", find.selected.map_or(0, |index| index + 1), total)
        };
        let input = find.input.clone();
        let wash = theme.muted;

        let step =
            |id: &'static str, glyph: &'static [u8], delta: isize, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(move |this| this.bg(wash))
                    .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        workspace.step_find(delta, cx);
                    }))
                    .child(icon_data(glyph, 12.0, theme.muted_foreground))
            };

        div()
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(34.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(div().flex_1().min_w_0().child(Input::new(&input).small()))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(if total == 0 {
                        theme.muted_foreground
                    } else {
                        theme.foreground
                    })
                    .child(counter),
            )
            .child(step("find-prev", ARROW_UP, -1, cx))
            .child(step("find-next", ARROW_DOWN, 1, cx))
            .child(pane_action(
                ("find-close", 0u64),
                WINDOW_CLOSE,
                theme,
                false,
                cx.listener(|workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                    workspace.close_find(window, cx);
                }),
            ))
            .into_any_element()
    }

    /// The command palette.
    ///
    /// Every command here already has a key; the palette exists so the keys can
    /// be discovered, not so commands can have a second home. That is why each
    /// row prints its shortcut instead of hiding it.
    /// The command palette.
    ///
    /// Every command here already has a key; the palette exists so the keys can
    /// be discovered, not so commands can have a second home. That is why each
    /// row prints its shortcut instead of hiding it — and why the rows are
    /// grouped: a flat list of ten answers says nothing about what kind of
    /// answer each one is.
    fn render_palette(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(palette) = self.palette.as_ref() else {
            return div().into_any_element();
        };

        let rows = self.palette_rows(&palette.query);
        let selected = palette.selected.min(rows.len().saturating_sub(1));
        let input = palette.input.clone();
        let wash = theme.muted;
        let accent = theme.ring;

        let mut list = div().flex().flex_col().p(px(6.0));
        let mut current_group = "";
        if rows.is_empty() {
            list = list.child(
                div()
                    .px(px(10.0))
                    .py(px(10.0))
                    .text_size(px(12.0))
                    .text_color(theme.muted_foreground)
                    .child("没有匹配的命令"),
            );
        }
        for (index, row) in rows.iter().enumerate() {
            let (label, group, hint) = self.palette_label(*row);
            if group != current_group {
                current_group = group;
                list = list.child(
                    div()
                        .px(px(10.0))
                        .pt(px(8.0))
                        .pb(px(4.0))
                        .text_size(px(9.5))
                        .text_color(theme.muted_foreground)
                        .child(group),
                );
            }

            let is_selected = index == selected;
            let keys = hint;
            list = list.child(
                div()
                    .id(ElementId::Name(format!("palette-{index}").into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(34.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(is_selected, |this| this.bg(accent.opacity(0.13)))
                    .when(!is_selected, |this| this.hover(move |this| this.bg(wash)))
                    .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.run_command(index, window, cx);
                    }))
                    .child(
                        div()
                            .w(px(16.0))
                            .flex_shrink_0()
                            .flex()
                            .justify_center()
                            .child(icon_data(
                                palette_icon(*row),
                                13.0,
                                if is_selected {
                                    accent
                                } else {
                                    theme.muted_foreground
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.0))
                            .text_color(if is_selected {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .child(label),
                    )
                    .when(!keys.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.0))
                                .text_color(theme.muted_foreground)
                                .child(keys),
                        )
                    }),
            );
        }

        let hint = |text: &'static str| {
            div()
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(text)
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(96.0))
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace: &mut Workspace, _: &MouseDownEvent, _, cx| {
                    workspace.palette = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(580.0))
                    .flex()
                    .flex_col()
                    .rounded(px(14.0))
                    .bg(theme.popover)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .h(px(46.0))
                            .flex()
                            .items_center()
                            .px(px(16.0))
                            .border_b_1()
                            .border_color(theme.border)
                            .child(Input::new(&input).appearance(false).bordered(false).small()),
                    )
                    .child(
                        div()
                            .id("palette-list")
                            .max_h(px(340.0))
                            .overflow_y_scroll()
                            .child(list),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(14.0))
                            .px(px(14.0))
                            .py(px(8.0))
                            .border_t_1()
                            .border_color(theme.border)
                            .child(hint("↑↓ 选择"))
                            .child(hint("Enter 执行"))
                            .child(hint("Esc 关闭")),
                    ),
            )
            .into_any_element()
    }

    /// The settings sheet.
    ///
    /// One surface, not two. An earlier version pinned the sheet to the window
    /// and then drew a second card inside it — 430px wide in a 360px drawer,
    /// with its own padding, border, fill and shadow. The sheet is the surface
    /// now: its own edge, its own inset, nothing nested.
    ///
    /// It hangs under the top bar (`render` mounts it inside the content
    /// region), so it never covers the bar or the controls that close it, and it
    /// keeps only a 12% scrim: every row takes effect immediately, and half the
    /// value of a font-size setting is watching the terminal behind it change
    /// size.
    fn render_settings(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.muted_foreground;
        let size = theme.mono_font_size.as_f32();
        let light = self.config.light;
        let wash = theme.muted;
        let accent = theme.ring;
        let id = |name: &str| ElementId::Name(name.to_owned().into());

        let tab = |id: &'static str, label: &'static str, active: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(if active { accent } else { muted })
                .when(active, |this| this.bg(theme.ring.opacity(0.13)))
                .when(!active, |this| {
                    this.hover(move |this| this.bg(wash).text_color(theme.muted_foreground))
                })
                .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                    workspace.settings_tab = match id {
                        "sh-tab-term" => SettingsTab::Terminal,
                        "sh-tab-keys" => SettingsTab::Keys,
                        _ => SettingsTab::Look,
                    };
                    cx.notify();
                }))
                .child(label)
        };

        let seg = |id: ElementId, label: &'static str, active: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(24.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(11.0))
                .text_color(if active { theme.foreground } else { muted })
                .when(active, |this| this.bg(theme.popover))
                .when(!active, |this| {
                    this.hover(move |this| this.text_color(theme.muted_foreground))
                })
                .child(label)
        };

        let track = || {
            div()
                .flex()
                .flex_row()
                .gap(px(2.0))
                .p(px(2.0))
                .rounded(px(8.0))
                .bg(theme.background)
        };

        // Each tab body is built here and the sheet picks one, so the sheet's
        // own structure stays a header, a body and a footer whatever the tab.
        let body = match self.settings_tab {
            SettingsTab::Look => {
                let theme_row = track()
                    .child(
                        seg(id("sh-theme-system"), "跟随", light.is_none()).on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.light = None;
                                workspace.config.save();
                                // Back to the desktop's own answer, resolved
                                // rather than remembered: the setting may have
                                // changed since start-up.
                                let mode = if system_prefers_light() {
                                    gpui_kit::component::ThemeMode::Light
                                } else {
                                    gpui_kit::component::ThemeMode::Dark
                                };
                                workspace.apply_theme(mode, cx);
                            },
                        )),
                    )
                    .child(
                        seg(id("sh-theme-light"), "浅色", light == Some(true)).on_click(
                            cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.apply_theme(gpui_kit::component::ThemeMode::Light, cx);
                            }),
                        ),
                    )
                    .child(
                        seg(id("sh-theme-dark"), "深色", light == Some(false)).on_click(
                            cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.apply_theme(gpui_kit::component::ThemeMode::Dark, cx);
                            }),
                        ),
                    );

                let density_row = track()
                    .child(
                        seg(
                            id("sh-density-compact"),
                            "紧凑",
                            self.config.density == config::Density::Compact,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.density = config::Density::Compact;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        seg(
                            id("sh-density-standard"),
                            "标准",
                            self.config.density == config::Density::Standard,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.density = config::Density::Standard;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        seg(
                            id("sh-density-comfy"),
                            "舒适",
                            self.config.density == config::Density::Comfy,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.density = config::Density::Comfy;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    );

                let mut swatches = div().flex().flex_row().items_center().gap(px(8.0));
                for option in config::Accent::ALL {
                    let active = self.config.accent == option;
                    let (ink, _) = palette::accent_ink(option, theme.mode);
                    swatches = swatches.child(
                        div()
                            .id(ElementId::Name(
                                format!("sh-accent-{}", option.as_str()).into(),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(26.0))
                            .rounded(px(999.0))
                            // A swatch is a colour chip, so it keeps a ring:
                            // it is the only control whose job is to show a
                            // colour, and the ring is what says which one is
                            // chosen.
                            .when(active, |this| this.border_1().border_color(ink))
                            .cursor_pointer()
                            .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                                workspace.set_accent(option, cx);
                            }))
                            .child(div().size(px(16.0)).rounded(px(999.0)).bg(ink)),
                    );
                }

                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("外观", theme))
                    .child(setting_row(
                        "主题",
                        "跟随系统时使用 Windows 的深浅设置",
                        theme,
                        theme_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "密度",
                        "侧栏的行高，紧凑时隐藏路径行",
                        theme,
                        density_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "强调色",
                        "只用于「当前」和「有动静」，不用于装饰",
                        theme,
                        swatches.into_any_element(),
                    ))
            }
            SettingsTab::Terminal => {
                let mut shells = track();
                for option in [
                    config::Shell::Auto,
                    config::Shell::Pwsh,
                    config::Shell::Windows,
                    config::Shell::Cmd,
                    config::Shell::Wsl,
                ] {
                    let label = match option {
                        config::Shell::Auto => "自动",
                        config::Shell::Pwsh => "pwsh",
                        config::Shell::Windows => "Windows PowerShell",
                        config::Shell::Cmd => "cmd",
                        config::Shell::Wsl => "wsl",
                    };
                    shells = shells.child(
                        seg(
                            ElementId::Name(format!("sh-shell-{}", option.as_str()).into()),
                            label,
                            self.config.shell == option,
                        )
                        .on_click(cx.listener(
                            move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.shell = option;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    );
                }

                let scrollback_row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("sh-scrollback-less")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_scrollback(
                                        workspace.config.scrollback.saturating_sub(SCROLLBACK_STEP),
                                        cx,
                                    );
                                },
                            ))
                            .child(icon_data(MINUS_ICON, 12.0, muted)),
                    )
                    .child(
                        div()
                            .w(px(64.0))
                            .text_center()
                            .text_size(px(11.5))
                            .text_color(theme.foreground)
                            .child(format!("{} 行", self.config.scrollback)),
                    )
                    .child(
                        div()
                            .id("sh-scrollback-more")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_scrollback(
                                        workspace.config.scrollback.saturating_add(SCROLLBACK_STEP),
                                        cx,
                                    );
                                },
                            ))
                            .child(icon_data(PLUS_SMALL, 12.0, muted)),
                    );

                let integration_row = track()
                    .child(
                        seg(
                            id("sh-integration-off"),
                            "关闭",
                            !self.config.shell_integration,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.set_shell_integration(false, cx);
                            },
                        )),
                    )
                    .child(
                        seg(
                            id("sh-integration-on"),
                            "启用",
                            self.config.shell_integration,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.set_shell_integration(true, cx);
                            },
                        )),
                    );

                let mut cursor_row = track();
                for option in [
                    None,
                    Some(config::Cursor::Bar),
                    Some(config::Cursor::Block),
                    Some(config::Cursor::Underline),
                ] {
                    let label = match option {
                        None => "跟随程序",
                        Some(config::Cursor::Bar) => "竖线",
                        Some(config::Cursor::Block) => "块",
                        Some(config::Cursor::Underline) => "下划线",
                    };
                    let id = match option {
                        None => "sh-cursor-auto".to_owned(),
                        Some(cursor) => format!("sh-cursor-{}", cursor.as_str()),
                    };
                    cursor_row = cursor_row.child(
                        seg(
                            ElementId::Name(id.into()),
                            label,
                            self.config.cursor == option,
                        )
                        .on_click(cx.listener(
                            move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.cursor = option;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    );
                }

                let font = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("sh-font-smaller")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(size - FONT_STEP, cx);
                                },
                            ))
                            .child(icon_data(MINUS_ICON, 12.0, muted)),
                    )
                    .child(
                        div()
                            .w(px(56.0))
                            .text_center()
                            .text_size(px(11.5))
                            .text_color(theme.foreground)
                            .child(format!("{size} px")),
                    )
                    .child(
                        div()
                            .id("sh-font-larger")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(size + FONT_STEP, cx);
                                },
                            ))
                            .child(icon_data(PLUS_SMALL, 12.0, muted)),
                    )
                    .child(
                        div()
                            .id("sh-font-reset")
                            .h(px(24.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_size(px(11.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(config::DEFAULT_FONT_SIZE, cx);
                                },
                            ))
                            .child("重置"),
                    );

                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("终端", theme))
                    .child(setting_row(
                        "默认 shell",
                        "新建窗格时启动的程序",
                        theme,
                        shells.into_any_element(),
                    ))
                    .child(setting_row(
                        "字号",
                        "只影响网格，不影响界面",
                        theme,
                        font.into_any_element(),
                    ))
                    .child(setting_row(
                        "光标",
                        "块状时会反色重绘它下面的字符",
                        theme,
                        cursor_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "回滚行数",
                        "超出后丢弃最旧的输出，搜索也只看这么多",
                        theme,
                        scrollback_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "shell 集成",
                        "让 shell 报告自己的目录，路径与分支才有内容；只对 PowerShell 生效，会改写它的提示符函数，新窗格开始使用",
                        theme,
                        integration_row.into_any_element(),
                    ))
            }
            SettingsTab::Keys => {
                let mut list = div().flex().flex_col();
                for (keys, label) in KEYMAP {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .h(px(32.0))
                            .border_t_1()
                            .border_color(theme.border)
                            .text_size(px(11.5))
                            .text_color(muted)
                            .child(*label)
                            .child(div().ml_auto().flex().flex_row().gap(px(3.0)).children(
                                keys.split('+').map(|part| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .h(px(16.0))
                                        .px(px(4.0))
                                        .rounded(px(4.0))
                                        .bg(theme.muted)
                                        .text_size(px(9.5))
                                        .text_color(muted)
                                        .child(part.to_owned())
                                }),
                            )),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("快捷键", theme))
                    .child(list)
            }
        };

        div()
            .absolute()
            // Under the bar, not over it: the sheet is a workspace tool, and
            // covering the window's own controls with it would make closing the
            // window depend on closing settings first.
            .top(px(TITLE_BAR_HEIGHT))
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .flex_row()
            .justify_end()
            // The scrim is a sheet's worth of dimming, not a dialog's: the point
            // of a live preview is to watch the window behind it.
            .child(
                div()
                    .id("settings-scrim")
                    .absolute()
                    .inset_0()
                    .bg(theme.overlay.opacity(0.32))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|workspace: &mut Workspace, _: &MouseDownEvent, _, cx| {
                            workspace.settings = false;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w(px(436.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(theme.title_bar)
                    .border_l_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    // A press inside the sheet is the sheet's. Without this the
                    // click falls through to the pane underneath and puts the
                    // caret there, which is a strange thing to have happen while
                    // you are changing a setting.
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .flex_shrink_0()
                            .h(px(52.0))
                            .pl(px(18.0))
                            .pr(px(10.0))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child("设置"),
                            )
                            .child(pane_action(
                                ("settings-close", 0u64),
                                WINDOW_CLOSE,
                                theme,
                                false,
                                cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.settings = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(2.0))
                            .px(px(14.0))
                            .pb(px(10.0))
                            .child(tab(
                                "sh-tab-look",
                                "外观",
                                self.settings_tab == SettingsTab::Look,
                                cx,
                            ))
                            .child(tab(
                                "sh-tab-term",
                                "终端",
                                self.settings_tab == SettingsTab::Terminal,
                                cx,
                            ))
                            .child(tab(
                                "sh-tab-keys",
                                "快捷键",
                                self.settings_tab == SettingsTab::Keys,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(14.0))
                            .pb(px(14.0))
                            .child(body),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .flex_shrink_0()
                            .px(px(16.0))
                            .py(px(12.0))
                            .child(
                                div()
                                    .id("settings-about")
                                    .cursor_pointer()
                                    .text_size(px(11.0))
                                    .text_color(muted)
                                    .hover(move |this| this.text_color(accent))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                            workspace.settings = false;
                                            workspace.about = true;
                                            cx.notify();
                                        },
                                    ))
                                    .child("关于 myterm"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(muted)
                                    .child("更改立即生效"),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The shell a new pane would start, for the settings row.
    fn shell_label(&self) -> String {
        let program = &self.command.program;
        std::path::Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(program)
            .trim_end_matches(".exe")
            .to_owned()
    }

    /// The about card, as a centred sheet over a scrim.
    fn render_about(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.muted_foreground;
        let accent = theme.ring;
        let text = theme.foreground;
        let install = self.install_state();
        let update = self.update_badge();

        let row = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(34.0))
                .text_size(px(11.5))
                .text_color(muted)
                .child(label)
                .child(div().ml_auto().text_color(text).child(value))
        };

        let ghost = |id: &'static str, label: String| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .bg(theme.muted)
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(muted)
                .hover(move |this| this.bg(theme.secondary_hover).text_color(theme.foreground))
                .child(label)
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseDownEvent, _, cx| {
                    workspace.about = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(400.0))
                    .p(px(20.0))
                    .flex()
                    .flex_col()
                    .rounded(px(16.0))
                    .bg(theme.popover)
                    // Fill and shadow, no outline: every floating surface in
                    // the window is separated that way now.
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(38.0))
                                    .rounded(px(11.0))
                                    .bg(accent.opacity(0.13))
                                    .child(icon_data(APP_MARK, 20.0, accent)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(text)
                                            .child("myterm"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(muted)
                                            .child(format!("{} · Windows", version_label())),
                                    ),
                            )
                            .child(pane_action(
                                ("about-close", 0u64),
                                WINDOW_CLOSE,
                                theme,
                                false,
                                cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.about = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .mt(px(18.0))
                            .child(row("终端解析", ghostty_label().to_owned()))
                            .child(row("界面框架", "GPUI Kit 0.9".to_owned()))
                            .child(row("默认 shell", self.shell_label()))
                            .child(row("安装状态", install.summary().to_owned()))
                            .child(row(
                                "更新",
                                match &update {
                                    Some((label, _)) => label.clone(),
                                    None => "尚未检查".to_owned(),
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .mt(px(16.0))
                            .child(ghost("about-check", "检查更新".to_owned()).on_click(
                                cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.update = None;
                                    workspace.check_for_update(cx);
                                }),
                            ))
                            .child(div().w(px(96.0)).child(install_action(
                                theme,
                                install,
                                cx.entity(),
                            )))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("about-ok")
                                    .flex()
                                    .items_center()
                                    .h(px(28.0))
                                    .px(px(16.0))
                                    .rounded(px(7.0))
                                    .bg(accent.opacity(0.13))
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(accent)
                                    .hover(move |this| this.bg(accent.opacity(0.2)))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                            workspace.about = false;
                                            cx.notify();
                                        },
                                    ))
                                    .child("好"),
                            ),
                    )
                    // Two pointers out of the card: the project, and the file
                    // the settings live in. The sheet's footer lost its copy of
                    // the second one — this card is already the place that
                    // answers "where is this program".
                    .child(
                        div()
                            .mt(px(14.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(14.0))
                            .child(
                                div()
                                    .id("about-link")
                                    .text_size(px(10.0))
                                    .text_color(muted)
                                    .cursor_pointer()
                                    .hover(move |this| this.text_color(accent))
                                    .on_click(|_: &ClickEvent, _, cx| {
                                        cx.open_url("https://github.com/tiramission/myterm");
                                    })
                                    .child("github.com/tiramission/myterm"),
                            )
                            .child(
                                div()
                                    .id("about-config")
                                    .text_size(px(10.0))
                                    .font_family(TERMINAL_FONT)
                                    .text_color(muted)
                                    .cursor_pointer()
                                    .hover(move |this| this.text_color(accent))
                                    .on_click(cx.listener(
                                        |_: &mut Workspace, _: &ClickEvent, _, _cx| {
                                            if let Some(path) = config::Config::location()
                                                && let Some(dir) =
                                                    std::path::Path::new(&path).parent()
                                            {
                                                // Explorer, not a URL: a `file://`
                                                // link opens a second Explorer
                                                // window for the file rather than
                                                // the folder it is in.
                                                let _ = std::process::Command::new("explorer")
                                                    .arg(dir)
                                                    .spawn();
                                            }
                                        },
                                    ))
                                    .child("配置文件"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_close_prompt(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(target) = self.closing else {
            return div().into_any_element();
        };
        let shells: usize = self.tabs.iter().map(|tab| tab.panes.len()).sum();

        // One question, three nouns. The copy says what will end and how much of
        // it, because "are you sure" is a question that answers nothing.
        let (title, body, confirm) = match target {
            CloseTarget::Window => (
                "关闭窗口？".to_owned(),
                format!("关闭窗口会结束 {shells} 个终端，正在运行的程序也会停止。"),
                "关闭窗口",
            ),
            CloseTarget::Workspace(index) => {
                let tab = self.tabs.get(index);
                let name = tab.map(|tab| tab.title.clone()).unwrap_or_default();
                let running = tab
                    .map(|tab| {
                        tab.panes
                            .values()
                            .filter(|pane| matches!(pane_status(pane), PaneStatus::Working))
                            .count()
                    })
                    .unwrap_or(0);
                (
                    format!("关闭工作区「{name}」？"),
                    if running == 0 {
                        "这个工作区里的终端都已经停止。".to_owned()
                    } else {
                        format!("{running} 个终端还在运行，关闭会结束这些进程。")
                    },
                    "关闭工作区",
                )
            }
            CloseTarget::Pane(tab, id) => {
                let label = self
                    .tabs
                    .get(tab)
                    .and_then(|tab| tab.panes.get(&id))
                    .map(pane_label)
                    .unwrap_or_default();
                (
                    format!("关闭「{label}」？"),
                    "它还在运行，关闭会结束这个进程。".to_owned(),
                    "关闭窗格",
                )
            }
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .child(
                div()
                    .w(px(380.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .p(px(18.0))
                    .rounded(px(14.0))
                    .bg(theme.popover)
                    .shadow_lg()
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(theme.muted_foreground)
                            .child(body),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.0))
                            .pt(px(10.0))
                            .child(
                                div()
                                    .id("cancel-close")
                                    .flex()
                                    .items_center()
                                    .h(px(28.0))
                                    .px(px(14.0))
                                    .rounded(px(7.0))
                                    .bg(theme.muted)
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(theme.muted_foreground)
                                    .hover(move |this| this.bg(theme.muted))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                            workspace.closing = None;
                                            cx.notify();
                                        },
                                    ))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("confirm-close")
                                    .flex()
                                    .items_center()
                                    .h(px(28.0))
                                    .px(px(14.0))
                                    .rounded(px(7.0))
                                    .bg(theme.danger.opacity(0.14))
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(theme.danger)
                                    .hover(move |this| this.bg(theme.danger.opacity(0.22)))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                                            workspace.confirm_close(window, cx);
                                        },
                                    ))
                                    .child(confirm),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The undo toast for the workspace that was just closed.
    ///
    /// Six seconds, one line, one button. Long enough to catch a slip, short
    /// enough that the shells behind it are not held open by a terminal that
    /// cannot be closed.
    fn render_toast(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(closed) = self.closed.as_ref() else {
            return div().into_any_element();
        };
        let name = closed.tab.title.clone();

        div()
            .absolute()
            .right(px(16.0))
            .bottom(px(40.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.0))
            .h(px(38.0))
            .pl(px(14.0))
            .pr(px(8.0))
            .rounded(px(9.0))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .text_size(px(11.5))
            .text_color(theme.muted_foreground)
            .child(format!("已关闭工作区「{name}」"))
            .child(
                div()
                    .id("toast-undo")
                    .flex()
                    .items_center()
                    .h(px(26.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(theme.ring)
                    .hover(move |this| this.bg(theme.ring.opacity(0.13)))
                    .on_click(
                        cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                            workspace.reopen_closed(cx);
                        }),
                    )
                    .child("撤销"),
            )
            .into_any_element()
    }

    fn close_others(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep >= self.tabs.len() {
            return;
        }
        let kept = self.tabs.remove(keep);
        self.tabs.clear();
        self.tabs.push(kept);
        self.active_tab = 0;
        self.menu = None;
        cx.notify();
    }

    /// Opens the find bar over the active pane.
    ///
    /// The input owns the text and the workspace owns the matches: the bar has
    /// to be able to show "3 / 12" while the caret is still in the field, and a
    /// controlled input that rebuilt its value every frame would fight the
    /// platform's own editing state.
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() || self.active_pane().is_none() {
            return;
        }

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("查找…"));
        let sub = cx.subscribe(
            &input,
            |workspace: &mut Workspace, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    workspace.refresh_find(cx);
                }
            },
        );
        input.update(cx, |state, cx| state.focus(window, cx));

        self.find = Some(FindState {
            input,
            matches: Vec::new(),
            current: 0,
            total: 0,
            selected: None,
            _sub: sub,
        });
        cx.notify();
    }

    /// Rereads the query and rescans the visible rows.
    fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        let query = find.input.read(cx).value().to_string();
        self.run_find(Some(&query), None, cx);
    }

    /// Runs the search — or walks it — and folds the answer into the bar.
    ///
    /// The search lives in the terminal, not here: it owns the scrollback, it
    /// knows where a match currently sits, and it is what scrolls the viewport
    /// when `Enter` moves to one. The bar only keeps the part it has to draw.
    fn run_find(&mut self, query: Option<&str>, step: Option<bool>, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane_mut() else {
            return;
        };
        let view = match step {
            Some(forward) => pane.session.find_step(forward),
            None => pane.session.find(query.unwrap_or_default()),
        };

        // The selected match is only worth marking when it is one of the ones on
        // screen; the library reports it in the same viewport cells.
        let current = view
            .current
            .and_then(|hit| view.ranges.iter().position(|range| *range == hit))
            .unwrap_or(0);

        if let Some(find) = self.find.as_mut() {
            find.matches = view.ranges;
            find.total = view.total;
            find.selected = view.selected;
            find.current = current;
        }
        cx.notify();
    }

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return;
        }
        self.run_find(None, Some(delta > 0), cx);
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            if let Some(pane) = self.active_pane_mut() {
                pane.session.end_find();
            }
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return;
        }

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("输入命令、会话或主题…"));
        let sub = cx.subscribe(
            &input,
            |workspace: &mut Workspace, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = workspace
                        .palette
                        .as_ref()
                        .map(|palette| palette.input.read(cx).value().to_string())
                        .unwrap_or_default();
                    if let Some(palette) = workspace.palette.as_mut() {
                        palette.query = query;
                        // The list is short and filtered as you type, so the
                        // first row is always the answer until a key says
                        // otherwise.
                        palette.selected = 0;
                    }
                    cx.notify();
                }
            },
        );
        input.update(cx, |state, cx| state.focus(window, cx));

        self.palette = Some(Palette {
            input,
            query: String::new(),
            selected: 0,
            _sub: sub,
        });
        cx.notify();
    }

    /// Runs the palette's selected command.
    fn run_command(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        let query = palette.query.clone();
        let Some(row) = self.palette_rows(&query).get(index).copied() else {
            return;
        };

        self.palette = None;
        window.focus(&self.focus, cx);

        match row {
            PaletteRow::Pane(tab, id) => {
                self.select_tab(tab, cx);
                self.focus_pane(id, cx);
            }
            PaletteRow::Command(index) => match PALETTE_COMMANDS[index].id {
                "new-tab" => self.new_tab(window, cx),
                "close-tab" => self.close_tab(self.active_tab, cx),
                "rename-tab" => self.open_rename(self.active_tab, window, cx),
                "close-pane" => self.close_pane(cx),
                "split-right" => self.split_active(Axis::Horizontal, window, cx),
                "split-down" => self.split_active(Axis::Vertical, window, cx),
                "zoom" => {
                    if let Some(pane) = self.active_tab().map(|tab| tab.active) {
                        self.toggle_zoom(pane, cx);
                    }
                }
                "theme" => self.toggle_theme(cx),
                "find" => self.open_find(window, cx),
                "settings" => self.settings = true,
                "close-window" => self.close_window(window, cx),
                _ => {}
            },
        }
        cx.notify();
    }
}

/// One row of a menu: the same row every menu in the app uses.
fn menu_row(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &gpui_kit::component::Theme,
) -> Stateful<Div> {
    let hover = theme.muted;
    let text = if enabled {
        theme.foreground
    } else {
        theme.muted_foreground
    };

    div()
        .id(ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .text_size(px(12.0))
        .text_color(text)
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |this| this.bg(hover))
        })
        .child(label)
}

impl Workspace {
    /// Starts dragging a tab.
    fn begin_tab_drag(&mut self, from: usize, start: f32) {
        self.tab_drag = Some(TabDrag {
            from,
            start,
            target: None,
            moved: false,
        });
        // A new press is a new gesture, whatever the last one left behind.
        self.swallow_tab_click = false;
    }

    /// Follows the pointer onto another row, once it has actually moved.
    ///
    /// The six-pixel threshold is what separates a drag from the click that
    /// selects the row: without it every click with a slightly unsteady hand
    /// would end up reordering the list.
    fn extend_tab_drag(&mut self, target: usize, y: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.as_mut() else {
            return;
        };
        if !drag.moved && (y - drag.start).abs() < 6.0 {
            return;
        }
        drag.moved = true;
        if drag.target != Some(target) {
            drag.target = Some(target);
            cx.notify();
        }
    }

    /// Drops the tab where the pointer left it.
    fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.take() else {
            return;
        };
        if drag.moved
            && let Some(to) = drag.target
            && to != drag.from
        {
            self.move_tab(drag.from, to);
            // The release will still produce a click whose `tab_index` points at
            // the slot the tab used to occupy. Acting on it would select the tab
            // that drag just pushed there.
            self.swallow_tab_click = true;
        }
        cx.notify();
    }

    /// Moves a tab to another position, keeping the current one current.
    fn move_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);

        // The selection follows the tab, not the slot: dragging the current tab
        // must not silently make a different one current.
        if self.active_tab == from {
            self.active_tab = to;
        } else if from < self.active_tab && to >= self.active_tab {
            self.active_tab -= 1;
        } else if to <= self.active_tab && from > self.active_tab {
            self.active_tab += 1;
        }
    }

    /// Starts dragging the list's edge.
    fn begin_sidebar_resize(&mut self, position: Point<Pixels>) {
        self.sidebar_drag = Some(SidebarDrag {
            start: position.x.as_f32(),
            width: self.sidebar_width,
        });
    }

    /// Follows the pointer, snapping to the rail at the narrow end.
    fn extend_sidebar_resize(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.sidebar_drag else {
            return;
        };

        let width =
            (drag.width + (position.x.as_f32() - drag.start)).clamp(RAIL_WIDTH, MAX_SIDEBAR_WIDTH);
        // Releasing past the rail width leaves it there rather than springing
        // back: dragging to the narrow end is how the list is collapsed.
        if (width - self.sidebar_width).abs() > 0.5 {
            self.sidebar_width = width;
            // Remember the last readable width, so the keyboard collapse has
            // somewhere to come back to.
            if width >= RAIL_THRESHOLD {
                self.sidebar_expanded = width;
            }
            cx.notify();
        }
    }

    /// The pane the keyboard is actually in.
    ///
    /// A zoomed pane is the one on screen whatever `active` says, so both the
    /// grid and the sidebar read this instead of deciding for themselves. Two
    /// derivations of "the current pane" is exactly how the sidebar ends up
    /// highlighting a pane the window is not showing.
    fn effective_active(&self) -> Option<(usize, u64)> {
        let tab = self.active_tab()?;
        let id = match self.zoomed {
            Some(zoomed) if tab.panes.contains_key(&zoomed) => zoomed,
            _ => tab.active,
        };
        Some((self.active_tab, id))
    }

    /// Takes the newest download progress report, if any arrived.
    ///
    /// Returns whether anything changed. Only the last value in the queue
    /// matters: intermediate byte counts are already stale by the time a frame
    /// is drawn, and applying them all would make the badge flicker through
    /// numbers nobody can read.
    fn drain_update_progress(&mut self) -> bool {
        let Some(rx) = &self.update_rx else {
            return false;
        };

        let mut latest = None;
        while let Ok(report) = rx.try_recv() {
            latest = Some(report);
        }

        match latest {
            Some(progress) => {
                self.update_progress = Some(progress);
                true
            }
            None => false,
        }
    }

    /// The badge text while a download is running.
    ///
    /// Shows a percentage when the server declared a length, and a byte count
    /// when it did not — a bar that cannot compute a fraction still has to show
    /// that something is moving.
    fn download_label(&self) -> String {
        let Some((written, total)) = self.update_progress else {
            return "正在更新…".to_owned();
        };

        match total.filter(|total| *total > 0) {
            Some(total) => {
                let percent = (written.saturating_mul(100) / total).min(100);
                format!("正在更新 {percent}%")
            }
            // No declared length, so there is no fraction to show. A byte count
            // still says the download is moving, but not before the first chunk
            // lands: "0 MB" is a number that reads as a stall.
            None if written == 0 => "正在更新…".to_owned(),
            None => format!("正在更新 {}", update::megabytes(written)),
        }
    }

    /// What the update badge should say, if anything.
    ///
    /// `None` hides it: no check yet, no newer release, or a check that failed.
    /// The second field says whether it can be acted on.
    fn update_badge(&self) -> Option<(String, bool)> {
        if self.updating {
            return Some((self.download_label(), false));
        }

        match &self.update {
            Some(Ok(release)) => Some((format!("更新到 {}", release.version), true)),
            // A failure stays clickable so the download can be retried without
            // restarting; the message is the whole explanation, and a dead badge
            // would leave the user with nothing to do about it.
            Some(Err(message)) if message.starts_with("更新失败") => {
                Some((message.clone(), true))
            }
            Some(Err(message)) if !message.is_empty() => Some((message.clone(), false)),
            _ => None,
        }
    }

    /// Asks GitHub for the latest release, once, off the UI thread.
    ///
    /// The answer only becomes a small badge in the title bar. A failed check is
    /// kept silent on purpose: an app that announces every network problem is
    /// worse than one that simply does not mention updates.
    fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if self.update.is_some() {
            return;
        }
        // Claim the slot before the work starts, so a second call is a no-op
        // rather than a second request.
        self.update = Some(Err(String::new()));

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { update::check() })
                .await;

            let found = match result {
                Ok(release) if update::is_newer(&release.version, update::current_version()) => {
                    Ok(release)
                }
                Ok(_) => Err(String::new()),
                Err(update::UpdateError::NoRelease) => Err(String::new()),
                Err(error) => Err(error.to_string()),
            };

            let _ = this.update(cx, |workspace, cx| {
                workspace.update = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads the pending release and hands off to the restart script.
    fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.updating {
            return;
        }
        let Some(Ok(release)) = self.update.clone() else {
            // Nothing to install, or the last attempt failed and there is no
            // release to retry with. Check again rather than doing nothing.
            self.update = None;
            self.check_for_update(cx);
            return;
        };
        self.updating = true;
        self.update_progress = Some((0, None));
        cx.notify();

        // The download runs on a background thread, so its progress has to come
        // back through the UI thread. The channel carries one update per chunk;
        // the loop below drains it on every frame rather than repainting per
        // chunk, which would ask for thousands of frames for one file.
        let (tx, rx) = std::sync::mpsc::channel::<(u64, Option<u64>)>();
        self.update_rx = Some(rx);

        cx.spawn(async move |this, cx| {
            let staged = cx
                .background_executor()
                .spawn(async move {
                    update::download(&release, |written, total| {
                        // A closed receiver means the window is gone; the
                        // download is still worth finishing, so the error is
                        // deliberately ignored.
                        let _ = tx.send((written, total));
                    })
                })
                .await;

            let _ = this.update(cx, |workspace, cx| {
                workspace.updating = false;
                workspace.update_progress = None;
                match staged {
                    Ok(path) => {
                        // The swap needs this process gone, so the app quits as
                        // soon as the script is running.
                        if update::install_and_restart(&path).is_ok() {
                            cx.quit();
                            return;
                        }
                        workspace.update = Some(Err("更新失败：无法启动替换程序".to_owned()));
                    }
                    Err(error) => {
                        workspace.update = Some(Err(format!("更新失败：{error}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The install state the about panel shows.
    fn install_state(&self) -> InstallState {
        InstallState {
            installed: install::is_installed(),
            present: install::is_present(),
            busy: self.installing,
        }
    }

    /// Installs or removes the app, depending on what is there now.
    ///
    /// Both directions end with this process gone: installing starts the copy
    /// that was just written, and uninstalling deletes the one that is running.
    /// The work is done off the UI thread because writing a shortcut and a
    /// registry key can each take a moment, and a frozen window during an
    /// install reads as a failed install.
    fn run_install(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }

        let removing = install::is_installed();
        self.installing = true;
        self.install_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    if removing {
                        install::uninstall()
                    } else {
                        install::install(&install::InstallOptions {
                            // Offered by default, the way every installer on
                            // Windows does it, but the panel can turn it off.
                            desktop_shortcut: true,
                        })
                        .map(|_| ())
                    }
                })
                .await;

            let _ = this.update(cx, |workspace, cx| {
                workspace.installing = false;
                match outcome {
                    Ok(()) => {
                        if removing {
                            // The files go away only after this process exits,
                            // so quitting is what completes the uninstall.
                            cx.quit();
                            return;
                        }
                        // Starting the installed copy before quitting means the
                        // user sees the app come back rather than disappear.
                        if install::launch_installed().is_ok() {
                            cx.quit();
                            return;
                        }
                        workspace.install_error = Some("安装完成，但无法启动新副本".to_owned());
                    }
                    Err(error) => {
                        workspace.install_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Collapses the list to its rail, or opens it back to the width it had.
    ///
    /// The width is the user's: it starts at the rail and only changes when the
    /// edge is dragged, this is pressed, or `Ctrl+Shift+B` is. Splitting a pane
    /// never widens it on its own — the column is navigation, and it has no
    /// reason to take the terminal's width without being asked.
    fn toggle_rail(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_width < RAIL_THRESHOLD {
            self.sidebar_width = self
                .sidebar_expanded
                .clamp(RAIL_THRESHOLD, MAX_SIDEBAR_WIDTH);
        } else {
            self.sidebar_expanded = self.sidebar_width;
            self.sidebar_width = RAIL_WIDTH;
        }
        cx.notify();
    }

    /// The band that offers to install this copy.
    ///
    /// Shown when nothing exists at the install location, which is the portable
    /// download case. It is an offer, not a gate: it can be dismissed, and the
    /// install button runs the same path as the about panel so the two entry
    /// points cannot drift. A failure stays in the band, where the user pressed.
    fn render_install_prompt(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme.ring;
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let hover = theme.muted;
        let foreground = theme.foreground;

        let (title, detail, tint, title_color) = if self.installing {
            ("正在安装…".to_owned(), String::new(), accent, foreground)
        } else if let Some(error) = &self.install_error {
            ("安装失败".to_owned(), error.clone(), danger, danger)
        } else {
            (
                "myterm 还没有安装到这台电脑".to_owned(),
                "安装后可从开始菜单启动，有新版本时会自动更新。".to_owned(),
                accent,
                foreground,
            )
        };

        let mut band = div()
            .mr(px(PANE_GAP))
            .mb(px(PANE_GAP))
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .gap(px(10.0))
            .h(px(38.0))
            .px(px(12.0))
            .rounded(px(9.0))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .child(icon_data(INFO_ICON, 14.0, tint))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.0))
                            .text_color(title_color)
                            .child(title),
                    )
                    .when(!detail.is_empty(), |this| {
                        this.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(11.0))
                                .text_color(muted)
                                .child(detail),
                        )
                    }),
            );

        // No buttons while the work runs: pressing install twice would replace
        // the copy twice, and this band is not a progress control.
        if !self.installing {
            band = band
                .child(
                    div()
                        .id("install-prompt-later")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .h(px(24.0))
                        .px(px(10.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(muted)
                        .hover(move |this| this.bg(hover))
                        .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                            workspace.show_install_prompt = false;
                            cx.notify();
                        }))
                        .child("稍后"),
                )
                .child(
                    div()
                        .id("install-prompt-install")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .h(px(24.0))
                        .px(px(12.0))
                        .rounded(px(6.0))
                        .bg(accent.opacity(0.12))
                        .text_size(px(11.5))
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(move |this| this.bg(accent.opacity(0.2)))
                        .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                            workspace.run_install(cx);
                        }))
                        .child("安装"),
                );
        }

        band.into_any_element()
    }

    /// The workspace column: the window's only tab surface.
    ///
    /// It runs the full height of the window, including the corner the bar would
    /// otherwise own, so the list opens onto the mark that toggles it. Width is
    /// the whole state — below `RAIL_THRESHOLD` the list draws as a rail of
    /// chips, which is how it collapses without a second control to learn.
    fn render_sidebar(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = self.sidebar_width;
        let rail = width < RAIL_THRESHOLD;
        let wash = theme.muted;

        // The mark and the fold control. The mark is the app's identity and the
        // switch for this column in one object: it sits in the corner the column
        // opens onto, so the thing you press is next to the thing it opens.
        let mut top = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(TITLE_BAR_HEIGHT))
            .gap(px(6.0))
            .when(!rail, |this| this.pl(px(14.0)).pr(px(10.0)))
            .when(rail, |this| this.justify_center())
            .child(
                div()
                    .id("toggle-rail")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded(px(9.0))
                    .bg(theme.ring.opacity(0.13))
                    .cursor_pointer()
                    .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_rail(cx);
                    }))
                    .child(icon_data(APP_MARK, 15.0, theme.ring)),
            );
        if !rail {
            top = top.child(div().flex_1()).child(
                icon_button("rail-collapse", COLLAPSE_ICON, 24.0, theme, cx).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_rail(cx);
                    },
                )),
            );
        }

        // Every workspace and pane, with enough state to jump to one.
        //
        // This is the whole model the list reads: it is rebuilt from the
        // workspace each frame rather than kept in step with it, so there is
        // nothing to keep in sync.
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .px(px(8.0))
            .pt(px(2.0))
            .pb(px(8.0))
            // A drag released over the empty part of the column still has to
            // end, or the insertion rule stays on screen until the next click.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _, cx| {
                    workspace.end_tab_drag(cx);
                }),
            );
        // One answer for "which pane is current", shared with the grid.
        let current = self.effective_active();
        // Both are copied out before the loop: the rows need to know where a
        // drag would land, and holding a borrow of `tab_drag` across the
        // listeners would make the list impossible to build.
        let drag = self.tab_drag.as_ref().filter(|drag| drag.moved);
        let dragging_tab = drag.map(|drag| drag.from);
        let drop_target = drag.and_then(|drag| drag.target);

        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let current_tab = tab_index == self.active_tab;
            let grouped = tab.panes.len() > 1;

            // Workspaces are separated by air rather than a rule: the selected
            // fill already marks the current place, and a line drawn across the
            // column would be a second edge saying the same thing louder.
            if tab_index > 0 {
                list = list.child(div().h(px(6.0)));
            }

            // Where the workspace would land. A two-pixel rule in the accent
            // colour, placed between rows rather than drawn on one: an insertion
            // point is a gap in the list, not a property of a row.
            if drop_target == Some(tab_index) && dragging_tab != Some(tab_index) {
                list = list.child(div().h(px(2.0)).mx(px(8.0)).rounded(px(1.0)).bg(theme.ring));
            }

            let closable = self.tabs.len() > 1;

            if grouped {
                list = list.child(sidebar_group_row(
                    tab_index,
                    tab.title.clone(),
                    tab.panes.len(),
                    current_tab,
                    tab.unseen,
                    closable,
                    dragging_tab == Some(tab_index),
                    theme,
                    cx,
                ));
            }

            for (id, pane) in &tab.panes {
                let id = *id;
                let selected = current == Some((tab_index, id));
                list = list.child(sidebar_pane_row(
                    tab_index,
                    id,
                    pane_label(pane),
                    pane_cwd(pane),
                    pane_status(pane),
                    self.config.density,
                    // A row under a header belongs to it and steps in; a row
                    // that stands for its own workspace stays on the top spine.
                    grouped,
                    selected,
                    tab.unseen,
                    // A one-pane workspace is closed from its own row, because
                    // that row is the workspace.
                    closable && !grouped,
                    dragging_tab == Some(tab_index),
                    theme,
                    cx,
                ));
            }
        }

        if self.tabs.is_empty() {
            list = list.child(sidebar_empty_row(theme, cx));
        }

        let scroll = if rail {
            div()
                .id("side-rail")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.render_rail(theme, cx))
        } else {
            div()
                .id("side-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(list)
        };

        // The foot holds the two things that are about the column rather than
        // about a row in it: adding another workspace, and opening settings.
        let mut foot = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .p(px(8.0))
            .border_t_1()
            .border_color(theme.border)
            .when(rail, |this| this.flex_col());

        let mut new_button = div()
            .id("new-session")
            .flex()
            .items_center()
            .gap(px(7.0))
            .h(px(30.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(11.5))
            .text_color(theme.muted_foreground)
            .hover(move |this| this.bg(wash).text_color(theme.foreground))
            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.new_tab(window, cx);
            }))
            .child(icon_data(PLUS_SMALL, 13.0, theme.muted_foreground));
        new_button = if rail {
            new_button.flex_shrink_0().justify_center().w(px(30.0))
        } else {
            new_button
                .flex_1()
                .min_w_0()
                .px(px(9.0))
                .child("新建工作区")
        };
        foot = foot.child(new_button).child(
            icon_button("side-settings", SETTINGS_ICON, 30.0, theme, cx).on_click(cx.listener(
                |workspace, _: &ClickEvent, _, cx| {
                    workspace.toggle_settings(cx);
                },
            )),
        );

        div()
            .w(px(width))
            .flex_shrink_0()
            .h_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.title_bar)
            .child(top)
            .child(scroll)
            .child(foot)
            // The edge is the control: it is where a pointer already goes when
            // it wants to resize a column, and dragging it to the narrow end is
            // how the list collapses into the rail.
            .child(sidebar_handle(theme, self.sidebar_drag.is_some(), cx))
            .into_any_element()
    }

    /// The list with its names dropped: one block per workspace, one chip per
    /// shell.
    ///
    /// At this width the column is navigation, not content. A workspace gets a
    /// block — its own tint when it is the one on screen, and a heading carrying
    /// its first letter — so the grouping that the list shows with indent and
    /// air is carried here by a surface and a letter.
    ///
    /// The heading is drawn even for a one-shell workspace. Without it the block
    /// shows the shell's letter instead of the workspace's, and `日志` and
    /// `部署` both read `P` because both run `pwsh` — the column would be
    /// naming the program and never the place.
    ///
    /// The chip carries a character because a column of identical dots says how
    /// many places there are but not which is which. Names collide often — both
    /// halves of a split usually run the same shell — so a workspace with more
    /// than one pane numbers its chips instead, the same numbers the pane
    /// headers wear.
    fn render_rail(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rail = div()
            .w_full()
            .flex()
            .flex_col()
            .px(px(6.0))
            .pt(px(2.0))
            .pb(px(8.0));
        let current = self.effective_active();

        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let grouped = tab.panes.len() > 1;
            let current_tab = tab_index == self.active_tab;
            let wash = theme.muted;

            // The block is the workspace. A tint says which one the window is
            // showing — it is the one fact this column exists to answer, and a
            // tint answers it for a whole group at once rather than for one chip.
            let mut block = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .py(px(4.0))
                .rounded(px(10.0))
                .when(tab_index > 0, |this| this.mt(px(8.0)))
                // One step off the shell, not a card: the block is a bracket
                // around a workspace, and a white card would out-shout the chip
                // it is only there to group.
                .when(current_tab, |this| this.bg(theme.muted));

            {
                block = block.child(
                    div()
                        .id(ElementId::Name(format!("rail-tab-{tab_index}").into()))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(26.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .hover(move |this| this.bg(wash))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.select_tab(tab_index, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(if current_tab {
                                    theme.ring
                                } else {
                                    theme.muted_foreground
                                })
                                .child(rail_initial(&tab.title)),
                        ),
                );
            }

            // `panes` is keyed by the pane id, so this iteration is the order
            // the panes were opened — the same order the layout grows in, and
            // the order `render_node` numbers the headers with. The chip and the
            // badge have to agree, or the rail points at a pane the user cannot
            // find.
            for (index, (id, pane)) in tab.panes.iter().enumerate() {
                let id = *id;
                let selected = current == Some((tab_index, id));
                let status = pane_status(pane);
                // A split numbers its chips, because the number is what the
                // pane headers wear. A lone shell keeps its own letter: the
                // heading above already carries the workspace's.
                let glyph = if grouped {
                    (index + 1).to_string()
                } else {
                    rail_initial(&pane_label(pane))
                };
                let ink = if selected {
                    theme.foreground
                } else {
                    status_color(status, theme)
                };

                block = block.child(
                    div()
                        // Pane ids restart at 1 in every workspace, so the chip
                        // has to carry the workspace too: without it two chips
                        // share an id and only one of them can be hit.
                        .id(ElementId::Name(
                            format!("rail-pane-{tab_index}-{id}").into(),
                        ))
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(theme.popover))
                        .when(!selected, |this| this.hover(move |this| this.bg(wash)))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.select_tab(tab_index, cx);
                            workspace.focus_pane(id, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .text_color(ink)
                                .child(glyph),
                        )
                        // The corner dot is the second encoding: the letter is
                        // already coloured by state, but colour alone is not a
                        // signal a person can rely on.
                        .child(
                            div()
                                .absolute()
                                .top(px(5.0))
                                .right(px(5.0))
                                .size(px(5.0))
                                .rounded(px(999.0))
                                .bg(status_color(status, theme)),
                        ),
                );
            }

            rail = rail.child(block);
        }

        rail.into_any_element()
    }

    /// Draws the bar across the top of the work area.
    ///
    /// It answers two questions and nothing else: which workspace this is — its
    /// name, where the focused shell is, the branch when the shell reports one —
    /// and how to reach everything else without knowing a chord, through the
    /// command entry and the menu. The workspace column owns the top-left
    /// corner, so this bar starts where the work starts.
    fn render_top_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let maximized = window.is_maximized();
        let muted = theme.muted_foreground;
        // Copied before the closures below capture `theme` by move.
        let field = theme.background;
        let hover_fill = theme.muted;

        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(TITLE_BAR_HEIGHT))
            .pl(px(16.0))
            .pr(px(6.0))
            .gap(px(12.0))
            .bg(theme.title_bar)
            .text_color(theme.foreground);

        // The workspace, not the window: a name the user gave this place, and
        // the one line of context that matters while reading output — where the
        // shell thinks it is.
        let identity = self.active_tab().map(|tab| {
            let pane = tab
                .panes
                .get(&tab.active)
                .or_else(|| tab.panes.values().next());
            (tab.title.clone(), pane.and_then(pane_cwd))
        });
        let (name, detail) = identity.unwrap_or_else(|| ("myterm".to_owned(), None));

        let mut crumbs = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .min_w_0()
            .h_full()
            // The crumbs are not controls, so the space they cover is drag
            // area: the bar keeps no strip reserved for moving the window, and
            // the part that carries text is as good a handle as any.
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(200.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(name),
            );
        if let Some(detail) = detail {
            crumbs = crumbs.child(
                div()
                    .min_w_0()
                    .max_w(px(240.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child(detail),
            );
        }
        // The branch, when the shell's directory is in a checkout. It is a pill
        // rather than more text because it answers a different question from the
        // path beside it, and the accent is what says "this one is a place in a
        // repository" without a second label.
        if let Some(branch) = self
            .active_pane()
            .and_then(pane_cwd)
            .and_then(|cwd| self.branches.get(&cwd).cloned().flatten())
        {
            crumbs = crumbs.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(4.0))
                    .h(px(18.0))
                    .px(px(7.0))
                    .rounded(px(999.0))
                    .bg(theme.ring.opacity(0.13))
                    .text_size(px(10.0))
                    .text_color(theme.ring)
                    .child(icon_data(BRANCH_ICON, 10.0, theme.ring))
                    .child(branch.clone()),
            );
        }

        bar = bar.child(crumbs);

        // The rest of the bar's left half is drag area too, so the window can be
        // moved from the emptiest part of it as well as from the name.
        bar = bar.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .window_control_area(WindowControlArea::Drag),
        );

        // A keycap in the bar. Small, quiet, and never the only way to find the
        // command it names.
        let kbd_fg = theme.muted_foreground;
        let kbd = |label: &'static str| {
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .h(px(16.0))
                .px(px(4.0))
                .rounded(px(4.0))
                .bg(theme.muted)
                .text_size(px(9.5))
                .text_color(kbd_fg)
                .child(label)
        };

        bar = bar
            .child(
                div()
                    .id("cmd-trigger")
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(8.0))
                    .w(px(300.0))
                    .h(px(28.0))
                    .px(px(8.0))
                    .rounded(px(8.0))
                    .bg(field)
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover_fill))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                        workspace.open_palette(window, cx);
                    }))
                    .child(icon_data(SEARCH_ICON, 12.0, muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.0))
                            .text_color(muted)
                            .child("输入命令或搜索会话"),
                    )
                    .child(kbd("Ctrl"))
                    .child(kbd("K")),
            )
            // Splitting and opening live here as well as in the pane header:
            // with one pane there is no header to hold them, and a window whose
            // only way to start a second shell is a chord is a window with one
            // shell.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        icon_button("top-split-h", SPLIT_HORIZONTAL, 30.0, &theme, cx).on_click(
                            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                                workspace.split_active(Axis::Horizontal, window, cx);
                            }),
                        ),
                    )
                    .child(
                        icon_button("top-split-v", SPLIT_VERTICAL, 30.0, &theme, cx).on_click(
                            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                                workspace.split_active(Axis::Vertical, window, cx);
                            }),
                        ),
                    )
                    .child(
                        icon_button("top-new", PLUS_SMALL, 30.0, &theme, cx).on_click(cx.listener(
                            |workspace, _: &ClickEvent, window, cx| {
                                workspace.new_tab(window, cx);
                            },
                        )),
                    ),
            )
            .child(
                icon_button("app-menu", MENU_ICON, 30.0, &theme, cx).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = !workspace.app_menu;
                        cx.notify();
                    },
                )),
            )
            // A gap wide enough that the caption buttons read as window controls
            // and not as more of the app's own toolbar.
            .child(div().w(px(8.0)));

        bar.child(window_button(
            "win-min",
            WINDOW_MINIMIZE,
            WindowControlArea::Min,
            false,
            &theme,
            cx.listener(|_workspace, _: &ClickEvent, window, _cx| {
                window.minimize_window();
            }),
        ))
        .child(window_button(
            "win-max",
            if maximized {
                WINDOW_RESTORE
            } else {
                WINDOW_MAXIMIZE
            },
            WindowControlArea::Max,
            false,
            &theme,
            cx.listener(|_workspace, _: &ClickEvent, window, _cx| {
                window.zoom_window();
            }),
        ))
        .child(window_button(
            "win-close",
            WINDOW_CLOSE,
            WindowControlArea::Close,
            true,
            &theme,
            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.close_window(window, cx);
            }),
        ))
    }

    /// The app menu under the bar's `⋯`.
    ///
    /// The controls that used to be icons on the bar — theme, settings, about,
    /// update — live here now. None of them is used often enough to spend a
    /// button of its own on in a bar that also has to hold the workspace's name,
    /// and a menu is what the `⋯` in that bar already promises.
    fn render_app_menu(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.muted_foreground;
        let wash = theme.muted;
        let hint = theme.muted_foreground.opacity(0.7);

        let row = move |id: &'static str, label: String, key: Option<&'static str>| {
            div()
                .id(ElementId::Name(id.into()))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .h(px(31.0))
                .px(px(9.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .hover(move |this| this.bg(wash))
                .text_size(px(11.5))
                .text_color(muted)
                .child(label)
                .when_some(key, |this, key| {
                    this.child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(hint)
                            .child(key),
                    )
                })
        };

        let update = self.update_badge();
        let menu = div()
            .w(px(210.0))
            .p(px(5.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .rounded(px(11.0))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .child(
                row("menu-settings", "设置".to_owned(), Some("Ctrl+,")).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.settings = true;
                        cx.notify();
                    },
                )),
            )
            .child(
                row("menu-about", "关于 myterm".to_owned(), None).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.about = true;
                        cx.notify();
                    },
                )),
            )
            .child(match &update {
                Some((label, true)) => row("menu-update", label.clone(), None)
                    .text_color(theme.ring)
                    .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.install_update(cx);
                    })),
                _ => row(
                    "menu-check",
                    if self.updating {
                        "正在下载…".to_owned()
                    } else {
                        "检查更新".to_owned()
                    },
                    None,
                )
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.app_menu = false;
                    workspace.check_for_update(cx);
                })),
            })
            .child(
                row(
                    "menu-theme",
                    if theme.mode == gpui_kit::component::ThemeMode::Light {
                        "切换到深色主题".to_owned()
                    } else {
                        "切换到浅色主题".to_owned()
                    },
                    Some("Ctrl+Shift+L"),
                )
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.app_menu = false;
                    workspace.toggle_theme(cx);
                })),
            )
            .child(div().h(px(1.0)).mx(px(8.0)).my(px(5.0)).bg(theme.border))
            .child(
                row("menu-quit", "退出".to_owned(), None).on_click(cx.listener(
                    |workspace, _: &ClickEvent, window, cx| {
                        workspace.app_menu = false;
                        workspace.close_window(window, cx);
                    },
                )),
            );

        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseDownEvent, _, cx| {
                    workspace.app_menu = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(TITLE_BAR_HEIGHT))
                    .right(px(140.0))
                    .child(menu),
            )
            .into_any_element()
    }

    /// What is left when every workspace is closed.
    ///
    /// Not a black window and not a consolation sentence: the one thing an empty
    /// list can still tell you is where the next one comes from, so it says that
    /// and gives the button.
    fn render_empty(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .pr(px(PANE_GAP))
            .pb(px(PANE_GAP))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.muted_foreground)
                    .child("没有打开的会话"),
            )
            .child(
                div()
                    .id("empty-new")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.0))
                    .h(px(30.0))
                    .px(px(12.0))
                    .rounded(px(8.0))
                    .bg(theme.muted)
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(theme.muted_foreground)
                    .hover(move |this| this.bg(theme.secondary_hover).text_color(theme.foreground))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                        workspace.new_tab(window, cx);
                    }))
                    .child(icon_data(PLUS_SMALL, 13.0, theme.muted_foreground))
                    .child("新建工作区"),
            )
            .into_any_element()
    }

    /// What the status bar has to say, if anything.
    ///
    /// Everything that used to sit there permanently — the session count, the
    /// branch, a reminder about keys — already has a better home in the bar
    /// above or the list beside it. What is left is the state that exists only
    /// while it is true.
    fn status_hint(&self) -> Option<String> {
        // One case, as the prototype has it. Background output and a pending
        // update are both already reported where they belong — a dot on the
        // workspace's row, a badge in the bar — and repeating them here made a
        // row that was on most of the time, which is the opposite of the point.
        self.zoomed
            .is_some()
            .then(|| "已缩放 · Esc 退出".to_owned())
    }

    /// The status bar: 26 px that exist only while `status_hint` has a line.
    fn render_status_bar(
        &self,
        hint: &str,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .h(px(STATUS_BAR_HEIGHT))
            .pl(px(16.0))
            .pr(px(10.0))
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(10.5))
            .text_color(theme.muted_foreground)
            .child(hint.to_owned())
            .child(div().flex_1())
            .when(self.zoomed.is_some(), |this| {
                this.child(
                    div()
                        .id("status-unzoom")
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(move |this| this.bg(theme.muted))
                        .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                            if let Some(id) = workspace.zoomed {
                                workspace.toggle_zoom(id, cx);
                            }
                        }))
                        .child("退出缩放"),
                )
            })
            .into_any_element()
    }
}

/// The draggable gutter between two panes of a split.
///
/// The gap already reads as a seam: each pane is a card with its own hairline
/// border, so two borders with space between them say "two things". What the gap
/// cannot say by itself is that it can be moved, so that is all this adds — a
/// grip that appears under the pointer, the same one the sidebar's edge uses.
///
/// The grip stays put while the drag runs. `group_hover` alone drops it the moment
/// the pointer leaves the eight-pixel band, which is exactly when the user is
/// watching it.
fn divider_element(
    axis: Axis,
    first: &Layout,
    second: &Layout,
    tab: &Tab,
    dragging: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let left = first.leaves().last().copied();
    let right = second.leaves().first().copied();

    // The extent the divider travels, as the union of both halves.
    let halves = (subtree_bounds(tab, first), subtree_bounds(tab, second));

    /// How long the grip is along the seam.
    const GRIP: f32 = 26.0;

    let grip = div()
        .when(axis == Axis::Horizontal, |this| this.w(px(3.0)).h(px(GRIP)))
        .when(axis == Axis::Vertical, |this| this.h(px(3.0)).w(px(GRIP)))
        .rounded(px(999.0))
        .bg(theme.ring)
        .when(dragging, |this| this.opacity(1.0))
        .when(!dragging, |this| {
            this.opacity(0.0)
                .group_hover("divider", |this| this.opacity(1.0))
        });

    let mut band = div()
        .group("divider")
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(grip);

    band = match axis {
        Axis::Horizontal => band.w(px(PANE_GAP)).h_full().cursor_col_resize(),
        Axis::Vertical => band.h(px(PANE_GAP)).w_full().cursor_row_resize(),
    };

    band.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
            let (Some(left), Some(right)) = (left, right) else {
                return;
            };
            let (Some(first), Some(second)) = halves else {
                return;
            };
            workspace.begin_resize(left, right, axis, event.position, (first, second), cx);
        }),
    )
    .into_any_element()
}

/// The box a pane's card occupies, in window coordinates.
///
/// Derived from the grid's box plus the inset the card gives it — the header and
/// the padding — rather than recorded separately. The grid is the only element
/// that reliably knows where it was painted, and the inset is a constant.
///
/// A split drag has to measure against the card, not the grid: using the grid put
/// the divider a header-height away from the pointer.
fn card_bounds(tab: &Tab, id: u64) -> Option<Bounds<Pixels>> {
    let pane = tab.panes.get(&id)?;
    let grid = pane.bounds.get();
    if grid.size.width <= px(0.0) || grid.size.height <= px(0.0) {
        return None;
    }

    // The grid is inset by the padding on three sides and by the header as well
    // when the window is divided.
    let header = if tab.panes.len() > 1 || tab.active == id {
        PANE_HEADER_HEIGHT
    } else {
        0.0
    };
    // The grid's padding is not symmetric, so the two axes are measured
    // separately; a single number put the divider four to seven pixels off.
    let top_pad = if tab.panes.len() > 1 || tab.active == id {
        0.0
    } else {
        GRID_PAD_TOP
    };
    let left = grid.left() - px(GRID_PAD_X);
    let top = grid.top() - px(top_pad + header);
    let right = grid.right() + px(GRID_PAD_X);
    let bottom = grid.bottom() + px(GRID_PAD_BOTTOM);

    Some(Bounds::new(
        point(left, top),
        size(right - left, bottom - top),
    ))
}

/// The union of every card's box in a subtree, in window coordinates.
fn subtree_bounds(tab: &Tab, layout: &Layout) -> Option<Bounds<Pixels>> {
    let mut union: Option<Bounds<Pixels>> = None;
    for id in layout.leaves() {
        let bounds = card_bounds(tab, id)?;
        union = Some(match union {
            None => bounds,
            Some(current) => {
                let left = current.left().min(bounds.left());
                let top = current.top().min(bounds.top());
                let right = current.right().max(bounds.right());
                let bottom = current.bottom().max(bounds.bottom());
                Bounds::new(point(left, top), size(right - left, bottom - top))
            }
        });
    }
    union
}

/// The grab strip on the sidebar's trailing edge.
///
/// It straddles the seam rather than sitting beside it, so the pointer finds it
/// exactly where the two columns meet. The grip is the same one the split
/// divider uses — a short accent capsule that fades in under the pointer and
/// stays while the width is being dragged — so the two edges in the window read
/// as one kind of control, because they are.
fn sidebar_handle(
    theme: &gpui_kit::component::Theme,
    dragging: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id("side-handle")
        .absolute()
        .top_0()
        .bottom_0()
        // The strip straddles the seam: it starts just inside the column's
        // padding, past the end of the rows, and stops a pixel short of the
        // card. Wide enough for the pointer to find, and it never sits over a
        // row or the terminal, so it cannot steal a click meant for them.
        .right(px(-3.0))
        .w(px(8.0))
        // Its own hover group, like the divider band. Revealing the accent
        // capsule whenever the pointer is anywhere over the list would make it
        // flash on every pass; the edge is a control, and it should look like
        // one only where it can be grabbed.
        .group("side-edge")
        .flex()
        .items_center()
        .justify_center()
        .cursor_col_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|workspace, event: &MouseDownEvent, _, _| {
                workspace.begin_sidebar_resize(event.position);
            }),
        )
        .child(
            div()
                .w(px(3.0))
                .h(px(26.0))
                .rounded(px(999.0))
                .bg(theme.ring)
                .when(dragging, |this| this.opacity(1.0))
                .when(!dragging, |this| {
                    this.opacity(0.0)
                        .group_hover("side-edge", |this| this.opacity(1.0))
                }),
        )
}

/// A small control in a pane header.
/// The strip at the top of a pane: what it is running, and what can be done to
/// it.
///
/// It only appears once the window is divided. A lone terminal is already named
/// by the tab beside it, and repeating that name in a second bar would spend
/// thirty pixels saying nothing.
///
/// The name starts on the grid's own leading spine. State sits beside the name,
/// because it describes the shell; the actions live at the trailing edge and
/// appear only while the pointer is over the pane.
#[allow(clippy::too_many_arguments)]
fn pane_header(
    id: u64,
    ordinal: Option<usize>,
    name: String,
    meta: Option<String>,
    status: PaneStatus,
    focused: bool,
    zoomed: bool,
    live_size: Option<(u16, u16)>,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .flex_shrink_0()
        // The row has to take the card's width rather than its own content's.
        // The header is a flex row of state, name, context and actions; a shell
        // that reports a long path — a WindowsApps title is seventy characters —
        // otherwise lays the row out at its own preferred width, pushes the
        // actions past the card's edge, and paints the text out over the chrome.
        .w_full()
        .min_w_0()
        .h(px(PANE_HEADER_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .pl(px(PANE_TEXT_INSET))
        .pr(px(6.0))
        // State leads, as a shape first and a colour second: filled means
        // writing, hollow means waiting, square means gone. It describes the
        // shell, so it sits against the shell's name.
        .child(
            div()
                .flex_shrink_0()
                .w(px(7.0))
                .flex()
                .justify_center()
                .child(status_glyph(status, theme)),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(200.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.0))
                .font_weight(if focused {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if focused {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(name),
        )
        // The ordinal rides with the name rather than on the header's leading
        // edge: the name shares its 10px spine with the terminal's first column,
        // and a tile in front of it would push that spine 26px right. After the
        // name it reads as part of the label, which is exactly the question —
        // "is this the pane my chip calls 3?"
        .when_some(ordinal, |this, number| {
            this.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(15.0))
                    .px(px(4.0))
                    .rounded(px(4.0))
                    .bg(if focused {
                        theme.ring.opacity(0.16)
                    } else {
                        theme.muted
                    })
                    .text_size(px(9.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if focused {
                        theme.ring
                    } else {
                        theme.muted_foreground
                    })
                    .child(number.to_string()),
            )
        })
        // Where this shell is, when it says. It is the line that tells three
        // PowerShell panes apart, and it is allowed to vanish.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(meta.unwrap_or_default()),
        )
        .when_some(live_size, |this, (cols, rows)| {
            this.child(
                // The cell count the divider is producing right now. Only while
                // dragging: a permanent readout of a number nobody is changing
                // is decoration, and the shell already knows its own size.
                div()
                    .flex_shrink_0()
                    .mr(px(6.0))
                    .text_size(px(10.0))
                    .text_color(theme.muted_foreground)
                    .child(format!("{cols}×{rows}")),
            )
        })
        .child(
            // The pane's own actions, on the pane under the pointer or holding
            // the keyboard. Four glyphs repeated in every header is the same
            // command set printed once per pane, and in a split those addresses
            // were the loudest thing in an otherwise quiet row. Nothing is lost:
            // each command has a key, and the pane is focused by the same click
            // that reveals the buttons.
            //
            // Hidden, not dimmed. At 0.55 the buttons read as a disabled row the
            // user could not press; the row is either there or it is not.
            div()
                .flex()
                .flex_row()
                .items_center()
                // No shared fill, so the buttons need real air between them: at
                // one pixel four glyphs read as one smear.
                .gap(px(1.0))
                .opacity(if focused { 1.0 } else { 0.0 })
                .when(!focused, |this| {
                    this.group_hover("pane", |this| this.opacity(1.0))
                })
                .child(pane_action(
                    ("pane-split-right", id),
                    SPLIT_HORIZONTAL,
                    theme,
                    false,
                    cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.split_pane(id, Axis::Horizontal, window, cx);
                    }),
                ))
                .child(pane_action(
                    ("pane-split-down", id),
                    SPLIT_VERTICAL,
                    theme,
                    false,
                    cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.split_pane(id, Axis::Vertical, window, cx);
                    }),
                ))
                .child(pane_action_marked(
                    ("pane-zoom", id),
                    if zoomed { ZOOM_OUT } else { ZOOM_IN },
                    theme,
                    false,
                    zoomed,
                    cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_zoom(id, cx);
                    }),
                ))
                .child(pane_action(
                    ("pane-close", id),
                    WINDOW_CLOSE,
                    theme,
                    true,
                    cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        workspace.close_pane_at(id, cx);
                    }),
                )),
        )
}

#[derive(Clone, Copy)]
pub struct InstallState {
    /// This process is the installed copy.
    pub installed: bool,
    /// A copy exists at the install location, but this is not it.
    pub present: bool,
    /// An install or uninstall is running.
    pub busy: bool,
}

impl InstallState {
    fn summary(self) -> &'static str {
        if self.busy {
            return "处理中…";
        }
        match (self.installed, self.present) {
            (true, _) => "已安装",
            (false, true) => "已安装（当前不是它）",
            (false, false) => "未安装",
        }
    }
}

/// The button that installs, or removes, this app.
fn install_action(
    theme: &gpui_kit::component::Theme,
    install: InstallState,
    entity: Entity<Workspace>,
) -> AnyElement {
    if install.busy {
        return div().into_any_element();
    }

    let accent = theme.ring;
    let muted = theme.muted_foreground;
    let danger = theme.danger;
    let (label, color, destructive) = if install.installed {
        ("卸载", danger, true)
    } else if install.present {
        // A copy exists but this is not it. Offering "install" would silently
        // overwrite it; the honest action is to remove what is there first.
        ("重新安装", accent, false)
    } else {
        ("安装到本机", accent, false)
    };

    div()
        .id("install-action")
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h(px(28.0))
        .w_full()
        .rounded(px(7.0))
        // A fill rather than an outline, like every other button in the
        // window: the card has no rules left, and a bordered button would be
        // the one rectangle drawn in it.
        .bg(color.opacity(0.12))
        .text_size(px(11.5))
        .text_color(color)
        .cursor_pointer()
        .hover(move |this| this.bg(color.opacity(0.2)))
        .on_click(move |_: &ClickEvent, _, cx| {
            // Called on the workspace directly rather than dispatched: the card
            // lives in a popover, which is painted in a deferred layer outside
            // the window's element tree, so an action dispatched here would
            // bubble through nodes that never see it.
            let _ = (destructive, muted);
            entity.update(cx, |workspace, cx| workspace.run_install(cx));
        })
        .child(label)
        .into_any_element()
}

/// The version as a `&'static str`, so the rows above can take plain strings
/// rather than each owning a `String`.
fn version_label() -> &'static str {
    // `current_version` is `env!`-derived, so this is one short string per
    // process; the leak is deliberate and bounded.
    Box::leak(format!("v{}", update::current_version()).into_boxed_str())
}

/// The terminal library's version, read from the crate that links it.
///
/// The about card used to spell this out by hand, and it drifted: it still said
/// 1.0.0 while the vendored archive was 1.0.1. `vt-sys` publishes the number its
/// `build.rs` builds against, so this row now describes the library that is
/// actually linked into this binary.
fn ghostty_label() -> &'static str {
    Box::leak(format!("libghostty-vt {}", vt::GHOSTTY_VERSION).into_boxed_str())
}

/// A small control in a pane header.
///
/// Ghost by default: the glyph is drawn straight on the header and only gains a
/// surface under the pointer, which is the rule the title bar's buttons follow.
/// They used to be filled chips, and four grey blocks in an otherwise quiet row
/// read as a segmented control — a group of related choices — when they are
/// four unrelated commands.
///
/// `destructive` gives the close button the same red wash the window's own
/// close button uses, so the one action that ends the pane is not the same
/// colour as the three that rearrange it.
fn pane_action(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    theme: &gpui_kit::component::Theme,
    destructive: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    pane_action_marked(id, icon, theme, destructive, false, on_click)
}

/// The same button, with the "this mode is on" fill the zoom wears while it is
/// zoomed.
fn pane_action_marked(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    theme: &gpui_kit::component::Theme,
    destructive: bool,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let wash = if destructive {
        theme.danger.opacity(0.16)
    } else {
        theme.muted
    };
    let accent_wash = theme.ring.opacity(0.13);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.0))
        // The same rounded square as the title bar's buttons, one step tighter:
        // 24px with a 6px corner. A circle inside a rounded card was the one
        // shape in the window that belonged to no family.
        .rounded(px(6.0))
        // Nothing at rest: the header already carries the name, the ordinal, the
        // title and the state, and a row of four tinted blocks was the heaviest
        // thing in it. The surface appears when the pointer is on the button,
        // which is also the only moment the button is revealed at all.
        .cursor_pointer()
        // A mode that is on says so with a fill, the way the palette's selected
        // row does — the button is the only thing in the header that has a
        // state to be in.
        .when(active, |this| this.bg(accent_wash))
        .when(!active, |this| this.hover(move |this| this.bg(wash)))
        .on_click(on_click)
        .child(icon_data(
            icon,
            16.0,
            if active {
                theme.ring
            } else {
                theme.muted_foreground
            },
        ))
}

/// A choice inside the settings sheet.
///
/// Selected is carried by fill, border and weight together rather than by a
/// single colour: the sheet has to stay readable to someone who cannot tell the
/// accent green from the muted grey.
fn setting_row(
    label: &'static str,
    hint: &'static str,
    theme: &gpui_kit::component::Theme,
    control: AnyElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(11.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.45))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme.foreground)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(hint),
                ),
        )
        .child(control)
}

/// A tab's row in the sidebar: the header of a group of panes.
///
/// Quieter than the rows under it — smaller, and marked as current by weight and
/// colour rather than by a fill. The fill belongs to the focused pane, and two
/// filled rows in one group would read as two active places.
#[allow(clippy::too_many_arguments)]
fn sidebar_group_row(
    tab_index: usize,
    title: String,
    pane_count: usize,
    current: bool,
    unseen: bool,
    closable: bool,
    dragging: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let hover_bg = theme.muted;
    let row_group = format!("side-tab-row-{tab_index}");
    let dot = theme.primary;

    div()
        .id(ElementId::Name(format!("side-tab-{tab_index}").into()))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .h(px(26.0))
        .pl(px(8.0))
        .pr(px(6.0))
        .rounded(px(7.0))
        .cursor_pointer()
        // The tab being moved stays in its place, faded, so the list does not
        // jump under the pointer while a landing spot is being chosen.
        .when(dragging, |this| this.opacity(0.5))
        .group(row_group.clone())
        .hover(move |this| this.bg(hover_bg))
        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
            if workspace.swallow_tab_click {
                workspace.swallow_tab_click = false;
                return;
            }
            workspace.select_tab(tab_index, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |workspace, event: &MouseDownEvent, _, _| {
                workspace.begin_tab_drag(tab_index, event.position.y.as_f32());
            }),
        )
        .on_mouse_move(
            cx.listener(move |workspace, event: &MouseMoveEvent, _, cx| {
                workspace.extend_tab_drag(tab_index, event.position.y.as_f32(), cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|workspace, _: &MouseUpEvent, _, cx| {
                workspace.end_tab_drag(cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                workspace.open_tab_menu(tab_index, event.position, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |workspace, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                workspace.close_tab(tab_index, cx);
            }),
        )
        .child(icon_data(WORKSPACE_ICON, 12.0, theme.muted_foreground))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.0))
                .font_weight(if current {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if current {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(title),
        )
        .when(unseen, |this| {
            // A filled dot at the trailing edge: something in this tab stopped
            // while you were somewhere else. It is a shape, not only a colour,
            // and it lives beside the count rather than in front of the name.
            this.child(
                div()
                    .flex_shrink_0()
                    .size(px(5.0))
                    .rounded(px(999.0))
                    .bg(dot),
            )
        })
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(pane_count.to_string()),
        )
        .when(closable, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    // Every row reveals its cross under the pointer, the way
                    // the prototype draws it: a permanent cross on the current
                    // row makes one row of the column permanently louder than
                    // the rest, for a control that is used once in a while.
                    .opacity(0.0)
                    .group_hover(row_group, move |this| this.opacity(1.0))
                    .child(sidebar_close_button(
                        format!("side-close-tab-{tab_index}"),
                        tab_index,
                        theme,
                        cx,
                    )),
            )
        })
}

/// The cross on a tab row, sized to sit inside the row rather than take a lane.
///
/// It is drawn only on the current tab, so it reads as "this is what you would
/// close" rather than as a column of buttons.
fn sidebar_close_button(
    id: String,
    tab_index: usize,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let wash = theme.muted;
    div()
        .id(ElementId::Name(id.into()))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(18.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .hover(move |this| this.bg(wash))
        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
            // The row underneath selects a tab; the cross closes one.
            cx.stop_propagation();
            workspace.close_tab(tab_index, cx);
        }))
        .child(icon_data(TAB_CLOSE, 10.0, theme.muted_foreground))
}

/// One pane in the sidebar, or the single row that stands for a one-pane tab.
///
/// `indented` steps a row under its group header, which is the only hierarchy
/// the list needs; nothing else about the row changes, because a pane is a pane
/// wherever it appears.
#[allow(clippy::too_many_arguments)]
fn sidebar_pane_row(
    tab_index: usize,
    id: u64,
    label: String,
    detail: Option<String>,
    status: PaneStatus,
    density: config::Density,
    _indented: bool,
    selected: bool,
    unseen: bool,
    closable: bool,
    dragging: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let hover_bg = theme.muted;
    let selected_bg = theme.popover;
    let text_color = if selected {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    // The cross belongs to the row, not to the column: it appears when the
    // pointer is on the row it would close, so close is one gesture away on any
    // workspace without turning the list into a stack of identical buttons.
    // The selected row is no exception — the fill already says which one it is.
    let row_group = format!("side-row-{tab_index}-{id}");
    let cross = 0.0;

    div()
        // Pane ids restart at 1 in every workspace, so the row has to carry the
        // workspace too: without it two rows share an id and only one of them
        // can be hit or highlighted.
        .id(ElementId::Name(
            format!("side-pane-{tab_index}-{id}").into(),
        ))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        // Two lines fit in one row: what this shell *is*, and where it is. The
        // second line is what tells three PowerShell tabs apart, and it costs
        // twelve pixels that the column has and the window does not.
        .h(px(density.row_height()))
        // The dot keeps its lane through the indent, so the status column runs
        // straight down the group while the names step in behind it.
        .pl(px(10.0))
        .pr(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(dragging, |this| this.opacity(0.5))
        .group(row_group.clone())
        .when(selected, |this| this.bg(selected_bg))
        .when(!selected, |this| this.hover(move |this| this.bg(hover_bg)))
        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
            if workspace.swallow_tab_click {
                workspace.swallow_tab_click = false;
                return;
            }
            workspace.select_tab(tab_index, cx);
            workspace.focus_pane(id, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |workspace, event: &MouseDownEvent, _, _| {
                workspace.begin_tab_drag(tab_index, event.position.y.as_f32());
            }),
        )
        .on_mouse_move(
            cx.listener(move |workspace, event: &MouseMoveEvent, _, cx| {
                workspace.extend_tab_drag(tab_index, event.position.y.as_f32(), cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|workspace, _: &MouseUpEvent, _, cx| {
                workspace.end_tab_drag(cx);
            }),
        )
        // Right click asks the row what can be done to it. Middle click is the
        // desktop shorthand for "close this", and it works on any workspace
        // without the row having to be current first.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                workspace.open_tab_menu(tab_index, event.position, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |workspace, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                workspace.close_tab(tab_index, cx);
            }),
        )
        .child(
            div()
                .w(px(7.0))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .child(status_glyph(status, theme)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.0))
                        .text_color(text_color)
                        .child(label),
                )
                .when_some(
                    detail.filter(|_| density.shows_directory()),
                    |this, detail| {
                        this.child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(10.0))
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        )
                    },
                ),
        )
        // The word is the third encoding, and the only one that survives a
        // reader who cannot separate lime from grey. `Idle` says nothing: it is
        // what a pane is most of the time.
        .when_some(status.label(), |this, label| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
        })
        .when(unseen, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .size(px(5.0))
                    .rounded(px(999.0))
                    .bg(theme.primary),
            )
        })
        .when(closable, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .opacity(cross)
                    .group_hover(row_group, move |this| this.opacity(1.0))
                    .child(sidebar_close_button(
                        format!("side-close-pane-tab-{tab_index}-{id}"),
                        tab_index,
                        theme,
                        cx,
                    )),
            )
        })
}

fn sidebar_empty_row(
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id("side-empty-new")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(30.0))
        .px(px(8.0))
        .rounded(px(CHIP_RADIUS))
        .cursor_pointer()
        .hover(move |this| this.bg(theme.muted))
        .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
            workspace.new_tab(window, cx);
        }))
        .child(icon_data(PLUS_SMALL, 12.0, theme.muted_foreground))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.muted_foreground)
                .child("新建终端"),
        )
}

/// The one character a rail chip can show to say which place it is.
///
/// Upper case for Latin labels, which is how the first letter of a name is
/// normally read; a CJK label keeps its own character. An empty name falls back
/// to a mark rather than an empty chip.
fn rail_initial(label: &str) -> String {
    label.chars().next().map_or_else(
        || "·".to_owned(),
        |first| first.to_uppercase().collect::<String>(),
    )
}

/// A rounded icon-only button.
///
/// Icon-only controls carry no visible label, so the caller must pass an id that
/// also reads as a name. `size` is the frame, and the glyph follows it: the title
/// bar uses 28, the sidebar's shorter heading uses 24.
fn icon_button(
    id: &'static str,
    icon: &'static [u8],
    size: f32,
    theme: &gpui_kit::component::Theme,
    _cx: &mut Context<Workspace>,
) -> Stateful<Div> {
    // A glyph is about four sevenths of its frame, which is the ratio the title
    // bar's buttons already use: 16 px inside 28.
    let glyph = icon_data(icon, (size * 0.57).round(), theme.muted_foreground);

    // A rounded square, not a circle. The circle was left over from a bar whose
    // other controls were pills; with tabs gone from the bar there is nothing to
    // echo, and a 28px circle at this size reads as a dot rather than as a
    // button. Eight pixels is the same corner the app mark wears one button
    // away, so the row looks like one row.
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |this| this.bg(theme.muted))
        .child(glyph)
}

/// Draws an inline SVG, for glyphs the bundled assets cannot provide.
///
/// The size is a parameter because the same drawing appears at different
/// scales: a toolbar glyph is sixteen pixels, while the cross inside a tab has
/// to be smaller than that or it fills the circle it sits in.
fn icon_data(svg: &'static [u8], size: f32, color: Hsla) -> AnyElement {
    gpui_kit::component::Icon::default()
        .data(svg)
        .with_size(px(size))
        .text_color(color)
        .into_any_element()
}

/// One of the three caption buttons at the trailing edge of the title bar.
///
/// These keep their own shape: a caption button is a rectangle flush to the
/// corner in Windows, and rounding it would make the window feel like it is
/// missing its edge. Only the tint follows the palette — the close button gets a
/// red wash on hover, the others the same grey the rest of the bar uses.
fn window_button(
    id: &'static str,
    icon: &'static [u8],
    area: WindowControlArea,
    destructive: bool,
    theme: &gpui_kit::component::Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = theme.muted_foreground;
    let wash = if destructive {
        theme.danger.opacity(0.14)
    } else {
        theme.muted
    };
    // The cross stays the same grey as its neighbours and only the wash turns
    // red: a permanently red close button draws the eye to the one control the
    // user is least likely to want.
    let glyph = icon_data(icon, 16.0, color);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(42.0))
        .h(px(32.0))
        .rounded(px(7.0))
        .window_control_area(area)
        .cursor_pointer()
        .hover(move |this| this.bg(wash))
        .on_click(on_click)
        .child(glyph)
}

fn key_input(keystroke: &Keystroke) -> Option<vt::KeyInput<'_>> {
    let key = map_key(&keystroke.key)?;
    let mut mods = 0u16;
    if keystroke.modifiers.shift {
        mods |= vt::MODS_SHIFT;
    }
    if keystroke.modifiers.control {
        mods |= vt::MODS_CTRL;
    }
    if keystroke.modifiers.alt {
        mods |= vt::MODS_ALT;
    }

    Some(vt::KeyInput {
        key,
        mods,
        utf8: keystroke.key_char.as_deref(),
    })
}

/// Maps GPUI's key names onto the parser's key codes.
fn map_key(key: &str) -> Option<i32> {
    use vt::key;

    let code = match key {
        "enter" => key::ENTER,
        "tab" => key::TAB,
        "backspace" => key::BACKSPACE,
        "delete" => key::DELETE,
        "escape" => key::ESCAPE,
        "up" => key::ARROW_UP,
        "down" => key::ARROW_DOWN,
        "left" => key::ARROW_LEFT,
        "right" => key::ARROW_RIGHT,
        "home" => key::HOME,
        "end" => key::END,
        "pageup" => key::PAGE_UP,
        "pagedown" => key::PAGE_DOWN,
        "insert" => key::INSERT,
        "space" => key::SPACE,
        "f1" => key::F1,
        "f2" => key::F2,
        "f3" => key::F3,
        "f4" => key::F4,
        "f5" => key::F5,
        "f6" => key::F6,
        "f7" => key::F7,
        "f8" => key::F8,
        "f9" => key::F9,
        "f10" => key::F10,
        "f11" => key::F11,
        "f12" => key::F12,
        other => {
            let mut chars = other.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match ch.to_ascii_lowercase() {
                'a' => key::A,
                'b' => key::B,
                'c' => key::C,
                'd' => key::D,
                'e' => key::E,
                'f' => key::F,
                'g' => key::G,
                'h' => key::H,
                'i' => key::I,
                'j' => key::J,
                'k' => key::K,
                'l' => key::L,
                'm' => key::M,
                'n' => key::N,
                'o' => key::O,
                'p' => key::P,
                'q' => key::Q,
                'r' => key::R,
                's' => key::S,
                't' => key::T,
                'u' => key::U,
                'v' => key::V,
                'w' => key::W,
                'x' => key::X,
                'y' => key::Y,
                'z' => key::Z,
                '0' => key::DIGIT_0,
                '1' => key::DIGIT_1,
                '2' => key::DIGIT_2,
                '3' => key::DIGIT_3,
                '4' => key::DIGIT_4,
                '5' => key::DIGIT_5,
                '6' => key::DIGIT_6,
                '7' => key::DIGIT_7,
                '8' => key::DIGIT_8,
                '9' => key::DIGIT_9,
                '-' => key::MINUS,
                '=' => key::EQUAL,
                ',' => key::COMMA,
                '.' => key::PERIOD,
                '/' => key::SLASH,
                ';' => key::SEMICOLON,
                '\'' => key::QUOTE,
                '[' => key::BRACKET_LEFT,
                ']' => key::BRACKET_RIGHT,
                '\\' => key::BACKSLASH,
                '`' => key::BACKQUOTE,
                _ => return None,
            }
        }
    };
    Some(code)
}

/// The title a pane wants its tab to show.
///
/// A shell usually sets one through OSC, but Windows shells often set the full
/// executable path, which is not a useful tab label. Only the final component is
/// kept, and the generic name is used when the shell says nothing.
fn tab_title(pane: &Pane) -> Option<String> {
    let title = pane.session.frame().title.trim();
    if title.is_empty() {
        return None;
    }

    let name = title
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(title)
        .trim_end_matches(".exe");

    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// What a pane is doing, as far as the sidebar can tell.
///
/// There is no agent detection here: a pane is working when it wrote something
/// in the last moment, idle when it has not, and exited when the process is
/// gone. That covers a shell, a build and a test watcher without asking any of
/// them to cooperate.
/// What a pane is doing. `Copy` so a row can ask for its colour and its label
/// without threading a reference through both.
#[derive(Clone, Copy)]
enum PaneStatus {
    Working,
    Idle,
    Exited,
}

impl PaneStatus {
    /// The text a row shows beside the name, if any.
    ///
    /// `Idle` is deliberately silent. It is what a pane is most of the time, so
    /// printing it on every row spends a column to say nothing — and a column
    /// that is usually the same word stops being read at all. The two states
    /// worth the pixels are the ones that differ from it.
    fn label(self) -> Option<&'static str> {
        match self {
            Self::Working => Some("输出中"),
            Self::Idle => None,
            Self::Exited => Some("已退出"),
        }
    }
}

fn pane_status(pane: &Pane) -> PaneStatus {
    match pane.session.exit_code() {
        Some(_) => PaneStatus::Exited,
        None if pane.session.idle_for() < Duration::from_millis(1200) => PaneStatus::Working,
        None => PaneStatus::Idle,
    }
}

/// The colour of a pane's state marker.
///
/// Focus is not part of this: a card's focus is already said by its name weight
/// and its outline, and letting focus recolour the state marker would mean the
/// marker stops describing the pane. The colour is the second line of the
/// encoding; the shape carries it on its own.
fn status_color(status: PaneStatus, theme: &gpui_kit::component::Theme) -> Hsla {
    match status {
        PaneStatus::Working => theme.ring,
        PaneStatus::Idle => theme.muted_foreground,
        PaneStatus::Exited => theme.danger,
    }
}

/// The marker itself: filled circle while writing, hollow circle while waiting,
/// square once the process is gone.
///
/// Shape first, colour second. A dot that only changed hue would leave `working`
/// and `idle` identical for a reader who cannot separate lime from grey — and
/// those are the two states a person checks most often.
fn status_glyph(status: PaneStatus, theme: &gpui_kit::component::Theme) -> impl IntoElement {
    let color = status_color(status, theme);

    div()
        .flex_shrink_0()
        .size(px(6.0))
        .when(matches!(status, PaneStatus::Idle), |this| {
            this.rounded(px(999.0)).border_1().border_color(color)
        })
        .when(matches!(status, PaneStatus::Working), |this| {
            this.rounded(px(999.0)).bg(color)
        })
        .when(matches!(status, PaneStatus::Exited), |this| {
            this.rounded(px(1.0)).bg(color)
        })
}

/// The directory a shell is working in, when it reports one.
///
/// There is no process inspection here: the shell's OSC title is the only thing
/// a terminal is told for free, and on Windows it is often the full path of the
/// executable rather than the working directory. That case is dropped — a bar
/// that prints the program's path next to the program's name has said where it
/// is zero times.
fn pane_cwd(pane: &Pane) -> Option<String> {
    // The announced directory is the real one; the title is only a guess that
    // happens to work for shells which title themselves with a path.
    if let Some(pwd) = pane.session.pwd() {
        return Some(shorten_path(&pwd));
    }

    let title = pane.session.frame().title.trim();
    if title.is_empty() || title.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let looks_like_path = title.contains('\\') || title.contains('/') || title.starts_with('~');
    if !looks_like_path {
        return None;
    }
    Some(shorten_path(title))
}

/// Replaces the user's home directory with `~`, which is how every shell prompt
/// already writes it.
fn shorten_path(path: &str) -> String {
    static HOME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| std::env::var("USERPROFILE").ok());
    match home {
        Some(home) if !home.is_empty() => {
            let lower = path.to_ascii_lowercase();
            let home_lower = home.to_ascii_lowercase();
            if lower.starts_with(&home_lower) {
                let rest = &path[home.len()..];
                return if rest.is_empty() {
                    "~".to_owned()
                } else {
                    format!("~{}", rest.replace('\\', "/"))
                };
            }
            path.replace('\\', "/")
        }
        _ => path.to_owned(),
    }
}

/// The one line of context a pane header carries: where the shell is, or the
/// title it set for itself when that title is not just its own path.
fn pane_meta(pane: &Pane, branches: &BTreeMap<String, Option<String>>) -> Option<String> {
    let Some(cwd) = pane_cwd(pane) else {
        return pane_detail(pane).filter(|detail| !detail.to_ascii_lowercase().ends_with(".exe"));
    };
    Some(match branches.get(&cwd).cloned().flatten() {
        // The branch rides with the directory, the way the prototype writes it:
        // `~/myterm · main`. It is the one line that tells two shells in the
        // same checkout apart from two shells in different ones.
        Some(branch) => format!("{cwd} · {branch}"),
        None => cwd,
    })
}

/// The branch a directory is on, when it is inside a git checkout.
///
/// This reads `.git/HEAD` and walks up a few levels, which is the whole of what
/// the pill in the bar needs. It does not shell out to git: spawning a process
/// every couple of seconds to paint one word is a poor trade, and a checkout
/// without git installed still has a HEAD file.
fn git_branch(directory: &str) -> Option<String> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let start = if let Some(rest) = directory.strip_prefix('~') {
        format!("{home}{rest}")
    } else {
        directory.to_owned()
    };

    let mut path = std::path::PathBuf::from(start.replace('/', "\\"));
    for _ in 0..8 {
        let head = path.join(".git").join("HEAD");
        if let Ok(text) = std::fs::read_to_string(&head) {
            let text = text.trim();
            if let Some(reference) = text.strip_prefix("ref:") {
                let name = reference.trim().rsplit('/').next().unwrap_or_default();
                return (!name.is_empty()).then(|| name.to_owned());
            }
            // A detached HEAD holds a bare commit id; the short form is what
            // every prompt shows for it.
            return Some(text.chars().take(7).collect());
        }
        if !path.pop() {
            break;
        }
    }
    None
}

/// The name a pane shows in its header.
///
/// The shell's OSC title when it sets one, otherwise the program it was started
/// with. Both end up with any path stripped, because a pane is named by what it
/// is, not by where it lives.
/// Anything the pane's title says beyond its name.
///
/// A shell sets the title to a path often enough that the difference is worth
/// showing; when it only repeats the program name, there is nothing to add and
/// the header keeps its space.
fn pane_detail(pane: &Pane) -> Option<String> {
    let title = pane.session.frame().title.trim();
    let name = pane_label(pane);

    (!title.is_empty() && title != name).then(|| title.to_owned())
}

fn pane_label(pane: &Pane) -> String {
    tab_title(pane).unwrap_or_else(|| {
        // `Path` handles both separators, which matters on Windows where a
        // program path arrives with backslashes.
        let program = &pane.command.program;
        let name = std::path::Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(program);
        name.trim_end_matches(".exe").to_owned()
    })
}

/// The grid size a freshly opened window should start with.
fn initial_grid(window: &Window) -> (u16, u16) {
    let bounds = window.bounds();
    // The first real resize corrects this; it only has to be large enough that
    // the shell starts with a usable width.
    let cols = (bounds.size.width.as_f32() / 8.0).max(20.0) as u16;
    let rows = (bounds.size.height.as_f32() / 18.0).max(6.0) as u16;
    (cols, rows)
}

/// The window and taskbar icon, decoded from the bundled PNG.
///
/// A missing or unreadable file leaves the window with the default icon rather
/// than refusing to start, since the icon is decoration and the shell is not.
fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../../../icon.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(image) => Some(std::sync::Arc::new(image.to_rgba8())),
        Err(error) => {
            log::warn!("could not decode the window icon: {error}");
            None
        }
    }
}

/// Applies the Mica material and the dark frame to the window.
///
/// Two things have to line up for the glass to read correctly. Mica takes its
/// tint from the window's light/dark mode, and GPUI decides that from the OS
/// setting at creation time, so a user on a light desktop running this dark
/// terminal would get a pale bar over a black grid. And `window_background` is
/// carried through `WindowOptions` but not applied by the Windows backend, so
/// the material has to be requested directly.
#[cfg(target_os = "windows")]
fn apply_window_material(window: &Window, light: bool) {
    use raw_window_handle::RawWindowHandle;
    use std::ffi::c_void;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    /// `DWMSBT_MAINWINDOW`: the Mica backdrop.
    const DWMSBT_MAINWINDOW: i32 = 2;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    // `Window` has its own `window_handle`, so the trait method is named in full.
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut c_void;

    // SAFETY: the handle comes from the live window and each value outlives its
    // call. A failure only costs the material or the frame tint, never the app.
    unsafe {
        let dark: i32 = if light { 0 } else { 1 };
        let status = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        if status != 0 {
            log::debug!("could not force the dark window frame: {status}");
        }

        let backdrop = DWMSBT_MAINWINDOW;
        let status = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        if status != 0 {
            log::debug!("could not apply the Mica backdrop: {status}");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_window_material(_window: &Window, _light: bool) {}

/// Whether the desktop is using the light theme.
///
/// Read before the event loop starts, because the first shell is spawned there
/// and needs the palette it will be drawn with. The registry key is the one
/// Settings writes; a missing key means light, which is the Windows default.
#[cfg(target_os = "windows")]
fn system_prefers_light() -> bool {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    // Both strings are NUL-terminated and live for the whole call.
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();

    let mut data: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffers are sized as declared and outlive the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut c_void,
            &mut size,
        )
    };

    status != 0 || data != 0
}

#[cfg(not(target_os = "windows"))]
fn system_prefers_light() -> bool {
    true
}

/// The font the terminal grid is drawn with.
///
/// Maple Mono NF CN is a programming face with CJK coverage and Nerd Font
/// glyphs, so box drawing, powerline separators and Chinese text all come from
/// one font instead of falling back mid-cell.
const TERMINAL_FONT: &str = "Maple Mono NF CN";

/// Points the theme's monospace family at [`TERMINAL_FONT`].
///
/// Both registered theme configs are rewritten, not just the live family: a
/// mode change re-applies the config, and a config that says nothing about the
/// mono family would put the old one back.
/// Applies the terminal's font family, and its size when one was chosen.
///
/// Kept in one place because a theme change re-reads the library's theme file:
/// anything set here has to be re-applied after every mode switch, or the grid
/// drifts back to the library's monospace.
fn apply_terminal_font(cx: &mut App, size: Option<f32>) {
    gpui_kit::component::Theme::update(cx, |theme| {
        // The library's theme files store the mono size as points, while the
        // resolved theme carries pixels. Both are written here so the two
        // cannot disagree after a mode change.
        let mut dark = (*theme.dark_theme).clone();
        dark.mono_font_family = Some(TERMINAL_FONT.into());
        if let Some(size) = size {
            dark.mono_font_size = Some(size);
        }
        theme.dark_theme = std::rc::Rc::new(dark);

        let mut light = (*theme.light_theme).clone();
        light.mono_font_family = Some(TERMINAL_FONT.into());
        if let Some(size) = size {
            light.mono_font_size = Some(size);
        }
        theme.light_theme = std::rc::Rc::new(light);

        theme.mono_font_family = TERMINAL_FONT.into();
        if let Some(size) = size {
            theme.mono_font_size = gpui_kit::px(size);
        }
    });
}

fn main() {
    env_logger::init();

    // `--uninstall` is what the registry entry points at. It runs the same
    // removal the about panel does, without opening a window: a user who
    // uninstalls from Settings → Apps expects the program to disappear, not to
    // flash a terminal first.
    if std::env::args().any(|arg| arg == "--uninstall") {
        match install::uninstall() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                log::error!("uninstall failed: {error}");
                std::process::exit(1);
            }
        }
    }

    // The first shell is started before the event loop exists. A pseudo console
    // takes about 50 ms to come up, and `App::run` keeps the app borrowed for
    // all of `on_finish_launching`, so doing this later re-enters the borrow as
    // soon as the window asks for its first frame.
    // A saved preference wins over the desktop's setting. Both are resolved
    // before the loop starts, because the first shell has to be painted in the
    // colors the window will use.
    let saved = config::Config::load();
    let light = saved.light.unwrap_or_else(system_prefers_light);
    let command = build_command(saved.shell, saved.shell_integration);

    // The pane needs the palette the window will end up with, and the theme is
    // only installed later in `gpui_kit::init`, so the colors are resolved
    // directly from the theme crate here.
    let mut theme = gpui_kit::component::Theme::default();
    theme.colors = if light {
        *gpui_kit::component::ThemeColor::light()
    } else {
        *gpui_kit::component::ThemeColor::dark()
    };
    theme.mode = if light {
        gpui_kit::component::ThemeMode::Light
    } else {
        gpui_kit::component::ThemeMode::Dark
    };
    let (mut first_pane, startup_error) = match Pane::new(&command, 120, 40, &theme) {
        Ok(pane) => (Some(pane), None),
        Err(message) => (None, Some(message)),
    };

    let font_size = saved.font_size;
    let workspace_config = saved;

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);

            apply_terminal_font(cx, font_size);

            // The app follows the desktop's light/dark setting, resolved before
            // the loop started so the first shell could be painted to match.
            let mode = if light {
                gpui_kit::component::ThemeMode::Light
            } else {
                gpui_kit::component::ThemeMode::Dark
            };
            palette::install(cx, mode, workspace_config.accent);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(160.0), px(120.0)),
                    size(px(1100.0), px(720.0)),
                ))),
                // The title bar is drawn by the app so the tabs can live in it,
                // which is what makes a terminal feel like one surface instead of
                // a window with a strip bolted on top.
                titlebar: Some(TitlebarOptions {
                    title: Some("myterm".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                icon: window_icon(),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, move |window, cx| {
                // The window exists now, so the frame can be told what theme it is
                // drawing for.
                apply_window_material(window, light);

                let pane = first_pane.take();
                let error = startup_error.clone();
                let settings = workspace_config.clone();
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx, settings);
                    match pane {
                        Some(pane) => workspace.adopt(pane),
                        None => workspace.error = error,
                    }
                    workspace
                })
            })
            .expect("failed to open window");
        });
}
