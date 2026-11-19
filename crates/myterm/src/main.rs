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

mod app_helpers;
mod config;
mod grid_element;
mod install;
mod layout;
mod palette;
mod pane_view;
mod pty_command;
mod settings;
mod terminal;
mod ui_sidebar;
mod update;
mod workspace_actions;
mod workspace_chrome;
mod workspace_geometry;
mod workspace_overlays;
mod workspace_runtime;
mod workspace_search;
mod workspace_tabs;
mod workspace_view;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::IndexPath;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use app_helpers::*;
use grid_element::{GridStyle, ImageCache, TerminalGrid};
use layout::{Axis, Direction, Layout, Rect};
use pane_view::*;
use pty::{Command, Pty};
use settings::ShellChoice;
use terminal::Session;
use ui_sidebar::*;
use vt::Rgb;
use workspace_geometry::*;

/// The shell to start, wrapped for shell integration when it is on.
fn build_command(
    shell: config::Shell,
    integration: bool,
    wsl_distribution: Option<&str>,
) -> Command {
    let command = pty_command::profile_for_distribution(shell, wsl_distribution);
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
const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"#;
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
const CHECK_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 4 4L19 6"/></svg>"#;

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
const PANE_HEADER_HEIGHT: f32 = 35.0;

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
const GRID_PAD_TOP: f32 = 3.0;
const GRID_PAD_BOTTOM: f32 = 14.0;

/// One press of the font size control. Half a point, so the sizes people
/// actually want — 12.5, 13.5 — are reachable.
const FONT_STEP: f32 = 0.5;

/// One press of the scrollback control.
const SCROLLBACK_STEP: usize = 1_000;

/// Width of the workspace list.
const SIDEBAR_WIDTH: f32 = 216.0;

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
    /// Stable identity: sidebar menus and editors outlive list reordering.
    id: u64,
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
        id: u64,
        pty: Pty,
        command: &Command,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Result<Self, String> {
        let session = Session::from_pty(pty, cols, rows)?;
        Ok(Self::from_pane(
            id,
            Pane::from_session(session, command, cols, rows, theme),
        ))
    }

    fn from_pane(id: u64, pane: Pane) -> Self {
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);

        Self {
            id,
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

    /// Adds a pane beside the original split target once the console is ready.
    #[allow(clippy::too_many_arguments)]
    fn split_with_pty(
        &mut self,
        source: u64,
        axis: Axis,
        pty: Pty,
        command: &Command,
        cols: u16,
        rows: u16,
        theme: &gpui_kit::component::Theme,
    ) -> Result<(), String> {
        let session = Session::from_pty(pty, cols, rows)?;
        let pane = Pane::from_session(session, command, cols, rows, theme);

        let id = self.next_id;
        self.next_id += 1;
        self.panes.insert(id, pane);

        if !self.layout.split(source, id, axis) {
            self.panes.remove(&id);
            return Err("无法拆分窗格".to_owned());
        }

        if self.active == source {
            self.active = id;
        }
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
    tab_id: u64,
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
    tab_id: u64,
    input: Entity<InputState>,
    _sub: Subscription,
    invalid: bool,
}

/// The workspace: tabs, panes, and the transient state of a pointer gesture.
pub struct Workspace {
    tabs: Vec<Tab>,
    active_tab: usize,
    next_tab_id: u64,
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
    folded_tabs: std::collections::BTreeSet<u64>,
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
    /// Stateful dropdown for the default shell and WSL distribution.
    shell_selector: Entity<SelectState<SearchableVec<ShellChoice>>>,
    _shell_selector_subscription: Subscription,
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
            if self.focus.is_focused(window) {
                window.invalidate_character_coordinates();
            }
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
            // One gradient continues behind the sidebar and title bar.
            .bg(palette::workspace_background(
                theme.mode,
                self.config.accent,
            ))
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

        if self.sidebar_width > 0.0 {
            body = body.child(self.render_sidebar(&theme, cx));
        }

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
            div().flex_1().min_w_0().min_h_0().flex().flex_col().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .pl(px(if self.sidebar_width == 0.0 {
                        PANE_GAP
                    } else {
                        0.0
                    }))
                    .pr(px(PANE_GAP))
                    .pb(px(PANE_GAP))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .child(self.render_panes(tab, style, cx)),
                    )
                    // The sheet is the work area's last column, not an
                    // overlay: the panes keep their full height and their
                    // rounded corners, and a font size can be tried on while
                    // watching the grid change beside the control.
                    .when(self.settings, |this| {
                        this.child(self.render_settings(&theme, cx))
                    }),
            ),
        );

        // The status bar is not furniture. It appears only while it has a line
        // to say — a zoom to release, an update to take — and costs nothing the
        // rest of the time. A permanent 26 px strip repeating what the bar above
        // already says is a row of chrome bought for an occasional glance.
        if let Some(hint) = self.status_hint() {
            content = content.child(self.render_status_bar(&hint, &theme, cx));
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
                let uses_theme_background = frame.background == to_rgb(style.background);
                let style = GridStyle {
                    background: grid_element::rgb(frame.background),
                    ..style
                };
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
                    // The default surface is the single window gradient.
                    // Honor a per-session background set by the terminal.
                    .when(!uses_theme_background, |this| this.bg(style.background))
                    .when(focused && multi && !zoomed, |this| {
                        this.child(pane_focus_edge(theme))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |workspace, event: &MouseDownEvent, window, cx| {
                            workspace.focus.focus(window, cx);
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

                {
                    // While a divider is being dragged the header lends its
                    // trailing space to the numbers that are changing, because
                    // that is the question the drag is asking.
                    let live_size = self.resize_drag.is_some().then(|| dims.get());
                    card = card.child(pane_header(
                        id,
                        ordinal,
                        name,
                        pty_command::display_name(&pane.command),
                        meta,
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
                        .pt(px(GRID_PAD_TOP))
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

/// One row of a menu: the same row every menu in the app uses.
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
    let command = build_command(
        saved.shell,
        saved.shell_integration,
        saved.wsl_distribution.as_deref(),
    );

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
    palette::apply(&mut theme, saved.accent);
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
