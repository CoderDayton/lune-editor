use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lune_core::config::ConfigPaths;
use lune_core::recovery::RecoveryState;
use lune_core::settings::{CliOverrides, Settings};
use lune_core::state_db::StateDb;

/// Print usage information to stdout.
///
/// Convention is stdout for successful `--help` / `--version`, so that
/// `lune-editor --help | less` works.
fn print_help() {
    println!(
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
    --version          Print version and exit
    --help, -h         Print this help and exit"
    );
}

/// Print version to stdout.
fn print_version() {
    println!(
        "lune-editor {}",
        option_env!("CARGO_PKG_VERSION").unwrap_or("dev")
    );
}

/// Outcome of parsing the CLI arguments.
enum ParseOutcome {
    /// Continue startup with the parsed overrides and positional paths.
    Continue(CliOverrides, Vec<PathBuf>),
    /// User asked for `--help` or `--version` — print and exit with 0.
    HelpOrVersion,
    /// Malformed invocation; the message has already been written to
    /// stderr.  Exit with a nonzero status so CI / automation can detect it.
    BadArgs,
}

/// Consume the next argument as a value for `flag`, or print an error.
fn next_arg_value<'a>(
    iter: &mut std::slice::Iter<'a, String>,
    flag: &str,
    what: &str,
) -> Option<&'a String> {
    let v = iter.next();
    if v.is_none() {
        eprintln!("Error: {flag} requires {what}");
    }
    v
}

/// Parse CLI arguments into overrides and positional paths.
fn parse_args() -> ParseOutcome {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut overrides = CliOverrides::default();
    let mut paths = Vec::new();
    let mut iter = raw.iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return ParseOutcome::HelpOrVersion;
            }
            "--version" => {
                print_version();
                return ParseOutcome::HelpOrVersion;
            }
            "--vim" => overrides.vim_mode = Some(true),
            "--no-vim" => overrides.vim_mode = Some(false),
            "--config" => {
                let Some(val) = next_arg_value(&mut iter, "--config", "a path argument") else {
                    return ParseOutcome::BadArgs;
                };
                overrides.config_path = Some(PathBuf::from(val));
            }
            "--theme" => {
                let Some(val) = next_arg_value(&mut iter, "--theme", "a name argument") else {
                    return ParseOutcome::BadArgs;
                };
                overrides.theme = Some(val.clone());
            }
            other if other.starts_with("--") => {
                eprintln!("Unknown option: {other}");
                print_help();
                return ParseOutcome::BadArgs;
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }

    ParseOutcome::Continue(overrides, paths)
}

#[allow(clippy::too_many_lines)] // top-level bootstrap; extraction would obscure startup order
fn main() -> Result<()> {
    // Parse CLI args.  Unknown flags / missing values exit nonzero so
    // shell pipelines and CI surface the failure; --help/--version exit
    // cleanly with 0.
    let (overrides, paths) = match parse_args() {
        ParseOutcome::Continue(o, p) => (o, p),
        ParseOutcome::HelpOrVersion => return Ok(()),
        ParseOutcome::BadArgs => std::process::exit(2),
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

    // Open the global state database. The per-workspace database is
    // attached later, once the workspace root has been determined.
    let mut state_db = config_paths
        .as_ref()
        .map(|cp| StateDb::open(&cp.state_dir()));

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

    // Spawn the shared port runtime (tokio) before building AppState so
    // adapters can attach as the workspace opens. Startup errors here are
    // non-fatal — we fall back to NullGitPort / NullAiPort / MemoryPersistence.
    let port_runtime = match lune_core::ports::PortRuntime::new() {
        Ok(rt) => Some(std::sync::Arc::new(rt)),
        Err(e) => {
            eprintln!("Warning: failed to start port runtime: {e}");
            None
        }
    };

    // Create application state.
    let mut state = lune_ui::app::AppState::new();
    if let Some(ref rt) = port_runtime {
        state.attach_port_runtime(rt.clone());
    }

    // Store config paths on state for use by settings commands and recovery.
    if let Some(ref cp) = config_paths {
        state.set_config_paths(cp.clone());
    }

    // Swap the default in-memory persistence port for the JSON-file one
    // if we have both a runtime and a resolved state directory. Path:
    // `<state_dir>/port-store.json` (separate from the StateDb file).
    if let (Some(rt), Some(cp)) = (port_runtime.as_ref(), config_paths.as_ref()) {
        let path = cp.state_dir().join("port-store.json");
        let port = lune_core::ports::JsonFilePersistencePort::shared(
            &rt.handle(),
            lune_core::ports::JsonFilePortConfig {
                path,
                debounce_ms: 200,
            },
        );
        state.attach_persistence_port(port);
    }

    // Determine the workspace root early so we can attach the per-workspace
    // DB before handing ownership of state_db to AppState.
    let workspace_root = determine_workspace_root(&paths);

    // Best-effort: attach the per-workspace JSON state file. The JSON
    // backend does not take file locks, so attach rarely fails — but if
    // the parent directory cannot be created, we fall back to
    // workspace persistence disabled and warn the user via the TUI
    // overlay (stderr is invisible when launched from a desktop launcher).
    if let (Some(db), Some(root)) = (state_db.as_mut(), workspace_root.as_deref()) {
        if let Err(e) = db.attach_workspace(root) {
            state.set_startup_warning(format!(
                "Workspace state disabled for {}: {e}. Another Lune instance may be editing this workspace.",
                root.display()
            ));
        }
    }

    // Warn when the global state DB is unavailable (recent workspaces +
    // agent layouts won't persist from this instance).
    if matches!(&state_db, Some(db) if !db.has_global()) {
        state.set_startup_warning(
            "Global state disabled: recent workspaces and saved agent layouts will not persist from this instance."
                .to_owned(),
        );
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

    // Apply settings to state (layout, vim, theme).
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

    // Open files/directories from CLI positional args and merge workspace settings.
    open_paths_or_cwd(&mut state, &mut settings, &overrides, &paths);

    // Check for crash recovery before restoring workspace state.
    check_crash_recovery(&mut state, config_paths.as_ref());

    // Restore saved workspace state (open files, cursors, layout).
    restore_workspace_state(&mut state);

    // Record this workspace in recent workspaces.
    record_recent_workspace(&mut state, workspace_root.as_deref());

    // If no file ended up open after CLI args + recovery + restore, land
    // the user on the file tree rather than an empty editor pane.
    state.focus_file_tree_if_no_buffer();

    // Verify we have a controlling terminal before entering the TUI.
    require_controlling_terminal()?;

    // Run the TUI event loop.
    lune_ui::app::run(&mut state)?;

    // ── Clean exit: persist state ──────────────────────────────────────

    // Final reactive save: workspace state + undo history (complements the
    // debounced mid-session saves). Then flush the state DB to disk.
    state.persist_full_state();
    if let Some(db) = state.state_db_mut() {
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
    // Empty args → fall back to CWD as a single directory "argument".
    let cwd_fallback: Vec<PathBuf>;
    let paths: &[PathBuf] = if paths.is_empty() {
        cwd_fallback = std::env::current_dir().ok().into_iter().collect();
        &cwd_fallback
    } else {
        paths
    };

    for path in paths {
        if path.is_dir() {
            merge_workspace_settings(state, settings, overrides, path);
            if let Err(e) = state.open_workspace(path) {
                eprintln!("Warning: could not open workspace {}: {e}", path.display());
            }
        } else if path.exists() {
            if let Err(e) = state.open_file(path) {
                eprintln!("Warning: could not open {}: {e}", path.display());
            }
        } else {
            eprintln!("Warning: path not found: {}", path.display());
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
    //
    // For each `(path, recovered_content)`:
    //   1. If the file still exists on disk, open it (so the buffer is
    //      associated with its real path and the canonical save target
    //      is correct) and push the recovered content into the buffer
    //      via `replace_all_text`.  The edit goes through a normal
    //      transaction, so the buffer is marked dirty and the disk
    //      version remains undoable.
    //   2. If the file no longer exists on disk, fall back to a scratch
    //      buffer holding the recovered content — losing the path is
    //      better than losing the work.  The buffer is named after the
    //      missing path so the user can recognize it and `Save As`.
    match RecoveryState::recover(cp) {
        Ok(recovered) if !recovered.is_empty() => {
            let mut restored = 0usize;
            let mut orphaned = 0usize;
            let mut failed = 0usize;
            for (original_path, content) in &recovered {
                if original_path.exists() {
                    match state.open_file(original_path) {
                        Ok(id) => {
                            if let Some(buf) = state.session.registry.get_mut(id) {
                                buf.replace_all_text(content);
                            }
                            restored += 1;
                        }
                        Err(e) => {
                            log::warn!(
                                "recovery: failed to open {} ({e}); falling back to scratch",
                                original_path.display()
                            );
                            if restore_to_scratch(state, original_path, content) {
                                orphaned += 1;
                            } else {
                                failed += 1;
                            }
                        }
                    }
                } else {
                    log::warn!(
                        "recovery: source file {} no longer exists; restoring into a scratch buffer",
                        original_path.display()
                    );
                    if restore_to_scratch(state, original_path, content) {
                        orphaned += 1;
                    } else {
                        failed += 1;
                    }
                }
            }
            notify_recovery_outcome(state, restored, orphaned, failed);
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

/// Restore a recovery entry into a fresh scratch buffer when its disk
/// path can no longer host it (deleted/moved, or `open_file` failed).
///
/// Returns `true` on success.  The buffer keeps the original path on
/// `file_path` so the tab title is recognizable and a plain `Save` will
/// re-create the missing file at its prior location; `replace_all_text`
/// runs as a normal edit transaction so the buffer is marked dirty and
/// the user is prompted on close.
fn restore_to_scratch(
    state: &mut lune_ui::app::AppState,
    original_path: &Path,
    content: &str,
) -> bool {
    let id = state.session.new_scratch();
    let Some(buf) = state.session.registry.get_mut(id) else {
        log::warn!(
            "recovery: scratch buffer {id:?} vanished before content could be restored for {}",
            original_path.display()
        );
        return false;
    };
    buf.file_path = Some(original_path.to_path_buf());
    buf.replace_all_text(content);
    true
}

/// Emit a single user-visible notification summarizing the recovery
/// outcome.  Splits restored-in-place, restored-into-scratch, and
/// outright failures so the user understands what happened.
fn notify_recovery_outcome(
    state: &mut lune_ui::app::AppState,
    restored: usize,
    orphaned: usize,
    failed: usize,
) {
    use lune_ui::widgets::overlay::NotificationLevel;
    if restored == 0 && orphaned == 0 && failed == 0 {
        return;
    }
    let mut parts: Vec<String> = Vec::with_capacity(3);
    if restored > 0 {
        parts.push(format!("Recovered {restored} unsaved file(s)"));
    }
    if orphaned > 0 {
        parts.push(format!(
            "{orphaned} orphaned recovery file(s) restored to scratch — use Save As to rewrite"
        ));
    }
    if failed > 0 {
        parts.push(format!("{failed} recovery entries could not be restored"));
    }
    let level = if failed > 0 {
        NotificationLevel::Error
    } else {
        NotificationLevel::Warning
    };
    state.overlay.notify(parts.join("; "), level);
}

/// Restore saved workspace state (open files, cursor positions, layout)
/// from the attached per-workspace database.
fn restore_workspace_state(state: &mut lune_ui::app::AppState) {
    let Some(db) = state.state_db() else {
        return;
    };

    match db.get_workspace() {
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
fn record_recent_workspace(state: &mut lune_ui::app::AppState, workspace_root: Option<&Path>) {
    let Some(root) = workspace_root else { return };
    let Some(db) = state.state_db_mut() else {
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
