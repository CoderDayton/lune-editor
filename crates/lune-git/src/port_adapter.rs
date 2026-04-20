//! Async-backed [`GitPort`] adapter.
//!
//! `git2::Repository` is synchronous and not `Sync`, so the adapter runs on
//! a `spawn_blocking` worker that owns the repo exclusively. Commands
//! arrive via an unbounded mpsc queue; status snapshots publish via
//! `SnapshotCell` (lock-free atomic swap).
//!
//! Periodic refresh is a separate `spawn` timer task that posts
//! `GitCommand::RefreshStatus` every 2s. The worker coalesces bursts by
//! draining the receiver before each expensive status walk.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lune_core::buffer::BufferId;
use lune_core::ports::{
    CommitInfo, FileEntry, FileState, GitCommand, GitPort, GutterSnapshot, PatchLocation,
    RuntimeHandle, Snapshot, SnapshotCell, StatusSnapshot,
};
use lune_core::workspace::FileStatus;
use rustc_hash::FxHashMap;
use tokio::sync::mpsc;

use crate::repo::service::{GitFileStatus, GitService};
use crate::status::gutter::{GutterMark, GutterMarks};

/// Shared reader-side map of per-buffer gutter snapshots. The worker
/// inserts new entries when it publishes the first snapshot for a buffer;
/// the UI reads them by `BufferId`.
type GutterReaders = Arc<Mutex<FxHashMap<BufferId, Snapshot<GutterSnapshot>>>>;

/// Async adapter. Cheap to clone; holds an `Arc` internally.
pub struct GitAdapter {
    inner: Arc<Inner>,
}

struct Inner {
    status_reader: Snapshot<StatusSnapshot>,
    gutter_readers: GutterReaders,
    tx: mpsc::UnboundedSender<GitCommand>,
}

impl GitAdapter {
    /// Open the repository at `path` and start the worker + refresh timer.
    ///
    /// Returns `Ok(None)` when `path` is not inside a git repo — the caller
    /// should fall back to `NullGitPort` in that case.
    pub fn spawn(rt: &RuntimeHandle, path: &Path) -> anyhow::Result<Option<Self>> {
        let Some(service) = GitService::open(path)? else {
            return Ok(None);
        };

        let (status_cell, status_reader) = SnapshotCell::new(StatusSnapshot::default());
        let (tx, rx) = mpsc::unbounded_channel::<GitCommand>();
        let gutter_readers: GutterReaders = Arc::new(Mutex::new(FxHashMap::default()));

        // Blocking worker owns the Repository and the producer handle.
        let readers_for_worker = gutter_readers.clone();
        rt.spawn_blocking(move || worker_loop(service, rx, status_cell, readers_for_worker));

        // Periodic refresh.
        let tx_timer = tx.clone();
        rt.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if tx_timer.send(GitCommand::RefreshStatus).is_err() {
                    break;
                }
            }
        });

        // Prime the first snapshot.
        let _ = tx.send(GitCommand::RefreshStatus);

        Ok(Some(Self {
            inner: Arc::new(Inner {
                status_reader,
                gutter_readers,
                tx,
            }),
        }))
    }

    // ── pure helpers (testable without a repo) ─────────────────────

    /// Convert a libgit2 `FileStatus` to a port-layer `FileState`.
    #[inline]
    pub(crate) const fn map_status(s: FileStatus) -> FileState {
        match s {
            FileStatus::Modified | FileStatus::Renamed => FileState::Modified,
            FileStatus::Added => FileState::Added,
            FileStatus::Deleted => FileState::Deleted,
            FileStatus::Untracked | FileStatus::Ignored => FileState::Untracked,
            FileStatus::Conflicted => FileState::Conflicted,
        }
    }

    /// Build a snapshot, attaching the most recent `last_error` /
    /// `last_commit` from the worker's mutation path. UI clears these
    /// on the next snapshot by virtue of `Option::take` on the worker
    /// side.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_snapshot_full(
        files: Vec<GitFileStatus>,
        branch: String,
        workdir_root: std::path::PathBuf,
        ahead: usize,
        behind: usize,
        last_error: Option<String>,
        last_commit: Option<CommitInfo>,
        revision: u64,
    ) -> StatusSnapshot {
        let files = files
            .into_iter()
            .map(|f| FileEntry {
                path: f.path,
                state: Self::map_status(f.status),
                staged: f.staged,
            })
            .collect();
        StatusSnapshot {
            branch: Some(branch),
            head_short: None,
            workdir_root: Some(workdir_root),
            files,
            ahead: u32::try_from(ahead).unwrap_or(u32::MAX),
            behind: u32::try_from(behind).unwrap_or(u32::MAX),
            last_error,
            last_commit,
            revision,
        }
    }
}

impl GitPort for GitAdapter {
    fn status(&self) -> Snapshot<StatusSnapshot> {
        self.inner.status_reader.clone()
    }

    fn gutter(&self, buffer: BufferId) -> Option<Snapshot<GutterSnapshot>> {
        self.inner.gutter_readers.lock().ok()?.get(&buffer).cloned()
    }

    fn dispatch(&self, cmd: GitCommand) {
        let _ = self.inner.tx.send(cmd);
    }
}

// ── worker ─────────────────────────────────────────────────────────

#[allow(clippy::needless_pass_by_value)] // worker owns all four for the loop's lifetime
fn worker_loop(
    service: GitService,
    mut rx: mpsc::UnboundedReceiver<GitCommand>,
    cell: SnapshotCell<StatusSnapshot>,
    readers: GutterReaders,
) {
    let mut revision: u64 = 0;
    let mut pending_refresh = false;
    let mut pending_error: Option<String> = None;
    let mut pending_commit: Option<CommitInfo> = None;
    let mut gutter_producers: FxHashMap<BufferId, SnapshotCell<GutterSnapshot>> =
        FxHashMap::default();
    let mut gutter_revision: FxHashMap<BufferId, u64> = FxHashMap::default();

    while let Some(cmd) = rx.blocking_recv() {
        let mut cmds = vec![cmd];
        while let Ok(next) = rx.try_recv() {
            cmds.push(next);
        }

        for c in cmds {
            match c {
                GitCommand::RefreshStatus => pending_refresh = true,
                GitCommand::Stage(path) => {
                    apply_mut_op(&service.stage(&path), "stage", &mut pending_error);
                    pending_refresh = true;
                }
                GitCommand::Unstage(path) => {
                    apply_mut_op(&service.unstage(&path), "unstage", &mut pending_error);
                    pending_refresh = true;
                }
                GitCommand::Discard(path) => {
                    apply_mut_op(&service.discard_file(&path), "discard", &mut pending_error);
                    pending_refresh = true;
                }
                GitCommand::Commit { message } => {
                    match service.commit(&message) {
                        Ok(oid) => {
                            let hex = oid.to_string();
                            let short = hex.get(..7).unwrap_or(&hex).to_string();
                            pending_commit = Some(CommitInfo {
                                short_oid: short,
                                message,
                            });
                        }
                        Err(e) => pending_error = Some(format!("commit failed: {e}")),
                    }
                    pending_refresh = true;
                }
                GitCommand::ApplyPatch { patch, location } => {
                    apply_mut_op(
                        &apply_patch(&service, &patch, location),
                        "apply patch",
                        &mut pending_error,
                    );
                    pending_refresh = true;
                }
                GitCommand::RecomputeGutter {
                    buffer,
                    path,
                    content,
                } => {
                    handle_gutter(
                        &service,
                        buffer,
                        &path,
                        &content,
                        &mut gutter_producers,
                        &mut gutter_revision,
                        &readers,
                    );
                }
            }
        }

        if pending_refresh {
            pending_refresh = false;
            match service.status() {
                Ok(st) => {
                    revision = revision.wrapping_add(1);
                    let snap = GitAdapter::build_snapshot_full(
                        st.files,
                        st.branch,
                        service.root().to_path_buf(),
                        st.ahead,
                        st.behind,
                        pending_error.take(),
                        pending_commit.take(),
                        revision,
                    );
                    cell.publish(snap);
                }
                Err(e) => log::warn!("git status refresh failed: {e}"),
            }
        }
    }
}

/// Route a libgit2 result to either the published snapshot's
/// `last_error` field or a debug log on success.
fn apply_mut_op<T>(result: &anyhow::Result<T>, label: &str, sink: &mut Option<String>) {
    if let Err(e) = result {
        *sink = Some(format!("{label} failed: {e}"));
    }
}

/// Dispatch a unified-diff patch to either the index or the workdir.
fn apply_patch(service: &GitService, patch: &str, location: PatchLocation) -> anyhow::Result<()> {
    use anyhow::Context;
    let diff = git2::Diff::from_buffer(patch.as_bytes()).context("failed to parse hunk patch")?;
    let loc = match location {
        PatchLocation::Index => git2::ApplyLocation::Index,
        PatchLocation::Workdir => git2::ApplyLocation::WorkDir,
    };
    service
        .repo()
        .apply(&diff, loc, None)
        .context("failed to apply patch")?;
    Ok(())
}

fn handle_gutter(
    service: &GitService,
    buffer: BufferId,
    path: &Path,
    content: &str,
    producers: &mut FxHashMap<BufferId, SnapshotCell<GutterSnapshot>>,
    revisions: &mut FxHashMap<BufferId, u64>,
    readers: &GutterReaders,
) {
    let marks = match service.gutter_marks(path, content) {
        Ok(m) => m,
        Err(e) => {
            log::debug!("gutter_marks({}) failed: {e}", path.display());
            return;
        }
    };

    let rev = revisions.entry(buffer).or_insert(0);
    *rev = rev.wrapping_add(1);
    let snap = marks_to_snapshot(buffer, &marks, *rev);

    // If this is the first publish for the buffer, register a reader.
    if let std::collections::hash_map::Entry::Vacant(slot) = producers.entry(buffer) {
        let (cell, reader) = SnapshotCell::new(snap);
        slot.insert(cell);
        if let Ok(mut map) = readers.lock() {
            map.insert(buffer, reader);
        }
        return; // already published via `new(snap)` above
    }

    // Subsequent publishes reuse the existing cell.
    if let Some(cell) = producers.get(&buffer) {
        cell.publish(snap);
    }
}

/// Convert `lune-git`'s internal gutter marks into the port-layer
/// [`GutterSnapshot`], splitting the single-map representation into three
/// parallel line-number lists for fast UI rendering.
pub(crate) fn marks_to_snapshot(
    buffer: BufferId,
    marks: &GutterMarks,
    revision: u64,
) -> GutterSnapshot {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for (&line, &mark) in &marks.marks {
        let line = u32::try_from(line).unwrap_or(u32::MAX);
        match mark {
            GutterMark::Added => added.push(line),
            GutterMark::Modified => modified.push(line),
            GutterMark::Deleted => deleted.push(line),
        }
    }
    added.sort_unstable();
    modified.sort_unstable();
    deleted.sort_unstable();
    GutterSnapshot {
        buffer,
        added,
        modified,
        deleted,
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn map_status_covers_all_variants() {
        assert_eq!(
            GitAdapter::map_status(FileStatus::Modified),
            FileState::Modified
        );
        assert_eq!(
            GitAdapter::map_status(FileStatus::Renamed),
            FileState::Modified
        );
        assert_eq!(GitAdapter::map_status(FileStatus::Added), FileState::Added);
        assert_eq!(
            GitAdapter::map_status(FileStatus::Deleted),
            FileState::Deleted
        );
        assert_eq!(
            GitAdapter::map_status(FileStatus::Untracked),
            FileState::Untracked
        );
        assert_eq!(
            GitAdapter::map_status(FileStatus::Ignored),
            FileState::Untracked
        );
        assert_eq!(
            GitAdapter::map_status(FileStatus::Conflicted),
            FileState::Conflicted
        );
    }

    #[test]
    fn build_snapshot_maps_and_increments() {
        let files = vec![GitFileStatus {
            path: PathBuf::from("src/lib.rs"),
            status: FileStatus::Modified,
            staged: false,
        }];
        let snap = GitAdapter::build_snapshot_full(
            files,
            "main".into(),
            std::path::PathBuf::from("/repo"),
            2,
            1,
            None,
            None,
            7,
        );
        assert_eq!(snap.revision, 7);
        assert_eq!(snap.ahead, 2);
        assert_eq!(snap.behind, 1);
        assert_eq!(snap.branch.as_deref(), Some("main"));
        assert_eq!(snap.files.len(), 1);
        assert_eq!(snap.files[0].state, FileState::Modified);
    }
}
