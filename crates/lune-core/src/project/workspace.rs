//! Workspace abstraction — directory tree and file operations.
//!
//! A [`Workspace`] represents an opened project directory. It provides:
//! - Lazy directory listing with caching
//! - Sorted entries (directories first, then files, alphabetical)
//! - Cache invalidation (for file watcher integration)
//! - File operations (create, rename, delete, move)

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// A workspace rooted at a directory.
#[derive(Debug)]
pub struct Workspace {
    /// Absolute path to the workspace root.
    root: PathBuf,
    /// Display name (last component of root path).
    name: String,
    /// Cached directory listings, keyed by directory path.
    tree_cache: FxHashMap<PathBuf, Vec<DirEntry>>,
    /// Whether to include hidden files (dotfiles).
    show_hidden: bool,
}

/// A single entry in a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Absolute path to this entry.
    pub path: PathBuf,
    /// File/directory name (last component).
    pub name: String,
    /// Whether this is a file, directory, or symlink.
    pub kind: EntryKind,
    /// Git status (set externally by git integration).
    pub git_status: Option<FileStatus>,
}

/// The kind of a directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory, which may be expanded or collapsed.
    Directory {
        /// Whether this directory is expanded in the tree view.
        expanded: bool,
    },
    /// A symbolic link.
    Symlink,
}

/// Git status indicator for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// File is modified.
    Modified,
    /// File is newly added / staged.
    Added,
    /// File is untracked.
    Untracked,
    /// File is deleted.
    Deleted,
    /// File is renamed.
    Renamed,
    /// File is ignored.
    Ignored,
    /// File is conflicted.
    Conflicted,
}

/// File operations that can be executed against a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOp {
    /// Create a new file at the given path.
    CreateFile(PathBuf),
    /// Create a new directory at the given path.
    CreateDir(PathBuf),
    /// Rename an entry from one path to another.
    Rename { from: PathBuf, to: PathBuf },
    /// Delete a file or directory.
    Delete(PathBuf),
    /// Move an entry from one path to another.
    Move { from: PathBuf, to: PathBuf },
}

impl Workspace {
    /// Open a workspace rooted at the given directory.
    ///
    /// # Errors
    /// Returns an error if the path does not exist or is not a directory.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let root = std::fs::canonicalize(&root).with_context(|| {
            format!("failed to canonicalize workspace root: {}", root.display())
        })?;

        if !root.is_dir() {
            bail!("workspace root is not a directory: {}", root.display());
        }

        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        Ok(Self {
            root,
            name,
            tree_cache: FxHashMap::default(),
            show_hidden: false,
        })
    }

    /// The absolute root path of the workspace.
    #[inline]
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The display name of the workspace.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether hidden files are shown.
    #[must_use]
    pub const fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Toggle hidden file visibility.
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        // Clear cache since filtering changed.
        self.tree_cache.clear();
    }

    /// Set hidden file visibility.
    pub fn set_show_hidden(&mut self, show: bool) {
        if self.show_hidden != show {
            self.show_hidden = show;
            self.tree_cache.clear();
        }
    }

    /// List the contents of a directory, returning sorted entries.
    ///
    /// Results are cached; call [`invalidate`] to force a re-read.
    /// Directories appear first, then files, both sorted alphabetically
    /// (case-insensitive).
    ///
    /// # Errors
    /// Returns an error if the directory cannot be read.
    pub fn list_dir(&mut self, dir: &Path) -> Result<&[DirEntry]> {
        if !self.tree_cache.contains_key(dir) {
            let entries = read_dir_sorted(dir, self.show_hidden)?;
            self.tree_cache.insert(dir.to_path_buf(), entries);
        }
        // SAFETY: we just inserted if missing, so unwrap is safe.
        Ok(self.tree_cache.get(dir).expect("just inserted"))
    }

    /// Invalidate the cache for a specific directory, forcing a re-read
    /// on the next [`list_dir`] call.
    pub fn invalidate(&mut self, dir: &Path) {
        self.tree_cache.remove(dir);
    }

    /// Invalidate the entire cache.
    pub fn invalidate_all(&mut self) {
        self.tree_cache.clear();
    }

    /// Convert an absolute path to a path relative to the workspace root.
    ///
    /// Returns `None` if the path is not inside the workspace.
    #[must_use]
    pub fn relative_path(&self, abs_path: &Path) -> Option<PathBuf> {
        abs_path.strip_prefix(&self.root).ok().map(PathBuf::from)
    }

    /// Execute a file operation.
    ///
    /// After execution, the cache for affected directories is invalidated.
    ///
    /// # Errors
    /// Returns an error if the file operation fails.
    pub fn execute(&mut self, op: &FileOp) -> Result<()> {
        match op {
            FileOp::CreateFile(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create parent dirs for {}", path.display())
                    })?;
                }
                std::fs::write(path, "")
                    .with_context(|| format!("failed to create file: {}", path.display()))?;
                self.invalidate_parent(path);
            }
            FileOp::CreateDir(path) => {
                std::fs::create_dir_all(path)
                    .with_context(|| format!("failed to create directory: {}", path.display()))?;
                self.invalidate_parent(path);
            }
            FileOp::Rename { from, to } | FileOp::Move { from, to } => {
                std::fs::rename(from, to).with_context(|| {
                    format!("failed to rename {} to {}", from.display(), to.display())
                })?;
                self.invalidate_parent(from);
                self.invalidate_parent(to);
            }
            FileOp::Delete(path) => {
                if path.is_dir() {
                    std::fs::remove_dir_all(path).with_context(|| {
                        format!("failed to delete directory: {}", path.display())
                    })?;
                } else {
                    std::fs::remove_file(path)
                        .with_context(|| format!("failed to delete file: {}", path.display()))?;
                }
                self.invalidate_parent(path);
                // Also invalidate the deleted dir itself if it was cached.
                self.tree_cache.remove(path);
            }
        }
        Ok(())
    }

    /// Check if a path is inside this workspace.
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }

    /// Get the expansion state of a directory in the cache.
    /// Returns `None` if the directory is not in the cache.
    #[must_use]
    pub fn is_expanded(&self, dir: &Path) -> Option<bool> {
        // Check if this dir appears as an entry in its parent's cache.
        let parent = dir.parent()?;
        let entries = self.tree_cache.get(parent)?;
        entries.iter().find(|e| e.path == dir).and_then(|e| {
            if let EntryKind::Directory { expanded } = e.kind {
                Some(expanded)
            } else {
                None
            }
        })
    }

    /// Set the expansion state of a directory entry.
    /// Returns `true` if the entry was found and updated.
    pub fn set_expanded(&mut self, dir: &Path, expanded: bool) -> bool {
        self.modify_expanded(dir, |_| expanded).is_some()
    }

    /// Toggle the expansion state of a directory entry.
    /// Returns the new expansion state, or `None` if not found.
    pub fn toggle_expanded(&mut self, dir: &Path) -> Option<bool> {
        self.modify_expanded(dir, |cur| !cur)
    }

    /// Apply `f` to the expansion state of a directory entry, returning
    /// the new state. Shared implementation for `set_expanded`/`toggle_expanded`.
    fn modify_expanded(&mut self, dir: &Path, f: impl FnOnce(bool) -> bool) -> Option<bool> {
        let parent = dir.parent()?;
        let entries = self.tree_cache.get_mut(parent)?;
        for entry in entries {
            if entry.path == dir {
                if let EntryKind::Directory {
                    expanded: ref mut exp,
                } = entry.kind
                {
                    *exp = f(*exp);
                    return Some(*exp);
                }
            }
        }
        None
    }

    /// Invalidate the parent directory of a path.
    fn invalidate_parent(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.invalidate(parent);
        }
    }
}

/// Read a directory and return sorted entries.
///
/// Sorting: directories first, then files; within each group,
/// alphabetical case-insensitive.
fn read_dir_sorted(dir: &Path, show_hidden: bool) -> Result<Vec<DirEntry>> {
    let read_dir = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    let mut entries = Vec::new();

    for entry in read_dir {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip hidden files unless show_hidden is true.
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        // Always skip .git directory.
        if name == ".git" {
            continue;
        }

        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to get file type for {}", path.display()))?;

        let kind = if file_type.is_dir() {
            EntryKind::Directory { expanded: false }
        } else if file_type.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };

        entries.push(DirEntry {
            path,
            name,
            kind,
            git_status: None,
        });
    }

    // Sort: directories first, then files; alphabetical within each group.
    // Uses char-by-char case-insensitive comparison to avoid allocating
    // two temporary Strings per comparison (was O(N log N) allocs).
    entries.sort_by(|a, b| {
        let a_is_dir = matches!(a.kind, EntryKind::Directory { .. });
        let b_is_dir = matches!(b.kind, EntryKind::Directory { .. });
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| cmp_ignore_ascii_case(&a.name, &b.name))
    });

    Ok(entries)
}

/// Flatten a workspace tree into a list of `(depth, DirEntry)` pairs
/// for rendering. Only expanded directories have their children included.
///
/// This is the primary API for the file tree widget to get a renderable
/// list of entries.
///
/// # Errors
/// Returns an error if any directory cannot be read.
pub fn flatten_tree(workspace: &mut Workspace) -> Result<Vec<(usize, DirEntry)>> {
    let root = workspace.root().to_path_buf();
    let mut result = Vec::new();
    flatten_dir(workspace, &root, 0, &mut result)?;
    Ok(result)
}

/// Recursively flatten a directory into the result vec.
fn flatten_dir(
    workspace: &mut Workspace,
    dir: &Path,
    depth: usize,
    result: &mut Vec<(usize, DirEntry)>,
) -> Result<()> {
    // Clone entries to avoid borrow conflict with workspace.
    let entries: Vec<DirEntry> = workspace.list_dir(dir)?.to_vec();

    for entry in entries {
        let is_expanded = matches!(entry.kind, EntryKind::Directory { expanded: true });
        let child_path = entry.path.clone();
        result.push((depth, entry));

        if is_expanded {
            flatten_dir(workspace, &child_path, depth + 1, result)?;
        }
    }

    Ok(())
}

/// Case-insensitive ASCII string comparison without allocating.
///
/// Falls back to Unicode lowercasing only for non-ASCII chars (rare for
/// typical file names).
fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars();
    let mut b_chars = b.chars();
    loop {
        match (a_chars.next(), b_chars.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) => {
                // Fast path for ASCII (the common case for filenames).
                let ord = if ac.is_ascii() && bc.is_ascii() {
                    ac.to_ascii_lowercase().cmp(&bc.to_ascii_lowercase())
                } else {
                    // Fallback: compare lowercased chars (may yield
                    // multiple chars per input, but for single-char
                    // comparison this is fine).
                    let al = ac.to_lowercase().next().unwrap_or(ac);
                    let bl = bc.to_lowercase().next().unwrap_or(bc);
                    al.cmp(&bl)
                };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp directory with a known structure for testing.
    fn setup_test_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let root = tmp.path().to_path_buf();

        // Create structure:
        // root/
        //   .hidden_file
        //   alpha/
        //     nested.txt
        //   beta.txt
        //   gamma.rs
        //   .secret/
        //     hidden.txt
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::create_dir_all(root.join(".secret")).unwrap();
        std::fs::write(root.join(".hidden_file"), "").unwrap();
        std::fs::write(root.join("alpha/nested.txt"), "nested content").unwrap();
        std::fs::write(root.join("beta.txt"), "beta").unwrap();
        std::fs::write(root.join("gamma.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join(".secret/hidden.txt"), "secret").unwrap();

        (tmp, root)
    }

    #[test]
    fn open_workspace() {
        let (_tmp, root) = setup_test_dir();
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.root(), root);
        assert!(!ws.name().is_empty());
    }

    #[test]
    fn open_nonexistent_errors() {
        let result = Workspace::open("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
    }

    #[test]
    fn open_file_as_workspace_errors() {
        let (_tmp, root) = setup_test_dir();
        let result = Workspace::open(root.join("beta.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn list_dir_sorted_dirs_first() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();
        let entries = ws.list_dir(&root).unwrap();

        // Should have: alpha (dir), beta.txt (file), gamma.rs (file)
        // Hidden files should be excluded by default.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "alpha");
        assert!(matches!(
            entries[0].kind,
            EntryKind::Directory { expanded: false }
        ));
        assert_eq!(entries[1].name, "beta.txt");
        assert!(matches!(entries[1].kind, EntryKind::File));
        assert_eq!(entries[2].name, "gamma.rs");
    }

    #[test]
    fn list_dir_cached() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        // First call populates cache.
        let entries1 = ws.list_dir(&root).unwrap().len();
        // Create a new file — should NOT appear because cache is used.
        std::fs::write(root.join("new_file.txt"), "new").unwrap();
        let entries2 = ws.list_dir(&root).unwrap().len();
        assert_eq!(entries1, entries2);

        // After invalidation, new file should appear.
        ws.invalidate(&root);
        let entries3 = ws.list_dir(&root).unwrap().len();
        assert_eq!(entries3, entries1 + 1);
    }

    #[test]
    fn show_hidden_includes_dotfiles() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();
        ws.set_show_hidden(true);

        let entries = ws.list_dir(&root).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        // Should include .hidden_file and .secret but NOT .git
        assert!(names.contains(&".hidden_file"));
        assert!(names.contains(&".secret"));
    }

    #[test]
    fn toggle_hidden_clears_cache() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        // Populate cache.
        let _ = ws.list_dir(&root).unwrap();
        assert!(!ws.tree_cache.is_empty());

        ws.toggle_hidden();
        assert!(ws.tree_cache.is_empty());
        assert!(ws.show_hidden());
    }

    #[test]
    fn relative_path() {
        let (_tmp, root) = setup_test_dir();
        let ws = Workspace::open(&root).unwrap();

        let abs = root.join("alpha/nested.txt");
        let rel = ws.relative_path(&abs).unwrap();
        assert_eq!(rel, PathBuf::from("alpha/nested.txt"));

        // Path outside workspace returns None.
        assert!(ws.relative_path(Path::new("/some/other/path")).is_none());
    }

    #[test]
    fn contains_path() {
        let (_tmp, root) = setup_test_dir();
        let ws = Workspace::open(&root).unwrap();

        assert!(ws.contains(&root.join("alpha")));
        assert!(!ws.contains(Path::new("/some/other/path")));
    }

    #[test]
    fn execute_create_file() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        let new_file = root.join("new_file.txt");
        ws.execute(&FileOp::CreateFile(new_file.clone())).unwrap();
        assert!(new_file.exists());
    }

    #[test]
    fn execute_create_dir() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        let new_dir = root.join("new_dir/sub_dir");
        ws.execute(&FileOp::CreateDir(new_dir.clone())).unwrap();
        assert!(new_dir.is_dir());
    }

    #[test]
    fn execute_rename() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        let from = root.join("beta.txt");
        let to = root.join("beta_renamed.txt");
        ws.execute(&FileOp::Rename {
            from: from.clone(),
            to: to.clone(),
        })
        .unwrap();
        assert!(!from.exists());
        assert!(to.exists());
    }

    #[test]
    fn execute_delete_file() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        let file = root.join("gamma.rs");
        assert!(file.exists());
        ws.execute(&FileOp::Delete(file.clone())).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn execute_delete_dir() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        let dir = root.join("alpha");
        assert!(dir.is_dir());
        ws.execute(&FileOp::Delete(dir.clone())).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn expand_toggle() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        // Populate cache so entries exist.
        let _ = ws.list_dir(&root).unwrap();

        let alpha_dir = root.join("alpha");

        // Initially collapsed.
        assert_eq!(ws.is_expanded(&alpha_dir), Some(false));

        // Toggle to expanded.
        assert_eq!(ws.toggle_expanded(&alpha_dir), Some(true));
        assert_eq!(ws.is_expanded(&alpha_dir), Some(true));

        // Toggle back.
        assert_eq!(ws.toggle_expanded(&alpha_dir), Some(false));
        assert_eq!(ws.is_expanded(&alpha_dir), Some(false));
    }

    #[test]
    fn flatten_tree_basic() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        // All collapsed — should get root level entries only.
        let flat = flatten_tree(&mut ws).unwrap();
        assert_eq!(flat.len(), 3); // alpha, beta.txt, gamma.rs

        // Check depths are all 0.
        for (depth, _) in &flat {
            assert_eq!(*depth, 0);
        }
    }

    #[test]
    fn flatten_tree_expanded() {
        let (_tmp, root) = setup_test_dir();
        let mut ws = Workspace::open(&root).unwrap();

        // Populate cache.
        let _ = ws.list_dir(&root).unwrap();

        // Expand alpha.
        let alpha_dir = root.join("alpha");
        ws.set_expanded(&alpha_dir, true);

        let flat = flatten_tree(&mut ws).unwrap();
        // Should have: alpha(0), nested.txt(1), beta.txt(0), gamma.rs(0)
        assert_eq!(flat.len(), 4);
        assert_eq!(flat[0].1.name, "alpha");
        assert_eq!(flat[0].0, 0);
        assert_eq!(flat[1].1.name, "nested.txt");
        assert_eq!(flat[1].0, 1);
        assert_eq!(flat[2].1.name, "beta.txt");
        assert_eq!(flat[2].0, 0);
    }
}
