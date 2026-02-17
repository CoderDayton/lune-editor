//! Staging, committing, and discarding operations.
//!
//! These methods modify the git index or working tree and require
//! confirmation from the user before calling destructive operations
//! (discard).

use std::path::Path;

use anyhow::{Context, Result};
use git2::Oid;

use super::service::GitService;

impl GitService {
    /// Stage a file by adding it to the index.
    pub fn stage(&self, rel_path: &Path) -> Result<()> {
        let mut index = self.repo().index().context("failed to get index")?;
        index
            .add_path(rel_path)
            .context("failed to add path to index")?;
        index.write().context("failed to write index")?;
        Ok(())
    }

    /// Unstage a file by resetting the index entry to the HEAD version.
    pub fn unstage(&self, rel_path: &Path) -> Result<()> {
        let head = self.repo().head().context("failed to get HEAD")?;
        let head_commit = head
            .peel_to_commit()
            .context("failed to peel HEAD to commit")?;
        self.repo()
            .reset_default(Some(head_commit.as_object()), [rel_path])
            .context("failed to reset index entry")?;
        Ok(())
    }

    /// Create a commit from the current index.
    ///
    /// Returns the new commit OID.
    pub fn commit(&self, message: &str) -> Result<Oid> {
        let sig = self.repo().signature().context("failed to get signature")?;
        let mut index = self.repo().index().context("failed to get index")?;
        let tree_oid = index.write_tree().context("failed to write tree")?;
        let tree = self
            .repo()
            .find_tree(tree_oid)
            .context("failed to find tree")?;

        let parents = match self.repo().head() {
            Ok(head) => {
                let commit = head
                    .peel_to_commit()
                    .context("failed to peel HEAD to commit")?;
                vec![commit]
            }
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => vec![],
            Err(e) => return Err(e).context("failed to get HEAD"),
        };

        let parent_refs: Vec<&git2::Commit<'_>> = parents.iter().collect();
        let oid = self
            .repo()
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .context("failed to create commit")?;

        Ok(oid)
    }

    /// Discard changes to a working tree file by checking out the HEAD version.
    ///
    /// **Destructive** — caller should confirm with user before calling.
    pub fn discard_file(&self, rel_path: &Path) -> Result<()> {
        let mut checkout_builder = git2::build::CheckoutBuilder::new();
        checkout_builder.path(rel_path).force();
        self.repo()
            .checkout_head(Some(&mut checkout_builder))
            .context("failed to discard file changes")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_with_file(name: &str, content: &str) -> (tempfile::TempDir, GitService) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let repo = git2::Repository::init(dir.path()).expect("init repo");
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@t.com").unwrap();

        fs::write(dir.path().join(name), content).unwrap();

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
    fn stage_and_check_status() {
        let (dir, svc) = repo_with_file("a.txt", "original\n");
        fs::write(dir.path().join("a.txt"), "modified\n").unwrap();

        svc.stage(Path::new("a.txt")).expect("stage");

        let status = svc.status().expect("status");
        let staged = status.files.iter().find(|f| f.staged);
        assert!(staged.is_some());
    }

    #[test]
    fn unstage_reverts_index() {
        let (dir, svc) = repo_with_file("a.txt", "original\n");
        fs::write(dir.path().join("a.txt"), "modified\n").unwrap();
        svc.stage(Path::new("a.txt")).unwrap();
        svc.unstage(Path::new("a.txt")).unwrap();

        let status = svc.status().expect("status");
        // File should be modified but not staged.
        let unstaged = status.files.iter().find(|f| !f.staged);
        assert!(unstaged.is_some());
        let staged = status.files.iter().find(|f| f.staged);
        assert!(staged.is_none());
    }

    #[test]
    fn commit_creates_new_head() {
        let (dir, svc) = repo_with_file("a.txt", "original\n");
        fs::write(dir.path().join("a.txt"), "modified\n").unwrap();
        svc.stage(Path::new("a.txt")).unwrap();

        let oid = svc.commit("test commit").expect("commit");
        assert!(!oid.is_zero());

        // HEAD should point to the new commit.
        let head = svc.repo().head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id(), oid);
        assert_eq!(head.message(), Some("test commit"));
    }

    #[test]
    fn discard_restores_head_version() {
        let (dir, svc) = repo_with_file("a.txt", "original\n");
        let file_path = dir.path().join("a.txt");
        fs::write(&file_path, "modified\n").unwrap();

        svc.discard_file(Path::new("a.txt")).expect("discard");

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "original\n");
    }
}
