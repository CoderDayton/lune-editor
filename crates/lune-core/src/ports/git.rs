//! Git port: async adapter publishes status/gutter snapshots; UI reads.
//!
//! The UI never calls git2 directly. It holds an `Arc<dyn GitPort>` and
//! reads `port.status()` every render. Commands (stage, discard, refresh)
//! are fire-and-forget via `dispatch`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::buffer::BufferId;
use crate::ports::snapshot::Snapshot;

/// Per-file status. Intentionally minimal — expand as UI needs arise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub state: FileState,
    /// Whether the change is staged (index-vs-HEAD) rather than
    /// unstaged (workdir-vs-index). A single repository state can emit
    /// two entries for the same path — one staged, one unstaged — when
    /// the user has partial stages.
    pub staged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Clean,
    Modified,
    Added,
    Deleted,
    Untracked,
    Conflicted,
}

/// Repository-wide status snapshot. Published atomically by the adapter.
#[derive(Clone, Debug, Default)]
pub struct StatusSnapshot {
    pub branch: Option<String>,
    pub head_short: Option<String>,
    /// Absolute path of the repository's working tree root. Consumers use
    /// it to resolve the repo-relative `FileEntry::path` values to
    /// absolute paths for file-tree matching and open-file commands.
    pub workdir_root: Option<PathBuf>,
    pub files: Vec<FileEntry>,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
    /// Error text from the most recent failed command, if any. Cleared
    /// on the next successful op's snapshot. UI renders this as a
    /// notification.
    pub last_error: Option<String>,
    /// Metadata from the most recent successful commit, if any. UI
    /// renders this as a success notification (`[abc1234] msg`).
    pub last_commit: Option<CommitInfo>,
    /// Monotonic counter — bumps on every publish. UI uses it to detect
    /// changes without deep-comparing the file list.
    pub revision: u64,
}

/// Subset of commit metadata the UI needs for the status notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub short_oid: String,
    pub message: String,
}

/// Where to apply a hunk patch. Mirrors `git2::ApplyLocation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchLocation {
    /// Apply to the index (stage/unstage hunk).
    Index,
    /// Apply to the working directory (discard hunk).
    Workdir,
}

/// Kind of a single diff line. Git-free mirror of `lune_git`'s
/// `DiffLineKind` so the core port layer carries no git2 dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkLineKind {
    Context,
    Addition,
    Deletion,
}

/// One line of a hunk, carrying only the fields a staleness check
/// compares. Git-free mirror of `lune_git`'s `DiffLine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkLine {
    pub kind: HunkLineKind,
    pub content: String,
    pub no_newline_eof: bool,
}

/// A hunk's freshness-relevant identity: header coordinates plus the
/// `(kind, content, no_newline_eof)` triple of every line.
///
/// This is the exact data the adapter's staleness check compares against
/// the live diff before applying a patch. It is git-free so it can travel
/// inside a [`GitCommand`] without leaking a git2 dependency into core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkIdentity {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<HunkLine>,
}

/// Per-line gutter annotations for a single buffer.
#[derive(Clone, Debug, Default)]
pub struct GutterSnapshot {
    pub buffer: BufferId,
    pub added: Vec<u32>,
    pub modified: Vec<u32>,
    pub deleted: Vec<u32>,
    pub revision: u64,
}

/// Commands the UI sends to the git adapter. Fire-and-forget.
#[derive(Clone, Debug)]
pub enum GitCommand {
    RefreshStatus,
    Stage(PathBuf),
    Unstage(PathBuf),
    Discard(PathBuf),
    /// Create a commit from the currently staged changes.
    Commit {
        message: String,
    },
    /// Apply a (possibly partial) hunk for stage / unstage / discard.
    ///
    /// `parent` identifies the live hunk the UI derived the change from;
    /// the worker re-verifies it against the current diff and rejects the
    /// op if the working tree drifted (staleness guard). `sub` is the
    /// actual slice to apply — equal to `parent` for a full-hunk op, or a
    /// `sub_hunk` slice for "stage selected lines". Applying without the
    /// freshness check on `parent` could stage the wrong lines if the
    /// file changed since the snapshot, corrupting unrelated content.
    ApplyHunk {
        path: PathBuf,
        parent: HunkIdentity,
        sub: HunkIdentity,
        location: PatchLocation,
        /// Verify `parent` against the staged (index-vs-HEAD) diff rather
        /// than the workdir diff — used for unstage-hunk.
        staged: bool,
    },
    /// Recompute the gutter snapshot for a buffer against its HEAD blob.
    ///
    /// `content` is a snapshot of the current buffer text at dispatch
    /// time — the worker owns it and must not touch the buffer. This
    /// keeps the port contract sync-out / fire-and-forget on the UI side.
    RecomputeGutter {
        buffer: BufferId,
        path: PathBuf,
        content: String,
    },
}

/// Port contract. Implementations run their work on the shared runtime.
pub trait GitPort: Send + Sync + 'static {
    /// Repo status snapshot reader. Load is lock-free.
    fn status(&self) -> Snapshot<StatusSnapshot>;

    /// Gutter snapshot reader for a specific buffer. Returns `None` if the
    /// buffer has no path or no snapshot has been published yet.
    fn gutter(&self, buffer: BufferId) -> Option<Snapshot<GutterSnapshot>>;

    /// Submit a command. Non-blocking; work runs on the adapter task.
    fn dispatch(&self, cmd: GitCommand);
}

/// Null adapter — useful when no repo is open or for tests.
pub struct NullGitPort {
    status: Snapshot<StatusSnapshot>,
}

impl NullGitPort {
    pub fn new() -> Self {
        // The reader holds its own `Arc<ArcSwap<_>>`; dropping the cell is
        // safe because a null port never publishes.
        let (_cell, reader) = crate::ports::snapshot::SnapshotCell::new(StatusSnapshot::default());
        Self { status: reader }
    }
}

impl Default for NullGitPort {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPort for NullGitPort {
    fn status(&self) -> Snapshot<StatusSnapshot> {
        self.status.clone()
    }
    fn gutter(&self, _buffer: BufferId) -> Option<Snapshot<GutterSnapshot>> {
        None
    }
    fn dispatch(&self, _cmd: GitCommand) {}
}

/// Convenience alias: the UI stores one of these on `AppState`.
pub type SharedGitPort = Arc<dyn GitPort>;

/// Test helper: a [`GitPort`] backed by a single mutable snapshot.
///
/// Useful for unit tests that want to assert the UI reads from the port
/// without spinning a tokio runtime or a real git repo. Call
/// [`StaticGitPort::publish_status`] to hand the UI a snapshot; reads
/// return it immediately. `dispatch` is a no-op. `gutter` is unimplemented
/// (add a per-buffer map here if tests need it).
pub struct StaticGitPort {
    status_cell: SnapshotCell<StatusSnapshot>,
    status_reader: Snapshot<StatusSnapshot>,
}

impl StaticGitPort {
    pub fn new() -> Self {
        let (cell, reader) = SnapshotCell::new(StatusSnapshot::default());
        Self {
            status_cell: cell,
            status_reader: reader,
        }
    }

    pub fn publish_status(&self, snap: StatusSnapshot) {
        self.status_cell.publish(snap);
    }
}

impl Default for StaticGitPort {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPort for StaticGitPort {
    fn status(&self) -> Snapshot<StatusSnapshot> {
        self.status_reader.clone()
    }
    fn gutter(&self, _buffer: BufferId) -> Option<Snapshot<GutterSnapshot>> {
        None
    }
    fn dispatch(&self, _cmd: GitCommand) {}
}

/// Imported by `SnapshotCell::new` above.
use crate::ports::snapshot::SnapshotCell;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_port_returns_default_status() {
        let port = NullGitPort::new();
        assert_eq!(port.status().load().revision, 0);
    }

    #[test]
    fn null_port_dispatch_does_not_panic() {
        let port = NullGitPort::new();
        port.dispatch(GitCommand::RefreshStatus);
    }
}
