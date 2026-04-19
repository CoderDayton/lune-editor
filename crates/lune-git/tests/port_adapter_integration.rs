//! End-to-end: spawn `GitAdapter` on a real tempdir repo, verify that a
//! status snapshot is published within a short window and reflects file
//! mutations dispatched via the port.

use std::fs;
use std::time::{Duration, Instant};

use git2::{Repository, Signature};
use lune_core::buffer::BufferId;
use lune_core::ports::{FileState, GitCommand, GitPort, PortRuntime};
use lune_git::GitAdapter;
use tempfile::TempDir;

fn init_repo_with_commit(dir: &std::path::Path) {
    let repo = Repository::init(dir).unwrap();
    fs::write(dir.join("README.md"), "hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = Signature::now("t", "t@t").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    index.write().unwrap();
}

fn wait_for_revision(
    reader: &lune_core::ports::Snapshot<lune_core::ports::StatusSnapshot>,
    min_rev: u64,
    max_wait: Duration,
) -> u64 {
    let start = Instant::now();
    loop {
        let snap = reader.load();
        if snap.revision >= min_rev {
            return snap.revision;
        }
        assert!(
            start.elapsed() <= max_wait,
            "timed out waiting for snapshot revision >= {min_rev}, last={}",
            snap.revision
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn adapter_publishes_initial_snapshot() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = PortRuntime::new().unwrap();
    let adapter = GitAdapter::spawn(&rt.handle(), tmp.path())
        .unwrap()
        .expect("repo exists, adapter should start");

    let reader = adapter.status();
    wait_for_revision(&reader, 1, Duration::from_secs(3));
    let snap = reader.load();
    let branch = snap.branch.as_deref().expect("branch published");
    assert!(
        branch == "main" || branch == "master",
        "unexpected branch name: {branch}"
    );
}

#[test]
fn adapter_detects_workdir_modification() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = PortRuntime::new().unwrap();
    let adapter = GitAdapter::spawn(&rt.handle(), tmp.path())
        .unwrap()
        .unwrap();
    let reader = adapter.status();
    wait_for_revision(&reader, 1, Duration::from_secs(3));

    // Modify the file and force a refresh.
    fs::write(tmp.path().join("README.md"), "changed\n").unwrap();
    adapter.dispatch(GitCommand::RefreshStatus);

    let start = Instant::now();
    loop {
        let snap = reader.load();
        if snap.files.iter().any(|f| f.state == FileState::Modified) {
            return;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(3),
            "modified file never appeared in snapshot: {:?}",
            snap.files
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn gutter_publishes_on_demand() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = PortRuntime::new().unwrap();
    let adapter = GitAdapter::spawn(&rt.handle(), tmp.path())
        .unwrap()
        .unwrap();

    let buffer = BufferId::new();
    assert!(
        adapter.gutter(buffer).is_none(),
        "no gutter before first publish"
    );

    // Dispatch with modified content; the committed file was "hello\n".
    adapter.dispatch(GitCommand::RecomputeGutter {
        buffer,
        path: std::path::PathBuf::from("README.md"),
        content: "hello\nnew line\n".to_string(),
    });

    // Wait for the reader to appear.
    let start = Instant::now();
    let reader = loop {
        if let Some(r) = adapter.gutter(buffer) {
            break r;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(3),
            "gutter snapshot never published"
        );
        std::thread::sleep(Duration::from_millis(20));
    };

    let snap = reader.load();
    assert!(snap.revision >= 1);
    assert!(!snap.added.is_empty(), "expected at least one added line");
}

#[test]
fn adapter_returns_none_outside_repo() {
    let tmp = TempDir::new().unwrap();
    let rt = PortRuntime::new().unwrap();
    let adapter = GitAdapter::spawn(&rt.handle(), tmp.path()).unwrap();
    assert!(adapter.is_none());
}
