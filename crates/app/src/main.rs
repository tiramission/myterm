//! The application shell: a single window with a tab strip, one or more panes,
//! and the key handling that drives them.

mod grid_element;
mod layout;
mod pty_command;
mod terminal;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

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

/// The 16 ANSI colors the app installs into every terminal.
const ANSI: [Rgb; 16] = [
    Rgb::new(0x1c, 0x1f, 0x26),
    Rgb::new(0xe0, 0x5c, 0x5c),
    Rgb::new(0x7f, 0xb0, 0x6a),
    Rgb::new(0xd8, 0xb0, 0x5a),
    Rgb::new(0x6a, 0x9f, 0xd8),
    Rgb::new(0xb0, 0x7f, 0xd8),
    Rgb::new(0x5a, 0xb0, 0xb0),
    Rgb::new(0xc0, 0xc4, 0xcc),
    Rgb::new(0x50, 0x55, 0x60),
    Rgb::new(0xf0, 0x7a, 0x7a),
    Rgb::new(0x9a, 0xc8, 0x82),
    Rgb::new(0xe8, 0xc4, 0x76),
    Rgb::new(0x8a, 0xb8, 0xe8),
    Rgb::new(0xc8, 0x9a, 0xe8),
    Rgb::new(0x7a, 0xc8, 0xc8),
    Rgb::new(0xe8, 0xec, 0xf2),
];

/// Builds the 256-entry palette the parser should report: the ANSI sixteen
/// followed by the library's own defaults for the rest.
fn palette() -> [Rgb; 256] {
    let mut palette = [Rgb::default(); 256];
    palette[..ANSI.len()].copy_from_slice(&ANSI);
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
        session.set_colors(
            to_rgb(theme.foreground),
            to_rgb(theme.background),
            Some(to_rgb(theme.caret)),
            &palette(),
        );
        Self {
            session,
            command: command.clone(),
            grid: (cols, rows),
            dims: Rc::new(Cell::new((cols, rows))),
        }
    }
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

/// The window's content.
pub struct Workspace {
    tabs: Vec<Tab>,
    active_tab: usize,
    command: Command,
    /// Set while a poll tick is scheduled, so only one timer runs at a time.
    ticking: bool,
    /// The most recent failure, shown in place of the grid.
    error: Option<String>,
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

    /// Drains every pane. Returns true when a frame changed.
    fn poll(&mut self) -> bool {
        let mut changed = false;
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

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let text = self
            .active_pane_mut()
            .and_then(|pane| pane.session.select_all_text());
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
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
        let theme = gpui_kit::component::Theme::global(cx).clone();

        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::handle_key));

        body = body.child(self.render_tabs(window, cx));

        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
            return body;
        }

        let Some(tab) = self.active_tab() else {
            return body;
        };

        body.child(self.render_panes(tab, style, cx))
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
                let exit = pane.session.exit_code();

                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .bg(style.background)
                    .when(!focused, |this| {
                        this.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |workspace, _: &MouseDownEvent, _, cx| {
                                workspace.focus_pane(id, cx);
                            }),
                        )
                    })
                    .on_scroll(cx.listener(move |workspace, event: &ScrollEvent, _, cx| {
                        workspace.handle_scroll(event, style, cx);
                    }))
                    // The grid fills the padded area, so text never touches the
                    // window edge and the padding is part of the pane, not a
                    // layout gap between panes.
                    .child(div().absolute().inset(px(6.0)).child(TerminalGrid::new(
                        ElementId::Name(format!("grid-{id}").into()),
                        frame,
                        style,
                        focused,
                        dims,
                    )))
                    // Focus is shown inside the pane rather than as a border, so
                    // gaining focus never shifts the text by a pixel.
                    .when(multi && focused, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(1.0))
                                .bg(theme.ring),
                        )
                    })
                    .when_some(exit, |this, code| {
                        this.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(22.0))
                                .px_3()
                                .flex()
                                .items_center()
                                .bg(theme.background)
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
                // A one-pixel gap shows the division without drawing a border on
                // either side, which keeps each pane's grid aligned to its box.
                let mut container = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .gap(px(1.0))
                    .bg(theme.border);
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
}

impl Workspace {
    /// Draws the tab strip and the pane controls.
    ///
    /// The strip carries three levels of meaning and each gets one signal:
    /// which tab is open (a filled surface), how many panes it holds (a count
    /// beside the name), and what else can be done (icon buttons on the trailing
    /// edge, separated from the tabs by empty space rather than a rule).
    fn render_tabs(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let active = self.active_tab;

        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(38.0))
            .pl_2()
            .pr_1()
            .gap(px(2.0))
            // The strip is a shade off the grid so the tabs sit in it rather than
            // floating on the terminal background. The selected tab then reads as
            // the one that connects down into the grid.
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border);

        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = index == active;
            let title = tab.title.clone();
            let pane_count = tab.panes.len();
            let closable = self.tabs.len() > 1;

            strip = strip.child(tab_chip(
                index, title, pane_count, selected, closable, &theme, cx,
            ));
        }

        strip = strip.child(div().w(px(4.0)));
        strip = strip.child(icon_button("new-tab", IconName::Plus, &theme, cx).on_click(
            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.new_tab(window, cx);
            }),
        ));

        // The pane actions sit at the far edge: they act on the current tab, not
        // on the tab list, and grouping them with the tabs would say otherwise.
        strip
            .child(div().flex_1())
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
    }
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
        .pl_3()
        .pr(if closable { px(4.0) } else { px(12.0) })
        .rounded(px(7.0))
        .when(selected, |this| this.bg(theme.secondary_active))
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
                this.bg(theme.accent).text_color(theme.foreground)
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

    if pane_count > 1 {
        chip = chip.child(
            div()
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
                .rounded(px(4.0))
                .opacity(0.0)
                .group_hover("tab", |this| this.opacity(1.0))
                .hover(|this| this.bg(theme.accent))
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

/// A square icon-only button, sized for a toolbar row.
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

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |this| this.bg(theme.accent))
        .child(glyph)
}

/// Draws a 24×24 icon by path, at a fixed size.
///
/// Kept in one place so every toolbar glyph shares the same optical size.
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

fn has_selection(workspace: &Workspace) -> bool {
    workspace
        .active_pane()
        .and_then(|pane| pane.session.frame().rows.first())
        .map(|row| row.cells.iter().any(|cell| cell.selected))
        .unwrap_or(false)
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

fn main() {
    env_logger::init();

    // The first shell is started before the event loop exists. A pseudo console
    // takes about 50 ms to come up, and `App::run` keeps the app borrowed for
    // all of `on_finish_launching`, so doing this later re-enters the borrow as
    // soon as the window asks for its first frame.
    let command = pty_command::default_profile();

    // The pane needs the palette the window will end up with, and the theme is
    // only installed later in `gpui_kit::init`, so the dark colors are resolved
    // directly from the theme crate here.
    let mut theme = gpui_kit::component::Theme::default();
    theme.colors = (*gpui_kit::component::ThemeColor::dark()).clone();
    let (mut first_pane, startup_error) = match Pane::new(&command, 120, 40, &theme) {
        Ok(pane) => (Some(pane), None),
        Err(message) => (None, Some(message)),
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);

            // A terminal is read as light-on-dark: the grid, the shell colors and
            // the chrome should agree, so the app starts in the dark theme.
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(160.0), px(120.0)),
                    size(px(1100.0), px(720.0)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("myterm".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, move |window, cx| {
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
