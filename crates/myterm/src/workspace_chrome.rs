use super::*;

impl Workspace {
    /// Starts dragging a tab.
    pub(super) fn begin_tab_drag(&mut self, from: usize, start: f32) {
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
    pub(super) fn extend_tab_drag(&mut self, target: usize, y: f32, cx: &mut Context<Self>) {
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
    pub(super) fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn move_tab(&mut self, from: usize, to: usize) {
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
    pub(super) fn begin_sidebar_resize(&mut self, position: Point<Pixels>) {
        self.sidebar_drag = Some(SidebarDrag {
            start: position.x.as_f32(),
            width: self.sidebar_width,
        });
    }

    /// Follows the pointer, snapping to the rail at the narrow end.
    pub(super) fn extend_sidebar_resize(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn effective_active(&self) -> Option<(usize, u64)> {
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
    pub(super) fn drain_update_progress(&mut self) -> bool {
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
    pub(super) fn download_label(&self) -> String {
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
    pub(super) fn update_badge(&self) -> Option<(String, bool)> {
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
    pub(super) fn check_for_update(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn install_state(&self) -> InstallState {
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
    pub(super) fn run_install(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn toggle_rail(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn render_install_prompt(
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
}
