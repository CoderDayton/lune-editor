//! Session model: the set of open buffers, their tab order, and the active
//! one. Owned by the editor core, not the UI layer.
//!
//! This replaces the ad-hoc `(BufferRegistry, Vec<BufferId>, Option<BufferId>)`
//! trio currently scattered across `AppState`. Centralizing it:
//!
//! - Removes dozens of `self.registry` / `self.tabs` / `self.active_buffer`
//!   touch points from UI code.
//! - Kills the O(n²) cursor-restore loop: path → buffer lookups now go
//!   through `BufferRegistry::by_path`, which is a hash probe.
//! - Gives tests a single seam to construct editor state without booting
//!   the whole TUI.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::buffer::{BufferId, TextBuffer};
use crate::registry::BufferRegistry;

/// The active editor session.
///
/// Fields are `pub` to support in-place migration from the legacy
/// `(registry, tabs, active_buffer)` trio on `AppState`. Future passes
/// can re-encapsulate once call sites are stable.
#[derive(Debug, Default)]
pub struct SessionModel {
    pub registry: BufferRegistry,
    pub tabs: Vec<BufferId>,
    pub active_buffer: Option<BufferId>,
}

impl SessionModel {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Buffer access ──────────────────────────────────────────────

    pub const fn registry(&self) -> &BufferRegistry {
        &self.registry
    }

    pub const fn registry_mut(&mut self) -> &mut BufferRegistry {
        &mut self.registry
    }

    pub fn get(&self, id: BufferId) -> Option<&TextBuffer> {
        self.registry.get(id)
    }

    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut TextBuffer> {
        self.registry.get_mut(id)
    }

    pub const fn active_id(&self) -> Option<BufferId> {
        self.active_buffer
    }

    pub fn active_buf(&self) -> Option<&TextBuffer> {
        self.registry.get(self.active_buffer?)
    }

    pub fn active_buf_mut(&mut self) -> Option<&mut TextBuffer> {
        let id = self.active_buffer?;
        self.registry.get_mut(id)
    }

    pub fn set_active(&mut self, id: Option<BufferId>) {
        // Invariant: active must be in the tab list, or None.
        self.active_buffer = id.filter(|i| self.tabs.contains(i));
    }

    // ── Tab list ───────────────────────────────────────────────────

    pub fn tabs(&self) -> &[BufferId] {
        &self.tabs
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn index_of(&self, id: BufferId) -> Option<usize> {
        self.tabs.iter().position(|&i| i == id)
    }

    pub fn tab_at(&self, index: usize) -> Option<BufferId> {
        self.tabs.get(index).copied()
    }

    /// Add a buffer to the tab list if absent. Idempotent.
    pub fn add_tab(&mut self, id: BufferId) {
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
    }

    /// Remove a buffer from both the tab list and the registry. Returns
    /// the new active buffer (picked as the neighbor of the removed tab).
    pub fn close_tab(&mut self, id: BufferId) -> Option<BufferId> {
        let idx = self.index_of(id)?;
        self.tabs.remove(idx);
        self.registry.close(id);

        if self.active_buffer == Some(id) {
            self.active_buffer = self
                .tabs
                .get(idx)
                .or_else(|| self.tabs.get(idx.saturating_sub(1)))
                .copied();
        }
        self.active_buffer
    }

    // ── Open helpers ───────────────────────────────────────────────

    /// Open a file, add it to the tab list, and make it active.
    /// Returns the buffer's ID (new or existing).
    pub fn open_file(&mut self, path: &Path) -> Result<BufferId> {
        let id = self.registry.open_file(path)?;
        self.add_tab(id);
        self.active_buffer = Some(id);
        Ok(id)
    }

    /// Create a scratch buffer, add it as a tab, and make it active.
    pub fn new_scratch(&mut self) -> BufferId {
        let id = self.registry.new_scratch();
        self.add_tab(id);
        self.active_buffer = Some(id);
        id
    }

    // ── Path lookups (O(1), not O(n²)) ─────────────────────────────

    /// Find a tab by its absolute path, using the registry's hash index.
    /// O(1) instead of the O(n) linear scan currently in `app.rs`.
    pub fn tab_by_path(&self, path: &Path) -> Option<BufferId> {
        let id = self.registry.by_path(path)?;
        self.tabs.contains(&id).then_some(id)
    }

    /// Apply saved cursor positions keyed by absolute path.
    ///
    /// Replaces the O(n²) nested loop in `app.rs:631-639` with O(m) where
    /// `m = map.len()`, because each lookup is one hash probe.
    pub fn apply_cursor_positions<I>(&mut self, entries: I)
    where
        I: IntoIterator<Item = (PathBuf, (usize, usize))>,
    {
        for (path, (line, col)) in entries {
            let Some(id) = self.tab_by_path(&path) else {
                continue;
            };
            let Some(buf) = self.registry.get_mut(id) else {
                continue;
            };
            let rope = buf.rope();
            let line_count = rope.len_lines().saturating_sub(1);
            let clamped_line = line.min(line_count);
            let line_len = rope
                .get_line(clamped_line)
                .map_or(0, |l: ropey::RopeSlice| l.len_chars().saturating_sub(1));
            let clamped_col = col.min(line_len);
            buf.cursor.primary.head.line = clamped_line;
            buf.cursor.primary.head.col = clamped_col;
            buf.cursor.primary.anchor = buf.cursor.primary.head;
        }
    }

    /// Collect cursor positions for all tabs with a file path.
    ///
    /// The caller is responsible for converting absolute paths to workspace-
    /// relative before persisting.
    pub fn cursor_snapshots(&self) -> Vec<(PathBuf, (usize, usize))> {
        self.tabs
            .iter()
            .filter_map(|&id| {
                let buf = self.registry.get(id)?;
                let path = buf.file_path.as_ref()?.clone();
                let pos = &buf.cursor.primary.head;
                Some((path, (pos.line, pos.col)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_scratch_is_active_and_in_tabs() {
        let mut s = SessionModel::new();
        let id = s.new_scratch();
        assert_eq!(s.active_id(), Some(id));
        assert_eq!(s.tabs(), &[id]);
        assert_eq!(s.tab_count(), 1);
    }

    #[test]
    fn add_tab_is_idempotent() {
        let mut s = SessionModel::new();
        let id = s.new_scratch();
        s.add_tab(id);
        s.add_tab(id);
        assert_eq!(s.tabs().len(), 1);
    }

    #[test]
    fn close_tab_picks_neighbor_as_active() {
        let mut s = SessionModel::new();
        let a = s.new_scratch();
        let b = s.new_scratch();
        let c = s.new_scratch();
        assert_eq!(s.active_id(), Some(c));
        // close the middle tab while it's active
        s.set_active(Some(b));
        let new_active = s.close_tab(b);
        assert_eq!(new_active, Some(c));
        assert_eq!(s.tabs(), &[a, c]);
    }

    #[test]
    fn close_last_tab_clears_active() {
        let mut s = SessionModel::new();
        let id = s.new_scratch();
        s.close_tab(id);
        assert_eq!(s.active_id(), None);
        assert!(s.is_empty());
    }

    #[test]
    fn set_active_rejects_non_tab() {
        let mut s = SessionModel::new();
        let a = s.new_scratch();
        let orphan_id = {
            let mut reg = BufferRegistry::new();
            reg.new_scratch()
        };
        s.set_active(Some(orphan_id));
        assert_eq!(s.active_id(), None);
        s.set_active(Some(a));
        assert_eq!(s.active_id(), Some(a));
    }

    #[test]
    fn index_of_matches_tab_order() {
        let mut s = SessionModel::new();
        let a = s.new_scratch();
        let b = s.new_scratch();
        assert_eq!(s.index_of(a), Some(0));
        assert_eq!(s.index_of(b), Some(1));
        assert_eq!(s.tab_at(1), Some(b));
    }

    #[test]
    fn apply_cursor_positions_missing_path_is_noop() {
        let mut s = SessionModel::new();
        let _ = s.new_scratch();
        // scratch buffers have no path, so lookup can never succeed
        s.apply_cursor_positions([(PathBuf::from("/nope.rs"), (5, 5))]);
        assert_eq!(s.active_buf().unwrap().cursor.primary.head.line, 0);
    }

    #[test]
    fn cursor_snapshots_skips_unsaved_buffers() {
        let mut s = SessionModel::new();
        let _ = s.new_scratch();
        assert!(s.cursor_snapshots().is_empty());
    }
}
