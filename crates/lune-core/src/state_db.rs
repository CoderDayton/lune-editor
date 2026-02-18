//! Reactive state persistence backed by sled.
//!
//! Provides a thin, typed wrapper around a [`sled::Db`] for persisting
//! workspace session state (layout, open files, cursor positions) and
//! the recent-workspaces index.
//!
//! # Key schema
//!
//! ```text
//! ws:<path_hash>          → bincode WorkspaceState
//! recent:workspaces       → bincode RecentWorkspaces
//! ```
//!
//! All values are serialized with [`bincode`] (via serde compat) for
//! compact, zero-copy-friendly encoding.  Reads are ~10 μs, writes are
//! append-only and batched by sled internally.

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};

use crate::workspace_state::{RecentWorkspaces, WorkspaceState};

/// Bincode configuration: little-endian, varint, limit 16 MiB.
const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

/// The sled database directory name.
const DB_DIR_NAME: &str = "lune.sled";

/// Key prefix for per-workspace state entries.
const WS_PREFIX: &str = "ws:";

/// Key for the recent-workspaces index.
const RECENT_KEY: &[u8] = b"recent:workspaces";

// ── StateDb ───────────────────────────────────────────────────────────

/// Reactive state database backed by sled.
///
/// Wraps a single `sled::Db` instance.  All operations are synchronous
/// (sled is lock-free internally) and safe to call from the main thread
/// without blocking the event loop for any noticeable duration.
pub struct StateDb {
    db: sled::Db,
}

impl StateDb {
    /// Open (or create) the state database in the given config directory.
    ///
    /// The sled directory will be at `<state_dir>/lune.sled/`.
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened.
    pub fn open(state_dir: &Path) -> anyhow::Result<Self> {
        let db_path = state_dir.join(DB_DIR_NAME);
        let db = sled::open(&db_path).map_err(|e| {
            anyhow::anyhow!("failed to open state db at {}: {e}", db_path.display())
        })?;
        Ok(Self { db })
    }

    /// Open a temporary in-memory database (for testing).
    ///
    /// # Errors
    /// Returns an error if the temporary database cannot be created.
    #[cfg(test)]
    pub fn open_temporary() -> anyhow::Result<Self> {
        let config = sled::Config::new().temporary(true);
        let db = config
            .open()
            .map_err(|e| anyhow::anyhow!("failed to open temporary state db: {e}"))?;
        Ok(Self { db })
    }

    // ── Workspace state ───────────────────────────────────────────────

    /// Build the sled key for a workspace root path.
    fn workspace_key(root: &Path) -> Vec<u8> {
        let hash = path_hash(root);
        format!("{WS_PREFIX}{hash:016x}").into_bytes()
    }

    /// Save workspace state.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_workspace(&self, state: &WorkspaceState) -> anyhow::Result<()> {
        let key = Self::workspace_key(&state.root);
        let val = encode(state)?;
        self.db.insert(key, val)?;
        Ok(())
    }

    /// Load workspace state for the given root.
    ///
    /// Returns `None` if no saved state exists for this workspace.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_workspace(&self, root: &Path) -> anyhow::Result<Option<WorkspaceState>> {
        let key = Self::workspace_key(root);
        match self.db.get(key)? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Delete workspace state for the given root.
    ///
    /// # Errors
    /// Returns an error if the sled delete fails.
    pub fn delete_workspace(&self, root: &Path) -> anyhow::Result<()> {
        let key = Self::workspace_key(root);
        self.db.remove(key)?;
        Ok(())
    }

    // ── Recent workspaces ─────────────────────────────────────────────

    /// Save the recent-workspaces index.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_recent(&self, recent: &RecentWorkspaces) -> anyhow::Result<()> {
        let val = encode(recent)?;
        self.db.insert(RECENT_KEY, val)?;
        Ok(())
    }

    /// Load the recent-workspaces index.
    ///
    /// Returns a default (empty) index if none is stored.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_recent(&self) -> anyhow::Result<RecentWorkspaces> {
        match self.db.get(RECENT_KEY)? {
            Some(bytes) => Ok(decode(&bytes)?),
            None => Ok(RecentWorkspaces::default()),
        }
    }

    // ── Generic helpers ───────────────────────────────────────────────

    /// Store an arbitrary serde-serializable value under a raw key.
    ///
    /// # Errors
    /// Returns an error if serialization or the sled write fails.
    pub fn put_raw<T: Serialize>(&self, key: &[u8], value: &T) -> anyhow::Result<()> {
        let val = encode(value)?;
        self.db.insert(key, val)?;
        Ok(())
    }

    /// Load an arbitrary serde-deserializable value from a raw key.
    ///
    /// Returns `None` if the key does not exist.
    ///
    /// # Errors
    /// Returns an error if the sled read or deserialization fails.
    pub fn get_raw<T: DeserializeOwned>(&self, key: &[u8]) -> anyhow::Result<Option<T>> {
        match self.db.get(key)? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Flush all pending writes to disk.
    ///
    /// Sled batches writes internally; this forces an explicit flush.
    /// Useful on clean exit to ensure durability.
    ///
    /// # Errors
    /// Returns an error if the flush fails.
    pub fn flush(&self) -> anyhow::Result<()> {
        self.db.flush()?;
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

/// Compute a deterministic u64 hash of a path for use as a key suffix.
///
/// Uses FNV-1a for speed and simplicity (not cryptographic).
/// Tries to canonicalize first for path equivalence.
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

// ── Migration helper ──────────────────────────────────────────────────

/// Migrate existing TOML workspace state files into the sled database.
///
/// Scans `<state_dir>/*.toml` for workspace state files and imports them.
/// Successfully imported files are deleted.  This is a one-time migration
/// that runs silently on the first launch after the upgrade.
///
/// # Errors
/// Returns an error only if the sled write fails.  Individual TOML parse
/// failures are logged and skipped.
pub fn migrate_toml_state(state_dir: &Path, db: &StateDb) -> anyhow::Result<usize> {
    let mut migrated = 0;

    // Migrate per-workspace state files.
    if let Ok(entries) = std::fs::read_dir(state_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_toml = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
            if !is_toml {
                continue;
            }

            // Skip workspaces.toml — that's the recent-workspaces index.
            let filename = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if filename == "workspaces" {
                // Migrate recent workspaces.
                match std::fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<RecentWorkspaces>(&content) {
                        Ok(recent) => {
                            db.put_recent(&recent)?;
                            let _ = std::fs::remove_file(&path);
                            migrated += 1;
                            log::info!("migrated recent workspaces from TOML to sled");
                        }
                        Err(e) => log::warn!("skip recent workspaces migration: {e}"),
                    },
                    Err(e) => log::warn!("skip recent workspaces migration: {e}"),
                }
                continue;
            }

            // Must be a workspace state file (hex hash).
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<WorkspaceState>(&content) {
                    Ok(ws) => {
                        db.put_workspace(&ws)?;
                        let _ = std::fs::remove_file(&path);
                        migrated += 1;
                        log::info!(
                            "migrated workspace state for {} from TOML to sled",
                            ws.root.display()
                        );
                    }
                    Err(e) => log::warn!("skip workspace state migration {}: {e}", path.display()),
                },
                Err(e) => log::warn!("skip workspace state migration {}: {e}", path.display()),
            }
        }
    }

    Ok(migrated)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_state::RecentEntry;
    use std::collections::HashMap;
    use std::path::PathBuf;

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
        let loaded = db
            .get_workspace(Path::new("/tmp/project"))
            .unwrap()
            .expect("should exist");

        assert_eq!(loaded.root, ws.root);
        assert_eq!(loaded.open_files, ws.open_files);
        assert_eq!(loaded.cursor_positions, ws.cursor_positions);
        assert_eq!(loaded.file_tree_width_pct, 25);
        assert!(!loaded.show_file_tree);
    }

    #[test]
    fn get_workspace_missing_returns_none() {
        let db = test_db();
        let result = db.get_workspace(Path::new("/nonexistent")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_workspace() {
        let db = test_db();
        let ws = WorkspaceState::new(PathBuf::from("/tmp/del-test"));
        db.put_workspace(&ws).unwrap();
        assert!(
            db.get_workspace(Path::new("/tmp/del-test"))
                .unwrap()
                .is_some()
        );

        db.delete_workspace(Path::new("/tmp/del-test")).unwrap();
        assert!(
            db.get_workspace(Path::new("/tmp/del-test"))
                .unwrap()
                .is_none()
        );
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
        assert_eq!(loaded.entries[0].root, PathBuf::from("/tmp/ws1"));
        assert_eq!(loaded.entries[1].root, PathBuf::from("/tmp/ws2"));
    }

    #[test]
    fn get_recent_empty_returns_default() {
        let db = test_db();
        let recent = db.get_recent().unwrap();
        assert!(recent.entries.is_empty());
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
        let root = PathBuf::from("/tmp/overwrite");

        let mut ws1 = WorkspaceState::new(root.clone());
        ws1.file_tree_width_pct = 20;
        db.put_workspace(&ws1).unwrap();

        let mut ws2 = WorkspaceState::new(root.clone());
        ws2.file_tree_width_pct = 35;
        db.put_workspace(&ws2).unwrap();

        let loaded = db.get_workspace(&root).unwrap().unwrap();
        assert_eq!(loaded.file_tree_width_pct, 35);
    }

    #[test]
    fn flush_does_not_error() {
        let db = test_db();
        db.flush().unwrap();
    }

    #[test]
    fn migrate_toml_state_files() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        // Create a fake workspace state TOML file.
        let ws = WorkspaceState::new(PathBuf::from("/tmp/migrate-test"));
        let toml_str = toml::to_string_pretty(&ws).unwrap();
        let filename = WorkspaceState::state_filename(Path::new("/tmp/migrate-test"));
        std::fs::write(state_dir.join(&filename), toml_str).unwrap();

        // Create a fake recent workspaces TOML file.
        let mut recent = RecentWorkspaces::default();
        recent.record_open(Path::new("/tmp/migrate-test"));
        let recent_toml = toml::to_string_pretty(&recent).unwrap();
        std::fs::write(state_dir.join("workspaces.toml"), recent_toml).unwrap();

        // Open DB and migrate.
        let db = test_db();
        let count = migrate_toml_state(state_dir, &db).unwrap();
        assert_eq!(count, 2);

        // Verify workspace state was migrated.
        let loaded_ws = db
            .get_workspace(Path::new("/tmp/migrate-test"))
            .unwrap()
            .expect("workspace should be migrated");
        assert_eq!(loaded_ws.root, PathBuf::from("/tmp/migrate-test"));

        // Verify recent workspaces was migrated.
        let loaded_recent = db.get_recent().unwrap();
        assert!(!loaded_recent.entries.is_empty());

        // Verify TOML files were removed.
        assert!(!state_dir.join(&filename).exists());
        assert!(!state_dir.join("workspaces.toml").exists());
    }
}
