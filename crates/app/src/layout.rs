//! The pane layout tree.
//!
//! A tab holds a tree of panes rather than a list, because splits nest: a pane
//! can be split again along either axis, and each split remembers its ratio.
//! Keeping the tree separate from the views makes the geometry testable and lets
//! the renderer stay a pure function of the layout plus the available rect.

/// Which way a split divides its area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children sit side by side; the ratio is the left child's share.
    Horizontal,
    /// Children stack; the ratio is the top child's share.
    Vertical,
}

/// A node in a tab's layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    /// A single terminal, identified by its stable id.
    Leaf(u64),
    /// Two subtrees and the share given to the first.
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

/// A rectangle in cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

impl Layout {
    /// Splits `id` in place, giving the new pane `new_id`.
    ///
    /// Returns false when `id` is not in the tree, which is how the caller finds
    /// out that the focused pane vanished.
    pub fn split(&mut self, id: u64, new_id: u64, axis: Axis) -> bool {
        match self {
            Layout::Leaf(leaf) if *leaf == id => {
                let first = Layout::Leaf(id);
                let second = Layout::Leaf(new_id);
                *self = Layout::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { first, second, .. } => {
                first.split(id, new_id, axis) || second.split(id, new_id, axis)
            }
        }
    }

    /// Removes `id`, promoting its sibling into its place.
    ///
    /// Returns false when the leaf is not present or is the only pane left; a tab
    /// must always keep one.
    pub fn remove(&mut self, id: u64) -> bool {
        let Layout::Split { first, second, .. } = self else {
            return false;
        };

        let first_is_leaf = matches!(**first, Layout::Leaf(leaf) if leaf == id);
        let second_is_leaf = matches!(**second, Layout::Leaf(leaf) if leaf == id);

        if first_is_leaf {
            *self = (**second).clone();
            return true;
        }
        if second_is_leaf {
            *self = (**first).clone();
            return true;
        }

        first.remove(id) || second.remove(id)
    }

    /// Visits every leaf id in layout order.
    /// Sets the share of the split that directly separates `a` from `b`.
    ///
    /// Used by the divider drag. The pair is what identifies the node: the
    /// deepest split whose two halves contain exactly one of the two panes is
    /// the one the user grabbed, which stays correct when splits are nested.
    pub fn resize_between(&mut self, a: u64, b: u64, ratio: f32) -> bool {
        let Some(ratio) = Some(ratio.clamp(0.1, 0.9)) else {
            return false;
        };

        match self {
            Layout::Leaf(_) => false,
            Layout::Split {
                ratio: share,
                first,
                second,
                ..
            } => {
                let first_has = first.contains(a) && second.contains(b);
                let second_has = first.contains(b) && second.contains(a);
                if first_has {
                    *share = ratio;
                    return true;
                }
                if second_has {
                    *share = 1.0 - ratio;
                    return true;
                }
                first.resize_between(a, b, ratio) || second.resize_between(a, b, ratio)
            }
        }
    }

    /// Whether this subtree holds `id`.
    pub fn contains(&self, id: u64) -> bool {
        match self {
            Layout::Leaf(leaf) => *leaf == id,
            Layout::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    pub fn leaves(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<u64>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Assigns a rectangle to every leaf.
    ///
    /// The split is done in whole cells so panes never overlap and never leave a
    /// gap: the first child gets `floor(ratio * size)` and the second the rest.
    pub fn rects(&self, area: Rect) -> Vec<(u64, Rect)> {
        let mut out = Vec::new();
        self.collect_rects(area, &mut out);
        out
    }

    fn collect_rects(&self, area: Rect, out: &mut Vec<(u64, Rect)>) {
        match self {
            Layout::Leaf(id) => out.push((*id, area)),
            Layout::Split {
                axis,
                ratio,
                first,
                second,
            } => match axis {
                Axis::Horizontal => {
                    let left = split_extent(area.cols, *ratio);
                    let right = area.cols - left;
                    first.collect_rects(Rect { cols: left, ..area }, out);
                    second.collect_rects(
                        Rect {
                            x: area.x + left,
                            cols: right,
                            ..area
                        },
                        out,
                    );
                }
                Axis::Vertical => {
                    let top = split_extent(area.rows, *ratio);
                    let bottom = area.rows - top;
                    first.collect_rects(Rect { rows: top, ..area }, out);
                    second.collect_rects(
                        Rect {
                            y: area.y + top,
                            rows: bottom,
                            ..area
                        },
                        out,
                    );
                }
            },
        }
    }

    /// The pane next to `id` in `direction`, if there is one.
    ///
    /// Navigation works on the geometry rather than the tree: the candidate is
    /// the pane whose rectangle overlaps the current one and lies on the
    /// requested side, which is what a user expects after an uneven split.
    pub fn neighbour(&self, id: u64, direction: Direction, area: Rect) -> Option<u64> {
        let rects = self.rects(area);
        let current = rects.iter().find(|(leaf, _)| *leaf == id)?.1;

        let mut best: Option<(u64, i32)> = None;
        for (leaf, rect) in &rects {
            if *leaf == id {
                continue;
            }
            let (overlaps, distance) = match direction {
                Direction::Left => (
                    rect.y < current.y + current.rows && current.y < rect.y + rect.rows,
                    current.x as i32 - (rect.x + rect.cols) as i32,
                ),
                Direction::Right => (
                    rect.y < current.y + current.rows && current.y < rect.y + rect.rows,
                    rect.x as i32 - (current.x + current.cols) as i32,
                ),
                Direction::Up => (
                    rect.x < current.x + current.cols && current.x < rect.x + rect.cols,
                    current.y as i32 - (rect.y + rect.rows) as i32,
                ),
                Direction::Down => (
                    rect.x < current.x + current.cols && current.x < rect.x + rect.cols,
                    rect.y as i32 - (current.y + current.rows) as i32,
                ),
            };

            if !overlaps || distance < 0 {
                continue;
            }
            if best.is_none_or(|(_, best)| distance < best) {
                best = Some((*leaf, distance));
            }
        }

        best.map(|(leaf, _)| leaf)
    }
}

/// Which way to look for a neighbouring pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

fn split_extent(size: u16, ratio: f32) -> u16 {
    if size < 2 {
        return size;
    }
    let first = (size as f32 * ratio.clamp(0.1, 0.9)).floor() as u16;
    first.clamp(1, size - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_replaces_the_leaf_with_two_panes() {
        let mut layout = Layout::Leaf(1);
        assert!(layout.split(1, 2, Axis::Horizontal));

        assert_eq!(layout.leaves(), vec![1, 2]);
        assert!(
            !layout.split(99, 3, Axis::Vertical),
            "unknown ids are ignored"
        );
    }

    #[test]
    fn rects_tile_the_area_without_gaps() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Horizontal);

        let rects = layout.rects(Rect {
            x: 0,
            y: 0,
            cols: 80,
            rows: 24,
        });

        assert_eq!(rects.len(), 2);
        let (_, left) = rects[0];
        let (_, right) = rects[1];
        assert_eq!(left.x + left.cols, right.x, "the halves must touch");
        assert_eq!(right.x + right.cols, 80, "the halves must fill the width");
        assert_eq!(left.rows, 24);
        assert_eq!(right.rows, 24);
    }

    #[test]
    fn odd_sizes_do_not_leave_a_gap() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Vertical);

        let rects = layout.rects(Rect {
            x: 0,
            y: 0,
            cols: 10,
            rows: 25,
        });

        let (_, top) = rects[0];
        let (_, bottom) = rects[1];
        assert_eq!(top.y + top.rows, bottom.y);
        assert_eq!(bottom.y + bottom.rows, 25, "an odd row count is fully used");
    }

    #[test]
    fn removing_a_leaf_promotes_its_sibling() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);

        assert_eq!(layout.leaves(), vec![1, 2, 3]);
        assert!(layout.remove(2));
        assert_eq!(layout.leaves(), vec![1, 3]);
    }

    #[test]
    fn the_last_pane_cannot_be_removed() {
        let mut layout = Layout::Leaf(1);
        assert!(!layout.remove(1));
        assert_eq!(layout.leaves(), vec![1]);
    }

    #[test]
    fn neighbours_follow_the_geometry() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);

        let area = Rect {
            x: 0,
            y: 0,
            cols: 80,
            rows: 24,
        };

        assert_eq!(layout.neighbour(1, Direction::Right, area), Some(2));
        assert_eq!(layout.neighbour(2, Direction::Left, area), Some(1));
        assert_eq!(layout.neighbour(2, Direction::Down, area), Some(3));
        assert_eq!(layout.neighbour(3, Direction::Up, area), Some(2));
        assert_eq!(
            layout.neighbour(1, Direction::Left, area),
            None,
            "nothing is left of the first pane"
        );
    }

    #[test]
    fn resizing_moves_only_the_split_between_the_pair() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);

        // 1 and 2 share the outer horizontal split.
        assert!(layout.resize_between(1, 2, 0.3));
        assert!(layout.resize_between(2, 1, 0.3), "the pair is symmetric");

        // 2 and 3 share the inner vertical split, not the outer one.
        assert!(layout.resize_between(2, 3, 0.7));

        // Panes that do not share a split are not resizable.
        assert!(!layout.resize_between(1, 9, 0.5));
    }

    #[test]
    fn the_share_stays_within_its_limits() {
        let mut layout = Layout::Leaf(1);
        layout.split(1, 2, Axis::Horizontal);

        assert!(layout.resize_between(1, 2, 0.0));
        assert!(layout.resize_between(1, 2, 1.0));

        let Layout::Split { ratio, .. } = layout else {
            panic!("still split");
        };
        assert!(ratio >= 0.1 && ratio <= 0.9, "got {ratio}");
    }
}

