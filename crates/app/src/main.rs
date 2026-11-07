//! The application shell: a single window with a tab strip, one or more panes,
//! and the key handling that drives them.

mod grid_element;
mod layout;
mod palette;
mod pty_command;
mod terminal;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;

use grid_element::{GridStyle, TerminalGrid};
use layout::{Axis, Direction, Layout, Rect};
use pty::{Command, Pty};
use terminal::Session;
use vt::Rgb;

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

/// The sidebar toggle: a column beside a page.
const SIDEBAR_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9.5 4v16"/></svg>"#;

const ZOOM_IN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V5.5A1.5 1.5 0 0 1 5.5 4H9"/><path d="M15 4h3.5A1.5 1.5 0 0 1 20 5.5V9"/><path d="M20 15v3.5a1.5 1.5 0 0 1-1.5 1.5H15"/><path d="M9 20H5.5A1.5 1.5 0 0 1 4 18.5V15"/></svg>"#;
const ZOOM_OUT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4v3.5A1.5 1.5 0 0 1 7.5 9H4"/><path d="M20 9h-3.5A1.5 1.5 0 0 1 15 7.5V4"/><path d="M15 20v-3.5a1.5 1.5 0 0 1 1.5-1.5H20"/><path d="M4 15h3.5A1.5 1.5 0 0 1 9 16.5V20"/></svg>"#;

const WINDOW_CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>"#;
const WINDOW_RESTORE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><rect x="4" y="8" width="12" height="12" rx="1.5"/><path d="M8 8V5.5A1.5 1.5 0 0 1 9.5 4h9A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H16"/></svg>"#;

/// The app mark in the title bar: a prompt caret inside a window frame.
///
/// Drawn inline because several icons from the bundled set render blank on this
/// platform, and the one glyph that identifies the app is the worst place to
/// gamble on that.
const APP_MARK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7 9.5l3 2.5-3 2.5"/><path d="M12.5 15h4.5"/></svg>"#;

/// The theme toggle: a sun in light mode, a moon in dark mode.
const THEME_SUN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4.5"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4"/></svg>"#;
const THEME_MOON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z"/></svg>"#;

/// Height of the drawn title bar.
///
/// 38 px matches the caption Windows draws for itself, so the tabs sit where
/// the eye expects a title to be rather than where a toolbar would go.
const TITLE_BAR_HEIGHT: f32 = 38.0;

/// Height of the strip naming a pane.
const PANE_HEADER_HEIGHT: f32 = 32.0;

/// Corner radius of a pane card.
const PANE_RADIUS: f32 = 12.0;

/// Width of the pane list.
const SIDEBAR_WIDTH: f32 = 214.0;

/// Builds the 256-entry palette the parser should report: the ANSI sixteen
/// followed by the library's defaults for the rest.
fn palette(mode: gpui_kit::component::ThemeMode) -> [Rgb; 256] {
    let ansi = palette::Palette::for_mode(mode).ansi();
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
    /// Filled by the grid element during prepaint with the cells that fit.
    dims: Rc<Cell<(u16, u16)>>,
    /// Where the grid landed in window coordinates, filled by the same prepaint.
    /// The IME needs it to place its candidate window, the mouse needs it to
    /// resolve a pointer to a cell, and a split drag needs the extent to turn a
    /// pointer position into a ratio.
    bounds: Rc<Cell<Bounds<Pixels>>>,
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
        Self {
            session,
            command: command.clone(),
            grid: (cols, rows),
            dims: Rc::new(Cell::new((cols, rows))),
            bounds: Rc::new(Cell::new(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(0.0), px(0.0)),
            ))),
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
    title: String,
    /// Panes keyed by the id the layout tree refers to. A map rather than a vec
    /// so removing a pane never invalidates the others' identity.
    panes: BTreeMap<u64, Pane>,
    layout: Layout,
    active: u64,
    /// The id the next pane in this tab will take.
    next_id: u64,
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
            panes,
            layout: Layout::Leaf(1),
            active: 1,
            next_id: 2,
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
    /// What the user asked for. `None` means "decide by how much there is to
    /// list", which is what keeps a single terminal free of a list of one.
    sidebar_choice: Option<bool>,
    /// The split being dragged, as the two panes on either side plus the span
    /// the divider travels across. Present only while the divider is held.
    resize_drag: Option<ResizeDrag>,
    focus: FocusHandle,
}

impl Workspace {
    /// Builds an empty workspace. The caller installs the first tab, because its
    /// shell is spawned before the event loop exists.
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            command: pty_command::default_profile(),
            ticking: false,
            error: None,
            cached_style: GridStyle::default(),
            ime_cursor: (0, 0),
            caret_moved: Instant::now(),
            caret_visible: true,
            drag_anchor: None,
            selection: false,
            zoomed: None,
            sidebar_choice: None,
            resize_drag: None,
            focus: cx.focus_handle(),
        }
    }

    /// Installs the pane spawned before start-up as the first tab.
    fn adopt(&mut self, pane: Pane) {
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        self.tabs = vec![Tab {
            title: "终端".to_owned(),
            panes,
            layout: Layout::Leaf(1),
            active: 1,
            next_id: 2,
        }];
    }

    /// Starts polling the shells, and focuses the grid once it exists.
    fn ensure_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_ticking(cx);

        if !self.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
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
                let changed = match this.update(cx, |workspace, _| workspace.poll()) {
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
        let visible = (elapsed.as_millis() / PERIOD.as_millis()) % 2 == 0;
        if visible != self.caret_visible {
            self.caret_visible = visible;
            true
        } else {
            false
        }
    }

    /// Drains every pane. Returns true when a frame changed.
    fn poll(&mut self) -> bool {
        let mut changed = self.advance_caret_blink();
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                if pane.session.poll() {
                    changed = true;
                }
            }
            // The shell announces its directory and command through OSC
            // sequences; adopting the title it sets keeps the tab honest.
            if let Some(pane) = tab.active_pane()
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
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                let dims = pane.dims.get();
                if pane.grid != dims {
                    pane.grid = dims;
                    pane.session.resize(
                        dims.0,
                        dims.1,
                        style.cell_width.as_f32().round() as u32,
                        style.cell_height.as_f32().round() as u32,
                    );
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

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        cx.notify();
    }

    /// Scrolls the focused terminal's viewport.
    ///
    /// A wheel notch is three lines; a precise delta is divided by the cell
    /// height so a trackpad feels the same as a mouse.
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
        if mods.control && mods.shift {
            match keystroke.key.as_str() {
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
                "b" => {
                    self.toggle_sidebar(cx);
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

        if mods.control && keystroke.key == "c" && has_selection(self) {
            self.copy_selection(cx);
            cx.stop_propagation();
            return;
        }

        if mods.control && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if let Some(pane) = self.active_pane_mut() {
                    pane.session.write(text.as_bytes());
                }
                cx.notify();
            }
            cx.stop_propagation();
            return;
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
    /// The span the divider travels is measured from the two panes' recorded
    /// bounds rather than from the split's own box, because only the panes know
    /// where they were actually painted.
    fn begin_resize(
        &mut self,
        left: u64,
        right: u64,
        axis: Axis,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(l), Some(r)) = (tab.panes.get(&left), tab.panes.get(&right)) else {
            return;
        };
        let (lb, rb) = (l.bounds.get(), r.bounds.get());

        let (start, span) = match axis {
            Axis::Horizontal => (
                position.x.as_f32(),
                rb.right().as_f32() - lb.left().as_f32(),
            ),
            Axis::Vertical => (
                position.y.as_f32(),
                rb.bottom().as_f32() - lb.top().as_f32(),
            ),
        };
        if span <= 0.0 {
            return;
        }

        let ratio = match axis {
            Axis::Horizontal => (position.x.as_f32() - lb.left().as_f32()) / span,
            Axis::Vertical => (position.y.as_f32() - lb.top().as_f32()) / span,
        };

        self.resize_drag = Some(ResizeDrag {
            left,
            right,
            axis,
            start,
            span,
            ratio: ratio.clamp(0.1, 0.9),
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
        let ratio = (drag.ratio + (now - drag.start) / drag.span).clamp(0.1, 0.9);
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
        // A drag leaves a real selection behind, so copy exactly that. With no
        // selection the old behaviour is kept: copy the whole screen, which is
        // what people reach for when they want the output they just ran.
        let text = match self.selection {
            true => self
                .active_pane_mut()
                .and_then(|pane| pane.session.selected_text()),
            false => self
                .active_pane_mut()
                .and_then(|pane| pane.session.select_all_text()),
        };
        if let Some(text) = text {
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

        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            // The body is deliberately left unpainted. The title bar wants the
            // Mica backdrop to show through it, and the grid paints its own
            // opaque background, so a fill here would only hide the material
            // and make the bar look flat.
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::handle_key))
            // A divider drag has to keep tracking once the pointer leaves the
            // few pixels of the grip, so the move and release are handled at the
            // root, which is under the pointer for the whole gesture.
            .on_mouse_move(cx.listener(|workspace, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some(drag) = &workspace.resize_drag
                {
                    let axis = drag.axis;
                    workspace.extend_resize(axis, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _, _| {
                    workspace.resize_drag = None;
                }),
            );

        body = body.child(self.render_tabs(window, cx));

        // The list and the panes sit side by side under the bar, so the window
        // still has exactly one title row.
        let mut content = div().flex_1().min_h_0().flex().flex_row();
        if self.sidebar_shown() {
            content = content.child(self.render_sidebar(&theme, cx));
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
            return body.child(content);
        };

        // The pane area is chrome, not grid: the space between panes shows this
        // tone, so the panes read as cards laid on a surface rather than as
        // rectangles that happen to stop.
        content = content.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .p(px(10.0))
                .bg(theme.title_bar)
                .child(self.render_panes(tab, style, cx)),
        );

        body.child(content)
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
        // the zoom puts every pane back exactly where it was.
        if let Some(id) = self.zoomed {
            if tab.panes.contains_key(&id) {
                return self
                    .render_node(&Layout::Leaf(id), tab, style, &theme, multi, id, cx)
                    .into_any_element();
            }
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
                let zoomed = self.zoomed == Some(id);
                let show_header = multi || zoomed;

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
                    .bg(style.background)
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
                    card = card.child(pane_header(
                        id,
                        name,
                        focused,
                        zoomed,
                        exit.is_some(),
                        &theme,
                        cx,
                    ));
                }

                // The terminal takes the rest of the card. Insetting it keeps the
                // text off the rounded corners and gives the block cursor a
                // margin at the edge of the pane.
                card = card.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .mx(px(6.0))
                        .mb(px(6.0))
                        .when(!show_header, |this| this.mt(px(6.0)))
                        .child(TerminalGrid::new(
                            ElementId::Name(format!("grid-{id}").into()),
                            frame,
                            style,
                            focused,
                            dims,
                            bounds_cell,
                            cx.entity(),
                            self.focus.clone(),
                        ))
                        // A pane the user is not in recedes: the card's surface
                        // lifts and the text fades behind a veil, so attention
                        // follows the caret without a border having to shout.
                        .when(multi && !focused, |this| {
                            this.child(div().absolute().inset_0().bg(theme.background.opacity(
                                if theme.mode == gpui_kit::component::ThemeMode::Light {
                                    0.3
                                } else {
                                    0.42
                                },
                            )))
                        }),
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

                // The ratio is applied as a flex basis so the engine distributes
                // the remaining space; a literal width would not account for a
                // neighbouring split.
                let first_share = (ratio.clamp(0.1, 0.9) * 1000.0).round();
                let second_share = 1000.0 - first_share;

                // The divider is a real flex child rather than a gap: a gap
                // cannot be grabbed, and splitting a window by eye is the one
                // adjustment a terminal user makes constantly. It stays one
                // pixel wide, but a wider transparent area around it gives the
                // pointer something to hit.
                let divider = divider_element(*axis, first, second, theme, cx);

                // The wrapper carries the basis, and it is itself a flex container so
                // the pane inside can fill it. Without that the pane has no box to
                // stretch into and collapses to nothing.
                container
                    .child(
                        div()
                            .flex()
                            .flex_basis(relative(first_share / 1000.0))
                            .min_w_0()
                            .min_h_0()
                            .child(first_el),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex()
                            .flex_basis(relative(second_share / 1000.0))
                            .min_w_0()
                            .min_h_0()
                            .child(second_el),
                    )
                    .into_any_element()
            }
        }
    }

    /// Closes the focused pane, or the tab when it is the last one.
    fn close_pane(&mut self, cx: &mut Context<Self>) {
        let last_in_tab = self
            .active_tab()
            .map(|tab| tab.panes.len() == 1)
            .unwrap_or(true);

        if last_in_tab {
            self.close_tab(self.active_tab, cx);
            return;
        }

        if let Some(tab) = self.active_tab_mut() {
            tab.close_active();
        }
        cx.notify();
    }

    /// Moves focus to `id`.
    fn focus_pane(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_tab_mut()
            && tab.panes.contains_key(&id)
        {
            tab.active = id;
            cx.notify();
        }
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
        palette::install(cx, next);

        let theme = gpui_kit::component::Theme::global(cx).clone();
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                pane.retheme(&theme);
            }
        }
        cx.notify();
    }

    /// Closes the window, which ends the app.
    ///
    /// Every shell is a child of this process, so dropping the workspace on the
    /// way out takes the whole tree with it rather than leaving orphaned
    /// consoles behind.
    fn close_window(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.tabs.clear();
        window.remove_window();
    }
}

impl Workspace {
    /// Whether the sidebar is on screen.
    ///
    /// With no explicit choice it follows how much there is to list: a list of
    /// one is not worth a column, and a window full of panes is worth one even
    /// if nobody asked. An explicit toggle wins either way.
    fn sidebar_shown(&self) -> bool {
        let panes: usize = self.tabs.iter().map(|tab| tab.panes.len()).sum();
        self.sidebar_choice.unwrap_or(panes > 1)
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_choice = Some(!self.sidebar_shown());
        cx.notify();
    }

    /// Every tab and pane, with enough state to jump to one.
    ///
    /// This is the whole model the sidebar reads: it is rebuilt from the
    /// workspace each frame rather than kept in step with it, so there is
    /// nothing to keep in sync.
    fn render_sidebar(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div().flex().flex_col().gap(px(1.0)).px(px(6.0)).pb(px(8.0));

        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let current_tab = tab_index == self.active_tab;

            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(26.0))
                    .px(px(7.0))
                    .mt(px(4.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(if current_tab {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.muted_foreground)
                            .child(tab.panes.len().to_string()),
                    ),
            );

            for (id, pane) in &tab.panes {
                let id = *id;
                let selected = current_tab && id == tab.active;
                let status = pane_status(pane);
                // Captured by the listeners below, which outlive this borrow.
                let hover_bg = theme.muted;
                let selected_bg = theme.background;

                list = list.child(
                    div()
                        .id(("side-pane", id))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(7.0))
                        .h(px(28.0))
                        .px(px(8.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(selected_bg))
                        .when(!selected, |this| this.hover(move |this| this.bg(hover_bg)))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.active_tab = tab_index;
                            workspace.focus_pane(id, cx);
                        }))
                        .child(div().size(px(6.0)).rounded(px(999.0)).bg(match status {
                            PaneStatus::Working => theme.ring,
                            PaneStatus::Idle => theme.border,
                            PaneStatus::Exited => theme.danger,
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.0))
                                .text_color(if selected {
                                    theme.foreground
                                } else {
                                    theme.muted_foreground
                                })
                                .child(pane_label(pane)),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(theme.muted_foreground.opacity(0.85))
                                .child(status.label()),
                        ),
                );
            }
        }

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(TITLE_BAR_HEIGHT))
            .pl(px(13.0))
            .pr(px(6.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.muted_foreground)
                    .child("窗格"),
            )
            .child(
                icon_button("side-new", IconName::Plus, theme, cx).on_click(cx.listener(
                    |workspace, _: &ClickEvent, window, cx| {
                        workspace.new_tab(window, cx);
                    },
                )),
            );

        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.title_bar)
            .border_r_1()
            .border_color(theme.border)
            .child(header)
            .child(
                div()
                    .id("side-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }

    /// Draws the title bar: window drag area, tabs, and window controls.
    ///
    /// This is the whole title bar, not a strip under one. The tabs live where
    /// the OS caption would be, which is what lets a terminal read as a single
    /// surface. Two things therefore have to be true at once: the empty areas
    /// must drag the window, and the buttons must not. The drag area is a
    /// `WindowControlArea::Drag` hitbox behind the content, and the tabs and
    /// buttons sit on top of it and swallow their own clicks.
    fn render_tabs(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let active = self.active_tab;
        let maximized = window.is_maximized();

        // The bar is chrome: one step warmer than the grid, flat rather than
        // glossy, and separated from the panes by a single hairline. The app mark
        // is the only thing allowed to wear the accent colour at rest, which is
        // what makes it read as the mark rather than another button.
        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(TITLE_BAR_HEIGHT))
            .relative()
            .pl(px(10.0))
            .pr(px(6.0))
            .gap(px(6.0))
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .window_control_area(WindowControlArea::Drag)
                    .child(icon_data(APP_MARK, theme.ring)),
            );

        // Tabs sit close together in a group, the way a pill row reads; the
        // spacing between them is smaller than the spacing to anything else.
        let mut tabs = div().flex().flex_row().items_center().gap(px(4.0));
        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = index == active;
            let title = tab.title.clone();
            let pane_count = tab.panes.len();
            let closable = self.tabs.len() > 1;

            tabs = tabs.child(tab_chip(
                index, title, pane_count, selected, closable, &theme, cx,
            ));
        }
        tabs = tabs.child(icon_button("new-tab", IconName::Plus, &theme, cx).on_click(
            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.new_tab(window, cx);
            }),
        ));
        strip = strip.child(tabs);

        // Pane actions act on the current tab rather than on the tab list, so
        // they are pushed to the far edge, away from the tabs they do not touch.
        strip = strip
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            // The list comes before the pane actions: it is a place to go, not
            // an action on the current pane.
            .child(
                icon_button("toggle-sidebar", Glyph::Inline(SIDEBAR_ICON), &theme, cx)
                    .when(self.sidebar_shown(), |this| this.bg(theme.muted))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_sidebar(cx);
                    })),
            )
            .child(
                icon_button("split-h", Glyph::Inline(SPLIT_HORIZONTAL), &theme, cx).on_click(
                    cx.listener(|workspace, _: &ClickEvent, window, cx| {
                        workspace.split_active(Axis::Horizontal, window, cx);
                    }),
                ),
            )
            .child(
                icon_button("split-v", Glyph::Inline(SPLIT_VERTICAL), &theme, cx).on_click(
                    cx.listener(|workspace, _: &ClickEvent, window, cx| {
                        workspace.split_active(Axis::Vertical, window, cx);
                    }),
                ),
            )
            // The theme toggle shows the theme it switches to, not the one in
            // use, which is the convention every app with this control follows.
            .child(
                icon_button(
                    "toggle-theme",
                    if theme.mode == gpui_kit::component::ThemeMode::Light {
                        Glyph::Inline(THEME_MOON)
                    } else {
                        Glyph::Inline(THEME_SUN)
                    },
                    &theme,
                    cx,
                )
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.toggle_theme(cx);
                })),
            )
            // A gap wide enough that the caption buttons read as window controls
            // and not as more of the app's own toolbar.
            .child(div().w(px(10.0)));

        strip
            .child(window_button(
                "win-min",
                Glyph::Inline(WINDOW_MINIMIZE),
                WindowControlArea::Min,
                false,
                &theme,
                cx.listener(|workspace, _: &ClickEvent, window, cx| {
                    window.minimize_window();
                    let _ = workspace;
                    let _ = cx;
                }),
            ))
            .child(window_button(
                "win-max",
                if maximized {
                    Glyph::Inline(WINDOW_RESTORE)
                } else {
                    Glyph::Inline(WINDOW_MAXIMIZE)
                },
                WindowControlArea::Max,
                false,
                &theme,
                cx.listener(|workspace, _: &ClickEvent, window, cx| {
                    window.zoom_window();
                    let _ = workspace;
                    let _ = cx;
                }),
            ))
            .child(window_button(
                "win-close",
                Glyph::Inline(WINDOW_CLOSE),
                WindowControlArea::Close,
                true,
                &theme,
                cx.listener(|workspace, _: &ClickEvent, window, cx| {
                    workspace.close_window(window, cx);
                }),
            ))
    }
}

/// The draggable gutter between two panes of a split.
///
/// The panes are separated by a strip of chrome rather than a hairline. A
/// terminal paints its own background, so a one-pixel rule between two panes of
/// the same colour is nearly invisible; a strip of the title bar's tone reads as
/// a deliberate division and makes the two panes feel like separate surfaces.
///
/// It is wide enough to grab, which a hairline is not, and it brightens on hover
/// to say it can be moved. The two panes on either side identify which split is
/// being resized, which is what keeps a nested split pointing at its own gutter.
fn divider_element(
    axis: Axis,
    first: &Layout,
    second: &Layout,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let left = first.leaves().last().copied();
    let right = second.leaves().first().copied();

    const GUTTER: f32 = 10.0;

    let mut grip = div().flex_shrink_0().bg(theme.title_bar);

    grip = match axis {
        Axis::Horizontal => grip.w(px(GUTTER)).h_full().cursor_col_resize(),
        Axis::Vertical => grip.h(px(GUTTER)).w_full().cursor_row_resize(),
    };

    // The gutter is the same tone as the surface behind it, so it is invisible
    // until the pointer finds it and the accent says it can be moved.
    grip.hover(move |this| this.bg(theme.ring.opacity(0.3)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                if let (Some(left), Some(right)) = (left, right) {
                    workspace.begin_resize(left, right, axis, event.position, cx);
                }
            }),
        )
        .into_any_element()
}

/// A small control in a pane header.
/// The strip at the top of a pane: what it is running, and what can be done to
/// it.
///
/// It only appears once the window is divided. A lone terminal is already named
/// by the tab above it, and repeating that name in a second bar would spend
/// forty pixels saying nothing.
fn pane_header(
    id: u64,
    name: String,
    focused: bool,
    zoomed: bool,
    exited: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let dot = if exited {
        theme.danger
    } else if focused {
        theme.ring
    } else {
        theme.border
    };

    div()
        .flex_shrink_0()
        .h(px(PANE_HEADER_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .pl(px(11.0))
        .pr(px(5.0))
        .child(div().size(px(6.0)).rounded(px(999.0)).bg(dot))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(if focused {
                    FontWeight::BOLD
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
        .child(div().flex_1())
        .child(
            // The pane's own actions. They fade in with the pointer, the way a
            // hover toolbar does, so the header is a label at rest and a set of
            // controls when it is being used.
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(1.0))
                .opacity(0.0)
                .group_hover("pane", |this| this.opacity(1.0))
                .child(pane_action(
                    "pane-split-right",
                    SPLIT_HORIZONTAL,
                    theme,
                    cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.split_pane(id, Axis::Horizontal, window, cx);
                    }),
                ))
                .child(pane_action(
                    "pane-split-down",
                    SPLIT_VERTICAL,
                    theme,
                    cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.split_pane(id, Axis::Vertical, window, cx);
                    }),
                ))
                .child(pane_action(
                    "pane-zoom",
                    if zoomed { ZOOM_OUT } else { ZOOM_IN },
                    theme,
                    cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_zoom(id, cx);
                    }),
                ))
                .child(pane_action(
                    "pane-close",
                    WINDOW_CLOSE,
                    theme,
                    cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        workspace.close_pane_at(id, cx);
                    }),
                )),
        )
}

/// A small split control floating inside a pane.
///
/// Smaller than [`icon_button`] and tinted like chrome rather than like content,
/// because it covers terminal text: it has to be reachable without becoming part
/// of what the terminal is saying.
fn pane_action(
    id: &'static str,
    icon: &'static [u8],
    theme: &gpui_kit::component::Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.0))
        .rounded(px(999.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(move |this| this.bg(theme.muted))
        .on_click(on_click)
        .child(icon_data(icon, theme.muted_foreground))
}

/// One tab: name, pane count, and a close affordance that only appears when
/// there is more than one tab to fall back to.
fn tab_chip(
    index: usize,
    title: String,
    pane_count: usize,
    selected: bool,
    closable: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let text_color = if selected {
        theme.foreground
    } else {
        theme.muted_foreground
    };

    let mut chip = div()
        .id(("tab", index))
        .group("tab")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .pl(px(11.0))
        .pr(if closable { px(5.0) } else { px(11.0) })
        .rounded(px(999.0))
        .when(selected, |this| this.bg(theme.background))
        .text_sm()
        .text_color(text_color)
        .cursor_pointer()
        .hover(move |this| {
            // Hovering an inactive tab previews what selecting it does; hovering
            // the active tab must not lighten it, which would read as a second
            // state it does not have.
            if selected {
                this
            } else {
                this.bg(theme.muted).text_color(theme.foreground)
            }
        })
        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
            workspace.active_tab = index;
            cx.notify();
        }))
        .child(
            div()
                .max_w(px(180.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        );

    // The pane count is a quiet badge rather than a number floating after the
    // name: it is a property of the tab, not part of its title.
    if pane_count > 1 {
        chip = chip.child(
            div()
                .px(px(6.0))
                .rounded(px(999.0))
                .bg(theme.muted)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(pane_count.to_string()),
        );
    }

    if closable {
        chip = chip.child(
            div()
                .id(("close-tab", index))
                .flex()
                .items_center()
                .justify_center()
                .size(px(18.0))
                .rounded(px(999.0))
                .opacity(0.0)
                .group_hover("tab", |this| this.opacity(1.0))
                .hover(|this| this.bg(theme.muted))
                .child(
                    gpui_kit::svg()
                        .path(IconName::X.path())
                        .size(px(12.0))
                        .text_color(theme.muted_foreground),
                )
                .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    workspace.close_tab(index, cx);
                })),
        );
    }

    chip
}

/// A glyph a toolbar button can draw.
enum Glyph {
    /// A path into the bundled icon assets.
    Asset(SharedString),
    /// SVG bytes, for shapes the bundled set does not render reliably.
    Inline(&'static [u8]),
}

impl From<IconName> for Glyph {
    fn from(name: IconName) -> Self {
        Self::Asset(name.path())
    }
}

/// A round icon-only button, sized for the title bar.
///
/// Icon-only controls carry no visible label, so the caller must pass an id that
/// also reads as a name.
fn icon_button(
    id: &'static str,
    icon: impl Into<Glyph>,
    theme: &gpui_kit::component::Theme,
    _cx: &mut Context<Workspace>,
) -> Stateful<Div> {
    let color = theme.muted_foreground;
    let glyph = match icon.into() {
        Glyph::Asset(path) => icon_element(path, color),
        Glyph::Inline(svg) => icon_data(svg, color),
    };

    // Round, like every other control in the bar. A square icon button next to
    // pill-shaped tabs reads as a different design language.
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .hover(move |this| this.bg(theme.muted))
        .child(glyph)
}

/// Draws a 16 px icon by path.
///
/// Kept in one place so every glyph in the bar shares the same optical size.
fn icon_element(path: SharedString, color: Hsla) -> AnyElement {
    gpui_kit::svg()
        .path(path)
        .size(px(16.0))
        .text_color(color)
        .into_any_element()
}

/// Draws an inline SVG, for glyphs the bundled assets cannot provide.
fn icon_data(svg: &'static [u8], color: Hsla) -> AnyElement {
    gpui_kit::component::Icon::default()
        .data(svg)
        .with_size(px(16.0))
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
    icon: Glyph,
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
    let glyph = match icon {
        Glyph::Asset(path) => icon_element(path, color),
        Glyph::Inline(svg) => icon_data(svg, color),
    };

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(40.0))
        .h(px(28.0))
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
enum PaneStatus {
    Working,
    Idle,
    Exited,
}

impl PaneStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Working => "输出中",
            Self::Idle => "空闲",
            Self::Exited => "已退出",
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

/// The name a pane shows in its header.
///
/// The shell's OSC title when it sets one, otherwise the program it was started
/// with. Both end up with any path stripped, because a pane is named by what it
/// is, not by where it lives.
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

fn has_selection(workspace: &Workspace) -> bool {
    workspace.selection
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
fn apply_terminal_font(cx: &mut App) {
    gpui_kit::component::Theme::update(cx, |theme| {
        let mut dark = (*theme.dark_theme).clone();
        dark.mono_font_family = Some(TERMINAL_FONT.into());
        theme.dark_theme = std::rc::Rc::new(dark);

        let mut light = (*theme.light_theme).clone();
        light.mono_font_family = Some(TERMINAL_FONT.into());
        theme.light_theme = std::rc::Rc::new(light);

        theme.mono_font_family = TERMINAL_FONT.into();
    });
}

fn main() {
    env_logger::init();

    // The first shell is started before the event loop exists. A pseudo console
    // takes about 50 ms to come up, and `App::run` keeps the app borrowed for
    // all of `on_finish_launching`, so doing this later re-enters the borrow as
    // soon as the window asks for its first frame.
    let command = pty_command::default_profile();

    // The pane needs the palette the window will end up with, and the theme is
    // only installed later in `gpui_kit::init`, so the colors are resolved
    // directly from the theme crate here.
    let light = system_prefers_light();
    let mut theme = gpui_kit::component::Theme::default();
    theme.colors = if light {
        (*gpui_kit::component::ThemeColor::light()).clone()
    } else {
        (*gpui_kit::component::ThemeColor::dark()).clone()
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

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);

            apply_terminal_font(cx);

            // The app follows the desktop's light/dark setting, resolved before
            // the loop started so the first shell could be painted to match.
            let mode = if light {
                gpui_kit::component::ThemeMode::Light
            } else {
                gpui_kit::component::ThemeMode::Dark
            };
            palette::install(cx, mode);

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
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
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
