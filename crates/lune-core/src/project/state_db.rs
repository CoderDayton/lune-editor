//! Reactive state persistence backed by sled.
//!
//! Layout on disk:
//! ```text
//! <state_dir>/
//! ├── global.sled/               # recent workspaces, saved agent layouts,
//! │                              # and other cross-workspace raw keys
//! └── workspaces/
//!     ├── <path_hash>.sled/      # per-workspace state + undo history
//!     └── ...
//! ```
//!
//! The layout enables multi-instance use: two Lune windows editing *different*
//! workspaces each open their own `workspaces/<hash>.sled`, while both share
//! the single `global.sled`. Opening the same workspace in two instances is
//! still a conflict — the second instance will fail to attach the workspace
//! DB and run without per-workspace persistence.
//!
//! All values are serialized with [`bincode`] (via serde compat).

use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use crate::workspace_state::{RecentWorkspaces, WorkspaceState};

/// Bincode configuration: little-endian, varint, limit 16 MiB.
const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

/// Directory name for the global (cross-workspace) sled DB.
const GLOBAL_DIR_NAME: &str = "global.sled";

/// Directory name for the per-workspace sled DB collection.
const WORKSPACES_SUBDIR: &str = "workspaces";

/// Key for the recent-workspaces index (in the global DB).
const RECENT_KEY: &[u8] = b"recent:workspaces";

/// Sole key for the workspace state blob (in the per-workspace DB).
const WORKSPACE_STATE_KEY: &[u8] = b"workspace:state";

/// Key prefix for per-file undo state (in the per-workspace DB).
const UNDO_PREFIX: &str = "undo:";

// ── StateDb ───────────────────────────────────────────────────────────

/// Reactive state database with split global / per-workspace storage.
///
/// Both `global` and `workspace` are `Option<sled::Db>` so that lock
/// contention on one doesn't prevent the other from being usable. Multiple
/// Lune instances can concurrently persist *per-workspace* data as long as
/// they're editing different workspaces; the global DB is held by whichever
/// instance opened it first, and the others fall back to no-op for global
/// reads/writes.
pub struct StateDb {
    state_dir: PathBuf,
    global: Option<sled::Db>,
    workspace: Option<sled::Db>,
}

impl StateDb {
    /// Open the state database in the given config directory.
    ///
    /// Best-effort: if the global sled DB cannot be opened (most commonly
    /// because another Lune instance holds the lock), returns a `StateDb`
    /// with global persistence disabled but still usable for per-workspace
    /// data after [`StateDb::attach_workspace`] succeeds.
    #[must_use]
    pub fn open(state_dir: &Path) -> Self {
        let global_path = state_dir.join(GLOBAL_DIR_NAME);
        let global = match sled::open(&global_path) {
            Ok(db) => Some(db),
            Err(e) => {
                eprintln!(
                    "Warning: global state db unavailable ({e}). Recent workspaces and saved agent layouts will not persist from this instance."
                );
                None
            }
        };
        Self {
            state_dir: state_dir.to_path_buf(),
            global,
            workspace: None,
        }
    }

    /// Open a fully in-memory database for testing (global + workspace
    /// both attached).
    #[cfg(test)]
    pub fn open_temporary() -> anyhow::Result<Self> {
        let global = sled::Config::new()
            .temporary(true)
            .open()
            .map_err(|e| anyhow::anyhow!("failed to open temporary global db: {e}"))?;
        let workspace = sled::Config::new()
            .temporary(true)
            .open()
            .map_err(|e| anyhow::anyhow!("failed to open temporary workspace db: {e}"))?;
        Ok(Self {
            state_dir: PathBuf::new(),
            global: Some(global),
            workspace: Some(workspace),
        })
    }

    /// Attempt to attach a per-workspace sled database for the given
    /// workspace root.
    ///
    /// On success, workspace-scoped operations persist to
    /// `<state_dir>/workspaces/<hash>.sled/`.
    ///
    /// # Errors
    /// Returns an error if the sled DB cannot be opened (most commonly,
    /// the directory lock is already held by another Lune instance editing
    /// the same workspace). The caller should log the error and continue;
    /// the global DB remains usable and workspace-scoped ops silently no-op.
    pub fn attach_workspace(&mut self, workspace_root: &Path) -> anyhow::Result<()> {
        // Retry briefly on lock contention: handles the common "I just closed
        // the other instance" race where the previous process is still
        // releasing its sled flock when the new one boots.
        const ATTEMPTS: usize = 3;
        const DELAY_MS: u64 = 50;

        let ws_path = self.workspace_db_path(workspace_root);
        if let Some(parent) = ws_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!(
                    "failed to create workspaces dir at {}: {e}",
                    parent.display()
                )
            })?;
        }
        let mut last_err = None;
        for attempt in 0..ATTEMPTS {
            match sled::open(&ws_path) {
                Ok(ws) => {
                    self.workspace = Some(ws);
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < ATTEMPTS {
                        std::thread::sleep(std::time::Duration::from_millis(DELAY_MS));
                    }
                }
            }
        }
        Err(anyhow::anyhow!(
            "failed to open workspace state db at {} after {ATTEMPTS} attempts: {}",
            ws_path.display(),
            last_err.expect("loop sets last_err on each failure")
        ))
    }

    /// Whether the global (cross-workspace) database is currently open.
    #[must_use]
    pub const fn has_global(&self) -> bool {
        self.global.is_some()
    }

    /// Whether a per-workspace database is currently attached.
    #[must_use]
    pub const fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// Path where this workspace's sled DB lives on disk.
    fn workspace_db_path(&self, workspace_root: &Path) -> PathBuf {
        let hash = path_hash(workspace_root);
        self.state_dir
            .join(WORKSPACES_SUBDIR)
            .join(format!("{hash:016x}.sled"))
    }
}

// ── Workspace state ───────────────────────────────────────────────────

impl StateDb {
    /// Save workspace state to the attached workspace DB.
    ///
    /// No-op if no workspace is attached.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_workspace(&self, state: &WorkspaceState) -> anyhow::Result<()> {
        let Some(ws_db) = &self.workspace else {
            return Ok(());
        };
        let val = encode(state)?;
        ws_db.insert(WORKSPACE_STATE_KEY, val)?;
        Ok(())
    }

    /// Load workspace state from the attached workspace DB.
    ///
    /// Returns `None` if no workspace is attached or no state is stored.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_workspace(&self) -> anyhow::Result<Option<WorkspaceState>> {
        let Some(ws_db) = &self.workspace else {
            return Ok(None);
        };
        match ws_db.get(WORKSPACE_STATE_KEY)? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Delete the workspace state blob from the attached workspace DB.
    ///
    /// No-op if no workspace is attached.
    ///
    /// # Errors
    /// Returns an error if the sled delete fails.
    pub fn delete_workspace(&self) -> anyhow::Result<()> {
        if let Some(ws_db) = &self.workspace {
            ws_db.remove(WORKSPACE_STATE_KEY)?;
        }
        Ok(())
    }

    // ── Recent workspaces (global) ────────────────────────────────────

    /// Save the recent-workspaces index to the global DB.
    ///
    /// No-op if the global DB is unavailable.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_recent(&self, recent: &RecentWorkspaces) -> anyhow::Result<()> {
        let Some(global) = &self.global else {
            return Ok(());
        };
        let val = encode(recent)?;
        global.insert(RECENT_KEY, val)?;
        Ok(())
    }

    /// Load the recent-workspaces index from the global DB.
    ///
    /// Returns a default (empty) index if no global DB is open or no
    /// index is stored.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_recent(&self) -> anyhow::Result<RecentWorkspaces> {
        let Some(global) = &self.global else {
            return Ok(RecentWorkspaces::default());
        };
        match global.get(RECENT_KEY)? {
            Some(bytes) => Ok(decode(&bytes)?),
            None => Ok(RecentWorkspaces::default()),
        }
    }

    // ── Undo state (per-workspace) ────────────────────────────────────

    /// Build a sled key for per-file undo state within the attached
    /// workspace DB.
    fn undo_key(file: &Path) -> Vec<u8> {
        let file_hash = path_hash(file);
        format!("{UNDO_PREFIX}{file_hash:016x}").into_bytes()
    }

    /// Persist undo/redo history for a file within the attached workspace.
    ///
    /// No-op if no workspace is attached.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_undo(&self, file: &Path, state: &crate::undo::UndoState) -> anyhow::Result<()> {
        let Some(ws_db) = &self.workspace else {
            return Ok(());
        };
        let key = Self::undo_key(file);
        let val = encode(state)?;
        ws_db.insert(key, val)?;
        Ok(())
    }

    /// Load persisted undo/redo history for a file.
    ///
    /// Returns `None` if no workspace is attached or no state is stored.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_undo(&self, file: &Path) -> anyhow::Result<Option<crate::undo::UndoState>> {
        let Some(ws_db) = &self.workspace else {
            return Ok(None);
        };
        let key = Self::undo_key(file);
        match ws_db.get(key)? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }
}

// ── Generic global helpers ────────────────────────────────────────────

impl StateDb {
    /// Store an arbitrary serde-serializable value under a raw key in the
    /// **global** DB. No-op if the global DB is unavailable.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_raw<T: Serialize>(&self, key: &[u8], value: &T) -> anyhow::Result<()> {
        let Some(global) = &self.global else {
            return Ok(());
        };
        let val = encode(value)?;
        global.insert(key, val)?;
        Ok(())
    }

    /// Load an arbitrary serde-deserializable value from a raw key in the
    /// **global** DB.
    ///
    /// Returns `None` if the global DB is unavailable or the key does
    /// not exist.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_raw<T: DeserializeOwned>(&self, key: &[u8]) -> anyhow::Result<Option<T>> {
        let Some(global) = &self.global else {
            return Ok(None);
        };
        match global.get(key)? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Flush all pending writes (global + attached workspace) to disk.
    ///
    /// # Errors
    /// Returns an error if either flush fails.
    pub fn flush(&self) -> anyhow::Result<()> {
        if let Some(global) = &self.global {
            global.flush()?;
        }
        if let Some(ws) = &self.workspace {
            ws.flush()?;
        }
        Ok(())
    }
}

// ── Encoding helpers ──────────────────────────────────────────────────

/// Encode a value to bincode bytes via serde.
fn encode<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, BINCODE_CONFIG)
        .map_err(|e| anyhow::anyhow!("bincode encode error: {e}"))
}

/// Decode a value from bincode bytes via serde.
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    let (val, _len) = bincode::serde::decode_from_slice(bytes, BINCODE_CONFIG)
        .map_err(|e| anyhow::anyhow!("bincode decode error: {e}"))?;
    Ok(val)
}

// ── Path hashing ──────────────────────────────────────────────────────

/// Compute a deterministic u64 hash of a path for use as a directory name
/// suffix. Uses FNV-1a (not cryptographic). Canonicalizes first when possible.
fn path_hash(path: &Path) -> u64 {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let bytes = canonical.to_string_lossy();

    // FNV-1a 64-bit
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_state::RecentEntry;
    use std::collections::HashMap;

    fn test_db() -> StateDb {
        StateDb::open_temporary().expect("temporary db")
    }

    #[test]
    fn put_get_workspace_roundtrip() {
        let db = test_db();
        let mut ws = WorkspaceState::new(PathBuf::from("/tmp/project"));
        ws.open_files = vec![PathBuf::from("src/main.rs")];
        ws.cursor_positions
            .insert(PathBuf::from("src/main.rs"), (42, 7));
        ws.file_tree_width_pct = 25;
        ws.show_file_tree = false;

        db.put_workspace(&ws).unwrap();
        let loaded = db.get_workspace().unwrap().expect("should exist");

        assert_eq!(loaded.root, ws.root);
        assert_eq!(loaded.open_files, ws.open_files);
        assert_eq!(loaded.cursor_positions, ws.cursor_positions);
        assert_eq!(loaded.file_tree_width_pct, 25);
        assert!(!loaded.show_file_tree);
    }

    #[test]
    fn get_workspace_without_attach_returns_none() {
        let db = StateDb {
            state_dir: PathBuf::new(),
            global: Some(sled::Config::new().temporary(true).open().unwrap()),
            workspace: None,
        };
        assert!(db.get_workspace().unwrap().is_none());
    }

    #[test]
    fn put_workspace_without_attach_is_noop() {
        let db = StateDb {
            state_dir: PathBuf::new(),
            global: Some(sled::Config::new().temporary(true).open().unwrap()),
            workspace: None,
        };
        let ws = WorkspaceState::new(PathBuf::from("/tmp/x"));
        db.put_workspace(&ws).unwrap();
        assert!(db.get_workspace().unwrap().is_none());
    }

    #[test]
    fn delete_workspace_removes_state() {
        let db = test_db();
        let ws = WorkspaceState::new(PathBuf::from("/tmp/del-test"));
        db.put_workspace(&ws).unwrap();
        assert!(db.get_workspace().unwrap().is_some());
        db.delete_workspace().unwrap();
        assert!(db.get_workspace().unwrap().is_none());
    }

    #[test]
    fn put_get_recent_roundtrip() {
        let db = test_db();
        let mut recent = RecentWorkspaces::default();
        recent.entries.push(RecentEntry {
            root: PathBuf::from("/tmp/ws1"),
            last_opened: 1_700_000_000,
        });
        recent.entries.push(RecentEntry {
            root: PathBuf::from("/tmp/ws2"),
            last_opened: 1_700_001_000,
        });

        db.put_recent(&recent).unwrap();
        let loaded = db.get_recent().unwrap();
        assert_eq!(loaded.entries.len(), 2);
    }

    #[test]
    fn get_recent_empty_returns_default() {
        let db = test_db();
        assert!(db.get_recent().unwrap().entries.is_empty());
    }

    #[test]
    fn put_get_raw_custom_type() {
        let db = test_db();
        let mut map: HashMap<String, u32> = HashMap::new();
        map.insert("a".to_owned(), 1);
        map.insert("b".to_owned(), 2);

        db.put_raw(b"test:custom", &map).unwrap();
        let loaded: HashMap<String, u32> = db.get_raw(b"test:custom").unwrap().unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn overwrite_workspace_state() {
        let db = test_db();
        let mut ws1 = WorkspaceState::new(PathBuf::from("/tmp/overwrite"));
        ws1.file_tree_width_pct = 20;
        db.put_workspace(&ws1).unwrap();

        let mut ws2 = WorkspaceState::new(PathBuf::from("/tmp/overwrite"));
        ws2.file_tree_width_pct = 35;
        db.put_workspace(&ws2).unwrap();

        assert_eq!(db.get_workspace().unwrap().unwrap().file_tree_width_pct, 35);
    }

    #[test]
    fn flush_does_not_error() {
        test_db().flush().unwrap();
    }

    #[test]
    fn attach_workspace_creates_directory_and_db() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();
        let mut db = StateDb::open(state_dir);
        assert!(!db.has_workspace());
        db.attach_workspace(&PathBuf::from("/tmp/proj-a")).unwrap();
        assert!(db.has_workspace());
        assert!(state_dir.join(WORKSPACES_SUBDIR).exists());
    }

    #[test]
    fn second_instance_on_different_workspace_keeps_its_own_state() {
        // Sled holds an exclusive lock on global.sled, but workspace DBs
        // are isolated by path hash. Simulate a second instance that can't
        // get the global DB but can still persist its workspace data.
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        // First instance: holds global.sled and workspaces/<a>.sled.
        let mut first = StateDb::open(state_dir);
        first
            .attach_workspace(&PathBuf::from("/tmp/proj-a"))
            .unwrap();
        assert!(first.global.is_some());

        // Second instance: global.sled is locked, so it gets None for global.
        // But it can still attach to a different workspace.
        let mut second = StateDb::open(state_dir);
        assert!(
            second.global.is_none(),
            "second instance should not hold the global lock"
        );
        second
            .attach_workspace(&PathBuf::from("/tmp/proj-b"))
            .expect("different workspace should attach cleanly");
        assert!(second.has_workspace());

        // Per-workspace writes from the second instance persist.
        let ws = WorkspaceState::new(PathBuf::from("/tmp/proj-b"));
        second.put_workspace(&ws).unwrap();
        assert!(second.get_workspace().unwrap().is_some());
    }

    #[test]
    fn undo_state_round_trips_within_workspace() {
        let db = test_db();
        let state = crate::undo::UndoState::default();
        db.put_undo(Path::new("/tmp/proj/src/main.rs"), &state)
            .unwrap();
        assert!(
            db.get_undo(Path::new("/tmp/proj/src/main.rs"))
                .unwrap()
                .is_some()
        );
    }
}
