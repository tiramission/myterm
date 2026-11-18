use super::*;

impl Workspace {
    /// Builds an empty workspace. The caller installs the first tab, because its
    /// shell is spawned before the event loop exists.
    /// Starts dragging the divider between `left` and `right`.
    ///
    /// `span` is the extent the divider travels, measured by the caller from the
    /// union of both halves. Measuring between the two panes touching the seam
    /// would be wrong in a nested split, where those panes are only part of their
    /// halves.
    ///
    /// The gap itself is excluded from the share. Flex hands out the space left
    /// after the gaps, so a ratio computed over the gap included sits about ten
    /// pixels off — which is what made the divider trail the pointer instead of
    /// following it.
    pub(super) fn begin_resize(
        &mut self,
        left: u64,
        right: u64,
        axis: Axis,
        position: Point<Pixels>,
        span: (Bounds<Pixels>, Bounds<Pixels>),
        cx: &mut Context<Self>,
    ) {
        let (first, second) = span;
        let gap = PANE_GAP;

        let (start, extent, origin) = match axis {
            Axis::Horizontal => (
                position.x.as_f32(),
                second.right().as_f32() - first.left().as_f32() - gap,
                first.left().as_f32(),
            ),
            Axis::Vertical => (
                position.y.as_f32(),
                second.bottom().as_f32() - first.top().as_f32() - gap,
                first.top().as_f32(),
            ),
        };
        if extent <= 0.0 {
            return;
        }

        // The pointer sits in the middle of the gap, so half of it belongs to each
        // side: subtracting it puts the divider under the pointer.
        let ratio = (start - origin - gap / 2.0) / extent;

        self.resize_drag = Some(ResizeDrag {
            left,
            right,
            axis,
            start,
            span: extent,
            ratio: ratio.clamp(0.05, 0.95),
        });
        cx.notify();
    }

    /// Moves the divider with the pointer.
    pub(super) fn extend_resize(
        &mut self,
        axis: Axis,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
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
        let ratio = (drag.ratio + (now - drag.start) / drag.span).clamp(0.05, 0.95);
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
    pub(super) fn cell_at(
        &self,
        pane: &Pane,
        position: Point<Pixels>,
        style: GridStyle,
    ) -> (u16, u32) {
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
    pub(super) fn begin_selection(
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
    pub(super) fn extend_selection(
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
    pub(super) fn end_selection(&mut self) {
        self.drag_anchor = None;
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        // Only a real selection is copied. Copying the whole screen when nothing
        // is selected looks like a convenience and is not one: a terminal's
        // screen is mostly the prompt and output from commands the user has
        // already dealt with, and pasting that somewhere is rarely what they
        // meant. Selecting nothing and getting nothing is the honest answer.
        if !self.selection {
            return;
        }
        if let Some(text) = self
            .active_pane_mut()
            .and_then(|pane| pane.session.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Shows `id` on its own, or returns to the whole split if it already is.
    pub(super) fn toggle_zoom(&mut self, id: u64, cx: &mut Context<Self>) {
        self.zoomed = if self.zoomed == Some(id) {
            None
        } else {
            Some(id)
        };
        self.focus_pane(id, cx);
    }

    /// Closes `id`, which is the pane the header's close button belongs to.
    pub(super) fn close_pane_at(&mut self, id: u64, cx: &mut Context<Self>) {
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
    pub(super) fn split_pane(
        &mut self,
        id: u64,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(id, cx);
        self.split_active(axis, window, cx);
    }

    /// Splits the focused pane and moves focus into the new one.
    ///
    /// Starting a pseudo console takes tens of milliseconds, which is long
    /// enough for the frame timer to fire while the input handler still holds the
    /// app borrow. The work is therefore queued with `defer`, so it runs after
    /// the current event is fully dispatched.
    pub(super) fn split_active(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn focus_neighbour(&mut self, direction: Direction, cx: &mut Context<Self>) {
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
    pub(super) fn grid_dims(&self, window: &Window, _cx: &App) -> (u16, u16) {
        self.active_pane()
            .map(|pane| pane.grid)
            .unwrap_or_else(|| initial_grid(window))
    }
}
