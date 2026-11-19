use super::*;

impl Workspace {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>, config: config::Config) -> Self {
        let shell_choices = ShellChoice::options();
        let selected_shell = ShellChoice::selected_index(&config, &shell_choices);
        let shell_selector = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(shell_choices),
                Some(IndexPath::new(selected_shell)),
                window,
                cx,
            )
        });
        let shell_selector_subscription = cx.subscribe(
            &shell_selector,
            |workspace, _, event: &SelectEvent<SearchableVec<ShellChoice>>, cx| {
                if let SelectEvent::Confirm(Some(choice)) = event {
                    workspace.apply_shell_choice(choice.clone(), cx);
                }
            },
        );

        Self {
            tabs: Vec::new(),
            active_tab: 0,
            next_tab_id: 1,
            command: build_command(
                config.shell,
                config.shell_integration,
                config.wsl_distribution.as_deref(),
            ),
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
            sidebar_width: SIDEBAR_WIDTH,
            folded_tabs: Default::default(),
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
            shell_selector,
            _shell_selector_subscription: shell_selector_subscription,
            resize_drag: None,
            focus: cx.focus_handle(),
            images: Rc::new(RefCell::new(ImageCache::default())),
        }
    }

    /// Installs the pane spawned before start-up as the first tab.
    pub(super) fn adopt(&mut self, pane: Pane) {
        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs = vec![Tab::from_pane(id, pane)];
    }

    /// Starts polling the shells, and focuses the grid once it exists.
    pub(super) fn ensure_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        if window.focused(cx).is_none() {
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
    pub(super) fn claim_tab_key(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn start_ticking(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn advance_caret_blink(&mut self) -> bool {
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
    pub(super) fn poll(&mut self) -> bool {
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

    pub(super) fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    pub(super) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub(super) fn active_pane(&self) -> Option<&Pane> {
        self.active_tab()?.active_pane()
    }

    pub(super) fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.active_tab_mut()?.active_pane_mut()
    }

    /// Sizes every pane to the grid its element reported it can hold.
    pub(super) fn sync_grid_sizes(&mut self, window: &Window, cx: &App) {
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
    pub(super) fn ime_anchor(&self) -> Bounds<Pixels> {
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

    pub(super) fn grid_style(&self, window: &Window, cx: &App) -> GridStyle {
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
}
