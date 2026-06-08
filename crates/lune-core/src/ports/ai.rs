//! AI port traits — split into per-session and lifecycle concerns.
//!
//! # Why two traits
//!
//! `AiManager` holds N independent AI sessions, one per pane in the
//! Agents tab. A single `AiPort` with one `Snapshot<_>` would either:
//!   - serialize all sessions into one bag (conflating unrelated panes'
//!     state into one atomic publish), or
//!   - expose `snapshot_for(id)` as part of the manager (but then the
//!     per-session adapter task has no clean home).
//!
//! Splitting [`AiSessionPort`] (one per live session) from
//! [`AiManagerPort`] (lifecycle + lookup) matches how the editor
//! consumes AI state:
//!   - Each pane renders from its own `Arc<dyn AiSessionPort>` — the
//!     pane's render loop reads one `Snapshot<SessionSnapshot>` per
//!     frame, independent of the other panes.
//!   - The Agents tab asks the `AiManagerPort` for the list of live
//!     sessions, spawns new ones, and kills dead ones.
//!
//! This mirrors `GitPort`'s split between repo-wide `StatusSnapshot`
//! and per-buffer `GutterSnapshot`: the manager-level trait fans out
//! into per-entity snapshot producers.

use std::sync::Arc;

use crate::ports::snapshot::{Snapshot, SnapshotCell};

/// Opaque session ID. Opaque so adapters can pick their own
/// representation (`uuid::Uuid`, a sequence counter, etc.) without
/// leaking it into `lune-core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub u128);

impl SessionId {
    #[must_use]
    pub fn new() -> Self {
        // Mix a monotonic timestamp with a process-wide counter so rapid
        // consecutive calls never collide, even on platforms with a coarse
        // clock. Real adapters may use `uuid::Uuid::new_v4().as_u128()`.
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let seq = u128::from(COUNTER.fetch_add(1, Ordering::Relaxed));
        Self(
            nanos
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(seq)
                .wrapping_add(1),
        )
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

// ── per-session layer ──────────────────────────────────────────────

/// High-level session state, surfaced to the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SessionPhase {
    #[default]
    Starting,
    Running,
    Finished {
        exit_code: i32,
    },
    Errored,
}

/// Per-session snapshot. Small enough to clone cheaply on publish.
#[derive(Clone, Debug, Default)]
pub struct SessionSnapshot {
    pub id: Option<SessionId>,
    pub phase: SessionPhase,
    /// Short status line for the header bar (e.g. "claude-code — running").
    pub status_line: String,
    /// Monotonic revision. Bumps on every publish.
    pub revision: u64,
}

/// Commands routed to a single session. Fire-and-forget.
#[derive(Clone, Debug)]
pub enum SessionCommand {
    /// Raw input bytes (stdin).
    Input(Vec<u8>),
    /// Terminal resize (cols, rows).
    Resize { cols: u16, rows: u16 },
    /// Request graceful shutdown. The manager observes the resulting
    /// `SessionPhase::Finished` on the next snapshot.
    Stop,
    /// Scroll the scrollback buffer up by N lines (UI-initiated).
    ScrollUp(usize),
}

pub trait AiSessionPort: Send + Sync + 'static {
    fn snapshot(&self) -> Snapshot<SessionSnapshot>;
    fn dispatch(&self, cmd: SessionCommand);
}

pub type SharedAiSessionPort = Arc<dyn AiSessionPort>;

// ── manager layer ──────────────────────────────────────────────────

/// Manager-level snapshot: what sessions exist and their IDs. The UI
/// reads this to know which panes to render.
#[derive(Clone, Debug, Default)]
pub struct ManagerSnapshot {
    pub sessions: Vec<SessionId>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub enum ManagerCommand {
    /// Request a new session of the given kind. The manager assigns a
    /// fresh `SessionId`; the caller observes it on the next snapshot.
    Spawn { kind: String },
    /// Kill the session with the given id.
    Kill(SessionId),
}

pub trait AiManagerPort: Send + Sync + 'static {
    fn snapshot(&self) -> Snapshot<ManagerSnapshot>;
    /// Resolve a session id to its per-session port. Returns `None` if
    /// the session has already been killed or never existed.
    fn session(&self, id: SessionId) -> Option<SharedAiSessionPort>;
    fn dispatch(&self, cmd: ManagerCommand);
}

pub type SharedAiManagerPort = Arc<dyn AiManagerPort>;

// ── null adapters (for default AppState / tests) ──────────────────

pub struct NullAiSessionPort {
    reader: Snapshot<SessionSnapshot>,
}

impl NullAiSessionPort {
    #[must_use]
    pub fn new() -> Self {
        let (_cell, reader) = SnapshotCell::new(SessionSnapshot::default());
        Self { reader }
    }
}

impl Default for NullAiSessionPort {
    fn default() -> Self {
        Self::new()
    }
}

impl AiSessionPort for NullAiSessionPort {
    fn snapshot(&self) -> Snapshot<SessionSnapshot> {
        self.reader.clone()
    }
    fn dispatch(&self, _cmd: SessionCommand) {}
}

pub struct NullAiManagerPort {
    reader: Snapshot<ManagerSnapshot>,
}

impl NullAiManagerPort {
    #[must_use]
    pub fn new() -> Self {
        let (_cell, reader) = SnapshotCell::new(ManagerSnapshot::default());
        Self { reader }
    }
}

impl Default for NullAiManagerPort {
    fn default() -> Self {
        Self::new()
    }
}

impl AiManagerPort for NullAiManagerPort {
    fn snapshot(&self) -> Snapshot<ManagerSnapshot> {
        self.reader.clone()
    }
    fn session(&self, _id: SessionId) -> Option<SharedAiSessionPort> {
        None
    }
    fn dispatch(&self, _cmd: ManagerCommand) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_distinct() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a, b, "consecutive SessionId::new() collided");
    }

    #[test]
    fn null_session_default_phase() {
        let port = NullAiSessionPort::new();
        assert_eq!(port.snapshot().load().phase, SessionPhase::Starting);
    }

    #[test]
    fn null_manager_starts_empty() {
        let mgr = NullAiManagerPort::new();
        assert!(mgr.snapshot().load().sessions.is_empty());
    }

    #[test]
    fn null_manager_session_lookup_returns_none() {
        let mgr = NullAiManagerPort::new();
        assert!(mgr.session(SessionId::new()).is_none());
    }

    #[test]
    fn null_session_dispatch_is_noop() {
        let port = NullAiSessionPort::new();
        port.dispatch(SessionCommand::Input(b"hello".to_vec()));
        port.dispatch(SessionCommand::Stop);
    }
}
