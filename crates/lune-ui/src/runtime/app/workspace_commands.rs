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
        AppCommand::OpenFileAtLine { path, line, col } => {
            handle_open_file_at_line(path, *line, *col, state)
        }
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

/// Open `path` and move the cursor to `line`/`col` (zero-based, each
/// clamped to the file). Used by project search to jump straight to a
/// match. The cursor is repositioned only when `path` actually opened as
/// the active text buffer — a failed open (e.g. the file vanished between
/// the search and Enter) or an image preview must not disturb whatever
/// buffer is already active.
fn handle_open_file_at_line(
    path: &std::path::Path,
    line: usize,
    col: usize,
    state: &mut AppState,
) -> Control<AppEvent> {
    let (control, opened) = open_file_routed(path, state);
    if opened {
        if let Some(buf) = state.active_buf_mut() {
            let line = line.min(buf.line_count().saturating_sub(1));
            let col = col.min(buf.line_len_no_newline(line));
            buf.cursor = CursorState::at(Position::new(line, col));
        }
        state.viewport_follow_cursor = true;
    }
    control
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
    let ids: Vec<_> = state.session.tabs.clone();
    let mut saved = 0;
    let mut errors = 0;
    for id in ids {
        if let Some(buf) = state.session.registry.get_mut(id)
            && buf.is_dirty()
        {
            match buf.save() {
                Ok(()) => saved += 1,
                Err(_) => errors += 1,
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
            }
            state.refresh_file_tree();
            Control::Changed
        }
        AppCommand::RevealInFileTree(path) => {
            let path = path.clone();
            if let Some(ref mut ws) = state.workspace
                && let Err(e) = state.file_tree.reveal_path(&path, ws)
            {
                log::error!("Failed to reveal path: {e}");
            }
            state.refresh_file_tree();
            state.file_tree.select_by_path(&path, 20);
            Control::Changed
        }
        AppCommand::NewFile => {
            if state.root_tab != RootTab::Editor {
                return Control::Continue;
            }
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
    let mut refreshed = false;
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::CreateFile(
            path.to_path_buf(),
        )) {
            Ok(()) => {
                refreshed = true;
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
    if refreshed {
        state.refresh_file_tree();
    }
    handle_open_file(path, state)
}

fn handle_create_dir(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    let mut refreshed = false;
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::CreateDir(path.to_path_buf())) {
            Ok(()) => {
                refreshed = true;
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
    if refreshed {
        state.refresh_file_tree();
        state.file_tree.select_by_path(path, 20);
    }
    Control::Changed
}

fn handle_rename(from: &Path, to: &Path, state: &mut AppState) -> Control<AppEvent> {
    let mut refreshed = false;
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        }) {
            Ok(()) => {
                refreshed = true;
                for &id in &state.session.tabs {
                    if let Some(buf) = state.session.registry.get_mut(id)
                        && buf.file_path.as_deref() == Some(from)
                    {
                        buf.file_path = Some(to.to_path_buf());
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
    if refreshed {
        state.refresh_file_tree();
        state.file_tree.select_by_path(to, 20);
    }
    Control::Changed
}

fn handle_delete(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    let mut refreshed = false;
    if let Some(ref mut ws) = state.workspace {
        match ws.execute(&lune_core::workspace::FileOp::Delete(path.to_path_buf())) {
            Ok(()) => {
                refreshed = true;
                let to_close: Vec<_> = state
                    .session
                    .tabs
                    .iter()
                    .copied()
                    .filter(|&id| {
                        state
                            .session
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
    if refreshed {
        state.refresh_file_tree();
    }
    Control::Changed
}

pub(super) fn handle_open_file(path: &std::path::Path, state: &mut AppState) -> Control<AppEvent> {
    open_file_routed(path, state).0
}

/// Open `path`, routing image files to the preview overlay and text files
/// to a new editor buffer. Returns the redraw control plus `true` when a
/// text buffer for `path` became the active buffer — a clean success
/// signal for jump-to-line, which must not move the cursor on a failed
/// open or an image preview.
fn open_file_routed(path: &std::path::Path, state: &mut AppState) -> (Control<AppEvent>, bool) {
    // Image files: route to the image preview overlay instead of opening
    // as a text buffer (which would garble binary data into the editor).
    if is_image_path(path) {
        state.overlay.open_image_preview(path, &state.image_decoder);
        state.focus.focus(PanelId::CommandPalette);
        state.status_message = format!("Previewing: {}", state.status_path_display(path));
        return (Control::Changed, false);
    }
    match state.open_file(path) {
        Ok(_) => {
            state.set_root_tab(RootTab::Editor);
            state.focus.focus(PanelId::Editor);
            state.viewport_follow_cursor = true;
            state.status_message.clear();
            (Control::Changed, true)
        }
        Err(e) => {
            state
                .overlay
                .notify(format!("Open failed: {e}"), NotificationLevel::Error);
            state.status_message = format!("Open failed: {e}");
            (Control::Changed, false)
        }
    }
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
            Settings::default().to_pretty_toml().unwrap_or_default(),
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

    if !path.exists()
        && let Err(e) = std::fs::write(&path, &default_content)
    {
        state.overlay.notify(
            format!("Failed to create {}: {e}", path.display()),
            NotificationLevel::Error,
        );
        return Control::Changed;
    }

    handle_open_file(&path, state)
}

fn handle_change_language(lang_id: LanguageId, state: &mut AppState) -> Control<AppEvent> {
    let Some(id) = state.session.active_buffer else {
        return Control::Continue;
    };

    let mut hl = highlight::create_highlighter(lang_id);
    if let Some(buf) = state.session.registry.get_mut(id) {
        // Discard any edit deltas captured against the previous highlighter
        // so they aren't replayed onto this fresh one, which reparses in full.
        let _ = buf.take_pending_edits();
        hl.update(buf, &[]);
    }
    state.highlighters.insert(id, hl);
    state.status_message = format!("Language: {}", lang_id.name());
    Control::Changed
}

/// Returns true if `path` looks like an image file ratatui-image can decode.
fn is_image_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_file_is_ignored_outside_editor_root_tab() {
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Agents);

        let result = handle_workspace_command(&AppCommand::NewFile, &mut state);

        assert_eq!(result, Some(Control::Continue));
        assert!(state.overlay.input_dialog.is_none());
    }

    #[test]
    fn is_image_path_matches_common_extensions() {
        for ext in ["png", "jpg", "jpeg", "gif", "bmp", "webp"] {
            let p = std::path::PathBuf::from(format!("photo.{ext}"));
            assert!(is_image_path(&p), "expected image: {ext}");
        }
    }

    #[test]
    fn is_image_path_is_case_insensitive() {
        assert!(is_image_path(std::path::Path::new("IMAGE.JPG")));
        assert!(is_image_path(std::path::Path::new("Photo.PNG")));
        assert!(is_image_path(std::path::Path::new("Icon.GiF")));
    }

    #[test]
    fn is_image_path_rejects_non_image_extensions() {
        for name in ["main.rs", "README.md", "Cargo.toml", "noext", "data.json"] {
            assert!(
                !is_image_path(std::path::Path::new(name)),
                "expected non-image: {name}"
            );
        }
    }

    #[test]
    fn open_file_at_line_places_and_clamps_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.rs");
        std::fs::write(&path, "fn main() {}\nlet value = 1;\nlast\n").unwrap();

        let mut state = AppState::new();
        handle_workspace_command(
            &AppCommand::OpenFileAtLine {
                path: path.clone(),
                line: 1,
                col: 4,
            },
            &mut state,
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(1, 4)
        );

        // A column past the line end is clamped to the line's length.
        handle_workspace_command(
            &AppCommand::OpenFileAtLine {
                path,
                line: 2,
                col: 999,
            },
            &mut state,
        );
        let head = state.active_buf().unwrap().cursor.primary.head;
        assert_eq!(head, Position::new(2, "last".chars().count()));
    }

    #[test]
    fn open_file_at_line_failed_open_leaves_active_cursor_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.rs");
        std::fs::write(&real, "alpha\nbeta\ngamma\n").unwrap();

        let mut state = AppState::new();
        handle_workspace_command(
            &AppCommand::OpenFileAtLine {
                path: real,
                line: 0,
                col: 2,
            },
            &mut state,
        );
        let before = state.active_buf().unwrap().cursor.primary.head;
        assert_eq!(before, Position::new(0, 2));

        // Jumping to a hit whose file vanished must not move the active
        // buffer's cursor (regression: it used to jump anyway).
        handle_workspace_command(
            &AppCommand::OpenFileAtLine {
                path: dir.path().join("vanished.rs"),
                line: 7,
                col: 3,
            },
            &mut state,
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            before,
            "cursor must stay put on a failed open"
        );
    }
}
