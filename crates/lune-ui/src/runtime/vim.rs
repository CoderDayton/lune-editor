//! Vim mode state machine.
//!
//! Implements a minimal vim emulation layer: Normal, Insert, Visual modes
//! with basic motions, operators, and numeric prefixes.

use lune_core::buffer::TextBuffer;

/// The current vim editing mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VimMode {
    /// Normal (command) mode — the default.
    #[default]
    Normal,
    /// Insert mode — typed characters are inserted.
    Insert,
    /// Visual (character-wise) mode — motions extend selection.
    Visual,
    /// Visual line mode — selections are whole lines.
    VisualLine,
    /// Command-line mode (`:` commands).
    Command,
}

impl VimMode {
    /// Whether characters should be inserted into the buffer.
    #[must_use]
    pub const fn is_insert(&self) -> bool {
        matches!(self, Self::Insert)
    }

    /// Whether we are in a visual selection mode.
    #[must_use]
    pub const fn is_visual(&self) -> bool {
        matches!(self, Self::Visual | Self::VisualLine)
    }
}

/// An operator waiting for a motion (e.g., `d` awaiting `w`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VimOp {
    /// Delete (d).
    Delete,
    /// Yank (y).
    Yank,
    /// Change (c) — delete then enter insert mode.
    Change,
    /// Indent (>).
    Indent,
    /// Outdent (<).
    Outdent,
}

/// A recorded vim command for `.` repeat.
#[derive(Clone, Debug)]
pub struct VimCommand {
    /// The operator (if any).
    pub op: Option<VimOp>,
    /// The motion key(s).
    pub motion: char,
    /// The count prefix.
    pub count: usize,
    /// Text inserted (for insert-mode commands like `ciw`).
    pub inserted_text: Option<String>,
}

/// Full vim state.
#[derive(Clone, Debug)]
pub struct VimState {
    /// Current mode.
    pub mode: VimMode,
    /// Accumulated numeric prefix (e.g., `5` in `5j`).
    pub count: Option<usize>,
    /// Pending operator awaiting a motion.
    pub pending_op: Option<VimOp>,
    /// Last change command (for `.` repeat).
    pub last_command: Option<VimCommand>,
    /// Active register (default `"`).
    pub register: char,
    /// Command-line buffer (text typed after `:`).
    pub cmdline: String,
}

impl VimState {
    /// Create a new vim state in normal mode.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: VimMode::Normal,
            count: None,
            pending_op: None,
            last_command: None,
            register: '"',
            cmdline: String::new(),
        }
    }

    /// Get the effective count (default 1 if no prefix).
    #[must_use]
    pub fn effective_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    /// Feed a digit for the numeric prefix.
    ///
    /// Returns `true` if the digit was consumed as part of a count.
    pub fn feed_digit(&mut self, digit: char) -> bool {
        if !digit.is_ascii_digit() {
            return false;
        }
        let d = digit as usize - '0' as usize;
        // `0` at the start is a motion (beginning of line), not a count.
        if d == 0 && self.count.is_none() {
            return false;
        }
        self.count = Some(self.count.unwrap_or(0) * 10 + d);
        true
    }

    /// Reset the count and pending operator.
    pub const fn reset_pending(&mut self) {
        self.count = None;
        self.pending_op = None;
    }

    /// Enter insert mode.
    pub const fn enter_insert(&mut self) {
        self.mode = VimMode::Insert;
        self.reset_pending();
    }

    /// Enter normal mode (from any mode).
    pub const fn enter_normal(&mut self) {
        self.mode = VimMode::Normal;
        self.reset_pending();
    }

    /// Enter visual mode.
    pub const fn enter_visual(&mut self) {
        self.mode = VimMode::Visual;
        self.reset_pending();
    }

    /// Enter visual-line mode.
    pub const fn enter_visual_line(&mut self) {
        self.mode = VimMode::VisualLine;
        self.reset_pending();
    }

    /// Enter command-line mode, clearing the command buffer.
    pub fn enter_command(&mut self) {
        self.mode = VimMode::Command;
        self.cmdline.clear();
        self.reset_pending();
    }

    /// Append a character to the command-line buffer.
    pub fn cmdline_push(&mut self, ch: char) {
        self.cmdline.push(ch);
    }

    /// Remove the last character from the command-line buffer.
    pub fn cmdline_pop(&mut self) {
        self.cmdline.pop();
    }

    /// Clear the command-line buffer and return to normal mode.
    pub fn cmdline_clear(&mut self) {
        self.cmdline.clear();
    }

    /// Process a normal-mode key press. Returns `VimAction` describing
    /// what the editor should do.
    pub fn handle_normal(&mut self, ch: char, buf: &TextBuffer) -> VimAction {
        // Check if it's a digit for the count prefix.
        if self.feed_digit(ch) {
            return VimAction::None;
        }

        // `:` enters command-line mode.
        if ch == ':' {
            self.enter_command();
            return VimAction::ModeChanged(VimMode::Command);
        }

        let count = self.effective_count();

        // Check for pending operator + motion.
        if self.pending_op.is_some() {
            return self.handle_operator_motion(ch, count, buf);
        }

        // Try mode transitions, then motions, then operators/actions.
        self.handle_mode_key(ch)
            .or_else(|| self.handle_motion_key(ch, count))
            .unwrap_or_else(|| self.handle_action_key(ch, count))
    }

    /// Handle mode-transition keys (i, a, o, O, I, A, v, V).
    const fn handle_mode_key(&mut self, ch: char) -> Option<VimAction> {
        let action = match ch {
            'i' => {
                self.enter_insert();
                VimAction::ModeChanged(VimMode::Insert)
            }
            'a' => {
                self.enter_insert();
                VimAction::MoveRight(1)
            }
            'o' => {
                self.enter_insert();
                VimAction::OpenLineBelow
            }
            'O' => {
                self.enter_insert();
                VimAction::OpenLineAbove
            }
            'I' => {
                self.enter_insert();
                VimAction::MoveLineStart
            }
            'A' => {
                self.enter_insert();
                VimAction::MoveLineEnd
            }
            'v' => {
                self.enter_visual();
                VimAction::ModeChanged(VimMode::Visual)
            }
            'V' => {
                self.enter_visual_line();
                VimAction::ModeChanged(VimMode::VisualLine)
            }
            _ => return None,
        };
        Some(action)
    }

    /// Handle motion keys (h, j, k, l, w, b, 0, $, G).
    const fn handle_motion_key(&mut self, ch: char, count: usize) -> Option<VimAction> {
        let action = match ch {
            'h' => VimAction::MoveLeft(count),
            'j' => VimAction::MoveDown(count),
            'k' => VimAction::MoveUp(count),
            'l' => VimAction::MoveRight(count),
            'w' => VimAction::MoveWordRight(count),
            'b' => VimAction::MoveWordLeft(count),
            '0' => VimAction::MoveLineStart,
            '$' => VimAction::MoveLineEnd,
            'G' => {
                if self.count.is_some() {
                    VimAction::MoveToLine(count.saturating_sub(1))
                } else {
                    VimAction::MoveBufferEnd
                }
            }
            _ => return None,
        };
        self.reset_pending();
        Some(action)
    }

    /// Handle operator keys (d, y, c) and single-key actions (x, u).
    const fn handle_action_key(&mut self, ch: char, count: usize) -> VimAction {
        match ch {
            'd' => {
                self.pending_op = Some(VimOp::Delete);
                VimAction::None
            }
            'y' => {
                self.pending_op = Some(VimOp::Yank);
                VimAction::None
            }
            'c' => {
                self.pending_op = Some(VimOp::Change);
                VimAction::None
            }
            'x' => {
                self.reset_pending();
                VimAction::DeleteCharForward(count)
            }
            'u' => {
                self.reset_pending();
                VimAction::Undo
            }
            _ => {
                self.reset_pending();
                VimAction::None
            }
        }
    }

    /// Handle a motion key when an operator is pending.
    fn handle_operator_motion(&mut self, ch: char, count: usize, _buf: &TextBuffer) -> VimAction {
        let op = self.pending_op.take().expect("operator was pending");
        self.reset_pending();

        let motion = match ch {
            'w' => VimMotion::WordRight(count),
            'b' => VimMotion::WordLeft(count),
            'j' => VimMotion::Down(count),
            'k' => VimMotion::Up(count),
            'h' => VimMotion::Left(count),
            'l' => VimMotion::Right(count),
            '$' => VimMotion::LineEnd,
            '0' => VimMotion::LineStart,
            'G' => VimMotion::BufferEnd,
            // Double operator (e.g., `dd`, `yy`, `cc`) = operate on current line.
            'd' if op == VimOp::Delete => return VimAction::DeleteLine(count),
            'y' if op == VimOp::Yank => return VimAction::YankLine(count),
            'c' if op == VimOp::Change => {
                self.enter_insert();
                return VimAction::ChangeLine(count);
            }
            _ => return VimAction::None,
        };

        match op {
            VimOp::Delete => VimAction::DeleteMotion(motion),
            VimOp::Yank => VimAction::YankMotion(motion),
            VimOp::Change => {
                self.enter_insert();
                VimAction::ChangeMotion(motion)
            }
            VimOp::Indent => VimAction::IndentMotion(motion),
            VimOp::Outdent => VimAction::OutdentMotion(motion),
        }
    }
}

impl Default for VimState {
    fn default() -> Self {
        Self::new()
    }
}

/// A motion specifier for operator+motion combinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMotion {
    /// Move left N characters.
    Left(usize),
    /// Move right N characters.
    Right(usize),
    /// Move up N lines.
    Up(usize),
    /// Move down N lines.
    Down(usize),
    /// Move forward N words.
    WordRight(usize),
    /// Move backward N words.
    WordLeft(usize),
    /// Move to start of line.
    LineStart,
    /// Move to end of line.
    LineEnd,
    /// Move to end of buffer.
    BufferEnd,
}

/// The action that the editor should take in response to a vim keypress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimAction {
    /// No action (key consumed but nothing to do).
    None,
    /// Mode has changed.
    ModeChanged(VimMode),
    /// Move cursor left N chars.
    MoveLeft(usize),
    /// Move cursor right N chars.
    MoveRight(usize),
    /// Move cursor up N lines.
    MoveUp(usize),
    /// Move cursor down N lines.
    MoveDown(usize),
    /// Move forward N words.
    MoveWordRight(usize),
    /// Move backward N words.
    MoveWordLeft(usize),
    /// Move to start of current line.
    MoveLineStart,
    /// Move to end of current line.
    MoveLineEnd,
    /// Move to end of buffer.
    MoveBufferEnd,
    /// Move to a specific line (0-based).
    MoveToLine(usize),
    /// Open a new line below and enter insert mode.
    OpenLineBelow,
    /// Open a new line above and enter insert mode.
    OpenLineAbove,
    /// Delete N characters forward.
    DeleteCharForward(usize),
    /// Delete the current line (dd with count).
    DeleteLine(usize),
    /// Yank the current line (yy with count).
    YankLine(usize),
    /// Change the current line (cc with count).
    ChangeLine(usize),
    /// Delete with a motion.
    DeleteMotion(VimMotion),
    /// Yank with a motion.
    YankMotion(VimMotion),
    /// Change with a motion (delete + insert mode).
    ChangeMotion(VimMotion),
    /// Indent with a motion.
    IndentMotion(VimMotion),
    /// Outdent with a motion.
    OutdentMotion(VimMotion),
    /// Undo.
    Undo,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buf() -> TextBuffer {
        TextBuffer::from_text("hello world\nfoo bar\nbaz qux\n")
    }

    #[test]
    fn normal_h_moves_left() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('h', &buf);
        assert_eq!(action, VimAction::MoveLeft(1));
    }

    #[test]
    fn normal_j_moves_down() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('j', &buf);
        assert_eq!(action, VimAction::MoveDown(1));
    }

    #[test]
    fn numeric_prefix_5j() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('5', &buf), VimAction::None);
        let action = vim.handle_normal('j', &buf);
        assert_eq!(action, VimAction::MoveDown(5));
    }

    #[test]
    fn i_enters_insert() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('i', &buf);
        assert_eq!(action, VimAction::ModeChanged(VimMode::Insert));
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn v_enters_visual() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('v', &buf);
        assert_eq!(action, VimAction::ModeChanged(VimMode::Visual));
        assert_eq!(vim.mode, VimMode::Visual);
    }

    #[test]
    fn dd_deletes_line() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('d', &buf), VimAction::None);
        let action = vim.handle_normal('d', &buf);
        assert_eq!(action, VimAction::DeleteLine(1));
    }

    #[test]
    fn d2w_deletes_two_words() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('d', &buf);
        vim.handle_normal('2', &buf);
        let action = vim.handle_normal('w', &buf);
        assert_eq!(action, VimAction::DeleteMotion(VimMotion::WordRight(2)));
    }

    #[test]
    fn escape_enters_normal_from_insert() {
        let mut vim = VimState::new();
        vim.enter_insert();
        assert_eq!(vim.mode, VimMode::Insert);
        vim.enter_normal();
        assert_eq!(vim.mode, VimMode::Normal);
    }

    #[test]
    fn zero_at_start_is_motion_not_count() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('0', &buf);
        assert_eq!(action, VimAction::MoveLineStart);
    }

    #[test]
    fn x_deletes_char_forward() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('x', &buf);
        assert_eq!(action, VimAction::DeleteCharForward(1));
    }

    #[test]
    fn u_undoes() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('u', &buf);
        assert_eq!(action, VimAction::Undo);
    }

    // ── Mode transition keys ──────────────────────────────────────

    #[test]
    fn a_enters_insert_and_moves_right() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('a', &buf);
        assert_eq!(action, VimAction::MoveRight(1));
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn o_opens_line_below() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('o', &buf);
        assert_eq!(action, VimAction::OpenLineBelow);
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn o_upper_opens_line_above() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('O', &buf);
        assert_eq!(action, VimAction::OpenLineAbove);
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn i_upper_enters_insert_at_line_start() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('I', &buf);
        assert_eq!(action, VimAction::MoveLineStart);
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn a_upper_enters_insert_at_line_end() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('A', &buf);
        assert_eq!(action, VimAction::MoveLineEnd);
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn v_upper_enters_visual_line() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('V', &buf);
        assert_eq!(action, VimAction::ModeChanged(VimMode::VisualLine));
        assert_eq!(vim.mode, VimMode::VisualLine);
    }

    // ── Motion keys ───────────────────────────────────────────────

    #[test]
    fn k_moves_up() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('k', &buf), VimAction::MoveUp(1));
    }

    #[test]
    fn l_moves_right() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('l', &buf), VimAction::MoveRight(1));
    }

    #[test]
    fn w_moves_word_right() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('w', &buf), VimAction::MoveWordRight(1));
    }

    #[test]
    fn b_moves_word_left() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('b', &buf), VimAction::MoveWordLeft(1));
    }

    #[test]
    fn dollar_moves_to_line_end() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('$', &buf), VimAction::MoveLineEnd);
    }

    #[test]
    fn g_upper_without_count_moves_buffer_end() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('G', &buf), VimAction::MoveBufferEnd);
    }

    #[test]
    fn g_upper_with_count_moves_to_line() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('5', &buf);
        assert_eq!(vim.handle_normal('G', &buf), VimAction::MoveToLine(4));
    }

    // ── Operator+motion combos ────────────────────────────────────

    #[test]
    fn yy_yanks_line() {
        let mut vim = VimState::new();
        let buf = make_buf();
        assert_eq!(vim.handle_normal('y', &buf), VimAction::None);
        assert_eq!(vim.handle_normal('y', &buf), VimAction::YankLine(1));
    }

    #[test]
    fn cc_changes_line() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('c', &buf);
        let action = vim.handle_normal('c', &buf);
        assert_eq!(action, VimAction::ChangeLine(1));
        assert_eq!(vim.mode, VimMode::Insert); // cc enters insert mode.
    }

    #[test]
    fn cw_changes_word() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('c', &buf);
        let action = vim.handle_normal('w', &buf);
        assert_eq!(action, VimAction::ChangeMotion(VimMotion::WordRight(1)));
        assert_eq!(vim.mode, VimMode::Insert);
    }

    #[test]
    fn dw_deletes_word() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('d', &buf);
        assert_eq!(
            vim.handle_normal('w', &buf),
            VimAction::DeleteMotion(VimMotion::WordRight(1))
        );
    }

    #[test]
    fn d_dollar_deletes_to_line_end() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('d', &buf);
        assert_eq!(
            vim.handle_normal('$', &buf),
            VimAction::DeleteMotion(VimMotion::LineEnd)
        );
    }

    #[test]
    fn y_g_upper_yanks_to_buffer_end() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('y', &buf);
        assert_eq!(
            vim.handle_normal('G', &buf),
            VimAction::YankMotion(VimMotion::BufferEnd)
        );
    }

    // ── VimMode helpers ───────────────────────────────────────────

    #[test]
    fn vim_mode_is_insert() {
        assert!(VimMode::Insert.is_insert());
        assert!(!VimMode::Normal.is_insert());
        assert!(!VimMode::Visual.is_insert());
    }

    #[test]
    fn vim_mode_is_visual() {
        assert!(VimMode::Visual.is_visual());
        assert!(VimMode::VisualLine.is_visual());
        assert!(!VimMode::Normal.is_visual());
        assert!(!VimMode::Insert.is_visual());
    }

    // ── VimState helpers ──────────────────────────────────────────

    #[test]
    fn effective_count_default_is_one() {
        let vim = VimState::new();
        assert_eq!(vim.effective_count(), 1);
    }

    #[test]
    fn feed_digit_builds_count() {
        let mut vim = VimState::new();
        assert!(vim.feed_digit('3'));
        assert!(vim.feed_digit('5'));
        assert_eq!(vim.effective_count(), 35);
    }

    #[test]
    fn feed_digit_zero_at_start_is_not_count() {
        let mut vim = VimState::new();
        assert!(!vim.feed_digit('0'));
        assert_eq!(vim.count, None);
    }

    #[test]
    fn feed_digit_non_digit_returns_false() {
        let mut vim = VimState::new();
        assert!(!vim.feed_digit('a'));
    }

    #[test]
    fn reset_pending_clears_state() {
        let mut vim = VimState::new();
        vim.count = Some(5);
        vim.pending_op = Some(VimOp::Delete);
        vim.reset_pending();
        assert!(vim.count.is_none());
        assert!(vim.pending_op.is_none());
    }

    #[test]
    fn enter_visual_line_mode() {
        let mut vim = VimState::new();
        vim.enter_visual_line();
        assert_eq!(vim.mode, VimMode::VisualLine);
    }

    #[test]
    fn unknown_key_returns_none() {
        let mut vim = VimState::new();
        let buf = make_buf();
        let action = vim.handle_normal('Z', &buf);
        assert_eq!(action, VimAction::None);
    }

    #[test]
    fn operator_with_unknown_motion_returns_none() {
        let mut vim = VimState::new();
        let buf = make_buf();
        vim.handle_normal('d', &buf); // Pending delete.
        let action = vim.handle_normal('Z', &buf); // Unknown motion.
        assert_eq!(action, VimAction::None);
        assert!(vim.pending_op.is_none()); // Should be cleared.
    }
}
