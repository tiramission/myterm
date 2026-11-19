use super::*;

/// A tab's row in the sidebar: the header of a group of panes.
///
/// Quieter than the rows under it — smaller, and marked as current by weight and
/// colour rather than by a fill. The fill belongs to the focused pane, and two
/// filled rows in one group would read as two active places.
#[allow(clippy::too_many_arguments)]
pub(super) fn sidebar_group_row(
    tab_index: usize,
    tab_id: u64,
    title: String,
    pane_count: usize,
    current: bool,
    folded: bool,
    unseen: bool,
    title_input: Option<Entity<InputState>>,
    invalid_name: bool,
    dragging: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let hover_bg = theme.muted;
    let editing = title_input.is_some();
    let save_button = div()
        .id(format!("side-rename-save-{tab_index}"))
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.0))
        .flex_shrink_0()
        .rounded(px(5.0))
        .bg(theme.ring.opacity(0.12))
        .cursor_pointer()
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("保存").build(window, cx))
        .on_click(cx.listener(
            move |workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                let name = workspace
                    .rename
                    .as_ref()
                    .filter(|rename| rename.tab_id == tab_id)
                    .map(|rename| rename.input.read(cx).value().to_string())
                    .unwrap_or_default();
                if workspace.rename_tab(tab_id, &name, cx) {
                    workspace.rename = None;
                    window.focus(&workspace.focus, cx);
                } else if let Some(rename) = workspace.rename.as_mut() {
                    rename.invalid = true;
                }
                cx.notify();
            },
        ))
        .child(icon_data(CHECK_ICON, 13.0, theme.ring));
    let cancel_button = div()
        .id(format!("side-rename-cancel-{tab_index}"))
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.0))
        .flex_shrink_0()
        .rounded(px(5.0))
        .cursor_pointer()
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("取消").build(window, cx))
        .hover(move |this| this.bg(theme.muted))
        .on_click(
            cx.listener(|workspace: &mut Workspace, _: &ClickEvent, window, cx| {
                workspace.rename = None;
                window.focus(&workspace.focus, cx);
                cx.notify();
            }),
        )
        .child(icon_data(WINDOW_CLOSE, 13.0, theme.muted_foreground));

    div()
        .id(ElementId::Name(format!("side-tab-{tab_index}").into()))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .h(px(30.0))
        .pl(px(7.0))
        .pr(px(6.0))
        .rounded(px(7.0))
        .cursor_pointer()
        // The tab being moved stays in its place, faded, so the list does not
        // jump under the pointer while a landing spot is being chosen.
        .when(dragging, |this| this.opacity(0.5))
        .hover(move |this| this.bg(hover_bg.opacity(0.5)))
        .when(!editing, |this| {
            this.on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                if workspace.swallow_tab_click {
                    workspace.swallow_tab_click = false;
                    return;
                }
                workspace.select_tab(tab_index, cx);
            }))
        })
        .when(!editing, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |workspace, event: &MouseDownEvent, _, _| {
                    workspace.begin_tab_drag(tab_index, event.position.y.as_f32());
                }),
            )
        })
        .when(!editing, |this| {
            this.on_mouse_move(
                cx.listener(move |workspace, event: &MouseMoveEvent, _, cx| {
                    workspace.extend_tab_drag(tab_index, event.position.y.as_f32(), cx);
                }),
            )
        })
        .when(!editing, |this| {
            this.on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _, cx| {
                    workspace.end_tab_drag(cx);
                }),
            )
        })
        .when(!editing, |this| {
            this.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    workspace.open_tab_menu(tab_index, event.position, cx);
                }),
            )
        })
        .when(!editing, |this| {
            this.on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |workspace, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    workspace.close_tab(tab_index, cx);
                }),
            )
        })
        .when(!editing, |this| {
            this.child(
                div()
                    .id(("fold-workspace", tab_id))
                    .w(px(18.0))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(move |this| this.bg(hover_bg))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if !workspace.folded_tabs.remove(&tab_id) {
                            workspace.folded_tabs.insert(tab_id);
                        }
                        cx.notify();
                    }))
                    .child(icon_data(
                        if folded { CHEVRON_RIGHT } else { CHEVRON_DOWN },
                        12.0,
                        theme.muted_foreground,
                    )),
            )
        })
        .when_some(title_input, |this, input| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.0))
                    .child(div().flex_1().min_w_0().child(Input::new(&input).small()))
                    .child(cancel_button)
                    .child(save_button),
            )
        })
        .when(!editing, |this| {
            this.child(
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
        })
        .when(unseen && !editing, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(theme.warning)
                    .child("待关注"),
            )
        })
        .when(editing && invalid_name, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(theme.danger)
                    .child("名称不能为空"),
            )
        })
        .when(!editing, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(theme.muted_foreground)
                    .child(pane_count.to_string()),
            )
        })
        .when(!editing, |this| {
            this.child(
                div()
                    .id(("workspace-actions", tab_id))
                    .size(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(move |this| this.bg(hover_bg))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |workspace, event: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        workspace.open_tab_menu(tab_index, event.position(), cx);
                    }))
                    .child(icon_data(MENU_ICON, 13.0, theme.muted_foreground)),
            )
        })
}

/// The cross on a tab row, sized to sit inside the row rather than take a lane.
///
/// It is drawn only on the current tab, so it reads as "this is what you would
/// close" rather than as a column of buttons.
pub(super) fn sidebar_close_button(
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
/// Each pane stays on one line; the surrounding tree supplies its workspace
/// context, while its own label stays concise.
#[allow(clippy::too_many_arguments)]
pub(super) fn sidebar_pane_row(
    tab_index: usize,
    id: u64,
    label: String,
    status: PaneStatus,
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
    let full_title = label.clone();
    let cross = 0.0;

    div()
        // Pane ids restart at 1 in every workspace, so the row has to carry the
        // workspace too: without it two rows share an id and only one of them
        // can be hit or highlighted.
        .id(ElementId::Name(
            format!("side-pane-{tab_index}-{id}").into(),
        ))
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(32.0))
        .pl(px(8.0))
        .pr(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(dragging, |this| this.opacity(0.5))
        .group(row_group.clone())
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(full_title.clone()).build(window, cx)
        })
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
        .child(div().flex_shrink_0().size(px(6.0)).rounded_full().bg(
            status_color(status, theme).opacity(if matches!(status, PaneStatus::Idle) {
                0.45
            } else {
                1.0
            }),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.0))
                .text_color(text_color)
                .child(label),
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

pub(super) fn sidebar_empty_row(
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
pub(super) fn rail_initial(label: &str) -> String {
    label.chars().next().map_or_else(
        || "·".to_owned(),
        |first| first.to_uppercase().collect::<String>(),
    )
}
