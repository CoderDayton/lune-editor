#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_workspace_command(
    cmd: &AppCommand,
    state: &mut AppState,
) -> Option<Control<AppEvent>> {
    let control = match cmd {
        AppCommand::Save => handle_save(state),
        AppCommand::SaveAll => handle_save_all(state),
        AppCommand::OpenFile(path) => handle_open_file(path, state),
        AppCommand::ToggleHiddenFiles
        | AppCommand::RevealInFileTree(_)
        | AppCommand::NewFile
        | AppCommand::NewDir
        | AppCommand::RenameEntry
        | AppCommand::DeleteEntry
        | AppCommand::CreateFileConfirmed(_)
        | AppCommand::CreateDirConfirmed(_)
        | AppCommand::RenameConfirmed { .. }
        | AppCommand::DeleteConfirmed(_) => handle_file_tree_command(cmd, state),
        AppCommand::ChangeLanguage(lang_id) => handle_change_language(*lang_id, state),
        AppCommand::OpenSettings | AppCommand::OpenKeybindings => {
            handle_open_config_file(cmd, state)
        }
        _ => return None,
    };
    Some(control)
}

pub(super) fn handle_save(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        match buf.save() {
            Ok(()) => {
                state.status_message = "Saved.".to_string();
                state.overlay.notify("File saved", NotificationLevel::Info);
            }
            Err(e) => {
                state.status_message = format!("Save failed: {e}");
                state
                    .overlay
                    .notify(format!("Save failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    state.refresh_git();
    Control::Changed
}

pub(super) fn handle_save_all(state: &mut AppState) -> Control<AppEvent> {
    let ids: Vec<_> = state.tabs.clone();
    let mut saved = 0;
    let mut errors = 0;
    for id in ids {
        if let Some(buf) = state.registry.get_mut(id) {
            if buf.is_dirty() {
                match buf.save() {
                    Ok(()) => saved += 1,
                    Err(_) => errors += 1,
                }
            }
        }
    }
    state.status_message = format!("Saved {saved} file(s), {errors} error(s).");
    state
        .overlay
        .notify(format!("Saved {saved} file(s)"), NotificationLevel::Info);
    Control::Changed
}

fn file_tree_context_dir(state: &AppState) -> PathBuf {
    if let Some(path) = state.file_tree.selected_path() {
        if state.file_tree.selected_is_dir() {
            return path.to_path_buf();
        }
        if let Some(parent) = path.parent() {
            return parent.to_path_buf();
        }
    }
    state.workspace.as_ref().map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        |ws| ws.root().to_path_buf(),
    )
}

fn handle_file_tree_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    match cmd {
        AppCommand::ToggleHiddenFiles => {
            if let Some(ref mut ws) = state.workspace {
                ws.toggle_hidden();
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
            }
            Control::Changed
        }
        AppCommand::RevealInFileTree(path) => {
            let path = path.clone();
            if let Some(ref mut ws) = state.workspace {
                if let Err(e) = state.file_tree.reveal_path(&path, ws) {
                    log::error!("Failed to reveal path: {e}");
                }
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                state.file_tree.select_by_path(&path, 20);
            }
            Control::Changed
        }
        AppCommand::NewFile => {
            let parent = file_tree_context_dir(state);
            let dialog = overlay::InputDialogState::new(
                "New File",
                "filename",
                overlay::InputDialogAction::CreateFile { parent },
            );
            state.overlay.open_input_dialog(dialog);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::NewDir => {
            let parent = file_tree_context_dir(state);
            let dialog = overlay::InputDialogState::new(
                "New Directory",
                "directory name",
                overlay::InputDialogAction::CreateDir { parent },
            );
            state.overlay.open_input_dialog(dialog);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::RenameEntry => {
            let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) else {
                return Control::Continue;
            };
            let current_name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let dialog = overlay::InputDialogState::new(
                "Rename",
                "new name",
                overlay::InputDialogAction::Rename { from: path },
            )
            .with_input(current_name);
            state.overlay.open_input_dialog(dialog);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::DeleteEntry => {
            let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) else {
                return Control::Continue;
            };
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            state.overlay.open_confirm(
                format!("Delete \"{name}\"?"),
                AppCommand::DeleteConfirmed(path),
            );
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::CreateFileConfirmed(path) => handle_create_file(path, state),
        AppCommand::CreateDirConfirmed(path) => handle_create_dir(path, state),
        AppCommand::RenameConfirmed { from, to } => handle_rename(from, to, state),
        AppCommand::DeleteConfirmed(path) => handle_delete(path, state),
        _ => Control::Continue,
    }
}

fn handle_create_file(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::CreateFile(
            path.to_path_buf(),
        )) {
            Ok(()) => {
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                state.overlay.notify(
                    format!(
                        "Created: {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    NotificationLevel::Info,
                );
            }
            Err(e) => {
                state
                    .overlay
                    .notify(format!("Create failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    handle_open_file(path, state)
}

fn handle_create_dir(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::CreateDir(path.to_path_buf())) {
            Ok(()) => {
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                state.file_tree.select_by_path(path, 20);
                state.overlay.notify(
                    format!(
                        "Created: {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    NotificationLevel::Info,
                );
            }
            Err(e) => {
                state
                    .overlay
                    .notify(format!("Create failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

fn handle_rename(from: &Path, to: &Path, state: &mut AppState) -> Control<AppEvent> {
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        }) {
            Ok(()) => {
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                state.file_tree.select_by_path(to, 20);
                for &id in &state.tabs {
                    if let Some(buf) = state.registry.get_mut(id) {
                        if buf.file_path.as_deref() == Some(from) {
                            buf.file_path = Some(to.to_path_buf());
                        }
                    }
                }
                state
                    .overlay
                    .notify("Renamed successfully", NotificationLevel::Info);
            }
            Err(e) => {
                state
                    .overlay
                    .notify(format!("Rename failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

fn handle_delete(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::Delete(path.to_path_buf())) {
            Ok(()) => {
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                let to_close: Vec<_> = state
                    .tabs
                    .iter()
                    .copied()
                    .filter(|&id| {
                        state
                            .registry
                            .get(id)
                            .is_some_and(|b| b.file_path.as_deref() == Some(path))
                    })
                    .collect();
                for id in to_close {
                    close_tab_by_id(state, id);
                }
                state.overlay.notify("Deleted", NotificationLevel::Info);
            }
            Err(e) => {
                state
                    .overlay
                    .notify(format!("Delete failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

pub(super) fn handle_open_file(path: &std::path::Path, state: &mut AppState) -> Control<AppEvent> {
    match state.open_file(path) {
        Ok(_) => {
            state.set_root_tab(RootTab::Editor);
            state.focus.focus(PanelId::Editor);
            state.viewport_follow_cursor = true;
            state.status_message = format!("Opened: {}", path.display());
        }
        Err(e) => {
            state
                .overlay
                .notify(format!("Open failed: {e}"), NotificationLevel::Error);
            state.status_message = format!("Open failed: {e}");
        }
    }
    Control::Changed
}

fn handle_open_config_file(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    let Some(ref cp) = state.config_paths else {
        state
            .overlay
            .notify("Config directory not available", NotificationLevel::Error);
        return Control::Changed;
    };

    let (path, default_content) = match cmd {
        AppCommand::OpenSettings => (
            cp.settings_file(),
            toml::to_string_pretty(&Settings::default()).unwrap_or_default(),
        ),
        AppCommand::OpenKeybindings => (
            cp.keybindings_file(),
            "# Keybinding overrides\n\
             # Format: \"key_combo\" = \"command\"\n\
             #\n\
             # [normal]\n\
             # \"ctrl+s\" = \"save\"\n\
             # \"ctrl+shift+p\" = \"command_palette\"\n\
             #\n\
             # [vim.normal]\n\
             # \"g d\" = \"go_to_definition\"\n"
                .to_owned(),
        ),
        _ => return Control::Continue,
    };

    if let Err(e) = cp.ensure_dirs() {
        state.overlay.notify(
            format!("Failed to create config dirs: {e}"),
            NotificationLevel::Error,
        );
        return Control::Changed;
    }

    if !path.exists() {
        if let Err(e) = std::fs::write(&path, &default_content) {
            state.overlay.notify(
                format!("Failed to create {}: {e}", path.display()),
                NotificationLevel::Error,
            );
            return Control::Changed;
        }
    }

    handle_open_file(&path, state)
}

fn handle_change_language(lang_id: LanguageId, state: &mut AppState) -> Control<AppEvent> {
    let Some(id) = state.active_buffer else {
        return Control::Continue;
    };

    let mut hl = highlight::create_highlighter(lang_id);
    if let Some(buf) = state.registry.get(id) {
        hl.update(buf, None);
    }
    state.highlighters.insert(id, hl);
    state.status_message = format!("Language: {}", lang_id.name());
    Control::Changed
}
