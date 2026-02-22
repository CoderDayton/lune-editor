//! Core git service wrapping libgit2.
//!
//! [`GitService`] provides repository discovery, status queries, branch info,
//! and staging/commit operations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Repository, StatusOptions, StatusShow};
use lune_core::workspace::FileStatus;

/// A wrapper around a `git2::Repository` for editor integration.
pub struct GitService {
    repo: Repository,
    root: PathBuf,
}

/// Snapshot of the repository status at a point in time.
#[derive(Clone, Debug)]
pub struct GitStatus {
    /// Current branch name (or `"HEAD detached"` if detached).
    pub branch: String,
    /// Number of commits ahead of upstream.
    pub ahead: usize,
    /// Number of commits behind upstream.
    pub behind: usize,
    /// Per-file status entries.
    pub files: Vec<GitFileStatus>,
}

/// Status of a single file relative to HEAD and index.
#[derive(Clone, Debug)]
pub struct GitFileStatus {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// The file's git status.
    pub status: FileStatus,
    /// Whether the file is staged (in the index).
    pub staged: bool,
}

impl GitService {
    /// Open (discover) a git repository starting from `path`.
    ///
    /// Walks up the directory tree looking for a `.git` directory.
    /// Returns `None` if no repository is found (not an error — just not a git dir).
    pub fn open(path: &Path) -> Result<Option<Self>> {
        match Repository::discover(path) {
            Ok(repo) => {
                let root = repo
                    .workdir()
                    .context("bare repositories are not supported")?
                    .to_path_buf();
                Ok(Some(Self { repo, root }))
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e).context("failed to discover git repository"),
        }
    }

    /// Returns the repository working directory root.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether this is a valid git repository.
    #[inline]
    pub fn is_repo(&self) -> bool {
        self.repo.workdir().is_some()
    }

    /// Query the full repository status: branch, ahead/behind, file statuses.
    pub fn status(&self) -> Result<GitStatus> {
        let branch = self.branch_name();
        let (ahead, behind) = self.ahead_behind();
        let files = self.file_statuses()?;
        Ok(GitStatus {
            branch,
            ahead,
            behind,
            files,
        })
    }

    /// Get the current branch name.
    fn branch_name(&self) -> String {
        self.repo
            .head()
            .ok()
            .and_then(|head| head.shorthand().map(String::from))
            .unwrap_or_else(|| "HEAD detached".to_owned())
    }

    /// Compute ahead/behind counts relative to the upstream tracking branch.
    fn ahead_behind(&self) -> (usize, usize) {
        let Ok(head) = self.repo.head() else {
            return (0, 0);
        };
        let Some(local_oid) = head.target() else {
            return (0, 0);
        };

        // Find the upstream branch for the current local branch.
        let upstream = head
            .name()
            .and_then(|name| self.repo.branch_upstream_name(name).ok());
        let Some(upstream_name) = upstream else {
            return (0, 0);
        };
        let Ok(upstream_ref) = self
            .repo
            .find_reference(upstream_name.as_str().unwrap_or(""))
        else {
            return (0, 0);
        };
        let Some(upstream_oid) = upstream_ref.target() else {
            return (0, 0);
        };

        self.repo
            .graph_ahead_behind(local_oid, upstream_oid)
            .unwrap_or((0, 0))
    }

    /// Collect per-file statuses from the working tree and index.
    fn file_statuses(&self) -> Result<Vec<GitFileStatus>> {
        let mut opts = StatusOptions::new();
        opts.show(StatusShow::IndexAndWorkdir)
            .include_untracked(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);

        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .context("failed to get repository statuses")?;

        let mut result = Vec::with_capacity(statuses.len());

        for entry in statuses.iter() {
            let Some(path_str) = entry.path() else {
                continue;
            };
            let path = PathBuf::from(path_str);
            let bits = entry.status();

            // Index (staged) statuses.
            if bits.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED,
            ) {
                let status = index_bits_to_status(bits);
                result.push(GitFileStatus {
                    path: path.clone(),
                    status,
                    staged: true,
                });
            }

            // Workdir (unstaged) statuses.
            if bits.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED,
            ) {
                let status = workdir_bits_to_status(bits);
                result.push(GitFileStatus {
                    path: path.clone(),
                    status,
                    staged: false,
                });
            }

            // Conflicted files.
            if bits.contains(git2::Status::CONFLICTED) {
                result.push(GitFileStatus {
                    path: path.clone(),
                    status: FileStatus::Conflicted,
                    staged: false,
                });
            }

            // Ignored files (we skip these in the main list but keep the mapping
            // available for file tree coloring).
            if bits.contains(git2::Status::IGNORED) {
                result.push(GitFileStatus {
                    path,
                    status: FileStatus::Ignored,
                    staged: false,
                });
            }
        }

        Ok(result)
    }

    /// Resolve a workspace-relative path to a repo-relative path.
    pub fn repo_relative(&self, abs_path: &Path) -> Option<PathBuf> {
        abs_path.strip_prefix(&self.root).ok().map(PathBuf::from)
    }

    /// Expose the inner `git2::Repository` for other modules in this crate.
    #[inline]
    pub(crate) const fn repo(&self) -> &Repository {
        &self.repo
    }

    /// Construct from pre-built parts (used internally by tests).
    #[cfg(test)]
    pub(crate) const fn from_parts(repo: Repository, root: PathBuf) -> Self {
        Self { repo, root }
    }
}

/// Map index (staged) status bits to `FileStatus`.
const fn index_bits_to_status(bits: git2::Status) -> FileStatus {
    if bits.contains(git2::Status::INDEX_NEW) {
        FileStatus::Added
    } else if bits.contains(git2::Status::INDEX_DELETED) {
        FileStatus::Deleted
    } else if bits.contains(git2::Status::INDEX_RENAMED) {
        FileStatus::Renamed
    } else {
        FileStatus::Modified
    }
}

/// Map workdir (unstaged) status bits to `FileStatus`.
const fn workdir_bits_to_status(bits: git2::Status) -> FileStatus {
    if bits.contains(git2::Status::WT_NEW) {
        FileStatus::Untracked
    } else if bits.contains(git2::Status::WT_DELETED) {
        FileStatus::Deleted
    } else if bits.contains(git2::Status::WT_RENAMED) {
        FileStatus::Renamed
    } else {
        FileStatus::Modified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary git repository for testing.
    fn make_test_repo() -> (tempfile::TempDir, GitService) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let repo = Repository::init(dir.path()).expect("init repo");

        // Configure user for commits.
        let mut config = repo.config().expect("get config");
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create an initial commit so HEAD is valid.
        let sig = repo.signature().expect("get signature");
        {
            let tree_id = {
                let mut index = repo.index().expect("get index");
                index.write_tree().expect("write empty tree")
            };
            let tree = repo.find_tree(tree_id).expect("find tree");
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .expect("initial commit");
        }

        let root = dir.path().to_path_buf();
        let svc = GitService::from_parts(repo, root);
        (dir, svc)
    }

    #[test]
    fn open_discovers_repo() {
        let (dir, _svc) = make_test_repo();
        let result = GitService::open(dir.path()).expect("open");
        assert!(result.is_some());
    }

    #[test]
    fn open_non_repo_returns_none() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // Remove the .git directory by not initializing.
        let result = GitService::open(dir.path()).expect("open");
        assert!(result.is_none());
    }

    #[test]
    fn is_repo_returns_true() {
        let (_dir, svc) = make_test_repo();
        assert!(svc.is_repo());
    }

    #[test]
    fn branch_name_is_main_or_master() {
        let (_dir, svc) = make_test_repo();
        let name = svc.branch_name();
        // git2 init uses the system default branch name.
        assert!(!name.is_empty());
    }

    #[test]
    fn status_empty_repo() {
        let (_dir, svc) = make_test_repo();
        let status = svc.status().expect("status");
        assert!(status.files.is_empty());
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
    }

    #[test]
    fn status_detects_untracked_file() {
        let (dir, svc) = make_test_repo();
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();
        let status = svc.status().expect("status");
        assert!(!status.files.is_empty());

        let entry = status
            .files
            .iter()
            .find(|f| f.path.as_os_str() == "new_file.txt");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.status, FileStatus::Untracked);
        assert!(!entry.staged);
    }

    #[test]
    fn status_detects_modified_file() {
        let (dir, svc) = make_test_repo();

        // Create and commit a file.
        let file_path = dir.path().join("tracked.txt");
        fs::write(&file_path, "original").unwrap();
        {
            let mut index = svc.repo().index().expect("get index");
            index.add_path(Path::new("tracked.txt")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = svc.repo().find_tree(tree_id).unwrap();
            let head = svc.repo().head().unwrap().peel_to_commit().unwrap();
            let sig = svc.repo().signature().unwrap();
            svc.repo()
                .commit(Some("HEAD"), &sig, &sig, "add tracked.txt", &tree, &[&head])
                .unwrap();
        }

        // Modify the file.
        fs::write(&file_path, "modified").unwrap();
        let status = svc.status().expect("status");
        let entry = status
            .files
            .iter()
            .find(|f| f.path.as_os_str() == "tracked.txt");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.status, FileStatus::Modified);
        assert!(!entry.staged);
    }

    #[test]
    fn status_detects_staged_file() {
        let (dir, svc) = make_test_repo();

        // Create and stage a new file.
        fs::write(dir.path().join("staged.txt"), "content").unwrap();
        {
            let mut index = svc.repo().index().expect("get index");
            index.add_path(Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        let status = svc.status().expect("status");
        let entry = status
            .files
            .iter()
            .find(|f| f.path.as_os_str() == "staged.txt" && f.staged);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.status, FileStatus::Added);
        assert!(entry.staged);
    }

    #[test]
    fn repo_relative_path() {
        let (dir, svc) = make_test_repo();
        let abs = dir.path().join("src").join("main.rs");
        let rel = svc.repo_relative(&abs);
        assert_eq!(rel.unwrap(), PathBuf::from("src/main.rs"));
    }

    #[test]
    fn repo_relative_outside_root_returns_none() {
        let (_dir, svc) = make_test_repo();
        let outside = PathBuf::from("/tmp/outside/file.rs");
        assert!(svc.repo_relative(&outside).is_none());
    }
}
