//! Root layout computation.
//!
//! Computes the VS Code–inspired layout: optional file tree on the left,
//! editor in the center, optional git panel on the right, and a status
//! bar at the bottom.

use crate::primitives::{Constraint, Direction, Layout, Rect};

// ── Layout state (persisted) ──────────────────────────────────────────

/// Configurable layout state that controls which panels are visible and
/// how wide they are.
#[derive(Clone, Debug)]
pub struct LayoutState {
    /// Whether the file tree sidebar is visible.
    pub show_file_tree: bool,
    /// Whether the git panel is visible (right side).
    pub show_git_panel: bool,
    /// File tree width as a percentage of terminal width (default 20).
    pub file_tree_width_pct: u16,
    /// Right panel width as a percentage of terminal width (default 30).
    pub right_panel_width_pct: u16,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            show_file_tree: false,
            show_git_panel: false,
            file_tree_width_pct: 20,
            right_panel_width_pct: 30,
        }
    }
}

impl LayoutState {
    /// Whether the right-side panel (git) is visible.
    #[must_use]
    pub const fn show_right_panel(&self) -> bool {
        self.show_git_panel
    }

    /// Toggle the file tree sidebar.
    pub const fn toggle_file_tree(&mut self) {
        self.show_file_tree = !self.show_file_tree;
    }

    /// Toggle the git panel.
    pub const fn toggle_git_panel(&mut self) {
        self.show_git_panel = !self.show_git_panel;
    }

    /// Clamp a panel width percentage to the valid range (10–50%).
    #[must_use]
    pub const fn clamp_pct(pct: u16) -> u16 {
        if pct < 10 {
            10
        } else if pct > 50 {
            50
        } else {
            pct
        }
    }

    /// Resize the file tree panel.
    pub const fn set_file_tree_width_pct(&mut self, pct: u16) {
        self.file_tree_width_pct = Self::clamp_pct(pct);
    }

    /// Resize the right panel.
    pub const fn set_right_panel_width_pct(&mut self, pct: u16) {
        self.right_panel_width_pct = Self::clamp_pct(pct);
    }
}

// ── Computed splits ───────────────────────────────────────────────────

/// The computed layout rectangles for a single frame.
#[derive(Clone, Debug)]
pub struct LayoutSplits {
    /// File tree sidebar (left). `None` if hidden.
    pub left: Option<Rect>,
    /// Editor pane (center). Always present.
    pub center: Rect,
    /// Right panel (git). `None` if hidden.
    pub right: Option<Rect>,
    /// Status bar (bottom row).
    pub status: Rect,
    /// The border column between the left panel and center (for resize
    /// dragging). `None` if no left panel.
    pub left_border_x: Option<u16>,
    /// The border column between center and right panel. `None` if no
    /// right panel.
    pub right_border_x: Option<u16>,
}

/// Minimum width for the center editor pane.
const MIN_CENTER_WIDTH: u16 = 20;

/// Status bar height (1 row).
const STATUS_HEIGHT: u16 = 1;

/// Compute the layout splits for the given terminal area and state.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // percentage math fits u16
pub fn compute_layout(area: Rect, state: &LayoutState) -> LayoutSplits {
    // Reserve the bottom row(s) for the status bar.
    let (upper_area, status_area) = if area.height > STATUS_HEIGHT {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(STATUS_HEIGHT)])
            .split(area);
        (chunks[0], chunks[1])
    } else {
        // Terminal too small — entire area is status.
        return LayoutSplits {
            left: None,
            center: Rect::ZERO,
            right: None,
            status: area,
            left_border_x: None,
            right_border_x: None,
        };
    };

    // Calculate column widths within the upper area.
    let total_width = upper_area.width;

    let left_width = if state.show_file_tree {
        let w = (u32::from(total_width) * u32::from(state.file_tree_width_pct) / 100) as u16;
        w.max(10).min(total_width.saturating_sub(MIN_CENTER_WIDTH))
    } else {
        0
    };

    let right_width = if state.show_right_panel() {
        let w = (u32::from(total_width) * u32::from(state.right_panel_width_pct) / 100) as u16;
        w.max(10)
            .min(total_width.saturating_sub(left_width + MIN_CENTER_WIDTH))
    } else {
        0
    };

    let center_width = total_width.saturating_sub(left_width + right_width);

    // If the center would be too narrow, hide panels.
    if center_width < MIN_CENTER_WIDTH {
        return LayoutSplits {
            left: None,
            center: upper_area,
            right: None,
            status: status_area,
            left_border_x: None,
            right_border_x: None,
        };
    }

    // Build the horizontal layout.
    // PERF: use fixed-size array literals per branch — eliminates the Vec<Constraint>
    // allocation (and the `idx` counter) since there are only 4 possible column combos.
    let (left, center, right) = match (left_width > 0, right_width > 0) {
        (false, false) => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(center_width)])
                .split(upper_area);
            (None, chunks[0], None)
        }
        (true, false) => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(left_width),
                    Constraint::Length(center_width),
                ])
                .split(upper_area);
            (Some(chunks[0]), chunks[1], None)
        }
        (false, true) => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(center_width),
                    Constraint::Length(right_width),
                ])
                .split(upper_area);
            (None, chunks[0], Some(chunks[1]))
        }
        (true, true) => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(left_width),
                    Constraint::Length(center_width),
                    Constraint::Length(right_width),
                ])
                .split(upper_area);
            (Some(chunks[0]), chunks[1], Some(chunks[2]))
        }
    };

    let left_border_x = left.map(|r| r.x + r.width);
    let right_border_x = right.map(|r| r.x);

    LayoutSplits {
        left,
        center,
        right,
        status: status_area,
        left_border_x,
        right_border_x,
    }
}

/// Check if a mouse column is on a panel border (within 1 cell tolerance).
#[must_use]
pub const fn is_on_left_border(splits: &LayoutSplits, col: u16) -> bool {
    if let Some(bx) = splits.left_border_x {
        col.abs_diff(bx) <= 1
    } else {
        false
    }
}

/// Check if a mouse column is on the right panel border.
#[must_use]
pub const fn is_on_right_border(splits: &LayoutSplits, col: u16) -> bool {
    if let Some(bx) = splits.right_border_x {
        col.abs_diff(bx) <= 1
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn layout_no_panels() {
        let state = LayoutState::default();
        let splits = compute_layout(area(120, 40), &state);

        assert!(splits.left.is_none());
        assert!(splits.right.is_none());
        assert_eq!(splits.center.width, 120);
        assert_eq!(splits.center.height, 39); // 40 - 1 status
        assert_eq!(splits.status.height, 1);
        assert_eq!(splits.status.y, 39);
    }

    #[test]
    fn status_bar_height_is_always_one_regardless_of_window_height() {
        let state = LayoutState::default();
        for h in [3_u16, 10, 20, 40, 60, 100, 200, 500] {
            // Simulate render_editor_tab's actual input: content_area after the
            // root tab bar is skimmed off, i.e. Rect(y=1, h=h-1).
            let content = Rect::new(0, 1, 120, h - 1);
            let splits = compute_layout(content, &state);
            assert_eq!(
                splits.status.height, 1,
                "status height should be 1 for window height {h}, got {}",
                splits.status.height
            );
            assert_eq!(
                splits.status.y,
                h - 1,
                "status should be at the last row for window height {h}",
            );
        }
    }

    #[test]
    fn root_layout_splits_give_single_row_for_tabs_bar() {
        // Mirror the root layout in runtime/app.rs: [Length(1), Min(1)].
        use ratatui::layout::{Constraint, Direction, Layout};
        for h in [3_u16, 10, 20, 40, 60, 100, 200, 500] {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(Rect::new(0, 0, 120, h));
            assert_eq!(
                chunks[0].height, 1,
                "root tabs should be 1 row for window height {h}, got {}",
                chunks[0].height
            );
            assert_eq!(chunks[1].y, 1);
            assert_eq!(chunks[1].height, h - 1);
        }
    }

    #[test]
    fn layout_with_file_tree() {
        let state = LayoutState {
            show_file_tree: true,
            file_tree_width_pct: 20,
            ..Default::default()
        };
        let splits = compute_layout(area(100, 30), &state);

        let left = splits.left.expect("file tree should be visible");
        assert_eq!(left.width, 20); // 20% of 100
        assert_eq!(splits.center.width, 80);
        assert!(splits.right.is_none());
    }

    #[test]
    fn layout_with_right_panel() {
        let state = LayoutState {
            show_git_panel: true,
            right_panel_width_pct: 30,
            ..Default::default()
        };
        let splits = compute_layout(area(100, 30), &state);

        assert!(splits.left.is_none());
        assert_eq!(splits.center.width, 70);
        let right = splits.right.expect("right panel should be visible");
        assert_eq!(right.width, 30);
    }

    #[test]
    fn layout_with_all_panels() {
        let state = LayoutState {
            show_file_tree: true,
            show_git_panel: true,
            file_tree_width_pct: 20,
            right_panel_width_pct: 30,
        };
        let splits = compute_layout(area(100, 30), &state);

        let left = splits.left.expect("left");
        let right = splits.right.expect("right");
        assert_eq!(left.width, 20);
        assert_eq!(right.width, 30);
        assert_eq!(splits.center.width, 50);
    }

    #[test]
    fn layout_tiny_terminal_no_panels() {
        let state = LayoutState::default();
        let splits = compute_layout(area(30, 5), &state);

        assert_eq!(splits.center.width, 30);
        assert_eq!(splits.center.height, 4);
    }

    #[test]
    fn layout_too_narrow_hides_panels() {
        let state = LayoutState {
            show_file_tree: true,
            show_git_panel: true,
            file_tree_width_pct: 40,
            right_panel_width_pct: 40,
        };
        // Terminal narrower than MIN_CENTER_WIDTH causes panels to hide.
        // With 18 cols: left_width clamped, center < 20 => fallback.
        let splits = compute_layout(area(18, 20), &state);

        assert!(splits.left.is_none());
        assert!(splits.right.is_none());
        assert_eq!(splits.center.width, 18);
    }

    #[test]
    fn layout_height_1_is_status_only() {
        let state = LayoutState::default();
        let splits = compute_layout(area(80, 1), &state);

        assert_eq!(splits.status, area(80, 1));
        assert_eq!(splits.center, Rect::ZERO);
    }

    #[test]
    fn border_detection() {
        let state = LayoutState {
            show_file_tree: true,
            show_git_panel: true,
            file_tree_width_pct: 25,
            right_panel_width_pct: 25,
        };
        let splits = compute_layout(area(100, 30), &state);

        let left_bx = splits.left_border_x.unwrap();
        assert!(is_on_left_border(&splits, left_bx));
        assert!(is_on_left_border(&splits, left_bx + 1));
        assert!(!is_on_left_border(&splits, left_bx + 2));

        let right_bx = splits.right_border_x.unwrap();
        assert!(is_on_right_border(&splits, right_bx));
    }

    #[test]
    fn clamp_pct_range() {
        assert_eq!(LayoutState::clamp_pct(5), 10);
        assert_eq!(LayoutState::clamp_pct(25), 25);
        assert_eq!(LayoutState::clamp_pct(60), 50);
    }

    #[test]
    fn toggle_methods() {
        let mut state = LayoutState::default();
        assert!(!state.show_file_tree);
        state.toggle_file_tree();
        assert!(state.show_file_tree);
        state.toggle_file_tree();
        assert!(!state.show_file_tree);

        state.toggle_git_panel();
        assert!(state.show_git_panel);
        assert!(state.show_right_panel());
    }
}
