use super::*;

impl Workspace {
    pub(super) fn render_find_bar(
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
            .bg(theme.title_bar)
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
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
    pub(super) fn render_palette(
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
                            .px(px(16.0))
                            .py(px(10.0))
                            .child(hint("↑↓ 选择"))
                            .child(hint("Enter 执行"))
                            .child(hint("Esc 关闭")),
                    ),
            )
            .into_any_element()
    }

    /// Opens the find bar over the active pane.
    ///
    /// The input owns the text and the workspace owns the matches: the bar has
    /// to be able to show "3 / 12" while the caret is still in the field, and a
    /// controlled input that rebuilt its value every frame would fight the
    /// platform's own editing state.
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn refresh_find(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn run_find(
        &mut self,
        query: Option<&str>,
        step: Option<bool>,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return;
        }
        self.run_find(None, Some(delta > 0), cx);
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            if let Some(pane) = self.active_pane_mut() {
                pane.session.end_find();
            }
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn run_command(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
