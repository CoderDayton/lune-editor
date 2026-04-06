#![allow(clippy::wildcard_imports)]

use super::*;
use super::editor_actions::{
    apply_vim_action, apply_vim_action_visual, handle_copy, handle_cut,
    handle_delete_word_left, handle_delete_word_right, handle_duplicate_line,
    handle_move_line_down, handle_move_line_up, handle_paste, handle_select_all,
    handle_shift_tab, handle_tab_or_indent,
};

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
            KeyCode::Up => handle_move_line_up(state),
            KeyCode::Down => handle_move_line_down(state),
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
                let sel = buf.cursor.primary.clone();
                if !sel.is_cursor() {
                    let (s, e) = sel.ordered();
                    buf.delete(s, e);
                }
                let pos = buf.cursor.primary.head;
                buf.insert(pos, &ch.to_string());
            }
            Control::Changed
        }
        KeyCode::Enter => {
            if let Some(buf) = state.active_buf_mut() {
                let sel = buf.cursor.primary.clone();
                if !sel.is_cursor() {
                    let (s, e) = sel.ordered();
                    buf.delete(s, e);
                }
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "\n");
            }
            Control::Changed
        }
        KeyCode::Backspace => {
            if let Some(buf) = state.active_buf_mut() {
                let sel = buf.cursor.primary.clone();
                if sel.is_cursor() {
                    let pos = buf.cursor.primary.head;
                    if pos.col > 0 {
                        buf.delete(Position::new(pos.line, pos.col - 1), pos);
                    } else if pos.line > 0 {
                        let prev_len = buf.line_len(pos.line - 1).saturating_sub(1);
                        buf.delete(Position::new(pos.line - 1, prev_len), pos);
                    }
                } else {
                    let (s, e) = sel.ordered();
                    buf.delete(s, e);
                }
            }
            Control::Changed
        }
        KeyCode::Delete => {
            if let Some(buf) = state.active_buf_mut() {
                let sel = buf.cursor.primary.clone();
                if sel.is_cursor() {
                    let pos = buf.cursor.primary.head;
                    buf.delete(pos, Position::new(pos.line, pos.col + 1));
                } else {
                    let (s, e) = sel.ordered();
                    buf.delete(s, e);
                }
            }
            Control::Changed
        }
        KeyCode::Tab => handle_tab_or_indent(state),
        KeyCode::BackTab => handle_shift_tab(state),
        KeyCode::Home => apply_motion(state, |buf| buf.move_line_start(extend)),
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
    if let KeyCode::Char(ch) = key.code {
        if !state.vim_enabled {
            return match ch {
                'i' => {
                    state.vim.enter_insert();
                    Control::Changed
                }
                'h' => apply_motion(state, |buf| buf.move_left(false)),
                'j' => apply_motion(state, |buf| buf.move_down(false)),
                'k' => apply_motion(state, |buf| buf.move_up(false)),
                'l' => apply_motion(state, |buf| buf.move_right(false)),
                _ => Control::Continue,
            };
        }
        let dummy = TextBuffer::new();
        let buf = state
            .active_buffer
            .and_then(|id| state.registry.get(id))
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
            .active_buffer
            .and_then(|id| state.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action_visual(&action, state)
    } else {
        Control::Continue
    }
}

pub(super) fn handle_vim_command_key(
    key: &KeyEvent,
    state: &mut AppState,
) -> Control<AppEvent> {
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
        "q" | "quit" | "q!" => Control::Event(AppEvent::Command(AppCommand::CloseTab)),
        "qa" | "qall" | "qa!" => Control::Event(AppEvent::Command(AppCommand::Quit)),
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
        (KeyCode::Up, _) => Some(TextBuffer::move_up),
        (KeyCode::Down, _) => Some(TextBuffer::move_down),
        _ => None,
    };
    method.map_or(Control::Continue, |m| apply_motion(state, |buf| m(buf, extend)))
}
