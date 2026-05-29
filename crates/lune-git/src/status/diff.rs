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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// Whether this line is context, addition, or deletion.
    pub kind: DiffLineKind,
    /// The line content (without leading +/-/ prefix).
    pub content: String,
    /// Line number in the old file (if applicable).
    pub old_lineno: Option<usize>,
    /// Line number in the new file (if applicable).
    pub new_lineno: Option<usize>,
    /// Whether this line is the last line of the file and lacks a
    /// trailing newline.  libgit2 reports this via a follow-up
    /// `=`/`>`/`<` origin line whose content is the standard
    /// `\ No newline at end of file` marker.  Hunk patches must round-
    /// trip this state, otherwise `git apply` either fails or silently
    /// rewrites the trailing newline state of the file.
    pub no_newline_eof: bool,
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

// ── Conversions to/from the git-free `lune_core` carrier ────────────
//
// These are pure field copies — no diff/freshness logic lives here.
// They let a `DiffHunk` cross the `GitCommand` boundary (which is
// intentionally git2-free) as a `HunkIdentity` and be reconstructed in
// the adapter so the existing `verify_hunk_fresh` can compare it.

impl From<DiffLineKind> for lune_core::ports::HunkLineKind {
    fn from(k: DiffLineKind) -> Self {
        match k {
            DiffLineKind::Context => Self::Context,
            DiffLineKind::Addition => Self::Addition,
            DiffLineKind::Deletion => Self::Deletion,
        }
    }
}

impl From<lune_core::ports::HunkLineKind> for DiffLineKind {
    fn from(k: lune_core::ports::HunkLineKind) -> Self {
        match k {
            lune_core::ports::HunkLineKind::Context => Self::Context,
            lune_core::ports::HunkLineKind::Addition => Self::Addition,
            lune_core::ports::HunkLineKind::Deletion => Self::Deletion,
        }
    }
}

impl From<&DiffHunk> for lune_core::ports::HunkIdentity {
    fn from(h: &DiffHunk) -> Self {
        Self {
            old_start: h.old_start,
            old_count: h.old_count,
            new_start: h.new_start,
            new_count: h.new_count,
            lines: h
                .lines
                .iter()
                .map(|l| lune_core::ports::HunkLine {
                    kind: l.kind.into(),
                    content: l.content.clone(),
                    no_newline_eof: l.no_newline_eof,
                })
                .collect(),
        }
    }
}

impl From<&lune_core::ports::HunkIdentity> for DiffHunk {
    fn from(h: &lune_core::ports::HunkIdentity) -> Self {
        let header = format!(
            "@@ -{},{} +{},{} @@",
            h.old_start, h.old_count, h.new_start, h.new_count
        );
        Self {
            header,
            old_start: h.old_start,
            old_count: h.old_count,
            new_start: h.new_start,
            new_count: h.new_count,
            lines: h
                .lines
                .iter()
                .map(|l| DiffLine {
                    kind: l.kind.into(),
                    content: l.content.clone(),
                    // `old_lineno`/`new_lineno` are not part of the
                    // freshness identity (`hunks_equivalent` ignores
                    // them) nor used by `to_patch`, so reconstruct as
                    // `None`.
                    old_lineno: None,
                    new_lineno: None,
                    no_newline_eof: l.no_newline_eof,
                })
                .collect(),
        }
    }
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

impl DiffHunk {
    /// Format this hunk as a valid unified diff patch string.
    ///
    /// The result can be parsed by [`git2::Diff::from_buffer`].  Lines
    /// flagged with `no_newline_eof` are followed by the standard
    /// `\ No newline at end of file` marker so `git apply` preserves
    /// the missing-newline state.
    ///
    /// # Errors
    /// Returns an error if `path` contains non-UTF-8 bytes — the patch
    /// header would otherwise be lossy and would not round-trip through
    /// `git2::Diff::from_buffer`.
    pub fn to_patch(&self, path: &Path) -> Result<String> {
        let path_str = path_to_patch_str(path)?;
        Ok(format_patch(self, path_str, false))
    }

    /// Format this hunk as a reverse patch (for unstaging/discarding).
    ///
    /// Swaps additions and deletions, and swaps old/new line counts.
    /// Preserves no-newline-EOF markers (with their + / - flipped).
    ///
    /// # Errors
    /// Returns an error if `path` contains non-UTF-8 bytes.
    pub fn to_reverse_patch(&self, path: &Path) -> Result<String> {
        let path_str = path_to_patch_str(path)?;
        Ok(format_patch(self, path_str, true))
    }

    /// Slice this hunk to a sub-range of its lines.
    ///
    /// Returns a new `DiffHunk` containing `self.lines[start..end]` with
    /// `old_start` / `new_start` / `old_count` / `new_count` recomputed so
    /// the slice forms a self-contained unified-diff hunk that `git apply`
    /// will accept at the correct file location.
    ///
    /// Pairs with [`Self::to_patch`] / [`Self::to_reverse_patch`] to enable
    /// staging or discarding an arbitrary line range within a hunk
    /// (VS Code-style "stage selected lines").
    ///
    /// # Errors
    /// Returns an error if `start >= end` (empty range) or `end` exceeds
    /// the line count.
    pub fn sub_hunk(&self, start: usize, end: usize) -> Result<Self> {
        if start >= end {
            anyhow::bail!("sub_hunk: empty range {start}..{end}");
        }
        if end > self.lines.len() {
            anyhow::bail!(
                "sub_hunk: range {start}..{end} out of bounds for {} lines",
                self.lines.len()
            );
        }

        // Shift the sub-hunk's start coordinates by counting how many
        // old/new lines the prefix [0..start] consumes from the parent.
        let (mut old_offset, mut new_offset) = (0usize, 0usize);
        for line in &self.lines[..start] {
            match line.kind {
                DiffLineKind::Context => {
                    old_offset += 1;
                    new_offset += 1;
                }
                DiffLineKind::Deletion => old_offset += 1,
                DiffLineKind::Addition => new_offset += 1,
            }
        }

        let lines: Vec<DiffLine> = self.lines[start..end].to_vec();
        let (mut old_count, mut new_count) = (0usize, 0usize);
        for line in &lines {
            match line.kind {
                DiffLineKind::Context => {
                    old_count += 1;
                    new_count += 1;
                }
                DiffLineKind::Deletion => old_count += 1,
                DiffLineKind::Addition => new_count += 1,
            }
        }

        let old_start = self.old_start + old_offset;
        let new_start = self.new_start + new_offset;
        let header = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@");

        Ok(Self {
            header,
            old_start,
            old_count,
            new_start,
            new_count,
            lines,
        })
    }
}

/// Reject non-UTF-8 paths up front — `display()` is lossy and the
/// resulting patch header would not survive a round-trip through
/// `git2::Diff::from_buffer`.
fn path_to_patch_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// Shared body for `to_patch` / `to_reverse_patch`.  When `reverse` is
/// true, additions become deletions and vice versa, and the hunk
/// header's old/new ranges are swapped.
fn format_patch(hunk: &DiffHunk, path_str: &str, reverse: bool) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "diff --git a/{path_str} b/{path_str}").unwrap();
    writeln!(buf, "--- a/{path_str}").unwrap();
    writeln!(buf, "+++ b/{path_str}").unwrap();
    if reverse {
        writeln!(
            buf,
            "@@ -{},{} +{},{} @@",
            hunk.new_start, hunk.new_count, hunk.old_start, hunk.old_count
        )
        .unwrap();
    } else {
        writeln!(
            buf,
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        )
        .unwrap();
    }

    for line in &hunk.lines {
        let prefix = match (line.kind, reverse) {
            (DiffLineKind::Context, _) => ' ',
            (DiffLineKind::Addition, false) | (DiffLineKind::Deletion, true) => '+',
            (DiffLineKind::Deletion, false) | (DiffLineKind::Addition, true) => '-',
        };
        let content = &line.content;
        if content.ends_with('\n') {
            write!(buf, "{prefix}{content}").unwrap();
        } else {
            writeln!(buf, "{prefix}{content}").unwrap();
        }
        if line.no_newline_eof {
            writeln!(buf, "\\ No newline at end of file").unwrap();
        }
    }
    buf
}

/// Walk a `git2::Diff` and collect all file diffs.
///
/// Uses `Diff::print` which provides a single callback instead of
/// multiple mutable closures, avoiding borrow checker issues.
fn collect_file_diffs(diff: &Diff<'_>) -> Result<Vec<FileDiff>> {
    // No `diff.stats()` pre-walk: the previous capacity hint walked the
    // entire diff once just to size this Vec, doubling the work.  A
    // small starting capacity is enough — `Vec::push` reallocates O(log
    // n) times and the cost is negligible compared to `diff.print` below.
    let mut file_diffs: Vec<FileDiff> = Vec::new();

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
            '=' | '>' | '<' => {
                // "No newline at end of file" marker — flag the most
                // recently-emitted content line in the current hunk.
                // The marker itself produces no patch line; the flag
                // controls whether `to_patch` re-emits the marker.
                if let Some(h) = file_diff.hunks.last_mut() {
                    if let Some(last_line) = h.lines.last_mut() {
                        last_line.no_newline_eof = true;
                    }
                }
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
                        no_newline_eof: false,
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
    fn hunk_to_patch_format() {
        let hunk = DiffHunk {
            header: "@@ -1,3 +1,4 @@".to_owned(),
            old_start: 1,
            old_count: 3,
            new_start: 1,
            new_count: 4,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "line1\n".to_owned(),
                    old_lineno: Some(1),
                    new_lineno: Some(1),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Deletion,
                    content: "old\n".to_owned(),
                    old_lineno: Some(2),
                    new_lineno: None,
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Addition,
                    content: "new\n".to_owned(),
                    old_lineno: None,
                    new_lineno: Some(2),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Addition,
                    content: "extra\n".to_owned(),
                    old_lineno: None,
                    new_lineno: Some(3),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "line3\n".to_owned(),
                    old_lineno: Some(3),
                    new_lineno: Some(4),
                    no_newline_eof: false,
                },
            ],
        };
        let patch = hunk.to_patch(Path::new("test.txt")).expect("to_patch");
        // Should be parseable by git2.
        assert!(git2::Diff::from_buffer(patch.as_bytes()).is_ok());
        assert!(patch.contains("+new\n"));
        assert!(patch.contains("-old\n"));
    }

    #[test]
    fn hunk_to_reverse_patch_swaps() {
        let hunk = DiffHunk {
            header: "@@ -1,2 +1,3 @@".to_owned(),
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 3,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "ctx\n".to_owned(),
                    old_lineno: Some(1),
                    new_lineno: Some(1),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Addition,
                    content: "added\n".to_owned(),
                    old_lineno: None,
                    new_lineno: Some(2),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "ctx2\n".to_owned(),
                    old_lineno: Some(2),
                    new_lineno: Some(3),
                    no_newline_eof: false,
                },
            ],
        };
        let patch = hunk
            .to_reverse_patch(Path::new("test.txt"))
            .expect("to_reverse_patch");
        // Reverse should swap + and -.
        assert!(patch.contains("-added\n"));
        assert!(!patch.contains("+added"));
        // Should be parseable.
        assert!(git2::Diff::from_buffer(patch.as_bytes()).is_ok());
    }

    /// Build a `DiffHunk` that mirrors
    ///   @@ -10,4 +10,4 @@
    ///    a       (context)
    ///   -b       (deletion of old line 11)
    ///   +B       (addition becomes new line 11)
    ///    c       (context)
    /// — used as a fixture by the `sub_hunk_*` tests below.
    fn fixture_hunk() -> DiffHunk {
        DiffHunk {
            header: "@@ -10,3 +10,3 @@".to_owned(),
            old_start: 10,
            old_count: 3,
            new_start: 10,
            new_count: 3,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "a\n".to_owned(),
                    old_lineno: Some(10),
                    new_lineno: Some(10),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Deletion,
                    content: "b\n".to_owned(),
                    old_lineno: Some(11),
                    new_lineno: None,
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Addition,
                    content: "B\n".to_owned(),
                    old_lineno: None,
                    new_lineno: Some(11),
                    no_newline_eof: false,
                },
                DiffLine {
                    kind: DiffLineKind::Context,
                    content: "c\n".to_owned(),
                    old_lineno: Some(12),
                    new_lineno: Some(12),
                    no_newline_eof: false,
                },
            ],
        }
    }

    #[test]
    fn sub_hunk_middle_slice_recomputes_coords() {
        let parent = fixture_hunk();
        // Slice [1..3] = [-b, +B] — drop both context lines.
        let sub = parent.sub_hunk(1, 3).expect("sub_hunk");
        assert_eq!(sub.old_start, 11, "prefix [a] consumes one old line");
        assert_eq!(sub.new_start, 11, "prefix [a] consumes one new line");
        assert_eq!(sub.old_count, 1);
        assert_eq!(sub.new_count, 1);
        assert_eq!(sub.header, "@@ -11,1 +11,1 @@");
        assert_eq!(sub.lines.len(), 2);
        assert_eq!(sub.lines[0].kind, DiffLineKind::Deletion);
        assert_eq!(sub.lines[1].kind, DiffLineKind::Addition);
    }

    #[test]
    fn sub_hunk_pure_addition_yields_zero_old_count() {
        let parent = fixture_hunk();
        // Slice [2..3] = [+B] alone — pure insertion.
        let sub = parent.sub_hunk(2, 3).expect("sub_hunk");
        assert_eq!(sub.old_count, 0, "pure addition has no old lines");
        assert_eq!(sub.new_count, 1);
        // Prefix [a, -b] consumes two old lines, one new line.
        assert_eq!(sub.old_start, 12);
        assert_eq!(sub.new_start, 11);
        assert_eq!(sub.header, "@@ -12,0 +11,1 @@");
    }

    #[test]
    fn sub_hunk_pure_deletion_yields_zero_new_count() {
        let parent = fixture_hunk();
        // Slice [1..2] = [-b] alone — pure deletion.
        let sub = parent.sub_hunk(1, 2).expect("sub_hunk");
        assert_eq!(sub.old_count, 1);
        assert_eq!(sub.new_count, 0);
        assert_eq!(sub.old_start, 11);
        assert_eq!(sub.new_start, 11);
        assert_eq!(sub.header, "@@ -11,1 +11,0 @@");
    }

    #[test]
    fn sub_hunk_full_range_round_trips() {
        let parent = fixture_hunk();
        let sub = parent.sub_hunk(0, parent.lines.len()).expect("sub_hunk");
        assert_eq!(sub.old_start, parent.old_start);
        assert_eq!(sub.new_start, parent.new_start);
        assert_eq!(sub.old_count, parent.old_count);
        assert_eq!(sub.new_count, parent.new_count);
        assert_eq!(sub.lines.len(), parent.lines.len());
    }

    #[test]
    fn sub_hunk_rejects_empty_range() {
        let parent = fixture_hunk();
        assert!(parent.sub_hunk(1, 1).is_err());
        assert!(parent.sub_hunk(2, 1).is_err());
    }

    #[test]
    fn sub_hunk_rejects_out_of_bounds() {
        let parent = fixture_hunk();
        let n = parent.lines.len();
        assert!(parent.sub_hunk(0, n + 1).is_err());
    }

    #[test]
    fn sub_hunk_patch_parses_through_git2() {
        // The whole point of recomputing coords is that the sliced hunk
        // serializes to a patch git2::Diff::from_buffer accepts.
        let parent = fixture_hunk();
        for (start, end) in [(0, 4), (1, 3), (1, 2), (2, 3), (0, 1), (3, 4)] {
            let sub = parent.sub_hunk(start, end).expect("sub_hunk");
            let patch = sub.to_patch(Path::new("f.txt")).expect("to_patch");
            git2::Diff::from_buffer(patch.as_bytes())
                .unwrap_or_else(|e| panic!("range {start}..{end} did not parse: {e}\n{patch}"));
        }
    }

    #[test]
    fn sub_hunk_applies_partial_hunk_to_index() {
        // End-to-end: stage one of two changes that live in a single hunk.
        // Initial: aaa / bbb / ccc.  Modified: AAA / bbb / CCC.
        // Both edits sit in the same hunk (context=3 default).
        let (dir, svc) = repo_with_file("h.txt", "aaa\nbbb\nccc\n");
        fs::write(dir.path().join("h.txt"), "AAA\nbbb\nCCC\n").unwrap();

        let diff = svc.diff_file(Path::new("h.txt")).unwrap().unwrap();
        assert_eq!(diff.hunks.len(), 1, "edits share a single hunk");
        let hunk = &diff.hunks[0];

        // The hunk has shape: -aaa +AAA  bbb -ccc +CCC.  Slice out the
        // leading [-aaa, +AAA] pair (indices 0..2) and stage just that.
        let leading = hunk.sub_hunk(0, 2).expect("sub_hunk leading change");
        let patch = leading.to_patch(Path::new("h.txt")).expect("to_patch");
        let parsed = git2::Diff::from_buffer(patch.as_bytes()).expect("git2 parses sub-hunk patch");
        svc.repo()
            .apply(&parsed, git2::ApplyLocation::Index, None)
            .expect("apply leading sub-hunk to index");

        // Staged diff: should contain ONLY the aaa→AAA change.
        let staged = svc
            .diff_staged(Path::new("h.txt"))
            .unwrap()
            .expect("staged diff present");
        let staged_lines: Vec<_> = staged.hunks.iter().flat_map(|h| &h.lines).collect();
        assert!(
            staged_lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Deletion && l.content.trim() == "aaa"),
            "staged hunk should delete aaa"
        );
        assert!(
            staged_lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Addition && l.content.trim() == "AAA"),
            "staged hunk should add AAA"
        );
        // The ccc→CCC change must NOT be staged.
        assert!(
            !staged_lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Deletion && l.content.trim() == "ccc"),
            "ccc→CCC change must remain unstaged"
        );

        // Workdir-vs-index diff should still show the ccc→CCC change.
        let workdir = svc.diff_file(Path::new("h.txt")).unwrap();
        assert!(workdir.is_some(), "trailing edit should still be unstaged");
    }

    #[test]
    fn no_newline_eof_marker_round_trips() {
        // Commit a file with a trailing newline, then write a version
        // without one — libgit2 should emit a `\ No newline at end of
        // file` marker that survives our patch round-trip.
        let (dir, svc) = repo_with_file("eof.txt", "line1\nline2\n");
        fs::write(dir.path().join("eof.txt"), "line1\nline2_no_nl").unwrap();

        let diff = svc.diff_file(Path::new("eof.txt")).unwrap().unwrap();
        let hunk = diff.hunks.first().expect("one hunk");
        assert!(
            hunk.lines.iter().any(|l| l.no_newline_eof),
            "expected at least one line flagged with no_newline_eof"
        );

        let patch = hunk.to_patch(Path::new("eof.txt")).unwrap();
        assert!(
            patch.contains("\\ No newline at end of file"),
            "patch must include the no-newline marker"
        );
        // The emitted patch must still round-trip through git2.
        git2::Diff::from_buffer(patch.as_bytes()).expect("patch parses");
    }

    #[test]
    #[cfg(unix)]
    fn non_utf8_path_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        let hunk = DiffHunk {
            header: "@@ -1 +1 @@".to_owned(),
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            lines: vec![],
        };
        // Invalid UTF-8 byte sequence in path.
        let bad: PathBuf = PathBuf::from(OsStr::from_bytes(b"bad\xFFpath"));
        assert!(hunk.to_patch(&bad).is_err());
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
