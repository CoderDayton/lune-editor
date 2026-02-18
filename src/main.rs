use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lune_core::config::ConfigPaths;
use lune_core::recovery::RecoveryState;
use lune_core::settings::{CliOverrides, Settings};
use lune_core::state_db::{self, StateDb};

/// Print usage information.
fn print_help() {
    eprintln!(
        "\
Lune Editor — an Agentic Development Environment

USAGE:
    lune-editor [OPTIONS] [PATH...]

ARGS:
    <PATH>...    File(s) or directory to open

OPTIONS:
    --config <path>    Use a custom config file
    --theme <name>     Override active theme
    --vim              Enable vim keybinding mode
    --no-vim           Disable vim keybinding mode
    --no-effects       Disable visual effects
    --version          Print version and exit
    --help, -h         Print this help and exit"
    );
}

/// Print version.
fn print_version() {
    eprintln!(
        "lune-editor {}",
        option_env!("CARGO_PKG_VERSION").unwrap_or("dev")
    );
}

/// Parse CLI arguments into overrides and positional paths.
fn parse_args() -> Option<(CliOverrides, Vec<PathBuf>)> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut overrides = CliOverrides::default();
    let mut paths = Vec::new();
    let mut iter = raw.iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return None;
            }
            "--version" => {
                print_version();
                return None;
            }
            "--no-effects" => overrides.effects_enabled = Some(false),
            "--vim" => overrides.vim_mode = Some(true),
            "--no-vim" => overrides.vim_mode = Some(false),
            "--config" => {
                if let Some(val) = iter.next() {
                    overrides.config_path = Some(PathBuf::from(val));
                } else {
                    eprintln!("Error: --config requires a path argument");
                    return None;
                }
            }
            "--theme" => {
                if let Some(val) = iter.next() {
                    overrides.theme = Some(val.clone());
                } else {
                    eprintln!("Error: --theme requires a name argument");
                    return None;
                }
            }
            other if other.starts_with("--") => {
                eprintln!("Unknown option: {other}");
                print_help();
                return None;
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }

    Some((overrides, paths))
}

fn main() -> Result<()> {
    // Parse CLI args.
    let Some((overrides, paths)) = parse_args() else {
        return Ok(());
    };

    // Resolve config paths.
    let config_paths = overrides
        .config_path
        .as_ref()
        .and_then(|p| {
            p.parent()
                .map(|dir| ConfigPaths::from_root(dir.to_path_buf()))
        })
        .or_else(ConfigPaths::resolve);

    // Ensure config directories exist on disk.
    if let Some(ref cp) = config_paths {
        if let Err(e) = cp.ensure_dirs() {
            eprintln!("Warning: failed to create config dirs: {e}");
        }
    }

    // Open the sled-backed state database and run TOML migration.
    let state_db = config_paths.as_ref().and_then(|cp| {
        match StateDb::open(&cp.state_dir()) {
            Ok(db) => {
                // One-time migration from legacy TOML files.
                match state_db::migrate_toml_state(&cp.state_dir(), &db) {
                    Ok(n) if n > 0 => eprintln!("Migrated {n} state file(s) from TOML to sled"),
                    Err(e) => eprintln!("Warning: TOML migration error: {e}"),
                    _ => {}
                }
                Some(db)
            }
            Err(e) => {
                eprintln!("Warning: failed to open state database: {e}");
                None
            }
        }
    });

    // Load settings from global config.
    let mut settings = config_paths.as_ref().map_or_else(Settings::default, |cp| {
        Settings::load(&cp.settings_file()).unwrap_or_else(|e| {
            eprintln!(
                "Warning: failed to load config {}: {e}",
                cp.settings_file().display()
            );
            Settings::default()
        })
    });

    // Apply CLI overrides (highest priority).
    settings.apply_cli_overrides(&overrides);

    // Create application state.
    let mut state = lune_ui::app::AppState::new();

    // Store config paths on state for use by settings commands and recovery.
    if let Some(ref cp) = config_paths {
        state.set_config_paths(cp.clone());
    }

    // Attach the state database for reactive persistence.
    if let Some(db) = state_db {
        state.set_state_db(db);
    }

    // Load user themes from config dir.
    if let Some(ref cp) = config_paths {
        let themes_dir = cp.themes_dir();
        if themes_dir.is_dir() {
            let loaded = state.theme_registry.load_dir(&themes_dir);
            if loaded > 0 {
                eprintln!("Loaded {loaded} custom theme(s)");
            }
        }
    }

    // Apply settings to state (layout, vim, theme, effects).
    state.apply_settings(&settings);

    // Load custom keybindings and merge with defaults.
    if let Some(ref cp) = config_paths {
        match lune_ui::keybindings::KeymapConfig::load(&cp.keybindings_file()) {
            Ok(keymap_config) => {
                let overrides_map = keymap_config.compile_normal();
                if !overrides_map.is_empty() {
                    state.keymap.merge(&overrides_map);
                    eprintln!("Loaded {} custom keybinding(s)", overrides_map.len());
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to load keybindings {}: {e}",
                    cp.keybindings_file().display()
                );
            }
        }
    }

    // Determine the workspace root for state persistence.
    let workspace_root = determine_workspace_root(&paths);

    // Open files/directories from CLI positional args and merge workspace settings.
    open_paths_or_cwd(&mut state, &mut settings, &overrides, &paths);

    // Check for crash recovery before restoring workspace state.
    check_crash_recovery(&mut state, config_paths.as_ref());

    // Restore saved workspace state (open files, cursors, layout).
    restore_workspace_state(&mut state, workspace_root.as_deref());

    // Record this workspace in recent workspaces.
    record_recent_workspace(&state, workspace_root.as_deref());

    // Verify we have a controlling terminal before entering the TUI.
    require_controlling_terminal()?;

    // Run the TUI event loop.
    lune_ui::app::run(&mut state)?;

    // ── Clean exit: persist state ──────────────────────────────────────

    // Final save of workspace state to sled (complements debounced saves).
    if let Some(db) = state.state_db() {
        if let Some(mut wstate) = state.collect_workspace_state() {
            wstate.touch();
            if let Err(e) = db.put_workspace(&wstate) {
                eprintln!("Warning: failed to save workspace state: {e}");
            }
        }
        if let Err(e) = db.flush() {
            eprintln!("Warning: failed to flush state database: {e}");
        }
    }

    // Clear crash recovery (clean exit = no recovery needed).
    if let Some(ref cp) = config_paths {
        if let Err(e) = RecoveryState::clear(cp) {
            eprintln!("Warning: failed to clear recovery state: {e}");
        }
    }

    Ok(())
}

/// Bail early with a clear message if there is no controlling terminal.
///
/// crossterm's `enable_raw_mode()` opens `/dev/tty` which returns ENXIO
/// when the process has no controlling terminal (pipes, CI, detached).
fn require_controlling_terminal() -> Result<()> {
    File::open("/dev/tty").context(
        "No controlling terminal found. \
         Lune must be run in an interactive terminal, not from a pipe, CI, or detached process.",
    )?;
    Ok(())
}

/// Determine the workspace root from CLI paths or CWD.
fn determine_workspace_root(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return std::env::current_dir().ok();
    }

    // Use the first directory argument as workspace root.
    for path in paths {
        if path.is_dir() {
            return Some(path.clone());
        }
    }

    // If only files were specified, use CWD.
    std::env::current_dir().ok()
}

/// Open files/directories from CLI args, or auto-open CWD if no args.
fn open_paths_or_cwd(
    state: &mut lune_ui::app::AppState,
    settings: &mut Settings,
    overrides: &CliOverrides,
    paths: &[PathBuf],
) {
    if paths.is_empty() {
        // No arguments: auto-open CWD as workspace.
        if let Ok(cwd) = std::env::current_dir() {
            merge_workspace_settings(state, settings, overrides, &cwd);
            match state.open_workspace(&cwd) {
                Ok(()) => {}
                Err(e) => eprintln!("Warning: could not open workspace {}: {e}", cwd.display()),
            }
        }
    } else {
        for path in paths {
            if path.is_dir() {
                merge_workspace_settings(state, settings, overrides, path);
                match state.open_workspace(path) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Warning: could not open workspace {}: {e}", path.display());
                    }
                }
            } else if path.exists() {
                match state.open_file(path) {
                    Ok(_id) => {}
                    Err(e) => eprintln!("Warning: could not open {}: {e}", path.display()),
                }
            } else {
                eprintln!("Warning: path not found: {}", path.display());
            }
        }
    }
}

/// Merge workspace-local settings if a `.lune/config.toml` exists.
fn merge_workspace_settings(
    state: &mut lune_ui::app::AppState,
    settings: &mut Settings,
    overrides: &CliOverrides,
    workspace_root: &Path,
) {
    if let Some(ws_config) = lune_core::config::workspace_config_file(workspace_root) {
        match Settings::load(&ws_config) {
            Ok(ws_settings) => {
                settings.merge_workspace(&ws_settings);
                settings.apply_cli_overrides(overrides);
                state.apply_settings(settings);
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to load workspace config {}: {e}",
                    ws_config.display()
                );
            }
        }
    }
}

/// Check for crash recovery files and notify the user.
fn check_crash_recovery(state: &mut lune_ui::app::AppState, config_paths: Option<&ConfigPaths>) {
    let Some(cp) = config_paths else {
        return;
    };

    if !RecoveryState::has_recovery(cp) {
        return;
    }

    // Attempt to recover dirty buffers from the previous session.
    match RecoveryState::recover(cp) {
        Ok(recovered) if !recovered.is_empty() => {
            let count = recovered.len();
            for (original_path, _content) in &recovered {
                // Open the original file (if it exists on disk).
                // The recovery content is available but we open the disk
                // version — the user can see the notification and decide
                // whether to investigate.
                if original_path.exists() {
                    let _ = state.open_file(original_path);
                }
            }
            state.overlay.notify(
                format!("Recovered {count} unsaved file(s) from previous session"),
                lune_ui::widgets::overlay::NotificationLevel::Warning,
            );
        }
        Ok(_) => {
            // Empty recovery — clean up stale manifest.
            let _ = RecoveryState::clear(cp);
        }
        Err(e) => {
            eprintln!("Warning: failed to read recovery state: {e}");
        }
    }
}

/// Restore saved workspace state (open files, cursor positions, layout).
fn restore_workspace_state(state: &mut lune_ui::app::AppState, workspace_root: Option<&Path>) {
    let (Some(db), Some(root)) = (state.state_db(), workspace_root) else {
        return;
    };

    match db.get_workspace(root) {
        Ok(Some(mut wstate)) => {
            wstate.prune_missing_files();
            state.restore_workspace_state(&wstate);
        }
        Ok(None) => {} // No saved state — first time opening this workspace.
        Err(e) => {
            eprintln!("Warning: failed to load workspace state: {e}");
        }
    }
}

/// Record the current workspace in the recent workspaces index.
fn record_recent_workspace(state: &lune_ui::app::AppState, workspace_root: Option<&Path>) {
    let (Some(db), Some(root)) = (state.state_db(), workspace_root) else {
        return;
    };

    match db.get_recent() {
        Ok(mut recent) => {
            recent.record_open(root);
            recent.prune_missing();
            if let Err(e) = db.put_recent(&recent) {
                eprintln!("Warning: failed to save recent workspaces: {e}");
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to load recent workspaces: {e}");
        }
    }
}
