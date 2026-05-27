//! Reactive state persistence backed by JSON files.
//!
//! Layout on disk:
//! ```text
//! <state_dir>/
//! ├── global.json                 # recent workspaces, saved agent layouts,
//! │                              # and other cross-workspace raw keys
//! └── workspaces/
//!     ├── <path_hash>.json        # per-workspace state + undo history
//!     └── ...
//! ```
//!
//! # Multi-instance semantics
//!
//! Unlike the previous sled-backed implementation, the JSON backend does
//! **not** take file locks. Two Lune instances writing the same file
//! last-writer-wins. This removes the user-visible "workspace state
//! disabled" failure mode at the cost of a small race window if two
//! instances edit the same workspace simultaneously — an edge case worth
//! the improved UX.
//!
//! # Encoding
//!
//! Stored values are bincode blobs (preserves compatibility with the
//! sled-era `encode` / `decode` helpers). The KV backing store
//! base64-encodes them inside the JSON file.

use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use crate::project::kv_store::KvStore;
use crate::workspace_state::{RecentFiles, RecentWorkspaces, WorkspaceState};

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

const GLOBAL_FILE_NAME: &str = "global.json";
const WORKSPACES_SUBDIR: &str = "workspaces";

const RECENT_KEY: &[u8] = b"recent:workspaces";
const RECENT_FILES_KEY: &[u8] = b"recent:files";
const WORKSPACE_STATE_KEY: &[u8] = b"workspace:state";
const UNDO_PREFIX: &str = "undo:";

pub struct StateDb {
    state_dir: PathBuf,
    global: Option<KvStore>,
    workspace: Option<KvStore>,
}

impl StateDb {
    #[must_use]
    pub fn open(state_dir: &Path) -> Self {
        let global_path = state_dir.join(GLOBAL_FILE_NAME);
        let global = Some(KvStore::open(global_path));
        Self {
            state_dir: state_dir.to_path_buf(),
            global,
            workspace: None,
        }
    }

    /// Open a fully in-memory database for testing. Uses a fresh tempdir
    /// so tests don't collide with each other's state files.
    #[cfg(test)]
    pub fn open_temporary() -> anyhow::Result<Self> {
        let tmp = tempfile::tempdir().map_err(|e| anyhow::anyhow!("tempdir: {e}"))?;
        let dir = tmp.path().to_path_buf();
        // Leak the TempDir so its contents survive for the test's lifetime.
        // The OS will reclaim them on process exit.
        std::mem::forget(tmp);
        let mut db = Self::open(&dir);
        // Also attach a dummy workspace so workspace-scoped ops work.
        db.workspace = Some(KvStore::open(dir.join("ws-test.json")));
        Ok(db)
    }

    pub fn attach_workspace(&mut self, workspace_root: &Path) -> anyhow::Result<()> {
        let ws_path = self.workspace_db_path(workspace_root);
        if let Some(parent) = ws_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!(
                    "failed to create workspaces dir at {}: {e}",
                    parent.display()
                )
            })?;
        }
        self.workspace = Some(KvStore::open(ws_path));
        Ok(())
    }

    #[must_use]
    pub const fn has_global(&self) -> bool {
        self.global.is_some()
    }

    #[must_use]
    pub const fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    fn workspace_db_path(&self, workspace_root: &Path) -> PathBuf {
        let hash = path_hash(workspace_root);
        self.state_dir
            .join(WORKSPACES_SUBDIR)
            .join(format!("{hash:016x}.json"))
    }

    // ── Workspace state ───────────────────────────────────────────────

    pub fn put_workspace(&mut self, state: &WorkspaceState) -> anyhow::Result<()> {
        let Some(ws) = self.workspace.as_mut() else {
            return Ok(());
        };
        let val = encode(state)?;
        ws.put(WORKSPACE_STATE_KEY, val);
        Ok(())
    }

    pub fn get_workspace(&self) -> anyhow::Result<Option<WorkspaceState>> {
        let Some(ws) = self.workspace.as_ref() else {
            return Ok(None);
        };
        match ws.get(WORKSPACE_STATE_KEY) {
            Some(bytes) => Ok(Some(decode(bytes)?)),
            None => Ok(None),
        }
    }

    pub fn delete_workspace(&mut self) -> anyhow::Result<()> {
        if let Some(ws) = self.workspace.as_mut() {
            ws.remove(WORKSPACE_STATE_KEY);
        }
        Ok(())
    }

    // ── Recent workspaces (global) ────────────────────────────────────

    pub fn put_recent(&mut self, recent: &RecentWorkspaces) -> anyhow::Result<()> {
        let Some(g) = self.global.as_mut() else {
            return Ok(());
        };
        let val = encode(recent)?;
        g.put(RECENT_KEY, val);
        Ok(())
    }

    pub fn get_recent(&self) -> anyhow::Result<RecentWorkspaces> {
        let Some(g) = self.global.as_ref() else {
            return Ok(RecentWorkspaces::default());
        };
        match g.get(RECENT_KEY) {
            Some(bytes) => Ok(decode(bytes)?),
            None => Ok(RecentWorkspaces::default()),
        }
    }

    // ── Recent files (global) ─────────────────────────────────────────

    pub fn put_recent_files(&mut self, recent: &RecentFiles) -> anyhow::Result<()> {
        let Some(g) = self.global.as_mut() else {
            return Ok(());
        };
        let val = encode(recent)?;
        g.put(RECENT_FILES_KEY, val);
        Ok(())
    }

    pub fn get_recent_files(&self) -> anyhow::Result<RecentFiles> {
        let Some(g) = self.global.as_ref() else {
            return Ok(RecentFiles::default());
        };
        match g.get(RECENT_FILES_KEY) {
            Some(bytes) => Ok(decode(bytes)?),
            None => Ok(RecentFiles::default()),
        }
    }

    // ── Undo state (per-workspace) ────────────────────────────────────

    fn undo_key(file: &Path) -> Vec<u8> {
        let file_hash = path_hash(file);
        format!("{UNDO_PREFIX}{file_hash:016x}").into_bytes()
    }

    pub fn put_undo(&mut self, file: &Path, state: &crate::undo::UndoState) -> anyhow::Result<()> {
        let Some(ws) = self.workspace.as_mut() else {
            return Ok(());
        };
        let key = Self::undo_key(file);
        let val = encode(state)?;
        ws.put(&key, val);
        Ok(())
    }

    pub fn get_undo(&self, file: &Path) -> anyhow::Result<Option<crate::undo::UndoState>> {
        let Some(ws) = self.workspace.as_ref() else {
            return Ok(None);
        };
        let key = Self::undo_key(file);
        match ws.get(&key) {
            Some(bytes) => Ok(Some(decode(bytes)?)),
            None => Ok(None),
        }
    }

    // ── Generic global helpers ────────────────────────────────────────

    pub fn put_raw<T: Serialize>(&mut self, key: &[u8], value: &T) -> anyhow::Result<()> {
        let Some(g) = self.global.as_mut() else {
            return Ok(());
        };
        let val = encode(value)?;
        g.put(key, val);
        Ok(())
    }

    pub fn get_raw<T: DeserializeOwned>(&self, key: &[u8]) -> anyhow::Result<Option<T>> {
        let Some(g) = self.global.as_ref() else {
            return Ok(None);
        };
        match g.get(key) {
            Some(bytes) => Ok(Some(decode(bytes)?)),
            None => Ok(None),
        }
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        if let Some(g) = self.global.as_mut() {
            g.flush()?;
        }
        if let Some(ws) = self.workspace.as_mut() {
            ws.flush()?;
        }
        Ok(())
    }
}

// ── Encoding helpers ──────────────────────────────────────────────────

fn encode<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, BINCODE_CONFIG)
        .map_err(|e| anyhow::anyhow!("bincode encode error: {e}"))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    let (val, _len) = bincode::serde::decode_from_slice(bytes, BINCODE_CONFIG)
        .map_err(|e| anyhow::anyhow!("bincode decode error: {e}"))?;
    Ok(val)
}

fn path_hash(path: &Path) -> u64 {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let bytes = canonical.to_string_lossy();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ws_state() -> WorkspaceState {
        WorkspaceState {
            root: PathBuf::from("/tmp"),
            open_files: vec![PathBuf::from("a.rs")],
            active_file: Some(PathBuf::from("a.rs")),
            ..WorkspaceState::default()
        }
    }

    #[test]
    fn global_round_trip_recent() {
        let tmp = tempfile::tempdir().unwrap();
        let mut db = StateDb::open(tmp.path());
        let mut r = RecentWorkspaces::default();
        r.record_open(&PathBuf::from("/a"));
        db.put_recent(&r).unwrap();
        drop(db);
        let db2 = StateDb::open(tmp.path());
        let loaded = db2.get_recent().unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].root, PathBuf::from("/a"));
    }

    #[test]
    fn workspace_state_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        let mut db = StateDb::open(tmp.path());
        db.attach_workspace(ws_dir.path()).unwrap();
        db.put_workspace(&ws_state()).unwrap();
        db.flush().unwrap();
        drop(db);

        let mut db2 = StateDb::open(tmp.path());
        db2.attach_workspace(ws_dir.path()).unwrap();
        let loaded = db2.get_workspace().unwrap().expect("state");
        assert_eq!(loaded.active_file, ws_state().active_file);
    }

    #[test]
    fn attach_workspace_twice_is_fine() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let mut db1 = StateDb::open(tmp.path());
        db1.attach_workspace(ws.path()).unwrap();
        let mut db2 = StateDb::open(tmp.path());
        // No lock conflict — should succeed where sled would have failed.
        db2.attach_workspace(ws.path()).unwrap();
    }
}
