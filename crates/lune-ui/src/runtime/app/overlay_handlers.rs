#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_overlay_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match &state.overlay.active {
        Some(overlay::OverlayKind::CommandPalette) => handle_palette_key(key, state),
        Some(overlay::OverlayKind::ConfirmDialog { on_confirm, .. }) => {
            let cmd = on_confirm.clone();
            match key.code {
                KeyCode::Enter => {
                    close_overlay(state);
                    Control::Event(AppEvent::Command(cmd))
                }
                KeyCode::Esc => {
                    close_overlay(state);
                    Control::Changed
                }
                _ => Control::Continue,
            }
        }
        Some(overlay::OverlayKind::FindReplace) => handle_find_replace_key(key, state),
        Some(overlay::OverlayKind::FilePicker) => handle_file_picker_key(key, state),
        Some(overlay::OverlayKind::AiClientPicker) => handle_ai_client_picker_key(key, state),
        Some(overlay::OverlayKind::InputDialog) => handle_input_dialog_key(key, state),
        Some(overlay::OverlayKind::LanguagePicker) => handle_language_picker_key(key, state),
        Some(overlay::OverlayKind::ThemePicker) => handle_theme_picker_key(key, state),
        Some(overlay::OverlayKind::LayoutPicker) => handle_layout_picker_key(key, state),
        None => Control::Continue,
    }
}

fn handle_palette_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => state
            .overlay
            .command_palette
            .selected_command()
            .cloned()
            .map_or(Control::Changed, |cmd| {
                close_overlay(state);
                Control::Event(AppEvent::Command(cmd))
            }),
        KeyCode::Up => {
            state.overlay.command_palette.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.command_palette.select_next();
            Control::Changed
        }
        KeyCode::Backspace => {
            state.overlay.command_palette.backspace();
            Control::Changed
        }
        KeyCode::Char(ch) => {
            state.overlay.command_palette.type_char(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn handle_file_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => handle_file_picker_enter(state),
        KeyCode::Up => {
            state.overlay.file_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.file_picker.select_next();
            Control::Changed
        }
        KeyCode::Backspace => {
            if !state.overlay.file_picker.backspace() {
                state.overlay.file_picker.go_up();
            }
            Control::Changed
        }
        KeyCode::Char(ch) => {
            state.overlay.file_picker.type_char(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn handle_file_picker_enter(state: &mut AppState) -> Control<AppEvent> {
    let Some(entry) = state.overlay.file_picker.selected_entry().cloned() else {
        return Control::Continue;
    };

    if entry.is_dir {
        state.overlay.file_picker.enter_directory(&entry.path);
        Control::Changed
    } else {
        let path = entry.path;
        close_overlay(state);
        Control::Event(AppEvent::Command(AppCommand::OpenFile(path)))
    }
}

fn handle_language_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            let lang = state.overlay.language_picker.selected_lang();
            lang.map_or(Control::Continue, |l| {
                close_overlay(state);
                Control::Event(AppEvent::Command(AppCommand::ChangeLanguage(l)))
            })
        }
        KeyCode::Up => {
            state.overlay.language_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.language_picker.select_next();
            Control::Changed
        }
        KeyCode::Backspace => {
            state.overlay.language_picker.backspace();
            Control::Changed
        }
        KeyCode::Char(c) => {
            state.overlay.language_picker.type_char(c);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn handle_theme_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            let original = state.overlay.theme_picker.original_idx;
            state.theme_registry.switch(ThemeId(original));
            state.apply_active_theme();
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Up => {
            state.overlay.theme_picker.select_prev();
            apply_theme_preview(state);
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.theme_picker.select_next();
            apply_theme_preview(state);
            Control::Changed
        }
        KeyCode::Backspace => {
            state.overlay.theme_picker.backspace();
            apply_theme_preview(state);
            Control::Changed
        }
        KeyCode::Char(c) => {
            state.overlay.theme_picker.type_char(c);
            apply_theme_preview(state);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn apply_theme_preview(state: &mut AppState) {
    if let Some(idx) = state.overlay.theme_picker.selected_idx() {
        state.theme_registry.switch(ThemeId(idx));
        state.apply_active_theme();
    }
}

fn handle_input_dialog_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    use overlay::InputDialogAction;

    match key.code {
        KeyCode::Esc => {
            state.overlay.input_dialog = None;
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            let Some(ref dialog) = state.overlay.input_dialog else {
                return Control::Continue;
            };
            if dialog.validate().is_some() {
                return Control::Continue;
            }
            let input = dialog.input.trim().to_owned();
            let action = dialog.action.clone();
            state.overlay.input_dialog = None;
            close_overlay(state);
            let cmd = match action {
                InputDialogAction::CreateFile { parent } => {
                    AppCommand::CreateFileConfirmed(parent.join(&input))
                }
                InputDialogAction::CreateDir { parent } => {
                    AppCommand::CreateDirConfirmed(parent.join(&input))
                }
                InputDialogAction::Rename { from } => {
                    let to = from
                        .parent()
                        .map_or_else(|| PathBuf::from(&input), |p| p.join(&input));
                    AppCommand::RenameConfirmed { from, to }
                }
                InputDialogAction::CommitMessage => AppCommand::GitCommitConfirmed(input),
                InputDialogAction::SaveAgentLayout => AppCommand::AgentSaveLayoutConfirmed(input),
                InputDialogAction::RenameAgentLayout { index } => {
                    AppCommand::AgentRenameSavedLayoutConfirmed { index, name: input }
                }
            };
            Control::Event(AppEvent::Command(cmd))
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.type_char(ch);
            }
            Control::Changed
        }
        KeyCode::Backspace => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.backspace();
            }
            Control::Changed
        }
        KeyCode::Delete => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.delete();
            }
            Control::Changed
        }
        KeyCode::Left => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.move_left();
            }
            Control::Changed
        }
        KeyCode::Right => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.move_right();
            }
            Control::Changed
        }
        KeyCode::Home => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.home();
            }
            Control::Changed
        }
        KeyCode::End => {
            if let Some(ref mut dialog) = state.overlay.input_dialog {
                dialog.end();
            }
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn handle_find_replace_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    use overlay::FindReplaceField;

    match key.code {
        KeyCode::Esc => {
            state.overlay.find_replace.search_state = SearchState::default();
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => find_next_match(state, !state.overlay.find_replace.show_replace),
        KeyCode::Tab | KeyCode::BackTab => {
            state.overlay.find_replace.toggle_field();
            Control::Changed
        }
        KeyCode::Char('n')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            find_next_match(state, false)
        }
        KeyCode::Char('p')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            find_prev_match(state)
        }
        KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::ALT) => {
            state.overlay.find_replace.toggle_case();
            update_find_search(state);
            Control::Changed
        }
        KeyCode::Char('r')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && state.overlay.find_replace.show_replace =>
        {
            let search = state.overlay.find_replace.search_state.clone();
            let replacement = state.overlay.find_replace.replace_input.clone();
            let new_state = state
                .active_buf_mut()
                .map(|buf| buf.replace_current(&search, &replacement));
            if let Some(new_state) = new_state {
                state.overlay.find_replace.search_state = new_state;
                state.update_active_highlighter();
            }
            Control::Changed
        }
        KeyCode::Char('l')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && state.overlay.find_replace.show_replace =>
        {
            let search = state.overlay.find_replace.search_state.clone();
            let replacement = state.overlay.find_replace.replace_input.clone();
            let count = search.match_count();
            let new_state = state
                .active_buf_mut()
                .map(|buf| buf.replace_all(&search, &replacement));
            if let Some(new_state) = new_state {
                state.overlay.find_replace.search_state = new_state;
                state.update_active_highlighter();
                state.overlay.notify(
                    format!("Replaced {count} occurrences"),
                    NotificationLevel::Info,
                );
            }
            Control::Changed
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            state.overlay.find_replace.type_char(ch);
            if state.overlay.find_replace.active_field == FindReplaceField::Find {
                update_find_search(state);
            }
            Control::Changed
        }
        KeyCode::Backspace => {
            state.overlay.find_replace.backspace();
            if state.overlay.find_replace.active_field == FindReplaceField::Find {
                update_find_search(state);
            }
            Control::Changed
        }
        _ => Control::Continue,
    }
}

pub(super) fn update_find_search(state: &mut AppState) {
    let query = state.overlay.find_replace.find_input.clone();
    let case_sensitive = state.overlay.find_replace.case_sensitive;
    let new_search = state
        .active_buf()
        .map(|buf| buf.search(&query, case_sensitive));
    if let Some(search) = new_search {
        state.overlay.find_replace.search_state = search;
    }
    navigate_to_current_match(state);
}

pub(super) fn find_next_match(
    state: &mut AppState,
    close_after_navigate: bool,
) -> Control<AppEvent> {
    advance_find_match(state, TextBuffer::search_next, close_after_navigate)
}

pub(super) fn find_prev_match(state: &mut AppState) -> Control<AppEvent> {
    advance_find_match(state, TextBuffer::search_prev, false)
}

fn navigate_to_current_match(state: &mut AppState) {
    if let Some(idx) = state.overlay.find_replace.search_state.current_match {
        if let Some(&(start, _end)) = state.overlay.find_replace.search_state.matches.get(idx) {
            if let Some(buf) = state.active_buf_mut() {
                buf.cursor.primary = Selection::cursor(start);
            }
            state.viewport_follow_cursor = true;
        }
    }
}

fn advance_find_match(
    state: &mut AppState,
    step: fn(&SearchState) -> Option<usize>,
    close_after_navigate: bool,
) -> Control<AppEvent> {
    let Some(idx) = step(&state.overlay.find_replace.search_state) else {
        return Control::Continue;
    };
    state.overlay.find_replace.search_state.current_match = Some(idx);
    navigate_to_current_match(state);
    if close_after_navigate {
        close_overlay(state);
    }
    Control::Changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_text(text: &str) -> AppState {
        let mut state = AppState::new();
        let id = state.registry.new_scratch();
        let buf = state.registry.get_mut(id).unwrap();
        buf.insert(Position::new(0, 0), text);
        buf.cursor = CursorState::at(Position::new(0, 0));
        state.active_buffer = Some(id);
        state.tabs.push(id);
        state
    }

    #[test]
    fn enter_in_find_overlay_closes_and_advances() {
        let mut state = state_with_text("foo bar foo");
        state.overlay.open_find();
        state.overlay.find_replace.search_state = state.active_buf().unwrap().search("foo", true);

        let result = handle_find_replace_key(
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert!(!state.overlay.is_active());
        assert_eq!(
            state.overlay.find_replace.search_state.current_match,
            Some(1)
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 8)
        );
    }

    #[test]
    fn ctrl_p_in_find_overlay_moves_to_previous_match() {
        let mut state = state_with_text("foo bar foo");
        state.overlay.open_find();
        let mut search = state.active_buf().unwrap().search("foo", true);
        search.current_match = Some(1);
        state.overlay.find_replace.search_state = search;

        let result = handle_find_replace_key(
            &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert_eq!(
            state.overlay.find_replace.search_state.current_match,
            Some(0)
        );
        assert!(state.overlay.is_active());
    }
}
