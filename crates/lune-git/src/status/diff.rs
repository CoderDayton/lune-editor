//! Diff computation for working tree and index.
//!
//! Provides [`FileDiff`], [`DiffHunk`], and [`DiffLine`] types, plus
//! methods on [`crate::service::GitService`] to compute diffs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Diff, DiffFormat, DiffOptions};

use crate::service::GitService;

/// A diff for a single file.
#[derive(Clone, Debug)]
pub struct FileDiff {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// Individual diff hunks.
    pub hunks: Vec<DiffHunk>,
}

/// A single hunk within a file diff.
#[derive(Clone, Debug)]
pub struct DiffHunk {
    /// The hunk header (e.g. `@@ -1,3 +1,5 @@`).
    pub header: String,
    /// Start line in the old version.
    pub old_start: usize,
    /// Number of lines in the old version.
    pub old_count: usize,
    /// Start line in the new version.
    pub new_start: usize,
    /// Number of lines in the new version.
    pub new_count: usize,
    /// Lines in this hunk.
    pub lines: Vec<DiffLine>,
}

/// A single line within a diff hunk.
#[derive(Clone, Debug)]
pub struct DiffLine {
    /// Whether this line is context, addition, or deletion.
    pub kind: DiffLineKind,
    /// The line content (without leading +/-/ prefix).
    pub content: String,
    /// Line number in the old file (if applicable).
    pub old_lineno: Option<usize>,
    /// Line number in the new file (if applicable).
    pub new_lineno: Option<usize>,
}

/// The kind of a diff line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Unchanged context line.
    Context,
    /// Added line.
    Addition,
    /// Deleted line.
    Deletion,
}

impl GitService {
    /// Compute the diff for a single file between the working tree and HEAD.
    #[inline]
    pub fn diff_file(&self, rel_path: &Path) -> Result<Option<FileDiff>> {
        self.diff_single(rel_path, false)
    }

    /// Compute the diff for a single file between the index (staged) and HEAD.
    #[inline]
    pub fn diff_staged(&self, rel_path: &Path) -> Result<Option<FileDiff>> {
        self.diff_single(rel_path, true)
    }

    /// Shared implementation for single-file diffs (working tree or staged).
    fn diff_single(&self, rel_path: &Path, staged_only: bool) -> Result<Option<FileDiff>> {
        let mut opts = DiffOptions::new();
        opts.pathspec(rel_path);

        let head_tree = self.head_tree()?;
        let diff = if staged_only {
            self.repo()
                .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))
                .context("failed to compute staged diff")?
        } else {
            self.repo()
                .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
                .context("failed to compute working tree diff")?
        };

        let diffs = collect_file_diffs(&diff)?;
        Ok(diffs.into_iter().next())
    }

    /// Compute diffs for all files in the working tree against HEAD.
    pub fn diff_all(&self) -> Result<Vec<FileDiff>> {
        let head_tree = self.head_tree()?;
        let diff = self
            .repo()
            .diff_tree_to_workdir_with_index(head_tree.as_ref(), None)
            .context("failed to compute full working tree diff")?;

        collect_file_diffs(&diff)
    }

    /// Get the tree object for HEAD, or `None` if the repository is empty.
    fn head_tree(&self) -> Result<Option<git2::Tree<'_>>> {
        match self.repo().head() {
            Ok(head) => {
                let tree = head.peel_to_tree().context("failed to peel HEAD to tree")?;
                Ok(Some(tree))
            }
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
            Err(e) => Err(e).context("failed to get HEAD reference"),
        }
    }
}

/// Walk a `git2::Diff` and collect all file diffs.
///
/// Uses `Diff::print` which provides a single callback instead of
/// multiple mutable closures, avoiding borrow checker issues.
fn collect_file_diffs(diff: &Diff<'_>) -> Result<Vec<FileDiff>> {
    let mut file_diffs: Vec<FileDiff> = Vec::with_capacity(diff.stats()?.files_changed());

    diff.print(DiffFormat::Patch, |delta, hunk, line| {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .unwrap_or_else(|| Path::new("<unknown>"));

        // Ensure we have a `FileDiff` entry for this file.
        let needs_new = file_diffs.last().is_none_or(|last| last.path != path);
        if needs_new {
            file_diffs.push(FileDiff {
                path: path.to_path_buf(),
                hunks: Vec::new(),
            });
        }
        let file_diff = file_diffs.last_mut().expect("just pushed");

        match line.origin() {
            'H' | 'F' => {
                // File header / file footer — skip.
            }
            _ => {
                // If we have a hunk header, create a new hunk entry.
                if let Some(h) = hunk {
                    let header = String::from_utf8_lossy(h.header()).trim().to_owned();
                    let needs_hunk = file_diff
                        .hunks
                        .last()
                        .is_none_or(|last| last.header != header);
                    if needs_hunk {
                        let line_capacity = (h.old_lines() + h.new_lines()) as usize;
                        file_diff.hunks.push(DiffHunk {
                            header,
                            old_start: h.old_start() as usize,
                            old_count: h.old_lines() as usize,
                            new_start: h.new_start() as usize,
                            new_count: h.new_lines() as usize,
                            lines: Vec::with_capacity(line_capacity),
                        });
                    }
                }

                // Add the line to the current hunk.
                let kind = match line.origin() {
                    '+' => DiffLineKind::Addition,
                    '-' => DiffLineKind::Deletion,
                    _ => DiffLineKind::Context,
                };
                let content = String::from_utf8_lossy(line.content()).to_string();
                let old_lineno = line.old_lineno().map(|n| n as usize);
                let new_lineno = line.new_lineno().map(|n| n as usize);

                if let Some(current_hunk) = file_diff.hunks.last_mut() {
                    current_hunk.lines.push(DiffLine {
                        kind,
                        content,
                        old_lineno,
                        new_lineno,
                    });
                }
            }
        }

        true
    })
    .context("failed to print diff")?;

    Ok(file_diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper to create a temp repo with one committed file.
    fn repo_with_file(name: &str, content: &str) -> (tempfile::TempDir, GitService) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let repo = git2::Repository::init(dir.path()).expect("init repo");
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@t.com").unwrap();

        // Write file.
        fs::write(dir.path().join(name), content).unwrap();

        // Stage and commit.
        let sig = repo.signature().unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new(name)).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }

        let root = dir.path().to_path_buf();
        let svc = GitService::from_parts(repo, root);
        (dir, svc)
    }

    #[test]
    fn diff_file_detects_modification() {
        let (dir, svc) = repo_with_file("hello.txt", "line1\nline2\nline3\n");

        // Modify the file.
        fs::write(dir.path().join("hello.txt"), "line1\nchanged\nline3\n").unwrap();

        let diff = svc.diff_file(Path::new("hello.txt")).expect("diff");
        let diff = diff.expect("file diff present");
        assert_eq!(diff.path, PathBuf::from("hello.txt"));
        assert!(!diff.hunks.is_empty());

        // Should have deletion of "line2" and addition of "changed".
        let lines: Vec<_> = diff.hunks.iter().flat_map(|h| &h.lines).collect();
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::Deletion));
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::Addition));
    }

    #[test]
    fn diff_file_no_changes() {
        let (_dir, svc) = repo_with_file("hello.txt", "content\n");
        let diff = svc.diff_file(Path::new("hello.txt")).expect("diff");
        assert!(diff.is_none());
    }

    #[test]
    fn diff_staged_detects_staged_changes() {
        let (dir, svc) = repo_with_file("hello.txt", "original\n");

        // Modify and stage.
        fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();
        {
            let mut index = svc.repo().index().unwrap();
            index.add_path(Path::new("hello.txt")).unwrap();
            index.write().unwrap();
        }

        let diff = svc.diff_staged(Path::new("hello.txt")).expect("diff");
        let diff = diff.expect("staged diff present");
        assert!(!diff.hunks.is_empty());
    }

    #[test]
    fn diff_all_returns_multiple_files() {
        let (dir, svc) = repo_with_file("a.txt", "aaa\n");

        // Commit a second file.
        fs::write(dir.path().join("b.txt"), "bbb\n").unwrap();
        {
            let mut index = svc.repo().index().unwrap();
            index.add_path(Path::new("b.txt")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = svc.repo().find_tree(tree_id).unwrap();
            let head = svc.repo().head().unwrap().peel_to_commit().unwrap();
            let sig = svc.repo().signature().unwrap();
            svc.repo()
                .commit(Some("HEAD"), &sig, &sig, "add b.txt", &tree, &[&head])
                .unwrap();
        }

        // Modify both files.
        fs::write(dir.path().join("a.txt"), "aaa modified\n").unwrap();
        fs::write(dir.path().join("b.txt"), "bbb modified\n").unwrap();

        let diffs = svc.diff_all().expect("diff_all");
        assert!(diffs.len() >= 2);
    }

    #[test]
    fn diff_line_numbers_present() {
        let (dir, svc) = repo_with_file("numbered.txt", "1\n2\n3\n4\n5\n");
        fs::write(dir.path().join("numbered.txt"), "1\n2\nINSERTED\n3\n4\n5\n").unwrap();

        let diff = svc.diff_file(Path::new("numbered.txt")).expect("diff");
        let diff = diff.expect("present");
        let additions: Vec<_> = diff
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == DiffLineKind::Addition)
            .collect();
        assert!(!additions.is_empty());
        // Addition lines should have new_lineno set.
        assert!(additions.iter().all(|l| l.new_lineno.is_some()));
    }
}
