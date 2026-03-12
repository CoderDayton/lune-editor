//! State and logic for the Agents tab — a tiling terminal multiplexer.
//!
//! Manages the mapping between [`PaneId`]s in the tiling layout and
//! [`AiSessionId`]s in the AI manager. Handles pane lifecycle (split, close,
//! focus cycling) and the leader-key state machine.

use rustc_hash::FxHashMap;

use lune_ai::AiSessionId;

use super::tiling::{PaneId, Presets, SplitDirection, TileNode, PRESET_LIST};

// ── Pane ───────────────────────────────────────────────────────────────

/// Metadata for a single terminal pane.
#[derive(Clone, Debug)]
pub struct AgentPane {
    /// The AI session driving this pane's PTY.
    pub session_id: AiSessionId,
    /// Display title (e.g. "Shell", "Claude Code").
    pub title: String,
}

// ── Leader key ─────────────────────────────────────────────────────────

/// Tracks the Ctrl+A leader key state machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeaderState {
    /// Normal mode — keystrokes go to the focused pane's PTY.
    #[default]
    Inactive,
    /// Leader key pressed — next keystroke is a pane command.
    Active,
}

// ── Drag state ─────────────────────────────────────────────────────────

/// Tracks an in-progress mouse drag on a split border.
#[derive(Clone, Debug)]
pub struct DragState {
    /// Path through the tree to the split being resized.
    pub split_path: Vec<usize>,
    /// Direction of the split (determines which axis to resize along).
    pub direction: SplitDirection,
}

// ── AgentsTabState ─────────────────────────────────────────────────────

/// Full state for the Agents tab tiling terminal multiplexer.
#[derive(Clone, Debug)]
pub struct AgentsTabState {
    /// The tiling layout tree. `None` when no panes exist yet.
    pub layout: Option<TileNode>,
    /// Pane metadata keyed by pane ID.
    pub panes: FxHashMap<PaneId, AgentPane>,
    /// Currently focused pane.
    pub focused: Option<PaneId>,
    /// Counter for generating unique pane IDs.
    next_id: u32,
    /// Leader key state machine.
    pub leader: LeaderState,
    /// Active mouse drag on a split border.
    pub drag: Option<DragState>,
    /// Whether a single pane is zoomed (temporarily full-screen).
    pub zoomed: bool,
}

impl Default for AgentsTabState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentsTabState {
    /// Create an empty agents tab (no panes).
    #[must_use]
    pub fn new() -> Self {
        Self {
            layout: None,
            panes: FxHashMap::default(),
            focused: None,
            next_id: 0,
            leader: LeaderState::Inactive,
            drag: None,
            zoomed: false,
        }
    }

    /// Allocate a new unique [`PaneId`].
    pub const fn alloc_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Register a pane with an AI session.
    pub fn register_pane(&mut self, pane_id: PaneId, session_id: AiSessionId, title: String) {
        self.panes.insert(
            pane_id,
            AgentPane { session_id, title },
        );
    }

    /// Add the first pane (when layout is `None`).
    ///
    /// Returns the new [`PaneId`] so the caller can spawn a session for it.
    pub fn add_first_pane(&mut self) -> PaneId {
        let id = self.alloc_pane_id();
        self.layout = Some(TileNode::leaf(id));
        self.focused = Some(id);
        id
    }

    /// Split the focused pane in the given direction.
    ///
    /// Returns the new [`PaneId`] so the caller can spawn a session for it.
    /// Returns `None` if there are no panes.
    pub fn split_focused(&mut self, direction: SplitDirection) -> Option<PaneId> {
        let focused = self.focused?;
        let new_id = self.alloc_pane_id();
        let layout = self.layout.as_mut()?;
        if layout.split_pane(focused, new_id, direction) {
            self.focused = Some(new_id);
            Some(new_id)
        } else {
            None
        }
    }

    /// Close the focused pane.
    ///
    /// Returns the [`AiSessionId`] that should be killed, or `None` if no
    /// pane was closed (e.g. only one pane left, or no panes at all).
    pub fn close_focused(&mut self) -> Option<AiSessionId> {
        let focused = self.focused?;

        // Don't close the last pane.
        if self.layout.as_ref()?.pane_count() <= 1 {
            return None;
        }

        // Move focus before removing.
        self.focus_next();

        let layout = self.layout.as_mut()?;
        if layout.remove_pane(focused) {
            let pane = self.panes.remove(&focused)?;
            Some(pane.session_id)
        } else {
            // Remove failed — revert focus.
            self.focused = Some(focused);
            None
        }
    }

    /// Cycle focus to the next pane (tree order).
    pub fn focus_next(&mut self) {
        self.focused = self.cycle_focus(1);
    }

    /// Cycle focus to the previous pane (tree order).
    pub fn focus_prev(&mut self) {
        self.focused = self.cycle_focus(-1);
    }

    fn cycle_focus(&self, delta: isize) -> Option<PaneId> {
        let layout = self.layout.as_ref()?;
        let focused = self.focused?;
        let ids = layout.pane_ids();
        let pos = ids.iter().position(|id| *id == focused)?;
        let len = isize::try_from(ids.len()).ok()?;
        let pos_i = isize::try_from(pos).ok()?;
        let new_pos = usize::try_from((pos_i + delta).rem_euclid(len)).ok()?;
        Some(ids[new_pos])
    }

    /// Focus the pane at a given screen position (for mouse click).
    ///
    /// `rects` should come from `layout.compute_rects(area)`.
    pub fn focus_at(&mut self, col: u16, row: u16, rects: &[(PaneId, crate::primitives::Rect)]) {
        for (pane_id, rect) in rects {
            if col >= rect.x
                && col < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height
            {
                self.focused = Some(*pane_id);
                return;
            }
        }
    }

    /// Get the session ID for the currently focused pane.
    #[must_use]
    pub fn focused_session(&self) -> Option<AiSessionId> {
        let focused = self.focused?;
        self.panes.get(&focused).map(|p| p.session_id)
    }

    /// Apply a preset layout by index (from [`PRESET_LIST`]).
    ///
    /// Existing pane IDs are reused in order. Any additional panes needed are
    /// allocated with new IDs and returned so the caller can spawn Shell
    /// sessions for them. Excess panes are closed and their session IDs
    /// returned for cleanup.
    ///
    /// Returns `(new_pane_ids, closed_session_ids)`.
    pub fn apply_preset(&mut self, preset_index: usize) -> (Vec<PaneId>, Vec<AiSessionId>) {
        let info = match PRESET_LIST.get(preset_index) {
            Some(info) => *info,
            None => return (Vec::new(), Vec::new()),
        };

        let needed = info.pane_count;
        let existing_ids: Vec<PaneId> = self
            .layout
            .as_ref()
            .map_or_else(Vec::new, super::tiling::TileNode::pane_ids);

        // Allocate IDs: reuse existing, allocate new ones as needed.
        let mut ids: Vec<PaneId> = Vec::with_capacity(needed);
        let mut new_ids: Vec<PaneId> = Vec::new();

        for i in 0..needed {
            if i < existing_ids.len() {
                ids.push(existing_ids[i]);
            } else {
                let id = self.alloc_pane_id();
                ids.push(id);
                new_ids.push(id);
            }
        }

        // Close excess panes.
        let mut closed_sessions = Vec::new();
        for &excess_id in existing_ids.iter().skip(needed) {
            if let Some(pane) = self.panes.remove(&excess_id) {
                closed_sessions.push(pane.session_id);
            }
        }

        // Build the new layout tree.
        let tree = match preset_index {
            0 => Presets::single(ids[0]),
            1 => Presets::side_by_side([ids[0], ids[1]]),
            2 => Presets::three_columns([ids[0], ids[1], ids[2]]),
            3 => Presets::grid([ids[0], ids[1], ids[2], ids[3]]),
            4 => Presets::main_stack([ids[0], ids[1], ids[2]]),
            _ => return (Vec::new(), Vec::new()),
        };

        self.layout = Some(tree);

        // Ensure focus is valid.
        if self.focused.is_none() || !ids.contains(&self.focused.unwrap_or(PaneId(u32::MAX))) {
            self.focused = Some(ids[0]);
        }

        self.zoomed = false;
        (new_ids, closed_sessions)
    }

    /// Toggle zoom on the focused pane. When zoomed, the focused pane
    /// renders full-screen; the layout tree is preserved for unzoom.
    pub const fn toggle_zoom(&mut self) {
        if self.focused.is_some() {
            self.zoomed = !self.zoomed;
        }
    }

    /// Whether the tab has any panes at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.layout.is_none()
    }

    /// Total number of panes.
    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.layout.as_ref().map_or(0, TileNode::pane_count)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a distinct dummy session ID for each call.
    fn dummy_session_id() -> AiSessionId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(1);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        lune_ai::AiSessionId::from_u64_pair(0, n)
    }

    #[test]
    fn new_is_empty() {
        let state = AgentsTabState::new();
        assert!(state.is_empty());
        assert_eq!(state.pane_count(), 0);
        assert!(state.focused.is_none());
    }

    #[test]
    fn add_first_pane() {
        let mut state = AgentsTabState::new();
        let id = state.add_first_pane();
        state.register_pane(id, dummy_session_id(), "Shell".into());
        assert_eq!(state.pane_count(), 1);
        assert_eq!(state.focused, Some(id));
    }

    #[test]
    fn split_focused_creates_new_pane() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "Shell".into());

        let second = state.split_focused(SplitDirection::Vertical).unwrap();
        state.register_pane(second, dummy_session_id(), "Shell 2".into());

        assert_eq!(state.pane_count(), 2);
        // Focus moves to the new pane.
        assert_eq!(state.focused, Some(second));
    }

    #[test]
    fn close_focused_promotes_sibling() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        let sid1 = dummy_session_id();
        state.register_pane(first, sid1, "Shell".into());

        let second = state.split_focused(SplitDirection::Vertical).unwrap();
        let sid2 = dummy_session_id();
        state.register_pane(second, sid2, "Shell 2".into());

        // Focus is on second; closing it should return sid2.
        let closed = state.close_focused();
        assert!(closed.is_some());
        assert_eq!(state.pane_count(), 1);
    }

    #[test]
    fn close_last_pane_is_noop() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "Shell".into());

        assert!(state.close_focused().is_none());
        assert_eq!(state.pane_count(), 1);
    }

    #[test]
    fn focus_cycling() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "A".into());

        let second = state.split_focused(SplitDirection::Vertical).unwrap();
        state.register_pane(second, dummy_session_id(), "B".into());

        // Focus is on second. Next should wrap to first.
        state.focus_next();
        assert_eq!(state.focused, Some(first));

        // Prev from first should wrap to second.
        state.focus_prev();
        assert_eq!(state.focused, Some(second));
    }

    #[test]
    fn apply_preset_creates_panes() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "Shell".into());

        // Apply "Side by Side" (needs 2, have 1 → 1 new).
        let (new_ids, closed) = state.apply_preset(1);
        assert_eq!(new_ids.len(), 1);
        assert!(closed.is_empty());
        assert_eq!(state.pane_count(), 2);
    }

    #[test]
    fn apply_preset_closes_excess() {
        let mut state = AgentsTabState::new();

        // Start with grid (4 panes).
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "A".into());
        let (new_ids, _) = state.apply_preset(3); // Grid = 4 panes
        for id in &new_ids {
            state.register_pane(*id, dummy_session_id(), "X".into());
        }
        assert_eq!(state.pane_count(), 4);

        // Apply "Single" (needs 1 → close 3).
        let (new_ids, closed) = state.apply_preset(0);
        assert!(new_ids.is_empty());
        assert_eq!(closed.len(), 3);
        assert_eq!(state.pane_count(), 1);
    }

    #[test]
    fn toggle_zoom() {
        let mut state = AgentsTabState::new();
        let first = state.add_first_pane();
        state.register_pane(first, dummy_session_id(), "Shell".into());

        assert!(!state.zoomed);
        state.toggle_zoom();
        assert!(state.zoomed);
        state.toggle_zoom();
        assert!(!state.zoomed);
    }
}
