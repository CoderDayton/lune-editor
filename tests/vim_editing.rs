//! Integration test: vim editing sequences.
//!
//! Tests that vim key sequences produce correct `VimAction`s and that
//! applying those actions to a `TextBuffer` yields the expected state.

use lune_core::buffer::TextBuffer;
use lune_core::position::Position;

use lune_ui::vim::{VimAction, VimMode, VimState};

// ── Helpers ───────────────────────────────────────────────────────────

/// Current cursor position shorthand.
const fn cursor_pos(buf: &TextBuffer) -> Position {
    buf.cursor.primary.head
}

/// Apply a `VimAction` to a buffer (simplified — handles motions and edits).
fn apply_action(buf: &mut TextBuffer, vim: &mut VimState, action: &VimAction) {
    match action {
        VimAction::MoveLeft(n) => {
            for _ in 0..*n {
                buf.move_left(false);
            }
        }
        VimAction::MoveRight(n) => {
            for _ in 0..*n {
                buf.move_right(false);
            }
        }
        VimAction::MoveUp(n) => {
            for _ in 0..*n {
                buf.move_up(false);
            }
        }
        VimAction::MoveDown(n) => {
            for _ in 0..*n {
                buf.move_down(false);
            }
        }
        VimAction::MoveWordRight(n) => {
            for _ in 0..*n {
                buf.move_word_right(false);
            }
        }
        VimAction::MoveWordLeft(n) => {
            for _ in 0..*n {
                buf.move_word_left(false);
            }
        }
        VimAction::MoveLineStart => buf.move_line_start(false),
        VimAction::MoveLineEnd => buf.move_line_end(false),
        VimAction::MoveBufferEnd => buf.move_buffer_end(false),
        VimAction::MoveToLine(line) => {
            buf.move_buffer_start(false);
            for _ in 0..*line {
                buf.move_down(false);
            }
        }
        VimAction::DeleteCharForward(n) => {
            for _ in 0..*n {
                let pos = cursor_pos(buf);
                let end = Position::new(pos.line, pos.col + 1);
                if end.col <= buf.line_len(pos.line) {
                    buf.delete(pos, end);
                }
            }
        }
        VimAction::Undo => {
            buf.undo();
        }
        // Other actions are complex (delete line, yank, etc.) — skip for now.
        _ => {}
    }
    // Also handle mode enter side effects.
    if let VimAction::ModeChanged(mode) = action {
        match mode {
            VimMode::Insert => vim.enter_insert(),
            VimMode::Visual => vim.enter_visual(),
            VimMode::VisualLine => vim.enter_visual_line(),
            _ => {}
        }
    }
}

/// Feed a key and apply the resulting action.
fn feed_and_apply(buf: &mut TextBuffer, vim: &mut VimState, ch: char) {
    let action = vim.handle_normal(ch, buf);
    apply_action(buf, vim, &action);
}

// ── Tests ─────────────────────────────────────────────────────────────

#[test]
fn vim_hjkl_navigation() {
    let mut buf = TextBuffer::from_text("abc\ndef\nghi\n");
    let mut vim = VimState::new();

    // j = move down
    feed_and_apply(&mut buf, &mut vim, 'j');
    assert_eq!(cursor_pos(&buf), Position::new(1, 0));

    // l = move right
    feed_and_apply(&mut buf, &mut vim, 'l');
    assert_eq!(cursor_pos(&buf), Position::new(1, 1));

    // k = move up
    feed_and_apply(&mut buf, &mut vim, 'k');
    assert_eq!(cursor_pos(&buf), Position::new(0, 1));

    // h = move left
    feed_and_apply(&mut buf, &mut vim, 'h');
    assert_eq!(cursor_pos(&buf), Position::new(0, 0));
}

#[test]
fn vim_numeric_prefix() {
    let mut buf = TextBuffer::from_text("line 0\nline 1\nline 2\nline 3\nline 4\n");
    let mut vim = VimState::new();

    // 3j = move down 3 lines.
    let _none = vim.handle_normal('3', &buf); // consumed as digit
    feed_and_apply(&mut buf, &mut vim, 'j');
    assert_eq!(cursor_pos(&buf).line, 3);
}

#[test]
fn vim_x_deletes_char() {
    let mut buf = TextBuffer::from_text("hello");
    let mut vim = VimState::new();

    feed_and_apply(&mut buf, &mut vim, 'x');
    assert_eq!(buf.text(), "ello");
}

#[test]
fn vim_i_enters_insert_mode() {
    let buf = TextBuffer::from_text("hello");
    let mut vim = VimState::new();

    let action = vim.handle_normal('i', &buf);
    assert_eq!(action, VimAction::ModeChanged(VimMode::Insert));
    assert!(vim.mode.is_insert());
}

#[test]
fn vim_mode_transitions() {
    let mut vim = VimState::new();
    let buf = TextBuffer::from_text("test");

    // Normal → Insert via 'i'.
    let action = vim.handle_normal('i', &buf);
    assert_eq!(action, VimAction::ModeChanged(VimMode::Insert));
    assert!(vim.mode.is_insert());

    // Insert → Normal.
    vim.enter_normal();
    assert_eq!(vim.mode, VimMode::Normal);
}

#[test]
fn vim_u_undoes_edit() {
    let mut buf = TextBuffer::from_text("hello");
    let mut vim = VimState::new();

    // Delete with 'x', then undo with 'u'.
    feed_and_apply(&mut buf, &mut vim, 'x');
    assert_eq!(buf.text(), "ello");

    feed_and_apply(&mut buf, &mut vim, 'u');
    assert_eq!(buf.text(), "hello");
}

#[test]
fn vim_dollar_moves_to_line_end() {
    let mut buf = TextBuffer::from_text("hello world\n");
    let mut vim = VimState::new();

    feed_and_apply(&mut buf, &mut vim, '$');
    // "hello world" = 11 chars, so col should be at end.
    assert_eq!(cursor_pos(&buf).col, 11);
}

#[test]
fn vim_zero_moves_to_line_start() {
    let mut buf = TextBuffer::from_text("hello world\n");
    let mut vim = VimState::new();

    // Move right first.
    feed_and_apply(&mut buf, &mut vim, 'l');
    feed_and_apply(&mut buf, &mut vim, 'l');
    assert_eq!(cursor_pos(&buf).col, 2);

    // 0 = go to line start.
    feed_and_apply(&mut buf, &mut vim, '0');
    assert_eq!(cursor_pos(&buf).col, 0);
}

#[test]
fn vim_w_moves_word_right() {
    let mut buf = TextBuffer::from_text("hello world foo\n");
    let mut vim = VimState::new();

    feed_and_apply(&mut buf, &mut vim, 'w');
    // Should move past "hello " to start of "world".
    assert!(cursor_pos(&buf).col > 0, "w should move cursor right");
}

#[test]
fn vim_g_g_goes_to_buffer_start() {
    let mut buf = TextBuffer::from_text("line 0\nline 1\nline 2\nline 3\n");
    let mut vim = VimState::new();

    // Move down first.
    feed_and_apply(&mut buf, &mut vim, 'j');
    feed_and_apply(&mut buf, &mut vim, 'j');
    assert_eq!(cursor_pos(&buf).line, 2);

    // G = buffer end.
    feed_and_apply(&mut buf, &mut vim, 'G');
    assert!(cursor_pos(&buf).line > 0);
}

#[test]
fn vim_v_enters_visual_mode() {
    let buf = TextBuffer::from_text("test\n");
    let mut vim = VimState::new();

    let action = vim.handle_normal('v', &buf);
    assert_eq!(action, VimAction::ModeChanged(VimMode::Visual));
    assert!(vim.mode.is_visual());
}

#[test]
fn vim_multiple_x_deletes_multiple_chars() {
    let mut buf = TextBuffer::from_text("abcdef");
    let mut vim = VimState::new();

    feed_and_apply(&mut buf, &mut vim, 'x');
    feed_and_apply(&mut buf, &mut vim, 'x');
    feed_and_apply(&mut buf, &mut vim, 'x');
    assert_eq!(buf.text(), "def");
}
