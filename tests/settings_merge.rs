//! Integration test: settings load, save, and merge workflow.

mod common;

use common::TestWorkspace;

use lune_core::settings::Settings;

#[test]
fn settings_save_and_load_roundtrip() {
    let ws = TestWorkspace::new();
    let path = ws.abs_path("config.toml");

    let mut settings = Settings::default();
    settings.editor.tab_size = 8;
    settings.editor.vim_mode = true;
    settings.theme = "Lune Light".to_string();

    settings.save(&path).expect("failed to save settings");

    let loaded = Settings::load(&path).expect("failed to load settings");
    assert_eq!(loaded.editor.tab_size, 8);
    assert!(loaded.editor.vim_mode);
    assert_eq!(loaded.theme, "Lune Light");

    // Non-overridden fields should be defaults.
    assert!(loaded.editor.use_spaces);
    assert!(loaded.ui.show_file_tree);
}

#[test]
fn settings_load_missing_returns_defaults() {
    let ws = TestWorkspace::new();
    let path = ws.abs_path("nonexistent.toml");

    let settings = Settings::load(&path).expect("failed to load settings");
    assert_eq!(settings, Settings::default());
}

#[test]
fn settings_merge_workspace_overrides() {
    let mut global = Settings::default();
    global.editor.tab_size = 4;
    global.editor.vim_mode = false;

    let mut workspace = Settings::default();
    workspace.editor.tab_size = 2; // Non-default override.
    workspace.editor.vim_mode = true; // Non-default override.

    global.merge_workspace(&workspace);
    assert_eq!(
        global.editor.tab_size, 2,
        "workspace should override tab_size"
    );
    assert!(global.editor.vim_mode, "workspace should override vim_mode");
}

#[test]
fn settings_merge_defaults_is_noop() {
    let mut original = Settings::default();
    original.editor.tab_size = 8;
    original.theme = "Custom Theme".to_string();

    let before = original.clone();
    original.merge_workspace(&Settings::default());

    assert_eq!(
        original, before,
        "merging defaults should not change settings"
    );
}

#[test]
fn settings_partial_toml_fills_defaults() {
    let ws = TestWorkspace::new();
    let path = ws.abs_path("partial.toml");

    // Write a minimal config with just one field.
    ws.write_file("partial.toml", "[editor]\ntab_size = 2\n");

    let settings = Settings::load(&path).expect("failed to load settings");
    assert_eq!(settings.editor.tab_size, 2);
    // Everything else should be defaults.
    assert!(settings.editor.use_spaces);
    assert!(!settings.editor.vim_mode);
    assert!(settings.ui.show_file_tree);
    assert_eq!(settings.theme, "Lune Dark");
}
