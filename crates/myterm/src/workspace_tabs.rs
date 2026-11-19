use super::*;

impl Workspace {
    /// Opens a new tab. The console is opened off the UI thread for the same
    /// reason a split is.
    pub(super) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (cols, rows) = initial_grid(window);
        let command = self.command.clone();
        let tab_id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        let entity = cx.entity();
        cx.spawn(async move |_, cx| {
            let launch = command.clone();
            let pty = cx
                .background_executor()
                .spawn(async move { Pty::spawn(&launch, cols, rows) })
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
                match Tab::from_pty(tab_id, pty, &command, cols, rows, &theme) {
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
    pub(super) fn prompt_close(&mut self, target: CloseTarget, cx: &mut Context<Self>) {
        self.closing = Some(target);
        cx.notify();
    }

    /// Renames a workspace and stops following its shell.
    ///
    /// The name is trimmed and an empty one is refused: a row with no label is
    /// a row you cannot pick out of a list, which is the one thing the column
    /// exists to do.
    pub(super) fn rename_tab(&mut self, tab_id: u64, name: &str, cx: &mut Context<Self>) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.title = name.to_owned();
            tab.auto_named = false;
            cx.notify();
            true
        } else {
            false
        }
    }

    /// Closes the workspace, asking first when something in it is still writing.
    pub(super) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
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
    pub(super) fn do_close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
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
    pub(super) fn reopen_closed(&mut self, cx: &mut Context<Self>) {
        let Some(closed) = self.closed.take() else {
            return;
        };
        let index = closed.index.min(self.tabs.len());
        self.tabs.insert(index, closed.tab);
        self.active_tab = index;
        cx.notify();
    }

    /// Closes the focused pane, or the workspace when it is the last one.
    pub(super) fn close_pane(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn do_close_pane(&mut self, tab_index: usize, id: u64, cx: &mut Context<Self>) {
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
    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
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
    pub(super) fn focus_pane(&mut self, id: u64, cx: &mut Context<Self>) {
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
    pub(super) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn apply_theme(
        &mut self,
        mode: gpui_kit::component::ThemeMode,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn set_shell_integration(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.shell_integration = on;
        self.config.save();
        self.command = build_command(
            self.config.shell,
            on,
            self.config.wsl_distribution.as_deref(),
        );
        cx.notify();
    }

    /// Changes how much history every pane keeps, now and for new ones.
    pub(super) fn set_scrollback(&mut self, lines: usize, cx: &mut Context<Self>) {
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
    pub(super) fn set_accent(&mut self, accent: config::Accent, cx: &mut Context<Self>) {
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
    pub(super) fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = size.clamp(config::MIN_FONT_SIZE, config::MAX_FONT_SIZE);
        self.config.font_size = Some(size);
        self.config.save();
        apply_terminal_font(cx, Some(size));
        cx.notify();
    }

    /// Opens or closes the settings sheet.
    pub(super) fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings = !self.settings;
        cx.notify();
    }

    pub(super) fn finish_window(&mut self, window: &mut Window) {
        self.tabs.clear();
        self.closed = None;
        window.remove_window();
    }

    /// Answers the close prompt with "yes".
    pub(super) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn handle_scroll(
        &mut self,
        event: &ScrollEvent,
        style: GridStyle,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Let the focused input own every keystroke while a workspace title is
        // being edited. In particular, IMEs use Enter and Escape to confirm or
        // dismiss composition before those keys should affect the workspace.
        if self.rename.is_some() {
            return;
        }

        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        let find_input = self
            .find
            .as_ref()
            .map(|find| &find.input)
            .filter(|input| input.read(cx).focus_handle(cx).is_focused(window));
        let palette_input = self
            .palette
            .as_ref()
            .map(|palette| &palette.input)
            .filter(|input| input.read(cx).focus_handle(cx).is_focused(window));
        if matches!(keystroke.key.as_str(), "enter" | "escape" | "up" | "down")
            && let Some(input) = palette_input.or(find_input)
            && input.update(cx, |state, cx| {
                state.marked_text_range(window, cx).is_some()
            })
        {
            return;
        }

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
        if find_input.is_some()
            || (self.find.is_some()
                && self.focus.is_focused(window)
                && matches!(keystroke.key.as_str(), "escape" | "f3"))
        {
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

        if !self.focus.is_focused(window) {
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
}
