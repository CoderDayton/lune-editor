//! End-to-end: bring up `AppState`, attach a `PortRuntime`, open a real
//! git workspace, and verify the `GitAdapter` publishes a snapshot through
//! `state.git_port()`.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use git2::{Repository, Signature};
use lune_core::ports::PortRuntime;
use lune_ui::app::AppState;
use tempfile::TempDir;

fn init_repo_with_commit(dir: &std::path::Path) {
    let repo = Repository::init(dir).unwrap();
    fs::write(dir.join("hello.txt"), "hi\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("hello.txt")).unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = Signature::now("t", "t@t").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    index.write().unwrap();
}

#[test]
fn open_workspace_installs_git_adapter() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = Arc::new(PortRuntime::new().unwrap());
    let mut state = AppState::new();
    state.attach_port_runtime(rt);
    state.open_workspace(tmp.path()).unwrap();

    // Wait for the first published snapshot (revision > 0 == real adapter,
    // not NullGitPort which stays at revision 0).
    let reader = state.git_port().status();
    let start = Instant::now();
    loop {
        let snap = reader.load();
        if snap.revision > 0 {
            let branch = snap.branch.as_deref().expect("branch published");
            assert!(
                branch == "main" || branch == "master",
                "unexpected branch: {branch}"
            );
            return;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(5),
            "GitAdapter never published; port still null?"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn status_bar_reads_branch_from_port_snapshot() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = Arc::new(PortRuntime::new().unwrap());
    let mut state = AppState::new();
    state.attach_port_runtime(rt);
    state.open_workspace(tmp.path()).unwrap();

    // Wait for adapter to publish its first snapshot.
    let reader = state.git_port().status();
    let start = Instant::now();
    while reader.load().revision == 0 {
        assert!(
            start.elapsed() <= Duration::from_secs(5),
            "adapter never published"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    // Clear the legacy field to prove the display is driven by the port,
    // not the in-state string. Rendering the status bar must still show
    // the branch name.
    // The legacy `git_branch/ahead/behind` fields were deleted; the
    // display is driven entirely by the port snapshot now.
    let display = state.build_git_branch_display();
    assert!(
        display == "main" || display == "master",
        "status bar did not pick up branch from port: got {display:?}"
    );
}

#[test]
fn gutter_for_render_reads_from_port_snapshot() {
    use lune_core::ports::GitCommand;

    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path());

    let rt = Arc::new(PortRuntime::new().unwrap());
    let mut state = AppState::new();
    state.attach_port_runtime(rt);
    state.open_workspace(tmp.path()).unwrap();

    let hello = tmp.path().join("hello.txt");
    let id = state.open_file(&hello).unwrap();

    // Dispatch gutter computation with content that differs from HEAD ("hi\n").
    state.git_port().dispatch(GitCommand::RecomputeGutter {
        buffer: id,
        path: std::path::PathBuf::from("hello.txt"),
        content: "hi\nadded line\n".to_string(),
    });

    // Poll until the port exposes a snapshot.
    let start = Instant::now();
    loop {
        if state.has_gutter(id) {
            break;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(3),
            "gutter never appeared in port"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let marks = state.gutter_for_render(id).expect("marks available");
    assert!(!marks.marks.is_empty());
}

#[test]
fn open_non_repo_keeps_null_port() {
    let tmp = TempDir::new().unwrap();
    let rt = Arc::new(PortRuntime::new().unwrap());
    let mut state = AppState::new();
    state.attach_port_runtime(rt);
    state.open_workspace(tmp.path()).unwrap();

    // NullGitPort never publishes, so revision stays at 0 after a short
    // wait. (Can't prove "forever" — a 200ms sample is enough to catch a
    // bug where we accidentally attached an adapter.)
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(state.git_port().status().load().revision, 0);
}
