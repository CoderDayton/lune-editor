#![allow(clippy::wildcard_imports)]

use super::editor_actions::{
    apply_vim_action, apply_vim_action_visual, handle_add_cursor_above, handle_add_cursor_below,
    handle_clear_secondary_cursors, handle_copy, handle_cut, handle_cut_line,
    handle_delete_word_left, handle_delete_word_right, handle_duplicate_line,
    handle_move_line_down, handle_move_line_up, handle_paste, handle_select_all, handle_shift_tab,
    handle_tab_or_indent,
};
use super::*;

#[allow(clippy::too_many_lines)]
pub(super) fn handle_insert_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('a') => handle_select_all(state),
            KeyCode::Char('c') => handle_copy(state),
            KeyCode::Char('x') => handle_cut(state),
            KeyCode::Char('v') => handle_paste(state),
            KeyCode::Char('d') => handle_duplicate_line(state),
            KeyCode::Char('k') => handle_cut_line(state),
            KeyCode::Backspace => handle_delete_word_left(state),
            KeyCode::Delete => handle_delete_word_right(state),
            KeyCode::Home => apply_motion(state, |buf| buf.move_buffer_start(extend)),
            KeyCode::End => apply_motion(state, |buf| buf.move_buffer_end(extend)),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                apply_arrow_motion(key, state, extend)
            }
            _ => Control::Continue,
        };
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Up if extend => handle_add_cursor_above(state),
            KeyCode::Down if extend => handle_add_cursor_below(state),
            KeyCode::Left => apply_motion(state, |buf| buf.move_line_start(extend)),
            KeyCode::Right => apply_motion(state, |buf| buf.move_line_end(extend)),
            KeyCode::Up => handle_move_line_up(state),
            KeyCode::Down => handle_move_line_down(state),
            KeyCode::Backspace => handle_delete_word_left(state),
            KeyCode::Char('c' | 'C') => handle_clear_secondary_cursors(state),
            _ => Control::Continue,
        };
    }

    let mutates_text = matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Tab
            | KeyCode::BackTab
    );

    let result = match key.code {
        KeyCode::Char(ch) => {
            if let Some(buf) = state.active_buf_mut() {
                let _ = buf.insert_at_cursor_set(&ch.to_string());
            }
            Control::Changed
        }
        KeyCode::Enter => {
            if let Some(buf) = state.active_buf_mut() {
                let _ = buf.insert_at_cursor_set("\n");
            }
            Control::Changed
        }
        KeyCode::Backspace => {
            if let Some(buf) = state.active_buf_mut() {
                let _ = buf.backspace_cursor_set();
            }
            Control::Changed
        }
        KeyCode::Delete => {
            if let Some(buf) = state.active_buf_mut() {
                let _ = buf.delete_cursor_set();
            }
            Control::Changed
        }
        KeyCode::Tab => handle_tab_or_indent(state),
        KeyCode::BackTab => handle_shift_tab(state),
        KeyCode::Home => apply_motion(state, |buf| buf.move_line_home(extend)),
        KeyCode::End => apply_motion(state, |buf| buf.move_line_end(extend)),
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
            apply_arrow_motion(key, state, extend)
        }
        _ => Control::Continue,
    };

    if mutates_text {
        state.update_active_highlighter();
        state.viewport_follow_cursor = true;
    }
    result
}

pub(super) fn handle_normal_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    // Only reached when vim keybindings are enabled — the dispatcher routes
    // non-vim input straight to Insert handling.
    if let KeyCode::Char(ch) = key.code {
        let dummy = TextBuffer::new();
        let buf = state
            .session
            .active_buffer
            .and_then(|id| state.session.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action(&action, state)
    } else {
        apply_arrow_motion(key, state, false)
    }
}

pub(super) fn handle_visual_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if let KeyCode::Char(ch) = key.code {
        let dummy = TextBuffer::new();
        let buf = state
            .session
            .active_buffer
            .and_then(|id| state.session.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action_visual(&action, state)
    } else {
        Control::Continue
    }
}

pub(super) fn handle_vim_command_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            state.vim.cmdline_clear();
            state.vim.enter_normal();
            Control::Changed
        }
        KeyCode::Enter => {
            let cmd = state.vim.cmdline.clone();
            state.vim.cmdline_clear();
            state.vim.enter_normal();
            execute_vim_command(&cmd, state)
        }
        KeyCode::Backspace => {
            if state.vim.cmdline.is_empty() {
                state.vim.enter_normal();
            } else {
                state.vim.cmdline_pop();
            }
            Control::Changed
        }
        KeyCode::Char(ch) => {
            state.vim.cmdline_push(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn execute_vim_command(cmd: &str, state: &mut AppState) -> Control<AppEvent> {
    let trimmed = cmd.trim();
    match trimmed {
        "w" | "write" => Control::Event(AppEvent::Command(AppCommand::Save)),
        "wa" | "wall" => Control::Event(AppEvent::Command(AppCommand::SaveAll)),
        "q" | "quit" => Control::Event(AppEvent::Command(AppCommand::CloseTab)),
        "q!" => Control::Event(AppEvent::Command(AppCommand::ForceCloseTab)),
        "qa" | "qall" => Control::Event(AppEvent::Command(AppCommand::Quit)),
        "qa!" => Control::Event(AppEvent::Command(AppCommand::ForceQuit)),
        "wq" | "x" => {
            let _ = handle_save(state);
            Control::Event(AppEvent::Command(AppCommand::CloseTab))
        }
        "wqa" | "xall" => {
            let _ = handle_save_all(state);
            Control::Event(AppEvent::Command(AppCommand::Quit))
        }
        _ if trimmed.starts_with("e ") || trimmed.starts_with("edit ") => {
            let path_str = trimmed.split_once(' ').map_or("", |x| x.1).trim();
            if path_str.is_empty() {
                state
                    .overlay
                    .notify("Usage: :e <path>", NotificationLevel::Warning);
                Control::Changed
            } else {
                Control::Event(AppEvent::Command(AppCommand::OpenFile(PathBuf::from(
                    path_str,
                ))))
            }
        }
        _ => {
            if !trimmed.is_empty() {
                state.overlay.notify(
                    format!("Unknown command: :{trimmed}"),
                    NotificationLevel::Warning,
                );
            }
            Control::Changed
        }
    }
}

fn apply_arrow_motion(key: &KeyEvent, state: &mut AppState, extend: bool) -> Control<AppEvent> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let method: Option<fn(&mut TextBuffer, bool)> = match (key.code, ctrl) {
        (KeyCode::Left, false) => Some(TextBuffer::move_left),
        (KeyCode::Left, true) => Some(TextBuffer::move_word_left),
        (KeyCode::Right, false) => Some(TextBuffer::move_right),
        (KeyCode::Right, true) => Some(TextBuffer::move_word_right),
        (KeyCode::Up, false) => Some(TextBuffer::move_up),
        (KeyCode::Down, false) => Some(TextBuffer::move_down),
        (KeyCode::Up, true) => Some(TextBuffer::move_buffer_start),
        (KeyCode::Down, true) => Some(TextBuffer::move_buffer_end),
        _ => None,
    };
    method.map_or(Control::Continue, |m| {
        apply_motion(state, |buf| m(buf, extend))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_text(text: &str) -> AppState {
        let mut state = AppState::new();
        let id = state.session.registry.new_scratch();
        let buf = state.session.registry.get_mut(id).unwrap();
        buf.insert(Position::new(0, 0), text);
        buf.cursor = CursorState::at(Position::new(0, 0));
        state.session.active_buffer = Some(id);
        state.session.tabs.push(id);
        state.vim.enter_insert();
        state
    }

    #[test]
    fn home_uses_smart_indent_behavior_in_insert_mode() {
        let mut state = state_with_text("    hello");
        state.active_buf_mut().unwrap().cursor = CursorState::at(Position::new(0, 8));

        let first = handle_insert_mode(
            &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &mut state,
        );
        assert!(matches!(first, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 4)
        );

        let second = handle_insert_mode(
            &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &mut state,
        );
        assert!(matches!(second, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 0)
        );
    }

    #[test]
    fn alt_left_and_right_move_to_line_edges() {
        let mut state = state_with_text("    hello");
        state.active_buf_mut().unwrap().cursor = CursorState::at(Position::new(0, 6));

        let left = handle_insert_mode(&KeyEvent::new(KeyCode::Left, KeyModifiers::ALT), &mut state);
        assert!(matches!(left, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 0)
        );

        let right = handle_insert_mode(
            &KeyEvent::new(KeyCode::Right, KeyModifiers::ALT),
            &mut state,
        );
        assert!(matches!(right, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 9)
        );
    }

    #[test]
    fn ctrl_up_and_down_jump_to_buffer_edges() {
        let mut state = state_with_text("one\ntwo\nthree");
        state.active_buf_mut().unwrap().cursor = CursorState::at(Position::new(1, 1));

        let up = handle_insert_mode(
            &KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL),
            &mut state,
        );
        assert!(matches!(up, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 0)
        );

        let down = handle_insert_mode(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL),
            &mut state,
        );
        assert!(matches!(down, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(2, 5)
        );
    }

    #[test]
    fn alt_shift_up_and_down_spawn_secondary_cursors() {
        let mut state = state_with_text("one\ntwo\nthree");
        state.active_buf_mut().unwrap().cursor = CursorState::at(Position::new(1, 1));

        let up = handle_insert_mode(
            &KeyEvent::new(KeyCode::Up, KeyModifiers::ALT | KeyModifiers::SHIFT),
            &mut state,
        );
        assert!(matches!(up, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.secondary,
            vec![Selection::cursor(Position::new(0, 1))]
        );

        let down = handle_insert_mode(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT),
            &mut state,
        );
        assert!(matches!(down, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.secondary,
            vec![
                Selection::cursor(Position::new(0, 1)),
                Selection::cursor(Position::new(2, 1)),
            ]
        );
    }

    #[test]
    fn typing_in_insert_mode_applies_to_secondary_cursors() {
        let mut state = state_with_text("one\ntwo");
        let buf = state.active_buf_mut().unwrap();
        buf.cursor = CursorState::at(Position::new(0, 1));
        assert!(buf.toggle_secondary_cursor(Position::new(1, 1)));

        let result = handle_insert_mode(
            &KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.active_buf().unwrap().text(), "o!ne\nt!wo");
    }
}
