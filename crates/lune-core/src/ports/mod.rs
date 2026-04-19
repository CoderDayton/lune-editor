//! Port layer: async adapters behind trait objects, lock-free snapshots
//! out to the UI.
//!
//! # Contract
//!
//! Every port follows the same shape:
//! - **Reads** (`snapshot()` / `status()` / similar) return a [`Snapshot<T>`].
//!   `snapshot.load()` is an atomic pointer load — never blocks, never awaits.
//!   The UI calls it on every render frame safely.
//! - **Writes** are [`dispatch`](GitPort::dispatch)-style: a command enum is
//!   handed to the adapter and the call returns immediately. Results surface
//!   on the next published snapshot.
//!
//! # Why this shape
//!
//! - `rat-salsa`'s event loop is sync. Exposing `async fn` on the UI seam
//!   would force `.block_on` calls, which would stall the render loop.
//! - Snapshots decouple producer cadence from consumer cadence: a git walker
//!   running at 2Hz and a UI rendering at 60Hz share state with zero contention.
//! - Trait objects keep the UI testable: every call site takes `&dyn GitPort`,
//!   so tests inject [`NullGitPort`] without spinning a runtime.
//!
//! # Composition
//!
//! Startup owns a [`PortRuntime`]. Each adapter constructor takes the
//! [`RuntimeHandle`], spawns its worker task(s), and returns the public
//! port + snapshot handles. The UI stores `Arc<dyn GitPort>` etc. on
//! `AppState`. That is the only coupling between UI and adapter code.

pub mod ai;
pub mod git;
pub mod json_persistence;
pub mod persistence;
pub mod runtime;
pub mod snapshot;

pub use ai::{
    AiManagerPort, AiSessionPort, ManagerCommand, ManagerSnapshot, NullAiManagerPort,
    NullAiSessionPort, SessionCommand, SessionId, SessionPhase, SessionSnapshot,
    SharedAiManagerPort, SharedAiSessionPort,
};
pub use git::{
    FileEntry, FileState, GitCommand, GitPort, GutterSnapshot, NullGitPort, SharedGitPort,
    StaticGitPort, StatusSnapshot,
};
pub use json_persistence::JsonFilePersistencePort;
pub use persistence::{
    JsonFilePortConfig, MemoryPersistencePort, PersistenceCommand, PersistencePort,
    SharedPersistencePort, StoreSnapshot,
};
pub use runtime::{PortRuntime, RuntimeHandle};
pub use snapshot::{Snapshot, SnapshotCell};
