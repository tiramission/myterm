use super::*;

/// A small control in a pane header.
/// The strip at the top of a pane: what it is running, and what can be done to
/// it.
///
/// Pane actions live at the trailing edge of the header.
#[allow(clippy::too_many_arguments)]
pub(super) fn pane_header(
    id: u64,
    ordinal: Option<usize>,
    name: String,
    shell: String,
    meta: Option<String>,
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
        .child(icon_data(APP_MARK, 13.0, theme.muted_foreground))
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(200.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.0))
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
        .child(
            div()
                .flex_shrink_0()
                .px(px(5.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .rounded(px(4.0))
                .bg(theme.muted)
                .text_size(px(9.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(shell),
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
                    .bg(theme.muted)
                    .text_size(px(9.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
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
                    ("pane-find", id),
                    SEARCH_ICON,
                    theme,
                    false,
                    cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                        workspace.focus_pane(id, cx);
                        workspace.open_find(window, cx);
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

/// What a pane is doing, as far as the sidebar can tell.
///
/// There is no agent detection here: a pane is working when it wrote something
/// in the last moment, idle when it has not, and exited when the process is
/// gone. That covers a shell, a build and a test watcher without asking any of
/// them to cooperate.
/// What a pane is doing. `Copy` so a row can ask for its colour and its label
/// without threading a reference through both.
#[derive(Clone, Copy)]
pub(super) enum PaneStatus {
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
    pub(super) fn label(self) -> Option<&'static str> {
        match self {
            Self::Working => Some("输出中"),
            Self::Idle => None,
            Self::Exited => Some("已退出"),
        }
    }
}

pub(super) fn pane_status(pane: &Pane) -> PaneStatus {
    match pane.session.exit_code() {
        Some(_) => PaneStatus::Exited,
        None if pane.session.idle_for() < Duration::from_millis(1200) => PaneStatus::Working,
        None => PaneStatus::Idle,
    }
}

/// The colour used for pane activity in the compact rail.
pub(super) fn status_color(status: PaneStatus, theme: &gpui_kit::component::Theme) -> Hsla {
    match status {
        PaneStatus::Working => theme.ring,
        PaneStatus::Idle => theme.muted_foreground,
        PaneStatus::Exited => theme.danger,
    }
}
