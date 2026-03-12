//! Binary split tree for tiling terminal panes in the Agents tab.
//!
//! Each node is either a [`Leaf`] (single pane) or a [`Split`] (two children
//! separated by a 1-cell border). The tree is recursively subdivided via
//! [`TileNode::compute_rects`] to produce a `(PaneId, Rect)` for every leaf.

use crate::primitives::Rect;

// ── Identifiers ────────────────────────────────────────────────────────

/// Opaque pane identifier, unique within a single [`AgentsTabState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(pub u32);

/// Direction of a binary split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    /// Left | Right
    Vertical,
    /// Top / Bottom
    Horizontal,
}

// ── Tree node ──────────────────────────────────────────────────────────

/// A node in the binary tiling tree.
#[derive(Clone, Debug)]
pub enum TileNode {
    /// A terminal pane.
    Leaf { pane_id: PaneId },
    /// Two children separated by a border.
    Split {
        direction: SplitDirection,
        /// Fraction of space given to `first` child (clamped to `0.1..=0.9`).
        ratio: f64,
        first: Box<Self>,
        second: Box<Self>,
    },
}

/// Minimum ratio for any split (10%).
const MIN_RATIO: f64 = 0.1;
/// Maximum ratio for any split (90%).
const MAX_RATIO: f64 = 0.9;
/// Resize step when nudging a border via keyboard.
pub const RESIZE_STEP: f64 = 0.05;

impl TileNode {
    // ── Constructors ───────────────────────────────────────────────

    /// Single pane.
    #[must_use]
    pub const fn leaf(id: PaneId) -> Self {
        Self::Leaf { pane_id: id }
    }

    /// Split two nodes with a given direction and ratio.
    #[must_use]
    pub fn split(direction: SplitDirection, ratio: f64, first: Self, second: Self) -> Self {
        Self::Split {
            direction,
            ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    // ── Layout computation ─────────────────────────────────────────

    /// Recursively compute the screen [`Rect`] for every leaf pane.
    ///
    /// Split borders consume 1 cell of the available space.
    pub fn compute_rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.compute_rects_inner(area, &mut out);
        out
    }

    fn compute_rects_inner(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Leaf { pane_id } => {
                out.push((*pane_id, area));
            }
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = subdivide(area, *direction, *ratio);
                first.compute_rects_inner(first_area, out);
                second.compute_rects_inner(second_area, out);
            }
        }
    }

    /// Collect the borders (position, length, direction) for rendering.
    pub fn compute_borders(&self, area: Rect) -> Vec<Border> {
        let mut out = Vec::new();
        self.compute_borders_inner(area, &mut out);
        out
    }

    fn compute_borders_inner(&self, area: Rect, out: &mut Vec<Border>) {
        if let Self::Split {
            direction,
            ratio,
            first,
            second,
        } = self
        {
            let border = border_rect(area, *direction, *ratio);
            out.push(border);
            let (first_area, second_area) = subdivide(area, *direction, *ratio);
            first.compute_borders_inner(first_area, out);
            second.compute_borders_inner(second_area, out);
        }
    }

    // ── Tree queries ───────────────────────────────────────────────

    /// Collect all leaf pane IDs in tree order (left-to-right, top-to-bottom).
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_ids(&mut out);
        out
    }

    fn collect_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Leaf { pane_id } => out.push(*pane_id),
            Self::Split { first, second, .. } => {
                first.collect_ids(out);
                second.collect_ids(out);
            }
        }
    }

    /// Number of leaf panes.
    #[must_use]
    pub fn pane_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    // ── Tree mutations ─────────────────────────────────────────────

    /// Replace the leaf with `target_id` by splitting it, putting the original
    /// leaf in `first` and a new leaf (`new_id`) in `second`.
    ///
    /// Returns `true` if the split was performed.
    pub fn split_pane(
        &mut self,
        target_id: PaneId,
        new_id: PaneId,
        direction: SplitDirection,
    ) -> bool {
        match self {
            Self::Leaf { pane_id } if *pane_id == target_id => {
                let original = Self::leaf(*pane_id);
                let new_leaf = Self::leaf(new_id);
                *self = Self::split(direction, 0.5, original, new_leaf);
                true
            }
            Self::Split { first, second, .. } => {
                first.split_pane(target_id, new_id, direction)
                    || second.split_pane(target_id, new_id, direction)
            }
            Self::Leaf { .. } => false,
        }
    }

    /// Remove a leaf from the tree, promoting its sibling to take its parent
    /// split's place. Returns `true` if removed.
    ///
    /// If the tree is a single leaf matching `target_id`, it is **not** removed
    /// (cannot have zero panes).
    pub fn remove_pane(&mut self, target_id: PaneId) -> bool {
        self.remove_pane_inner(target_id)
    }

    fn remove_pane_inner(&mut self, target_id: PaneId) -> bool {
        let replacement = match self {
            Self::Leaf { .. } => return false,
            Self::Split { first, second, .. } => {
                // Check if either direct child is the target leaf.
                if matches!(first.as_ref(), Self::Leaf { pane_id } if *pane_id == target_id) {
                    Some(*second.clone())
                } else if matches!(second.as_ref(), Self::Leaf { pane_id } if *pane_id == target_id)
                {
                    Some(*first.clone())
                } else {
                    None
                }
            }
        };

        if let Some(promoted) = replacement {
            *self = promoted;
            return true;
        }

        // Recurse into children.
        match self {
            Self::Split { first, second, .. } => {
                first.remove_pane_inner(target_id) || second.remove_pane_inner(target_id)
            }
            Self::Leaf { .. } => false,
        }
    }

    /// Find the split node whose border is nearest to `(col, row)` within
    /// `tolerance` cells. Returns a path of indices (0 = first, 1 = second)
    /// to reach the split node, plus the split direction.
    ///
    /// Used for mouse drag-to-resize hit testing.
    pub fn hit_test_border(
        &self,
        area: Rect,
        col: u16,
        row: u16,
        tolerance: u16,
    ) -> Option<(Vec<usize>, SplitDirection)> {
        self.hit_test_inner(area, col, row, tolerance, &mut Vec::new())
    }

    fn hit_test_inner(
        &self,
        area: Rect,
        col: u16,
        row: u16,
        tolerance: u16,
        path: &mut Vec<usize>,
    ) -> Option<(Vec<usize>, SplitDirection)> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let border = border_rect(area, *direction, *ratio);
                if border.hit_test(col, row, tolerance) {
                    return Some((path.clone(), *direction));
                }
                let (first_area, second_area) = subdivide(area, *direction, *ratio);
                path.push(0);
                if let Some(result) =
                    first.hit_test_inner(first_area, col, row, tolerance, path)
                {
                    return Some(result);
                }
                path.pop();
                path.push(1);
                if let Some(result) =
                    second.hit_test_inner(second_area, col, row, tolerance, path)
                {
                    return Some(result);
                }
                path.pop();
                None
            }
        }
    }

    /// Get a mutable reference to the split node at `path`. Each element is
    /// 0 (first child) or 1 (second child).
    pub fn node_at_path_mut(&mut self, path: &[usize]) -> Option<&mut Self> {
        if path.is_empty() {
            return Some(self);
        }
        match self {
            Self::Split { first, second, .. } => match path[0] {
                0 => first.node_at_path_mut(&path[1..]),
                1 => second.node_at_path_mut(&path[1..]),
                _ => None,
            },
            Self::Leaf { .. } => None,
        }
    }

    /// Adjust the ratio of this node (must be a Split). Returns the new ratio.
    pub fn adjust_ratio(&mut self, delta: f64) -> Option<f64> {
        if let Self::Split { ratio, .. } = self {
            *ratio = (*ratio + delta).clamp(MIN_RATIO, MAX_RATIO);
            Some(*ratio)
        } else {
            None
        }
    }
}

// ── Border description ─────────────────────────────────────────────────

/// A rendered split border.
#[derive(Clone, Copy, Debug)]
pub struct Border {
    /// Position and size of the 1-cell-wide border line.
    pub rect: Rect,
    /// Direction of the split that created this border.
    pub direction: SplitDirection,
}

impl Border {
    /// Check if `(col, row)` is within `tolerance` cells of this border.
    #[must_use]
    pub const fn hit_test(&self, col: u16, row: u16, tolerance: u16) -> bool {
        match self.direction {
            SplitDirection::Vertical => {
                // Border is a vertical line: check column proximity.
                let border_col = self.rect.x;
                col.abs_diff(border_col) <= tolerance
                    && row >= self.rect.y
                    && row < self.rect.y + self.rect.height
            }
            SplitDirection::Horizontal => {
                // Border is a horizontal line: check row proximity.
                let border_row = self.rect.y;
                row.abs_diff(border_row) <= tolerance
                    && col >= self.rect.x
                    && col < self.rect.x + self.rect.width
            }
        }
    }
}

// ── Geometry helpers ───────────────────────────────────────────────────

/// Subdivide `area` into two rects separated by a 1-cell border.
fn subdivide(area: Rect, direction: SplitDirection, ratio: f64) -> (Rect, Rect) {
    match direction {
        SplitDirection::Vertical => {
            // 1-cell vertical border between left and right.
            let usable = area.width.saturating_sub(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let left_w = (f64::from(usable) * ratio).round() as u16;
            let right_w = usable.saturating_sub(left_w);
            let left = Rect::new(area.x, area.y, left_w, area.height);
            let right = Rect::new(area.x + left_w + 1, area.y, right_w, area.height);
            (left, right)
        }
        SplitDirection::Horizontal => {
            // 1-cell horizontal border between top and bottom.
            let usable = area.height.saturating_sub(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let top_h = (f64::from(usable) * ratio).round() as u16;
            let bottom_h = usable.saturating_sub(top_h);
            let top = Rect::new(area.x, area.y, area.width, top_h);
            let bottom = Rect::new(area.x, area.y + top_h + 1, area.width, bottom_h);
            (top, bottom)
        }
    }
}

/// Compute the 1-cell border rect for a split.
fn border_rect(area: Rect, direction: SplitDirection, ratio: f64) -> Border {
    match direction {
        SplitDirection::Vertical => {
            let usable = area.width.saturating_sub(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let left_w = (f64::from(usable) * ratio).round() as u16;
            Border {
                rect: Rect::new(area.x + left_w, area.y, 1, area.height),
                direction,
            }
        }
        SplitDirection::Horizontal => {
            let usable = area.height.saturating_sub(1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let top_h = (f64::from(usable) * ratio).round() as u16;
            Border {
                rect: Rect::new(area.x, area.y + top_h, area.width, 1),
                direction,
            }
        }
    }
}

// ── Preset layouts ─────────────────────────────────────────────────────

/// Preset layout definitions.
///
/// Each function takes a slice of [`PaneId`]s. If fewer IDs are provided than
/// the layout requires, the remaining slots are filled by calling `next_id`.
pub struct Presets;

impl Presets {
    /// One full-screen pane.
    #[must_use]
    pub const fn single(id: PaneId) -> TileNode {
        TileNode::leaf(id)
    }

    /// Two vertical panes, 50/50.
    #[must_use]
    pub fn side_by_side(ids: [PaneId; 2]) -> TileNode {
        TileNode::split(
            SplitDirection::Vertical,
            0.5,
            TileNode::leaf(ids[0]),
            TileNode::leaf(ids[1]),
        )
    }

    /// Three equal vertical columns.
    #[must_use]
    pub fn three_columns(ids: [PaneId; 3]) -> TileNode {
        // First split: 33% | 67%
        // Second split of the 67%: 50% | 50% (which gives 33% | 33% of total)
        TileNode::split(
            SplitDirection::Vertical,
            1.0 / 3.0,
            TileNode::leaf(ids[0]),
            TileNode::split(
                SplitDirection::Vertical,
                0.5,
                TileNode::leaf(ids[1]),
                TileNode::leaf(ids[2]),
            ),
        )
    }

    /// 2×2 grid of four panes.
    #[must_use]
    pub fn grid(ids: [PaneId; 4]) -> TileNode {
        TileNode::split(
            SplitDirection::Horizontal,
            0.5,
            TileNode::split(
                SplitDirection::Vertical,
                0.5,
                TileNode::leaf(ids[0]),
                TileNode::leaf(ids[1]),
            ),
            TileNode::split(
                SplitDirection::Vertical,
                0.5,
                TileNode::leaf(ids[2]),
                TileNode::leaf(ids[3]),
            ),
        )
    }

    /// 60% main pane on the left, two stacked panes on the right (50/50).
    #[must_use]
    pub fn main_stack(ids: [PaneId; 3]) -> TileNode {
        TileNode::split(
            SplitDirection::Vertical,
            0.6,
            TileNode::leaf(ids[0]),
            TileNode::split(
                SplitDirection::Horizontal,
                0.5,
                TileNode::leaf(ids[1]),
                TileNode::leaf(ids[2]),
            ),
        )
    }
}

/// Named preset for the layout picker.
#[derive(Clone, Copy, Debug)]
pub struct PresetInfo {
    pub name: &'static str,
    pub pane_count: usize,
}

/// All available preset layouts.
pub const PRESET_LIST: &[PresetInfo] = &[
    PresetInfo { name: "Single", pane_count: 1 },
    PresetInfo { name: "Side by Side", pane_count: 2 },
    PresetInfo { name: "Three Columns", pane_count: 3 },
    PresetInfo { name: "Grid (2×2)", pane_count: 4 },
    PresetInfo { name: "Main + Stack", pane_count: 3 },
];

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: u32) -> PaneId {
        PaneId(id)
    }

    fn area() -> Rect {
        Rect::new(0, 0, 120, 40)
    }

    #[test]
    fn single_pane_fills_area() {
        let tree = Presets::single(p(0));
        let rects = tree.compute_rects(area());
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], (p(0), area()));
    }

    #[test]
    fn side_by_side_splits_vertically() {
        let tree = Presets::side_by_side([p(0), p(1)]);
        let rects = tree.compute_rects(area());
        assert_eq!(rects.len(), 2);
        // 119 usable cols, 50% = 60 left, 59 right, 1 border
        let (left, right) = (rects[0].1, rects[1].1);
        assert_eq!(left.x, 0);
        assert_eq!(left.height, 40);
        assert_eq!(right.height, 40);
        assert_eq!(left.width + 1 + right.width, 120); // border = 1
    }

    #[test]
    fn three_columns_equal() {
        let tree = Presets::three_columns([p(0), p(1), p(2)]);
        let rects = tree.compute_rects(area());
        assert_eq!(rects.len(), 3);
        // All three should have height 40.
        for (_, r) in &rects {
            assert_eq!(r.height, 40);
        }
    }

    #[test]
    fn grid_four_panes() {
        let tree = Presets::grid([p(0), p(1), p(2), p(3)]);
        let rects = tree.compute_rects(area());
        assert_eq!(rects.len(), 4);
    }

    #[test]
    fn main_stack_three_panes() {
        let tree = Presets::main_stack([p(0), p(1), p(2)]);
        let rects = tree.compute_rects(area());
        assert_eq!(rects.len(), 3);
        // First pane (main) should be ~60% width.
        let main = rects[0].1;
        assert!(main.width > 60, "main pane should be >60 cols, got {}", main.width);
    }

    #[test]
    fn pane_ids_in_order() {
        let tree = Presets::grid([p(0), p(1), p(2), p(3)]);
        assert_eq!(tree.pane_ids(), vec![p(0), p(1), p(2), p(3)]);
    }

    #[test]
    fn pane_count() {
        assert_eq!(Presets::single(p(0)).pane_count(), 1);
        assert_eq!(Presets::three_columns([p(0), p(1), p(2)]).pane_count(), 3);
        assert_eq!(Presets::grid([p(0), p(1), p(2), p(3)]).pane_count(), 4);
    }

    #[test]
    fn split_pane_inserts_new_leaf() {
        let mut tree = Presets::single(p(0));
        assert!(tree.split_pane(p(0), p(1), SplitDirection::Vertical));
        assert_eq!(tree.pane_count(), 2);
        assert_eq!(tree.pane_ids(), vec![p(0), p(1)]);
    }

    #[test]
    fn remove_pane_promotes_sibling() {
        let mut tree = Presets::side_by_side([p(0), p(1)]);
        assert!(tree.remove_pane(p(0)));
        assert_eq!(tree.pane_count(), 1);
        assert_eq!(tree.pane_ids(), vec![p(1)]);
    }

    #[test]
    fn remove_last_pane_is_noop() {
        let mut tree = Presets::single(p(0));
        assert!(!tree.remove_pane(p(0)));
        assert_eq!(tree.pane_count(), 1);
    }

    #[test]
    fn borders_count() {
        let tree = Presets::grid([p(0), p(1), p(2), p(3)]);
        let borders = tree.compute_borders(area());
        // Grid has 3 splits: 1 horizontal + 2 vertical.
        assert_eq!(borders.len(), 3);
    }

    #[test]
    fn border_hit_test() {
        let tree = Presets::side_by_side([p(0), p(1)]);
        let borders = tree.compute_borders(area());
        assert_eq!(borders.len(), 1);
        let b = &borders[0];
        // Border should be a vertical line at x=60 (approx).
        assert!(b.hit_test(b.rect.x, 20, 1));
        assert!(!b.hit_test(0, 20, 1));
    }

    #[test]
    fn ratio_clamped() {
        let tree = TileNode::split(SplitDirection::Vertical, 0.0, TileNode::leaf(p(0)), TileNode::leaf(p(1)));
        if let TileNode::Split { ratio, .. } = &tree {
            assert!(*ratio >= MIN_RATIO);
        }
    }

    #[test]
    fn adjust_ratio() {
        let mut tree = Presets::side_by_side([p(0), p(1)]);
        let new = tree.adjust_ratio(0.1);
        assert_eq!(new, Some(0.6));
        // Clamp at max.
        tree.adjust_ratio(0.5);
        if let TileNode::Split { ratio, .. } = &tree {
            assert!((*ratio - MAX_RATIO).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn no_overlap_no_gap() {
        // Verify that pane rects + borders perfectly tile the area.
        let a = area();
        let tree = Presets::side_by_side([p(0), p(1)]);
        let rects = tree.compute_rects(a);
        let borders = tree.compute_borders(a);

        let total_cells: u32 = rects.iter().map(|(_, r)| u32::from(r.width) * u32::from(r.height)).sum::<u32>()
            + borders.iter().map(|b| u32::from(b.rect.width) * u32::from(b.rect.height)).sum::<u32>();
        assert_eq!(total_cells, u32::from(a.width) * u32::from(a.height));
    }
}
