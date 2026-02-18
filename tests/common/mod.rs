//! Shared test helpers for integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use lune_core::buffer::TextBuffer;
use lune_core::workspace::Workspace;

// ── Test buffer helpers ───────────────────────────────────────────────

/// Create a `TextBuffer` with known content for testing.
pub fn test_buffer(content: &str) -> TextBuffer {
    TextBuffer::from_text(content)
}

/// Create a `TextBuffer` with numbered lines for position-based tests.
///
/// Generates lines like `"line 0\nline 1\nline 2\n..."`.
pub fn numbered_buffer(line_count: usize) -> TextBuffer {
    let content = (0..line_count).fold(String::new(), |mut acc, i| {
        use std::fmt::Write;
        let _ = writeln!(acc, "line {i}");
        acc
    });
    TextBuffer::from_text(&content)
}

// ── Test workspace ────────────────────────────────────────────────────

/// A temporary workspace for integration tests.
///
/// Wraps a `TempDir` and a `Workspace`, providing helper methods to
/// create files, directories, and initialise git repos.
pub struct TestWorkspace {
    pub dir: TempDir,
    pub workspace: Workspace,
}

impl TestWorkspace {
    /// Create a new empty temporary workspace.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let workspace = Workspace::open(dir.path()).expect("failed to open temp workspace");
        Self { dir, workspace }
    }

    /// Root path of the workspace.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file relative to the workspace root, creating parent dirs.
    pub fn write_file(&self, rel_path: &str, content: &str) {
        let path = self.dir.path().join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, content).expect("failed to write file");
    }

    /// Read a file relative to the workspace root.
    pub fn read_file(&self, rel_path: &str) -> String {
        let path = self.dir.path().join(rel_path);
        std::fs::read_to_string(&path).expect("failed to read file")
    }

    /// Absolute path for a relative path.
    pub fn abs_path(&self, rel_path: &str) -> PathBuf {
        self.dir.path().join(rel_path)
    }

    /// Initialise a git repository in the workspace root.
    ///
    /// Creates an initial commit with all files currently in the workspace.
    pub fn init_git(&self) {
        let repo = git2::Repository::init(self.dir.path()).expect("failed to git init");

        // Configure test identity.
        let mut config = repo.config().expect("failed to get config");
        config
            .set_str("user.name", "Test User")
            .expect("failed to set user.name");
        config
            .set_str("user.email", "test@example.com")
            .expect("failed to set user.email");

        // Stage all files and create initial commit.
        let mut index = repo.index().expect("failed to get index");
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .expect("failed to add files");
        index.write().expect("failed to write index");
        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = repo.signature().expect("failed to get signature");
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("failed to create initial commit");
    }
}
