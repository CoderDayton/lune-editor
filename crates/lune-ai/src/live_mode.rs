//! Live Mode controller — detects AI-driven file changes, computes diffs,
//! and provides a read-only overlay of what changed on disk.
//!
//! When active, the controller watches open buffers for on-disk changes,
//! recomputes diffs, and returns follow targets so the UI can auto-switch
//! to the changed file and scroll to the latest edit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lune_core::buffer::BufferId;
use lune_core::diff::{compute_diff, compute_diff_incremental, LiveHunk};
use lune_core::ropey::Rope;

// ── State machine ───────────────────────────────────────────────────────

/// Live Mode state — either off or actively following AI changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiveModeState {
    /// No live tracking. Files only refresh on manual reload.
    #[default]
    Off,
    /// Actively tracking AI edits. Auto-follows the latest changed file
    /// and scrolls to the most recent change region.
    On,
}

// ── Follow target ───────────────────────────────────────────────────────

/// Returned by `on_file_changed` so the UI can auto-follow the latest edit.
#[derive(Clone, Debug)]
pub struct LiveChangeInfo {
    /// The buffer that was updated.
    pub buffer_id: BufferId,
    /// Line (0-based) in the new (disk) content where the latest change starts.
    /// The UI should scroll to this line.
    pub follow_line: usize,
}

// ── Per-buffer diff state ───────────────────────────────────────────────

/// Per-buffer diff tracking.
pub struct LiveDiffState {
    /// Snapshot of buffer content when Live Mode was activated.
    pub baseline: Rope,
    /// Current on-disk content (updated by watcher).
    pub disk_content: Rope,
    /// File path associated with this buffer.
    pub path: PathBuf,
    /// Computed diff hunks between baseline and disk.
    pub hunks: Vec<LiveHunk>,
    /// Last time this buffer's diff was updated.
    pub last_updated: Instant,
}

impl LiveDiffState {
    /// Create a new diff state from a baseline snapshot.
    fn new(baseline: Rope, path: PathBuf) -> Self {
        Self {
            baseline,
            disk_content: Rope::new(),
            path,
            hunks: Vec::new(),
            last_updated: Instant::now(),
        }
    }

    /// Recompute the diff from baseline vs current disk content.
    fn recompute_diff(&mut self) {
        self.hunks = compute_diff(&self.baseline, &self.disk_content);
        self.last_updated = Instant::now();
    }

    /// Recompute diff incrementally for a changed line range.
    fn recompute_diff_incremental(&mut self, changed_range: std::ops::Range<usize>) {
        let previous = std::mem::take(&mut self.hunks);
        self.hunks =
            compute_diff_incremental(&self.baseline, &self.disk_content, changed_range, &previous);
        self.last_updated = Instant::now();
    }

    /// Return the start line of the last hunk (the most recent change region).
    fn last_change_line(&self) -> usize {
        self.hunks.last().map_or(0, |h| h.new_range.start)
    }
}

// ── Global stats ────────────────────────────────────────────────────────

/// Aggregate statistics across all tracked buffers.
#[derive(Clone, Debug, Default)]
pub struct LiveModeStats {
    /// Total number of diff hunks across all files.
    pub total_hunks: usize,
    /// Number of files with at least one hunk.
    pub total_files_changed: usize,
    /// Timestamp of the most recent change.
    pub last_change_at: Option<Instant>,
}

// ── Controller ──────────────────────────────────────────────────────────

/// Manages Live Mode lifecycle, per-buffer diff tracking, and global stats.
pub struct LiveModeController {
    /// Current mode.
    pub state: LiveModeState,
    /// Per-buffer diff state, keyed by `BufferId`.
    pub tracked_buffers: HashMap<BufferId, LiveDiffState>,
    /// Aggregate statistics.
    pub global_stats: LiveModeStats,
    /// Mapping from file path → buffer ID for fast lookup on file events.
    path_to_buffer: HashMap<PathBuf, BufferId>,
}

impl Default for LiveModeController {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveModeController {
    /// Create a new controller in the Off state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: LiveModeState::Off,
            tracked_buffers: HashMap::new(),
            global_stats: LiveModeStats::default(),
            path_to_buffer: HashMap::new(),
        }
    }

    /// Transition to a new state.
    ///
    /// When entering On from Off, the caller should follow up with
    /// `register_buffer()` for each open buffer.
    /// When entering Off, all tracked state is cleared.
    pub fn set_state(&mut self, new_state: LiveModeState) {
        let old_state = self.state;
        self.state = new_state;

        if new_state == LiveModeState::Off && old_state != LiveModeState::Off {
            self.clear_all();
        }
    }

    /// Toggle Live Mode: Off ↔ On.
    pub fn toggle(&mut self) {
        let next = match self.state {
            LiveModeState::Off => LiveModeState::On,
            LiveModeState::On => LiveModeState::Off,
        };
        self.set_state(next);
    }

    /// Is Live Mode active?
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, LiveModeState::On)
    }

    /// Register a buffer for live tracking.
    ///
    /// Takes a snapshot of the current content as the baseline.
    pub fn register_buffer(&mut self, id: BufferId, path: PathBuf, content: Rope) {
        self.path_to_buffer.insert(path.clone(), id);
        let mut diff_state = LiveDiffState::new(content.clone(), path);
        diff_state.disk_content = content;
        self.tracked_buffers.insert(id, diff_state);
    }

    /// Unregister a buffer (e.g., when closing a tab).
    pub fn unregister_buffer(&mut self, id: BufferId) {
        if let Some(state) = self.tracked_buffers.remove(&id) {
            self.path_to_buffer.remove(&state.path);
        }
        self.update_stats();
    }

    /// Handle a file-changed event from the watcher.
    ///
    /// Recomputes the diff against the baseline. Returns a [`LiveChangeInfo`]
    /// with the buffer ID and the line to follow, or `None` if the file
    /// is not tracked or Live Mode is off.
    pub fn on_file_changed(&mut self, path: &Path, new_content: Rope) -> Option<LiveChangeInfo> {
        if !self.is_active() {
            return None;
        }

        let &buffer_id = self.path_to_buffer.get(path)?;
        let diff_state = self.tracked_buffers.get_mut(&buffer_id)?;

        diff_state.disk_content = new_content;
        diff_state.recompute_diff();
        let follow_line = diff_state.last_change_line();
        self.update_stats();

        Some(LiveChangeInfo {
            buffer_id,
            follow_line,
        })
    }

    /// Handle a file-changed event with an incremental hint.
    ///
    /// `changed_range` is the line range (0-based) that was modified.
    pub fn on_file_changed_incremental(
        &mut self,
        path: &Path,
        new_content: Rope,
        changed_range: std::ops::Range<usize>,
    ) -> Option<LiveChangeInfo> {
        if !self.is_active() {
            return None;
        }

        let &buffer_id = self.path_to_buffer.get(path)?;
        let diff_state = self.tracked_buffers.get_mut(&buffer_id)?;

        diff_state.disk_content = new_content;
        diff_state.recompute_diff_incremental(changed_range);
        let follow_line = diff_state.last_change_line();
        self.update_stats();

        Some(LiveChangeInfo {
            buffer_id,
            follow_line,
        })
    }

    /// Get the diff state for a buffer (read-only).
    #[must_use]
    pub fn get_diff_state(&self, buffer_id: BufferId) -> Option<&LiveDiffState> {
        self.tracked_buffers.get(&buffer_id)
    }

    /// Get all file paths that have diff hunks.
    #[must_use]
    pub fn files_with_hunks(&self) -> Vec<(&PathBuf, usize)> {
        self.tracked_buffers
            .values()
            .filter_map(|s| {
                let count = s.hunks.len();
                if count > 0 {
                    Some((&s.path, count))
                } else {
                    None
                }
            })
            .collect()
    }

    // ── Internal ────────────────────────────────────────────────────────

    /// Clear all tracking state (entering Off mode).
    fn clear_all(&mut self) {
        self.tracked_buffers.clear();
        self.path_to_buffer.clear();
        self.global_stats = LiveModeStats::default();
    }

    /// Recompute aggregate statistics from all tracked buffers.
    fn update_stats(&mut self) {
        let mut total_hunks = 0;
        let mut files_changed = 0;
        let mut last_change: Option<Instant> = None;

        for diff_state in self.tracked_buffers.values() {
            let count = diff_state.hunks.len();
            if count > 0 {
                total_hunks += count;
                files_changed += 1;
            }
            match last_change {
                Some(prev) if diff_state.last_updated > prev => {
                    last_change = Some(diff_state.last_updated);
                }
                None => last_change = Some(diff_state.last_updated),
                _ => {}
            }
        }

        self.global_stats = LiveModeStats {
            total_hunks,
            total_files_changed: files_changed,
            last_change_at: last_change,
        };
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn initial_state_is_off() {
        let ctrl = LiveModeController::new();
        assert_eq!(ctrl.state, LiveModeState::Off);
        assert!(!ctrl.is_active());
    }

    #[test]
    fn toggle_state() {
        let mut ctrl = LiveModeController::new();
        ctrl.toggle();
        assert_eq!(ctrl.state, LiveModeState::On);
        assert!(ctrl.is_active());
        ctrl.toggle();
        assert_eq!(ctrl.state, LiveModeState::Off);
        assert!(!ctrl.is_active());
    }

    #[test]
    fn set_state_off_clears_tracking() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id = BufferId::new();
        ctrl.register_buffer(id, PathBuf::from("test.rs"), rope("hello\n"));
        assert!(!ctrl.tracked_buffers.is_empty());

        ctrl.set_state(LiveModeState::Off);
        assert!(ctrl.tracked_buffers.is_empty());
        assert!(!ctrl.is_active());
    }

    #[test]
    fn register_and_unregister_buffer() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id = BufferId::new();
        let path = PathBuf::from("src/main.rs");
        ctrl.register_buffer(id, path.clone(), rope("fn main() {}\n"));

        assert!(ctrl.tracked_buffers.contains_key(&id));
        assert!(ctrl.path_to_buffer.contains_key(&path));

        ctrl.unregister_buffer(id);
        assert!(!ctrl.tracked_buffers.contains_key(&id));
        assert!(!ctrl.path_to_buffer.contains_key(&path));
    }

    #[test]
    fn file_change_triggers_diff_and_returns_follow_info() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id = BufferId::new();
        let path = PathBuf::from("test.rs");
        ctrl.register_buffer(id, path.clone(), rope("line1\nline2\nline3\n"));

        // Simulate AI modifying the file.
        let info = ctrl.on_file_changed(&path, rope("line1\nMODIFIED\nline3\n"));
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.buffer_id, id);
        // follow_line should point to the change region.
        assert!(info.follow_line > 0 || !ctrl.get_diff_state(id).unwrap().hunks.is_empty());

        let diff_state = ctrl.get_diff_state(id).expect("tracked");
        assert!(!diff_state.hunks.is_empty());

        assert!(ctrl.global_stats.total_hunks > 0);
        assert_eq!(ctrl.global_stats.total_files_changed, 1);
    }

    #[test]
    fn file_change_when_off_returns_none() {
        let mut ctrl = LiveModeController::new();
        let path = PathBuf::from("test.rs");
        let info = ctrl.on_file_changed(&path, rope("new content\n"));
        assert!(info.is_none());
    }

    #[test]
    fn files_with_hunks_lists_changed() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id1 = BufferId::new();
        let id2 = BufferId::new();
        ctrl.register_buffer(id1, PathBuf::from("a.rs"), rope("a\n"));
        ctrl.register_buffer(id2, PathBuf::from("b.rs"), rope("b\n"));

        // Change only the first file.
        ctrl.on_file_changed(Path::new("a.rs"), rope("A\n"));

        let changed = ctrl.files_with_hunks();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, &PathBuf::from("a.rs"));
    }

    #[test]
    fn incremental_file_change() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id = BufferId::new();
        let path = PathBuf::from("test.rs");
        ctrl.register_buffer(id, path.clone(), rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"));

        // First change.
        ctrl.on_file_changed(&path, rope("1\n2\nTHREE\n4\n5\n6\n7\n8\n9\n10\n"));
        let count_after_first = ctrl.get_diff_state(id).unwrap().hunks.len();
        assert!(count_after_first > 0);

        // Incremental change to different region.
        let info = ctrl.on_file_changed_incremental(
            &path,
            rope("1\n2\nTHREE\n4\n5\n6\n7\n8\nNINE\n10\n"),
            8..9,
        );
        assert!(info.is_some());
    }

    #[test]
    fn follow_line_points_to_last_hunk() {
        let mut ctrl = LiveModeController::new();
        ctrl.set_state(LiveModeState::On);

        let id = BufferId::new();
        let path = PathBuf::from("test.rs");
        // Use enough lines so changes at the end are clearly at a high line number.
        ctrl.register_buffer(
            id,
            path.clone(),
            rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n"),
        );

        // Change lines near the end — the follow target should be near line 13.
        let info = ctrl
            .on_file_changed(
                &path,
                rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nCHANGED\n15\n"),
            )
            .unwrap();

        // The last hunk should be around the changed line.
        assert!(info.follow_line >= 10, "follow_line={}", info.follow_line);
    }

    #[test]
    fn default_impl() {
        let ctrl = LiveModeController::default();
        assert_eq!(ctrl.state, LiveModeState::Off);
    }
}
