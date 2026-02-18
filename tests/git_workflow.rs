//! Integration test: git workflow — init repo, modify, stage, commit.

mod common;

use std::path::Path;

use common::TestWorkspace;

use lune_git::GitService;

#[test]
fn git_init_and_status() {
    let ws = TestWorkspace::new();
    ws.write_file("README.md", "# Hello\n");
    ws.init_git();

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");
    let status = git.status().expect("failed to get status");

    // After initial commit with all files staged+committed, status should be clean.
    assert!(
        status.files.is_empty(),
        "Expected clean status after initial commit, got {} entries",
        status.files.len()
    );
}

#[test]
fn git_detects_modified_file() {
    let ws = TestWorkspace::new();
    ws.write_file("src/main.rs", "fn main() {}\n");
    ws.init_git();

    // Modify a tracked file.
    ws.write_file("src/main.rs", "fn main() { println!(\"hi\"); }\n");

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");
    let status = git.status().expect("failed to get status");

    assert!(
        !status.files.is_empty(),
        "Expected dirty status after modification"
    );
    let modified = status.files.iter().any(|e| e.path.ends_with("main.rs"));
    assert!(modified, "Expected main.rs to be in status entries");
}

#[test]
fn git_detects_untracked_file() {
    let ws = TestWorkspace::new();
    ws.write_file("tracked.txt", "tracked\n");
    ws.init_git();

    // Add new untracked file.
    ws.write_file("new_file.txt", "new content\n");

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");
    let status = git.status().expect("failed to get status");

    let has_new = status
        .files
        .iter()
        .any(|e| e.path.ends_with("new_file.txt"));
    assert!(has_new, "Expected new_file.txt in status entries");
}

#[test]
fn git_stage_and_commit() {
    let ws = TestWorkspace::new();
    ws.write_file("file.txt", "initial\n");
    ws.init_git();

    // Modify file.
    ws.write_file("file.txt", "modified\n");

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");

    // Stage.
    git.stage(Path::new("file.txt")).expect("failed to stage");
    let status = git.status().expect("failed to get status");
    let staged = status
        .files
        .iter()
        .any(|e| e.path.ends_with("file.txt") && e.staged);
    assert!(staged, "Expected file.txt to be staged");

    // Commit.
    git.commit("test commit").expect("failed to commit");

    // After commit, status should be clean.
    let status = git.status().expect("failed to get status");
    assert!(
        status.files.is_empty(),
        "Expected clean status after commit, got {} entries",
        status.files.len()
    );
}

#[test]
fn git_diff_shows_changes() {
    let ws = TestWorkspace::new();
    ws.write_file("code.py", "x = 1\ny = 2\n");
    ws.init_git();

    ws.write_file("code.py", "x = 1\ny = 3\nz = 4\n");

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");
    let diffs = git.diff_all().expect("failed to get diffs");

    assert!(!diffs.is_empty(), "Expected at least one file diff");
    let code_diff = diffs.iter().find(|d| d.path.ends_with("code.py"));
    assert!(code_diff.is_some(), "Expected diff for code.py");

    let diff = code_diff.unwrap();
    assert!(!diff.hunks.is_empty(), "Expected at least one hunk");
}

#[test]
fn git_branch_name() {
    let ws = TestWorkspace::new();
    ws.write_file("README.md", "# Test\n");
    ws.init_git();

    let git = GitService::open(ws.root())
        .expect("should open")
        .expect("should detect git repo");
    let status = git.status().expect("failed to get status");
    let branch = &status.branch;

    // git init typically creates "main" or "master".
    assert!(
        branch == "main" || branch == "master",
        "Expected main or master, got {branch:?}",
    );
}
