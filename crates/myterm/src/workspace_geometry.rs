use super::*;

/// The draggable gutter between two panes of a split.
///
/// The gap already reads as a seam: each pane is a card with its own hairline
/// border, so two borders with space between them say "two things". What the gap
/// cannot say by itself is that it can be moved, so that is all this adds — a
/// grip that appears under the pointer, the same one the sidebar's edge uses.
///
/// The grip stays put while the drag runs. `group_hover` alone drops it the moment
/// the pointer leaves the eight-pixel band, which is exactly when the user is
/// watching it.
pub(super) fn divider_element(
    axis: Axis,
    first: &Layout,
    second: &Layout,
    tab: &Tab,
    dragging: bool,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let left = first.leaves().last().copied();
    let right = second.leaves().first().copied();

    // The extent the divider travels, as the union of both halves.
    let halves = (subtree_bounds(tab, first), subtree_bounds(tab, second));

    /// How long the grip is along the seam.
    const GRIP: f32 = 26.0;

    let grip = div()
        .when(axis == Axis::Horizontal, |this| this.w(px(3.0)).h(px(GRIP)))
        .when(axis == Axis::Vertical, |this| this.h(px(3.0)).w(px(GRIP)))
        .rounded(px(999.0))
        .bg(theme.ring)
        .when(dragging, |this| this.opacity(1.0))
        .when(!dragging, |this| {
            this.opacity(0.0)
                .group_hover("divider", |this| this.opacity(1.0))
        });

    let mut band = div()
        .group("divider")
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(grip);

    band = match axis {
        Axis::Horizontal => band.w(px(PANE_GAP)).h_full().cursor_col_resize(),
        Axis::Vertical => band.h(px(PANE_GAP)).w_full().cursor_row_resize(),
    };

    band.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
            let (Some(left), Some(right)) = (left, right) else {
                return;
            };
            let (Some(first), Some(second)) = halves else {
                return;
            };
            workspace.begin_resize(left, right, axis, event.position, (first, second), cx);
        }),
    )
    .into_any_element()
}

/// The box a pane's card occupies, in window coordinates.
///
/// Derived from the grid's box plus the inset the card gives it — the header and
/// the padding — rather than recorded separately. The grid is the only element
/// that reliably knows where it was painted, and the inset is a constant.
///
/// A split drag has to measure against the card, not the grid: using the grid put
/// the divider a header-height away from the pointer.
pub(super) fn card_bounds(tab: &Tab, id: u64) -> Option<Bounds<Pixels>> {
    let pane = tab.panes.get(&id)?;
    let grid = pane.bounds.get();
    if grid.size.width <= px(0.0) || grid.size.height <= px(0.0) {
        return None;
    }

    // The grid is inset by the padding on three sides and by the header as well
    // when the window is divided.
    let header = if tab.panes.len() > 1 || tab.active == id {
        PANE_HEADER_HEIGHT
    } else {
        0.0
    };
    // The grid's padding is not symmetric, so the two axes are measured
    // separately; a single number put the divider four to seven pixels off.
    let top_pad = if tab.panes.len() > 1 || tab.active == id {
        0.0
    } else {
        GRID_PAD_TOP
    };
    let left = grid.left() - px(GRID_PAD_X);
    let top = grid.top() - px(top_pad + header);
    let right = grid.right() + px(GRID_PAD_X);
    let bottom = grid.bottom() + px(GRID_PAD_BOTTOM);

    Some(Bounds::new(
        point(left, top),
        size(right - left, bottom - top),
    ))
}

/// The union of every card's box in a subtree, in window coordinates.
pub(super) fn subtree_bounds(tab: &Tab, layout: &Layout) -> Option<Bounds<Pixels>> {
    let mut union: Option<Bounds<Pixels>> = None;
    for id in layout.leaves() {
        let bounds = card_bounds(tab, id)?;
        union = Some(match union {
            None => bounds,
            Some(current) => {
                let left = current.left().min(bounds.left());
                let top = current.top().min(bounds.top());
                let right = current.right().max(bounds.right());
                let bottom = current.bottom().max(bounds.bottom());
                Bounds::new(point(left, top), size(right - left, bottom - top))
            }
        });
    }
    union
}

/// The grab strip on the sidebar's trailing edge.
///
/// It straddles the seam rather than sitting beside it, so the pointer finds it
/// exactly where the two columns meet. The grip is the same one the split
/// divider uses — a short accent capsule that fades in under the pointer and
/// stays while the width is being dragged — so the two edges in the window read
/// as one kind of control, because they are.
pub(super) fn sidebar_handle(
    theme: &gpui_kit::component::Theme,
    dragging: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id("side-handle")
        .absolute()
        .top_0()
        .bottom_0()
        // The strip straddles the seam: it starts just inside the column's
        // padding, past the end of the rows, and stops a pixel short of the
        // card. Wide enough for the pointer to find, and it never sits over a
        // row or the terminal, so it cannot steal a click meant for them.
        .right(px(-3.0))
        .w(px(8.0))
        // Its own hover group, like the divider band. Revealing the accent
        // capsule whenever the pointer is anywhere over the list would make it
        // flash on every pass; the edge is a control, and it should look like
        // one only where it can be grabbed.
        .group("side-edge")
        .flex()
        .items_center()
        .justify_center()
        .cursor_col_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|workspace, event: &MouseDownEvent, _, _| {
                workspace.begin_sidebar_resize(event.position);
            }),
        )
        .child(
            div()
                .w(px(3.0))
                .h(px(26.0))
                .rounded(px(999.0))
                .bg(theme.ring)
                .when(dragging, |this| this.opacity(1.0))
                .when(!dragging, |this| {
                    this.opacity(0.0)
                        .group_hover("side-edge", |this| this.opacity(1.0))
                }),
        )
}
