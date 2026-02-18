//! Workspace state types for session persistence.
//!
//! Defines the data structures for workspace session state — open files,
//! cursor positions, and layout.  Actual persistence is handled by
//! [`crate::state_db::StateDb`] (sled-backed).
//!
//! The structs derive `Serialize`/`Deserialize` for both bincode (sled)
//! and TOML (migration from legacy format).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

// ── Per-workspace state ───────────────────────────────────────────────

/// Persistent workspace session state.
///
/// Stored in the sled database keyed by workspace root path hash.
/// Use [`StateDb::put_workspace`] / [`StateDb::get_workspace`] for I/O.
///
/// [`StateDb::put_workspace`]: crate::state_db::StateDb::put_workspace
/// [`StateDb::get_workspace`]: crate::state_db::StateDb::get_workspace
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceState {
    /// Absolute path to the workspace root directory.
    pub root: PathBuf,
    /// Ordered list of open file paths (relative to workspace root).
    pub open_files: Vec<PathBuf>,
    /// The currently active file (relative to workspace root).
    pub active_file: Option<PathBuf>,
    /// Cursor positions keyed by relative file path.
    ///
    /// Stored as `(line, col)` pairs (0-based).
    pub cursor_positions: HashMap<PathBuf, (usize, usize)>,
    /// Whether the file tree sidebar was visible.
    pub show_file_tree: bool,
    /// File tree width percentage.
    pub file_tree_width_pct: u16,
    /// Whether the right panel (AI/Git) was visible.
    pub show_right_panel: bool,
    /// Right panel width percentage.
    pub right_panel_width_pct: u16,
    /// Unix timestamp of last save (seconds since epoch).
    pub last_saved: u64,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            open_files: Vec::new(),
            active_file: None,
            cursor_positions: HashMap::new(),
            show_file_tree: true,
            file_tree_width_pct: 20,
            show_right_panel: false,
            right_panel_width_pct: 30,
            last_saved: 0,
        }
    }
}

impl WorkspaceState {
    /// Create a new workspace state for the given root.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            ..Self::default()
        }
    }

    /// Compute the deterministic state filename for a workspace root.
    ///
    /// Used by the TOML-to-sled migration path.  New code should use
    /// [`StateDb`] directly.
    ///
    /// [`StateDb`]: crate::state_db::StateDb
    #[must_use]
    pub fn state_filename(root: &Path) -> String {
        let hash = path_hash(root);
        format!("{hash:016x}.toml")
    }

    /// Update the `last_saved` timestamp to now.
    pub fn touch(&mut self) {
        self.last_saved = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
    }

    /// Strip open files that no longer exist on disk.
    ///
    /// Call this after loading to clean up stale entries. Resolves
    /// relative paths against the workspace root.
    pub fn prune_missing_files(&mut self) {
        let root = self.root.clone();
        self.open_files.retain(|rel| root.join(rel).exists());
        if let Some(ref active) = self.active_file {
            if !root.join(active).exists() {
                self.active_file = None;
            }
        }
        self.cursor_positions
            .retain(|rel, _| root.join(rel).exists());
    }
}

// ── Recent workspaces ─────────────────────────────────────────────────

/// Recently opened workspaces index.
///
/// Stored in the sled database under the `recent:workspaces` key.
/// Use [`StateDb::put_recent`] / [`StateDb::get_recent`] for I/O.
///
/// [`StateDb::put_recent`]: crate::state_db::StateDb::put_recent
/// [`StateDb::get_recent`]: crate::state_db::StateDb::get_recent
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RecentWorkspaces {
    /// Ordered list of recently opened workspace roots (most recent first).
    pub entries: Vec<RecentEntry>,
    /// Maximum number of entries to retain.
    pub max_entries: usize,
}

/// A single entry in the recent workspaces list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEntry {
    /// Absolute path to the workspace root.
    pub root: PathBuf,
    /// Unix timestamp of last open (seconds since epoch).
    pub last_opened: u64,
}

impl Default for RecentWorkspaces {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 20,
        }
    }
}

impl RecentWorkspaces {
    /// Record that a workspace was opened. Moves it to the front if
    /// already present, or adds a new entry.
    pub fn record_open(&mut self, root: &Path) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());

        // Remove existing entry for this root.
        self.entries.retain(|e| e.root != root);

        // Insert at front.
        self.entries.insert(
            0,
            RecentEntry {
                root: root.to_path_buf(),
                last_opened: now,
            },
        );

        // Trim excess.
        self.entries.truncate(self.max_entries);
    }

    /// Prune entries whose workspace roots no longer exist on disk.
    pub fn prune_missing(&mut self) {
        self.entries.retain(|e| e.root.exists());
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Compute a deterministic u64 hash of a path for use as a filename.
///
/// Uses FNV-1a for speed and simplicity (not cryptographic).
fn path_hash(path: &Path) -> u64 {
    // Canonicalize if possible, fall back to the provided path.
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

/// Convert an absolute path to a path relative to a workspace root.
///
/// Returns the original path if it's not under the root.
#[must_use]
pub fn make_relative(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_state_defaults() {
        let ws = WorkspaceState::default();
        assert!(ws.open_files.is_empty());
        assert!(ws.active_file.is_none());
        assert!(ws.cursor_positions.is_empty());
        assert!(ws.show_file_tree);
        assert_eq!(ws.file_tree_width_pct, 20);
    }

    #[test]
    fn workspace_state_new() {
        let ws = WorkspaceState::new(PathBuf::from("/tmp/project"));
        assert_eq!(ws.root, PathBuf::from("/tmp/project"));
        assert!(ws.open_files.is_empty());
    }

    #[test]
    fn state_filename_deterministic() {
        let f1 = WorkspaceState::state_filename(Path::new("/tmp/project"));
        let f2 = WorkspaceState::state_filename(Path::new("/tmp/project"));
        assert_eq!(f1, f2);
        assert!(Path::new(&f1)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml")));
    }

    #[test]
    fn state_filename_differs_for_different_paths() {
        let f1 = WorkspaceState::state_filename(Path::new("/tmp/project-a"));
        let f2 = WorkspaceState::state_filename(Path::new("/tmp/project-b"));
        assert_ne!(f1, f2);
    }

    #[test]
    fn touch_updates_timestamp() {
        let mut ws = WorkspaceState::new(PathBuf::from("/tmp/touch-test"));
        assert_eq!(ws.last_saved, 0);
        ws.touch();
        assert!(ws.last_saved > 0);
    }

    #[test]
    fn toml_roundtrip_for_migration() {
        // TOML round-trip is still needed for the migration path.
        let mut ws = WorkspaceState::new(PathBuf::from("/tmp/test"));
        ws.open_files = vec![PathBuf::from("src/main.rs"), PathBuf::from("Cargo.toml")];
        ws.active_file = Some(PathBuf::from("src/main.rs"));
        ws.cursor_positions
            .insert(PathBuf::from("src/main.rs"), (10, 5));
        ws.last_saved = 1_700_000_000;

        let toml_str = toml::to_string_pretty(&ws).unwrap();
        let parsed: WorkspaceState = toml::from_str(&toml_str).unwrap();
        assert_eq!(ws, parsed);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let toml_str = r#"
root = "/tmp/test"
"#;
        let ws: WorkspaceState = toml::from_str(toml_str).unwrap();
        assert_eq!(ws.root, PathBuf::from("/tmp/test"));
        assert!(ws.open_files.is_empty());
        assert!(ws.show_file_tree);
    }

    #[test]
    fn prune_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create one file, leave another missing.
        std::fs::write(root.join("exists.txt"), "hi").unwrap();

        let mut ws = WorkspaceState::new(root);
        ws.open_files = vec![PathBuf::from("exists.txt"), PathBuf::from("gone.txt")];
        ws.active_file = Some(PathBuf::from("gone.txt"));
        ws.cursor_positions
            .insert(PathBuf::from("exists.txt"), (1, 0));
        ws.cursor_positions
            .insert(PathBuf::from("gone.txt"), (5, 3));

        ws.prune_missing_files();

        assert_eq!(ws.open_files, vec![PathBuf::from("exists.txt")]);
        assert!(ws.active_file.is_none()); // was pointing to gone.txt
        assert_eq!(ws.cursor_positions.len(), 1);
        assert!(ws.cursor_positions.contains_key(Path::new("exists.txt")));
    }

    #[test]
    fn recent_workspaces_record_open() {
        let mut recent = RecentWorkspaces::default();
        recent.record_open(Path::new("/tmp/a"));
        recent.record_open(Path::new("/tmp/b"));
        recent.record_open(Path::new("/tmp/a")); // re-open moves to front

        assert_eq!(recent.entries.len(), 2);
        assert_eq!(recent.entries[0].root, PathBuf::from("/tmp/a"));
        assert_eq!(recent.entries[1].root, PathBuf::from("/tmp/b"));
    }

    #[test]
    fn recent_workspaces_truncate() {
        let mut recent = RecentWorkspaces {
            entries: Vec::new(),
            max_entries: 3,
        };
        for i in 0..5 {
            recent.record_open(&PathBuf::from(format!("/tmp/ws{i}")));
        }
        assert_eq!(recent.entries.len(), 3);
        // Most recent should be first
        assert_eq!(recent.entries[0].root, PathBuf::from("/tmp/ws4"));
    }

    #[test]
    fn make_relative_under_root() {
        let root = Path::new("/home/user/project");
        let file = Path::new("/home/user/project/src/main.rs");
        assert_eq!(make_relative(file, root), PathBuf::from("src/main.rs"));
    }

    #[test]
    fn make_relative_outside_root() {
        let root = Path::new("/home/user/project");
        let file = Path::new("/etc/config.toml");
        assert_eq!(make_relative(file, root), PathBuf::from("/etc/config.toml"));
    }

    #[test]
    fn path_hash_consistency() {
        let h1 = path_hash(Path::new("/tmp/test"));
        let h2 = path_hash(Path::new("/tmp/test"));
        assert_eq!(h1, h2);
    }
}
