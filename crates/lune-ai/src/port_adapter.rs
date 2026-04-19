//! `AiManagerAdapter` — hosts an owned [`AiManager`] and implements
//! [`lune_core::ports::AiManagerPort`] for port-based consumers.
//!
//! # Design notes
//!
//! `AiSession` is explicitly main-thread-only (vt100 parser without a
//! Mutex, for render-loop latency). Wrapping `AiManager` in
//! `Arc<Mutex<_>>` would add a lock/unlock per frame to every render
//! and input path — up to 60+ acquisitions per second even without
//! contention. That's measurable overhead on TUI repaints and buys
//! nothing: the UI is single-threaded.
//!
//! Instead, the adapter OWNS the `AiManager` directly (no mutex). The
//! UI accesses the manager via `adapter.manager` (for reads) or
//! `adapter.manager_mut()` (for mutations). Read paths go through
//! published `ManagerSnapshot`s.
//!
//! That makes this adapter a facade: it formalizes the port boundary
//! and unblocks consumers who can read from snapshots, while keeping
//! the direct-access escape hatch for the existing render/input code.

use std::sync::Arc;

use lune_core::ports::{
    AiManagerPort, ManagerCommand, ManagerSnapshot, SessionId, SharedAiManagerPort,
    SharedAiSessionPort, Snapshot, SnapshotCell,
};

use crate::manager::AiManager;

/// Shareable `AiManagerPort` implementation. Holds only the reader
/// half of the manager-level snapshot, so it's `Send + Sync` and can
/// live in an `Arc<dyn AiManagerPort>` on `AppState`. The writer half
/// stays on the owning [`AiManagerAdapter`].
struct ReaderPort {
    reader: Snapshot<ManagerSnapshot>,
}

impl AiManagerPort for ReaderPort {
    fn snapshot(&self) -> Snapshot<ManagerSnapshot> {
        self.reader.clone()
    }
    fn session(&self, _id: SessionId) -> Option<SharedAiSessionPort> {
        None
    }
    fn dispatch(&self, _cmd: ManagerCommand) {}
}

pub struct AiManagerAdapter {
    /// Owned manager. UI reads and writes through this field.
    pub manager: AiManager,
    /// Published manager-level snapshot (list of live session IDs).
    /// Re-published via `publish_snapshot` after session lifecycle
    /// changes (spawn / kill / active switch).
    cell: SnapshotCell<ManagerSnapshot>,
    reader: Snapshot<ManagerSnapshot>,
    /// Monotonic revision counter for the manager snapshot.
    revision: u64,
}

impl AiManagerAdapter {
    #[must_use]
    pub fn new() -> Self {
        let (cell, reader) = SnapshotCell::new(ManagerSnapshot::default());
        Self {
            manager: AiManager::new(),
            cell,
            reader,
            revision: 0,
        }
    }

    /// Mutable access to the owned manager. Prefer this over touching
    /// `adapter.manager` directly when the access is mutation-heavy;
    /// keeps the call-site shape parallel to the read accessor.
    pub const fn manager_mut(&mut self) -> &mut AiManager {
        &mut self.manager
    }

    /// Recompute the manager-level snapshot from the live session map
    /// and publish. Call after spawn / kill / active-change. Cheap —
    /// the snapshot is a small `Vec<SessionId>`.
    pub fn publish_snapshot(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        // Translate lune-ai session IDs into the port-layer type. Both
        // are UUIDs under the hood, so we reuse the bytes.
        let sessions: Vec<SessionId> = self
            .manager
            .session_ids()
            .iter()
            .copied()
            .map(|id| SessionId(id.as_u128()))
            .collect();
        self.cell.publish(ManagerSnapshot {
            sessions,
            revision: self.revision,
        });
    }
}

impl Default for AiManagerAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AiManagerAdapter {
    /// Hand out a read-only port handle suitable for storage as a
    /// `SharedAiManagerPort`. The returned `Arc` shares the adapter's
    /// published snapshot; mutation still requires `&mut` access to
    /// the owning adapter on `AppState`.
    #[must_use]
    pub fn shared_port(&self) -> SharedAiManagerPort {
        Arc::new(ReaderPort {
            reader: self.reader.clone(),
        })
    }
}

/// Transparent `Deref`/`DerefMut` to the owned `AiManager`. Lets the
/// 30+ existing `state.ai_manager.X` call sites keep working without
/// a global rename: `state.ai_manager.active_session()` auto-derefs
/// through the adapter to the underlying manager.
///
/// This is normally a code smell (non-smart-pointer Deref), but here
/// the adapter's sole purpose is to host the manager + publish
/// snapshots — every public surface of `AiManager` should transparently
/// be reachable, and the `manager: pub` field already exposes it.
impl std::ops::Deref for AiManagerAdapter {
    type Target = AiManager;
    fn deref(&self) -> &AiManager {
        &self.manager
    }
}

impl std::ops::DerefMut for AiManagerAdapter {
    fn deref_mut(&mut self) -> &mut AiManager {
        &mut self.manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_snapshot_is_empty() {
        let adapter = AiManagerAdapter::new();
        let port = adapter.shared_port();
        let snap = port.snapshot().load();
        assert!(snap.sessions.is_empty());
        assert_eq!(snap.revision, 0);
    }

    #[test]
    fn publish_snapshot_bumps_revision() {
        let mut adapter = AiManagerAdapter::new();
        let port = adapter.shared_port();
        adapter.publish_snapshot();
        assert_eq!(port.snapshot().load().revision, 1);
        adapter.publish_snapshot();
        assert_eq!(port.snapshot().load().revision, 2);
    }

    #[test]
    fn port_dispatch_is_noop() {
        let adapter = AiManagerAdapter::new();
        let port = adapter.shared_port();
        port.dispatch(ManagerCommand::Spawn {
            kind: "claude".into(),
        });
        assert_eq!(port.snapshot().load().revision, 0);
    }
}
