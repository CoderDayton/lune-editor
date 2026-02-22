//! Crash recovery via periodic autosave.
//!
//! Stores snapshots of dirty buffers to disk at a configurable interval.
//! On startup, the editor detects recovery files and offers to restore
//! unsaved work from a prior session that exited uncleanly.
//!
//! # File layout
//!
//! ```text
//! ~/.config/lune-editor/recovery/
//! ├── recovery.toml          # manifest listing all recovery entries
//! └── <path-hash>.bak        # raw text content of a dirty buffer
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::ConfigPaths;

// ── FNV-1a constants ──────────────────────────────────────────────────

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0100_0000_01b3;

// ── Serde helper: u64 as hex string ───────────────────────────────────

/// Serialize / deserialize a `u64` as a hex string in TOML.
///
/// TOML integers are signed 64-bit, so values above `i64::MAX` would
/// fail to round-trip.  This module stores the value as `"0x…"` instead.
mod hex_u64 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    // Serde's `serialize_with` contract requires `&T`, so we cannot
    // change this to pass-by-value.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:016x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let s = String::deserialize(deserializer)?;
        u64::from_str_radix(&s, 16).map_err(serde::de::Error::custom)
    }
}

// ── Manifest types ────────────────────────────────────────────────────

/// Manifest tracking recovery file entries.
///
/// Serialized as `recovery.toml` inside the recovery directory.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecoveryState {
    /// Recovery entries for dirty buffers.
    pub entries: Vec<RecoveryEntry>,
    /// Unix timestamp of last autosave (seconds since epoch).
    pub last_autosave: u64,
}

/// A single recovery entry for a dirty buffer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryEntry {
    /// Original file path on disk.
    pub original_path: PathBuf,
    /// Recovery filename (relative to the recovery directory).
    pub recovery_filename: String,
    /// FNV-1a hash of the buffer content (for change detection).
    ///
    /// Serialized as a hex string because TOML integers are signed 64-bit
    /// and cannot represent the full `u64` range.
    #[serde(with = "hex_u64")]
    pub content_hash: u64,
    /// Unix timestamp when the snapshot was taken.
    pub timestamp: u64,
}

// ── Core API ──────────────────────────────────────────────────────────

impl RecoveryState {
    /// Autosave dirty buffers to the recovery directory.
    ///
    /// For each `(original_path, content)` pair the method:
    /// 1. Computes an FNV-1a hash of `content` for change detection.
    /// 2. Writes `content` to a `.bak` file named after the path hash.
    /// 3. Updates the manifest entry.
    ///
    /// Entries whose original paths are **not** in `dirty_buffers` are
    /// treated as stale — their `.bak` files are deleted and their
    /// manifest entries removed.
    ///
    /// The manifest (`recovery.toml`) is written atomically via a
    /// temporary file + rename.
    pub fn autosave(config: &ConfigPaths, dirty_buffers: &[(PathBuf, &str)]) -> anyhow::Result<()> {
        let recovery_dir = config.recovery_dir();
        std::fs::create_dir_all(&recovery_dir)?;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());

        // Build the set of original paths for quick lookup when pruning.
        let active_paths: std::collections::HashSet<&Path> =
            dirty_buffers.iter().map(|(p, _)| p.as_path()).collect();

        // Load existing manifest to detect stale entries.
        let old_state = Self::load(config)?.unwrap_or_default();

        // Remove stale .bak files.
        for entry in &old_state.entries {
            if !active_paths.contains(entry.original_path.as_path()) {
                let bak = recovery_dir.join(&entry.recovery_filename);
                let _ = std::fs::remove_file(bak);
            }
        }

        // Write new entries.
        let mut entries = Vec::with_capacity(dirty_buffers.len());
        for (original_path, content) in dirty_buffers {
            let hash = content_hash(content);
            let filename = format!("{:016x}.bak", path_hash(original_path));
            let bak_path = recovery_dir.join(&filename);

            std::fs::write(&bak_path, content)?;

            entries.push(RecoveryEntry {
                original_path: original_path.clone(),
                recovery_filename: filename,
                content_hash: hash,
                timestamp: now,
            });
        }

        let state = Self {
            entries,
            last_autosave: now,
        };

        // Atomic manifest write: tmp → rename.
        let manifest_path = recovery_dir.join("recovery.toml");
        let tmp_path = manifest_path.with_extension("toml.tmp");
        let toml_content = toml::to_string_pretty(&state)?;
        std::fs::write(&tmp_path, toml_content)?;
        std::fs::rename(&tmp_path, &manifest_path)?;

        Ok(())
    }

    /// Load the recovery manifest from the recovery directory.
    ///
    /// Returns `None` if no `recovery.toml` exists.
    pub fn load(config: &ConfigPaths) -> anyhow::Result<Option<Self>> {
        let manifest_path = config.recovery_dir().join("recovery.toml");
        if !manifest_path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&manifest_path)?;
        let state: Self = toml::from_str(&content)?;
        Ok(Some(state))
    }

    /// Recover buffer contents from the recovery directory.
    ///
    /// Reads each `.bak` file referenced in the manifest and returns
    /// pairs of `(original_path, recovered_content)`.
    ///
    /// Entries whose `.bak` file is missing on disk are silently skipped.
    pub fn recover(config: &ConfigPaths) -> anyhow::Result<Vec<(PathBuf, String)>> {
        let Some(state) = Self::load(config)? else {
            return Ok(Vec::new());
        };

        let recovery_dir = config.recovery_dir();
        let mut results = Vec::with_capacity(state.entries.len());

        for entry in &state.entries {
            let bak_path = recovery_dir.join(&entry.recovery_filename);
            match std::fs::read_to_string(&bak_path) {
                Ok(content) => results.push((entry.original_path.clone(), content)),
                Err(_) => {
                    // .bak file missing — skip gracefully.
                    log::warn!(
                        "recovery: missing .bak file for {}, skipping",
                        entry.original_path.display()
                    );
                }
            }
        }

        Ok(results)
    }

    /// Delete all recovery files and the manifest.
    ///
    /// Called on clean exit to indicate no crash recovery is needed.
    pub fn clear(config: &ConfigPaths) -> anyhow::Result<()> {
        let recovery_dir = config.recovery_dir();

        // Remove all .bak files.
        if let Ok(read_dir) = std::fs::read_dir(&recovery_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "bak") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        // Remove the manifest.
        let manifest = recovery_dir.join("recovery.toml");
        let _ = std::fs::remove_file(manifest);

        // Also remove any leftover tmp file.
        let tmp = recovery_dir.join("recovery.toml.tmp");
        let _ = std::fs::remove_file(tmp);

        Ok(())
    }

    /// Quick check: does a recovery manifest with entries exist?
    #[must_use]
    pub fn has_recovery(config: &ConfigPaths) -> bool {
        let manifest_path = config.recovery_dir().join("recovery.toml");
        if !manifest_path.exists() {
            return false;
        }
        // Try to parse and check for non-empty entries.
        std::fs::read_to_string(&manifest_path).is_ok_and(|content| {
            toml::from_str::<Self>(&content).is_ok_and(|state| !state.entries.is_empty())
        })
    }
}

// ── Hash helpers ──────────────────────────────────────────────────────

/// Compute a deterministic FNV-1a 64-bit hash of a path.
///
/// Used to derive the `.bak` filename from the original file path.
fn path_hash(path: &Path) -> u64 {
    let bytes = path.to_string_lossy();
    fnv1a(bytes.as_bytes())
}

/// Compute an FNV-1a 64-bit hash of buffer content.
///
/// Used for change detection between autosave cycles.
fn content_hash(content: &str) -> u64 {
    fnv1a(content.as_bytes())
}

/// FNV-1a 64-bit hash of a byte slice.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a `ConfigPaths` rooted in a temp dir and ensure
    /// the directory structure exists.
    fn test_config(dir: &std::path::Path) -> ConfigPaths {
        let config = ConfigPaths::from_root(dir.to_path_buf());
        config.ensure_dirs().unwrap();
        config
    }

    #[test]
    fn recovery_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        let buffers: Vec<(PathBuf, &str)> = vec![
            (PathBuf::from("/tmp/foo.rs"), "fn main() {}"),
            (PathBuf::from("/tmp/bar.txt"), "hello world"),
        ];

        RecoveryState::autosave(&config, &buffers).unwrap();

        // Manifest should exist.
        assert!(config.recovery_dir().join("recovery.toml").exists());

        // Recover should return matching content.
        let recovered = RecoveryState::recover(&config).unwrap();
        assert_eq!(recovered.len(), 2);

        for (path, content) in &recovered {
            let expected = buffers
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, c)| *c)
                .unwrap();
            assert_eq!(content, expected);
        }
    }

    #[test]
    fn clear_removes_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        let buffers: Vec<(PathBuf, &str)> = vec![(PathBuf::from("/tmp/file.rs"), "let x = 1;")];

        RecoveryState::autosave(&config, &buffers).unwrap();
        assert!(config.recovery_dir().join("recovery.toml").exists());

        RecoveryState::clear(&config).unwrap();

        // Manifest gone.
        assert!(!config.recovery_dir().join("recovery.toml").exists());

        // No .bak files remain.
        let bak_count = std::fs::read_dir(config.recovery_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "bak"))
            .count();
        assert_eq!(bak_count, 0);
    }

    #[test]
    fn has_recovery_false_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        assert!(!RecoveryState::has_recovery(&config));
    }

    #[test]
    fn has_recovery_true_after_autosave() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        let buffers: Vec<(PathBuf, &str)> = vec![(PathBuf::from("/tmp/x.rs"), "code")];

        RecoveryState::autosave(&config, &buffers).unwrap();
        assert!(RecoveryState::has_recovery(&config));
    }

    #[test]
    fn stale_entries_removed() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        // Autosave two buffers.
        let buffers_two: Vec<(PathBuf, &str)> = vec![
            (PathBuf::from("/tmp/a.rs"), "aaa"),
            (PathBuf::from("/tmp/b.rs"), "bbb"),
        ];
        RecoveryState::autosave(&config, &buffers_two).unwrap();

        let stale_filename = format!("{:016x}.bak", path_hash(Path::new("/tmp/b.rs")));
        assert!(config.recovery_dir().join(&stale_filename).exists());

        // Autosave only one buffer — b.rs is now stale.
        let buffers_one: Vec<(PathBuf, &str)> = vec![(PathBuf::from("/tmp/a.rs"), "aaa updated")];
        RecoveryState::autosave(&config, &buffers_one).unwrap();

        // The stale .bak file should be removed.
        assert!(!config.recovery_dir().join(&stale_filename).exists());

        // Manifest should only have one entry.
        let state = RecoveryState::load(&config).unwrap().unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].original_path, PathBuf::from("/tmp/a.rs"));
    }

    #[test]
    fn content_hash_consistency() {
        let h1 = content_hash("the quick brown fox");
        let h2 = content_hash("the quick brown fox");
        assert_eq!(h1, h2);

        // Different content produces a different hash.
        let h3 = content_hash("the slow brown fox");
        assert_ne!(h1, h3);
    }

    #[test]
    fn missing_bak_file_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        // Write a manifest with an entry but no corresponding .bak file.
        let state = RecoveryState {
            entries: vec![RecoveryEntry {
                original_path: PathBuf::from("/tmp/phantom.rs"),
                recovery_filename: "0000000000000000.bak".to_owned(),
                content_hash: 0,
                timestamp: 0,
            }],
            last_autosave: 0,
        };

        let manifest = config.recovery_dir().join("recovery.toml");
        let content = toml::to_string_pretty(&state).unwrap();
        std::fs::write(&manifest, content).unwrap();

        // recover() should return an empty vec, not an error.
        let recovered = RecoveryState::recover(&config).unwrap();
        assert!(recovered.is_empty());
    }
}
