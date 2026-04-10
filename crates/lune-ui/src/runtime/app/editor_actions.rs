#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn apply_vim_action(action: &VimAction, state: &mut AppState) -> Control<AppEvent> {
    match action {
        VimAction::ModeChanged(mode) => {
            if !state.vim_enabled && !matches!(mode, VimMode::Normal | VimMode::Insert) {
                state.vim.enter_normal();
                return Control::Continue;
            }
            Control::Changed
        }
        VimAction::MoveLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_left);
        }),
        VimAction::MoveRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_right);
        }),
        VimAction::MoveUp(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_up);
        }),
        VimAction::MoveDown(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_down);
        }),
        VimAction::MoveWordRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_word_right);
        }),
        VimAction::MoveWordLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_word_left);
        }),
        VimAction::MoveLineStart => apply_motion(state, |buf| buf.move_line_start(false)),
        VimAction::MoveLineEnd => apply_motion(state, |buf| buf.move_line_end(false)),
        VimAction::MoveBufferEnd => apply_motion(state, |buf| buf.move_buffer_end(false)),
        VimAction::MoveToLine(line) => {
            if let Some(buf) = state.active_buf_mut() {
                let clamped = (*line).min(buf.line_count().saturating_sub(1));
                buf.cursor = CursorState::at(Position::new(clamped, 0));
            }
            state.viewport_follow_cursor = true;
            Control::Changed
        }
        VimAction::OpenLineBelow => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_line_end(false);
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "\n");
            }
            state.viewport_follow_cursor = true;
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::OpenLineAbove => {
            if let Some(buf) = state.active_buf_mut() {
                let line = buf.cursor.primary.head.line;
                let pos = Position::new(line, 0);
                buf.insert(pos, "\n");
                buf.cursor = CursorState::at(pos);
            }
            state.viewport_follow_cursor = true;
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::DeleteCharForward(n) => {
            if let Some(buf) = state.active_buf_mut() {
                for _ in 0..*n {
                    let pos = buf.cursor.primary.head;
                    let end = Position::new(pos.line, pos.col + 1);
                    buf.delete(pos, end);
                }
            }
            state.viewport_follow_cursor = true;
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::DeleteLine(n) => {
            if let Some(buf) = state.active_buf_mut() {
                for _ in 0..*n {
                    let line = buf.cursor.primary.head.line;
                    let start = Position::new(line, 0);
                    let end = if line + 1 < buf.line_count() {
                        Position::new(line + 1, 0)
                    } else {
                        let len = buf.line_len(line);
                        Position::new(line, len)
                    };
                    buf.delete(start, end);
                }
            }
            state.viewport_follow_cursor = true;
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::Undo => {
            apply_buf_edit(state, TextBuffer::undo);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

pub(super) fn apply_vim_action_visual(
    action: &VimAction,
    state: &mut AppState,
) -> Control<AppEvent> {
    match action {
        VimAction::MoveLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_left);
        }),
        VimAction::MoveRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_right);
        }),
        VimAction::MoveUp(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_up);
        }),
        VimAction::MoveDown(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_down);
        }),
        VimAction::ModeChanged(VimMode::Normal | VimMode::Insert) => Control::Changed,
        _ => Control::Continue,
    }
}

pub(super) fn handle_select_all(state: &mut AppState) -> Control<AppEvent> {
    apply_motion(state, |buf| {
        buf.move_buffer_start(false);
        buf.move_buffer_end(true);
    })
}

pub(super) fn handle_copy(state: &mut AppState) -> Control<AppEvent> {
    let text = state.active_buf().and_then(|buf| {
        let sel = &buf.cursor.primary;
        if sel.is_cursor() {
            return None;
        }
        let (s, e) = sel.ordered();
        Some(buf.text_range(s, e))
    });
    let Some(t) = text else {
        return Control::Continue;
    };
    if let Err(e) = Clipboard::new().and_then(|mut cb| cb.set_text(t)) {
        state
            .overlay
            .notify(format!("Clipboard error: {e}"), NotificationLevel::Error);
    }
    Control::Changed
}

pub(super) fn handle_cut(state: &mut AppState) -> Control<AppEvent> {
    let text = state.active_buf().and_then(|buf| {
        let sel = &buf.cursor.primary;
        if sel.is_cursor() {
            return None;
        }
        let (s, e) = sel.ordered();
        Some(buf.text_range(s, e))
    });
    let Some(t) = text else {
        return Control::Continue;
    };
    if let Err(e) = Clipboard::new().and_then(|mut cb| cb.set_text(t)) {
        state
            .overlay
            .notify(format!("Clipboard error: {e}"), NotificationLevel::Error);
    }
    if let Some(buf) = state.active_buf_mut() {
        let (s, e) = buf.cursor.primary.ordered();
        buf.delete(s, e);
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_paste(state: &mut AppState) -> Control<AppEvent> {
    let Some(text) = read_clipboard_text(state) else {
        return Control::Changed;
    };
    if let Some(buf) = state.active_buf_mut() {
        let _ = buf.insert_at_cursor_set(&text);
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_paste_at_position(
    state: &mut AppState,
    pos: Position,
) -> Control<AppEvent> {
    let Some(text) = read_clipboard_text(state) else {
        return Control::Changed;
    };
    if let Some(buf) = state.active_buf_mut() {
        let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
        let clamped_col = pos.col.min(buf.line_len_no_newline(clamped_line));
        let clamped = Position::new(clamped_line, clamped_col);
        buf.cursor = CursorState::at(clamped);
        buf.insert(clamped, &text);
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_cut_line(state: &mut AppState) -> Control<AppEvent> {
    let text = state.active_buf().map(|buf| {
        let sel = &buf.cursor.primary;
        if !sel.is_cursor() {
            let (s, e) = sel.ordered();
            return buf.text_range(s, e);
        }
        let (start, end) = current_line_range(buf);
        buf.text_range(start, end)
    });
    let Some(text) = text else {
        return Control::Continue;
    };

    if let Err(e) = Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
        state
            .overlay
            .notify(format!("Clipboard error: {e}"), NotificationLevel::Error);
    }

    if let Some(buf) = state.active_buf_mut() {
        let sel = buf.cursor.primary.clone();
        if sel.is_cursor() {
            let (start, end) = current_line_range(buf);
            buf.delete(start, end);
            buf.cursor = CursorState::at(start);
        } else {
            let (s, e) = sel.ordered();
            buf.delete(s, e);
        }
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_delete_word_left(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let sel = buf.cursor.primary.clone();
        if sel.is_cursor() {
            let head = buf.cursor.primary.head;
            buf.move_word_left(false);
            let new_head = buf.cursor.primary.head;
            if new_head != head {
                buf.delete(new_head, head);
            }
        } else {
            let (s, e) = sel.ordered();
            buf.delete(s, e);
        }
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_delete_word_right(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let sel = buf.cursor.primary.clone();
        if sel.is_cursor() {
            let head = buf.cursor.primary.head;
            buf.move_word_right(false);
            let new_head = buf.cursor.primary.head;
            if new_head != head {
                buf.delete(head, new_head);
            }
        } else {
            let (s, e) = sel.ordered();
            buf.delete(s, e);
        }
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_tab_or_indent(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let sel = buf.cursor.primary.clone();
        if sel.is_cursor() {
            let pos = buf.cursor.primary.head;
            buf.insert(pos, "    ");
        } else {
            let (start, end) = sel.ordered();
            buf.begin_transaction();
            for line in start.line..=end.line {
                buf.insert(Position::new(line, 0), "    ");
            }
            buf.commit_transaction();
        }
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_shift_tab(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let sel = buf.cursor.primary.clone();
        let (start_line, end_line) = if sel.is_cursor() {
            (sel.head.line, sel.head.line)
        } else {
            let (s, e) = sel.ordered();
            (s.line, e.line)
        };
        buf.begin_transaction();
        for line in start_line..=end_line {
            if let Some(line_text) = buf.line(line) {
                let spaces = line_text.chars().take(4).take_while(|&c| c == ' ').count();
                if spaces > 0 {
                    buf.delete(Position::new(line, 0), Position::new(line, spaces));
                }
            }
        }
        buf.commit_transaction();
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_duplicate_line(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let line_idx = buf.cursor.primary.head.line;
        if let Some(line_text) = buf.line(line_idx) {
            let content = line_text
                .strip_suffix('\n')
                .map_or(line_text.as_str(), |s| s.strip_suffix('\r').unwrap_or(s));
            buf.insert(
                Position::new(line_idx, content.chars().count()),
                &format!("\n{content}"),
            );
        }
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_move_line_up(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let line = buf.cursor.primary.head.line;
        if line == 0 {
            return Control::Changed;
        }
        let col = buf.cursor.primary.head.col;
        let curr = buf.line(line).unwrap_or_default();
        let prev = buf.line(line - 1).unwrap_or_default();
        let start = Position::new(line - 1, 0);
        let end = if line + 1 < buf.line_count() {
            Position::new(line + 1, 0)
        } else {
            Position::new(line, buf.line_len(line))
        };
        buf.begin_transaction();
        buf.delete(start, end);
        buf.insert(start, &format!("{curr}{prev}"));
        let new_col = col.min(buf.line_len_no_newline(line - 1));
        buf.cursor.primary = Selection::cursor(Position::new(line - 1, new_col));
        buf.commit_transaction();
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_move_line_down(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        let line = buf.cursor.primary.head.line;
        let last_line = buf.line_count().saturating_sub(1);
        if line >= last_line {
            return Control::Changed;
        }
        let col = buf.cursor.primary.head.col;
        let curr = buf.line(line).unwrap_or_default();
        let next = buf.line(line + 1).unwrap_or_default();
        let start = Position::new(line, 0);
        let end = if line + 2 < buf.line_count() {
            Position::new(line + 2, 0)
        } else {
            Position::new(line + 1, buf.line_len(line + 1))
        };
        buf.begin_transaction();
        buf.delete(start, end);
        buf.insert(start, &format!("{next}{curr}"));
        let new_col = col.min(buf.line_len_no_newline(line + 1));
        buf.cursor.primary = Selection::cursor(Position::new(line + 1, new_col));
        buf.commit_transaction();
    }
    state.update_active_highlighter();
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn apply_motion(
    state: &mut AppState,
    f: impl FnOnce(&mut TextBuffer),
) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
        f(buf);
    }
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn apply_buf_edit(state: &mut AppState, f: fn(&mut TextBuffer) -> bool) {
    if let Some(buf) = state.active_buf_mut() {
        let _ = f(buf);
    }
    state.viewport_follow_cursor = true;
    state.update_active_highlighter();
}

fn move_n(buf: &mut TextBuffer, n: usize, extend: bool, method: fn(&mut TextBuffer, bool)) {
    for _ in 0..n {
        method(buf, extend);
    }
}

fn read_clipboard_text(state: &mut AppState) -> Option<String> {
    match Clipboard::new().and_then(|mut cb| cb.get_text()) {
        Ok(text) => Some(text),
        Err(e) => {
            state
                .overlay
                .notify(format!("Clipboard error: {e}"), NotificationLevel::Error);
            None
        }
    }
}

fn current_line_range(buf: &TextBuffer) -> (Position, Position) {
    let line = buf.cursor.primary.head.line;
    let start = Position::new(line, 0);
    let end = if line + 1 < buf.line_count() {
        Position::new(line + 1, 0)
    } else {
        Position::new(line, buf.line_len(line))
    };
    (start, end)
}

pub(super) fn handle_add_cursor_above(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        let _ = buf.add_cursor_above();
    }
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_add_cursor_below(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        let _ = buf.add_cursor_below();
    }
    state.viewport_follow_cursor = true;
    Control::Changed
}

pub(super) fn handle_clear_secondary_cursors(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        buf.clear_secondary_cursors();
    }
    state.viewport_follow_cursor = true;
    Control::Changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_line_range_includes_newline_when_available() {
        let mut buf = TextBuffer::from_text("one\ntwo\n");
        buf.cursor = CursorState::at(Position::new(0, 1));

        assert_eq!(
            current_line_range(&buf),
            (Position::new(0, 0), Position::new(1, 0))
        );
    }

    #[test]
    fn current_line_range_clamps_last_line() {
        let mut buf = TextBuffer::from_text("one\ntwo");
        buf.cursor = CursorState::at(Position::new(1, 1));

        assert_eq!(
            current_line_range(&buf),
            (Position::new(1, 0), Position::new(1, 3))
        );
    }
}
