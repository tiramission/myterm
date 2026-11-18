use super::*;

impl Workspace {
    /// Opens the tab menu for `tab` at the pointer.
    ///
    /// Right click does not select the row it lands on: the menu acts on that
    /// row, and silently switching the focused terminal because a menu was
    /// opened somewhere else would be a click with two meanings.
    pub(super) fn open_tab_menu(
        &mut self,
        tab: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if tab >= self.tabs.len() {
            return;
        }
        self.menu = Some(TabMenu {
            tab_id: self.tabs[tab].id,
            position,
        });
        cx.notify();
    }

    /// The tab menu, as one card over a click-away layer.
    pub(super) fn render_tab_menu(
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
        let tab_id = menu.tab_id;
        let Some(tab) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return div().into_any_element();
        };
        let title = self.tabs[tab].title.clone();
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
                if let Some(index) = workspace.tabs.iter().position(|tab| tab.id == tab_id) {
                    workspace.close_tab(index, cx);
                }
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
                if let Some(index) = workspace.tabs.iter().position(|tab| tab.id == tab_id) {
                    workspace.close_others(index, cx);
                }
            }),
        );
        let rename = menu_row("menu-rename", "重命名工作区", true, &theme).on_click(cx.listener(
            move |workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                workspace.menu = None;
                if let Some(index) = workspace.tabs.iter().position(|tab| tab.id == tab_id) {
                    workspace.open_rename(index, window, cx);
                }
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
    pub(super) fn open_rename(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((tab_id, title)) = self.tabs.get(index).map(|tab| (tab.id, tab.title.clone()))
        else {
            return;
        };
        if self.sidebar_width < RAIL_THRESHOLD {
            self.sidebar_width = self.sidebar_expanded;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("工作区名字")
                .default_value(title)
        });
        let sub = cx.subscribe(
            &input,
            |workspace: &mut Workspace, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Some(rename) = workspace.rename.as_mut() {
                        rename.invalid = false;
                    }
                    cx.notify();
                }
            },
        );
        self.rename = Some(Rename {
            tab_id,
            input,
            _sub: sub,
            invalid: false,
        });
        if let Some(rename) = self.rename.as_ref() {
            rename.input.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// The resolved executable name used for new panes.
    pub(super) fn shell_label(&self) -> String {
        pty_command::display_name(&self.command)
    }

    /// The about card, as a centred sheet over a scrim.
    pub(super) fn render_about(
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

    pub(super) fn render_close_prompt(
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
    pub(super) fn render_toast(
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

    pub(super) fn close_others(&mut self, keep: usize, cx: &mut Context<Self>) {
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
}
