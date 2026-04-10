//! End-to-end integration tests for the state persistence pipeline.
//!
//! Covers the full multi-instance flow exposed by [`StateDb`]:
//!
//! 1. Open global DB.
//! 2. Attach a per-workspace DB.
//! 3. Write workspace state + undo history.
//! 4. Drop the handle (cleanly — sled releases the flock on Drop).
//! 5. Reopen, attach the same workspace, verify everything round-trips.
//!
//! Plus the multi-instance scenario: two concurrent handles on the same
//! state directory editing *different* workspaces must not collide.

use std::path::{Path, PathBuf};

use lune_core::state_db::StateDb;
use lune_core::undo::UndoState;
use lune_core::workspace_state::{RecentEntry, RecentWorkspaces, WorkspaceState};

fn make_workspace(root: &Path, file: &str, cursor: (usize, usize)) -> WorkspaceState {
    let mut ws = WorkspaceState::new(root.to_path_buf());
    ws.open_files.push(PathBuf::from(file));
    ws.cursor_positions.insert(PathBuf::from(file), cursor);
    ws.file_tree_width_pct = 22;
    ws
}

#[test]
fn full_pipeline_round_trip_across_restarts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_dir = dir.path();
    let workspace_root = state_dir.join("project-alpha");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let tracked_file = workspace_root.join("src/lib.rs");
    std::fs::create_dir_all(tracked_file.parent().unwrap()).unwrap();
    std::fs::write(&tracked_file, "// hi\n").unwrap();

    // ── Session 1: write workspace state + undo history + recent index ──
    {
        let mut db = StateDb::open(state_dir);
        assert!(
            db.has_global(),
            "global DB should open in an empty state dir"
        );
        db.attach_workspace(&workspace_root)
            .expect("first attach should succeed");
        assert!(db.has_workspace());

        db.put_workspace(&make_workspace(&workspace_root, "src/lib.rs", (12, 4)))
            .unwrap();
        db.put_undo(&tracked_file, &UndoState::default()).unwrap();

        let mut recent = RecentWorkspaces::default();
        recent.entries.push(RecentEntry {
            root: workspace_root.clone(),
            last_opened: 1_700_000_000,
        });
        db.put_recent(&recent).unwrap();

        db.flush().unwrap();
        // Drop releases the sled flocks.
    }

    // ── Session 2: reopen, verify every persisted piece survives ────────
    {
        let mut db = StateDb::open(state_dir);
        assert!(db.has_global());
        db.attach_workspace(&workspace_root)
            .expect("reattach after clean drop should succeed");

        let loaded = db
            .get_workspace()
            .unwrap()
            .expect("workspace state should survive restart");
        assert_eq!(loaded.root, workspace_root);
        assert_eq!(loaded.open_files, vec![PathBuf::from("src/lib.rs")]);
        assert_eq!(
            loaded.cursor_positions.get(&PathBuf::from("src/lib.rs")),
            Some(&(12, 4))
        );
        assert_eq!(loaded.file_tree_width_pct, 22);

        assert!(db.get_undo(&tracked_file).unwrap().is_some());

        let recent = db.get_recent().unwrap();
        assert_eq!(recent.entries.len(), 1);
        assert_eq!(recent.entries[0].root, workspace_root);
    }
}

#[test]
fn two_instances_on_different_workspaces_persist_independently() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_dir = dir.path();
    let proj_a = state_dir.join("proj-a");
    let proj_b = state_dir.join("proj-b");
    std::fs::create_dir_all(&proj_a).unwrap();
    std::fs::create_dir_all(&proj_b).unwrap();

    // First instance: owns the global lock + workspaces/<proj-a>.sled.
    let mut first = StateDb::open(state_dir);
    assert!(first.has_global(), "first instance should hold global");
    first
        .attach_workspace(&proj_a)
        .expect("first workspace attach should succeed");
    first
        .put_workspace(&make_workspace(&proj_a, "main.rs", (1, 1)))
        .unwrap();
    first.flush().unwrap();

    // Second instance: global is locked by `first`, so it degrades to
    // global-disabled. It can still attach to a *different* workspace
    // directory and persist per-workspace state independently.
    let mut second = StateDb::open(state_dir);
    assert!(
        !second.has_global(),
        "second instance should not hold the global lock"
    );
    second
        .attach_workspace(&proj_b)
        .expect("second instance can attach a disjoint workspace");
    assert!(second.has_workspace());

    second
        .put_workspace(&make_workspace(&proj_b, "lib.rs", (9, 9)))
        .unwrap();
    second.flush().unwrap();

    // Both see their own workspace data.
    let a = first.get_workspace().unwrap().unwrap();
    assert_eq!(a.root, proj_a);
    assert_eq!(
        a.cursor_positions.get(&PathBuf::from("main.rs")),
        Some(&(1, 1))
    );

    let b = second.get_workspace().unwrap().unwrap();
    assert_eq!(b.root, proj_b);
    assert_eq!(
        b.cursor_positions.get(&PathBuf::from("lib.rs")),
        Some(&(9, 9))
    );

    // Global reads from the degraded instance return defaults, not errors.
    assert!(second.get_recent().unwrap().entries.is_empty());
}

#[test]
fn same_workspace_in_two_instances_degrades_gracefully() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_dir = dir.path();
    let project = state_dir.join("shared-project");
    std::fs::create_dir_all(&project).unwrap();

    let mut first = StateDb::open(state_dir);
    first.attach_workspace(&project).unwrap();
    first
        .put_workspace(&make_workspace(&project, "a.rs", (1, 1)))
        .unwrap();
    first.flush().unwrap();

    // Second instance: same workspace path. attach_workspace should fail
    // (sled flock on the per-workspace db is held by first), but the
    // StateDb itself stays usable — just with workspace ops no-oping.
    let mut second = StateDb::open(state_dir);
    assert!(
        second.attach_workspace(&project).is_err(),
        "same-workspace attach must fail while first holds the lock"
    );
    assert!(!second.has_workspace());

    // No-op semantics: writes succeed silently, reads return None.
    second
        .put_workspace(&make_workspace(&project, "b.rs", (2, 2)))
        .unwrap();
    assert!(second.get_workspace().unwrap().is_none());
}
