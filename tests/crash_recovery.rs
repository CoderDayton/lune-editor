//! Integration test: crash recovery round-trip.

mod common;

use std::path::PathBuf;

use common::TestWorkspace;

use lune_core::config::ConfigPaths;
use lune_core::recovery::RecoveryState;

#[test]
fn autosave_and_recover_roundtrip() {
    let ws = TestWorkspace::new();
    let config = ConfigPaths::from_root(ws.root().join(".config"));
    config.ensure_dirs().expect("failed to create dirs");

    let file_a = PathBuf::from("/tmp/test_a.rs");
    let file_b = PathBuf::from("/tmp/test_b.rs");
    let content_a = "fn a() { }\n";
    let content_b = "fn b() { todo!() }\n";

    // Autosave two dirty buffers.
    RecoveryState::autosave(
        &config,
        &[(file_a.clone(), content_a), (file_b.clone(), content_b)],
    )
    .expect("autosave failed");

    // Verify has_recovery is true.
    assert!(RecoveryState::has_recovery(&config));

    // Recover.
    let recovered = RecoveryState::recover(&config).expect("recover failed");
    assert_eq!(recovered.len(), 2);

    let a_recovered = recovered
        .iter()
        .find(|(p, _)| p == &file_a)
        .expect("file_a not found in recovered");
    assert_eq!(a_recovered.1, content_a);

    let b_recovered = recovered
        .iter()
        .find(|(p, _)| p == &file_b)
        .expect("file_b not found in recovered");
    assert_eq!(b_recovered.1, content_b);
}

#[test]
fn clear_removes_recovery() {
    let ws = TestWorkspace::new();
    let config = ConfigPaths::from_root(ws.root().join(".config"));
    config.ensure_dirs().expect("failed to create dirs");

    let file = PathBuf::from("/tmp/test.rs");
    RecoveryState::autosave(&config, &[(file, "content")]).expect("autosave failed");

    assert!(RecoveryState::has_recovery(&config));

    RecoveryState::clear(&config).expect("clear failed");
    assert!(!RecoveryState::has_recovery(&config));
}

#[test]
fn recover_with_no_recovery_returns_empty() {
    let ws = TestWorkspace::new();
    let config = ConfigPaths::from_root(ws.root().join(".config"));
    config.ensure_dirs().expect("failed to create dirs");

    assert!(!RecoveryState::has_recovery(&config));

    let recovered = RecoveryState::recover(&config).expect("recover failed");
    assert!(recovered.is_empty());
}

#[test]
fn autosave_prunes_stale_entries() {
    let ws = TestWorkspace::new();
    let config = ConfigPaths::from_root(ws.root().join(".config"));
    config.ensure_dirs().expect("failed to create dirs");

    let file_a = PathBuf::from("/tmp/a.rs");
    let file_b = PathBuf::from("/tmp/b.rs");

    // Autosave two files.
    RecoveryState::autosave(&config, &[(file_a.clone(), "aaa"), (file_b, "bbb")])
        .expect("first autosave failed");

    // Autosave with only file_a — file_b should be pruned.
    RecoveryState::autosave(&config, &[(file_a.clone(), "aaa_v2")])
        .expect("second autosave failed");

    let recovered = RecoveryState::recover(&config).expect("recover failed");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].0, file_a);
    assert_eq!(recovered[0].1, "aaa_v2");
}
