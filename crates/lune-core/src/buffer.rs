//! Text buffer backed by a rope data structure.
//!
//! [`TextBuffer`] is the fundamental editing primitive. It wraps a
//! [`ropey::Rope`] and provides position-aware insert/delete/replace
//! operations, undo/redo, cursor management, and dirty tracking.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ropey::Rope;
use uuid::Uuid;

use crate::position::{CursorState, Position, Selection};
use crate::undo::{end_position_after_insert, EditOp, RevisionId, Transaction, UndoStack};

/// Unique identifier for a buffer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferId(pub Uuid);

impl BufferId {
    /// Generate a new random buffer ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BufferId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BufferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A rope-backed text buffer with undo/redo and cursor state.
#[derive(Debug)]
pub struct TextBuffer {
    /// Unique buffer ID.
    pub id: BufferId,
    /// The underlying rope.
    rope: Rope,
    /// Associated file path, if any.
    pub file_path: Option<PathBuf>,
    /// Current cursor/selection state.
    pub cursor: CursorState,
    /// Undo history.
    undo_stack: UndoStack,
    /// Redo history.
    redo_stack: UndoStack,
    /// Current revision number (increments on each edit).
    current_revision: RevisionId,
    /// Revision at last save.
    last_saved_revision: RevisionId,

    /// When `Some`, we are inside a `begin_transaction` / `commit_transaction`
    /// block. Ops accumulate here until committed.
    pending_ops: Option<Vec<EditOp>>,
    /// Cursor state captured at `begin_transaction`.
    pending_cursor_before: Option<CursorState>,
}

impl TextBuffer {
    // ── Constructors ──────────────────────────────────────────────────

    /// Create a new empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: BufferId::new(),
            rope: Rope::new(),
            file_path: None,
            cursor: CursorState::default(),
            undo_stack: UndoStack::new(),
            redo_stack: UndoStack::new(),
            current_revision: 0,
            last_saved_revision: 0,
            pending_ops: None,
            pending_cursor_before: None,
        }
    }

    /// Create a buffer from a string slice.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            ..Self::new()
        }
    }

    /// Create a buffer by reading a file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut buf = Self::from_text(&content);
        buf.file_path = Some(path.to_path_buf());
        Ok(buf)
    }

    // ── Accessors ─────────────────────────────────────────────────────

    /// Number of lines in the buffer (always >= 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Total number of characters in the buffer.
    #[must_use]
    pub fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    /// Get the text of a line (0-based). Returns `None` if out of bounds.
    #[must_use]
    pub fn line(&self, idx: usize) -> Option<String> {
        if idx >= self.rope.len_lines() {
            return None;
        }
        Some(self.rope.line(idx).to_string())
    }

    /// Get the character count of a given line (0-based), including any
    /// trailing newline. Returns 0 if out of bounds.
    #[must_use]
    pub fn line_len(&self, idx: usize) -> usize {
        if idx >= self.rope.len_lines() {
            return 0;
        }
        self.rope.line(idx).len_chars()
    }

    /// Get the full buffer text as a `String`.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Get a reference to the underlying rope.
    #[must_use]
    pub const fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Whether the buffer has been modified since last save.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.current_revision != self.last_saved_revision
    }

    /// Current revision number.
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.current_revision
    }

    // ── Position conversion ───────────────────────────────────────────

    /// Convert a `Position` to a rope char index.
    ///
    /// Clamps to valid range. Returns `None` only if the buffer is empty
    /// and position is non-zero.
    #[must_use]
    pub fn pos_to_char(&self, pos: Position) -> usize {
        let line = pos.line.min(self.rope.len_lines().saturating_sub(1));
        let line_start = self.rope.line_to_char(line);
        let line_len = self.rope.line(line).len_chars();
        // Clamp col to line length (allowing cursor at end of line).
        let col = pos.col.min(line_len);
        line_start + col
    }

    /// Convert a rope char index to a `Position`.
    #[must_use]
    pub fn char_to_pos(&self, char_idx: usize) -> Position {
        let idx = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        let line_start = self.rope.line_to_char(line);
        Position::new(line, idx - line_start)
    }

    // ── Edit operations ───────────────────────────────────────────────

    /// Insert text at the given position.
    ///
    /// Returns the `EditOp` that was applied.
    pub fn insert(&mut self, pos: Position, text: &str) -> EditOp {
        let char_idx = self.pos_to_char(pos);
        self.rope.insert(char_idx, text);
        self.current_revision += 1;

        let op = EditOp::Insert {
            pos,
            text: text.to_string(),
        };

        // Move cursor to end of inserted text.
        let end = end_position_after_insert(pos, text);
        self.cursor = CursorState::at(end);

        self.record_op(op.clone());
        op
    }

    /// Delete text in the given range `[start, end)`.
    ///
    /// Returns the `EditOp` that was applied.
    pub fn delete(&mut self, start: Position, end: Position) -> EditOp {
        let (lo, hi) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let start_idx = self.pos_to_char(lo);
        let end_idx = self.pos_to_char(hi);

        let deleted_text: String = self.rope.slice(start_idx..end_idx).to_string();
        self.rope.remove(start_idx..end_idx);
        self.current_revision += 1;

        let op = EditOp::Delete {
            start: lo,
            end: hi,
            deleted_text,
        };

        self.cursor = CursorState::at(lo);

        self.record_op(op.clone());
        op
    }

    /// Replace the range `[start, end)` with `text`.
    ///
    /// This is a compound operation (delete + insert) recorded as a single
    /// transaction step.
    pub fn replace(&mut self, start: Position, end: Position, text: &str) {
        let was_in_transaction = self.pending_ops.is_some();
        if !was_in_transaction {
            self.begin_transaction();
        }
        self.delete(start, end);
        self.insert(start, text);
        if !was_in_transaction {
            self.commit_transaction();
        }
    }

    // ── Undo / Redo ───────────────────────────────────────────────────

    /// Begin a transaction. All subsequent edit ops will be grouped into a
    /// single undo step until [`commit_transaction`](Self::commit_transaction)
    /// is called.
    pub fn begin_transaction(&mut self) {
        self.pending_cursor_before = Some(self.cursor.clone());
        self.pending_ops = Some(Vec::new());
    }

    /// Commit the current transaction, pushing it onto the undo stack.
    ///
    /// No-op if no transaction is in progress or if the transaction is empty.
    pub fn commit_transaction(&mut self) {
        if let Some(ops) = self.pending_ops.take() {
            let cursor_before = self.pending_cursor_before.take().unwrap_or_default();

            if ops.is_empty() {
                return;
            }

            let txn = Transaction {
                revision: self.current_revision,
                ops,
                cursor_before,
                cursor_after: self.cursor.clone(),
            };
            self.undo_stack.push(txn);
            self.redo_stack.clear();
        }
    }

    /// Record an edit op — either into the pending transaction or as a
    /// standalone transaction.
    fn record_op(&mut self, op: EditOp) {
        if let Some(ref mut ops) = self.pending_ops {
            ops.push(op);
        } else {
            // Auto-wrap in a single-op transaction.
            let cursor_before = self.cursor.clone();
            let txn = Transaction {
                revision: self.current_revision,
                ops: vec![op],
                cursor_before,
                cursor_after: self.cursor.clone(),
            };
            self.undo_stack.push(txn);
            self.redo_stack.clear();
        }
    }

    /// Undo the last transaction.
    ///
    /// Returns `true` if an undo was performed.
    pub fn undo(&mut self) -> bool {
        let Some(txn) = self.undo_stack.pop() else {
            return false;
        };

        // Apply inverse ops in reverse order.
        for op in txn.ops.iter().rev() {
            self.apply_op_raw(&op.inverse());
        }
        self.cursor = txn.cursor_before.clone();
        self.current_revision += 1;
        self.redo_stack.push(txn);
        true
    }

    /// Redo the last undone transaction.
    ///
    /// Returns `true` if a redo was performed.
    pub fn redo(&mut self) -> bool {
        let Some(txn) = self.redo_stack.pop() else {
            return false;
        };

        // Apply ops in forward order.
        for op in &txn.ops {
            self.apply_op_raw(op);
        }
        self.cursor = txn.cursor_after.clone();
        self.current_revision += 1;
        self.undo_stack.push(txn);
        true
    }

    /// Apply an edit op directly to the rope without recording it.
    fn apply_op_raw(&mut self, op: &EditOp) {
        match op {
            EditOp::Insert { pos, text } => {
                let idx = self.pos_to_char(*pos);
                self.rope.insert(idx, text);
            }
            EditOp::Delete { start, end, .. } => {
                let start_idx = self.pos_to_char(*start);
                let end_idx = self.pos_to_char(*end);
                self.rope.remove(start_idx..end_idx);
            }
        }
    }

    // ── Cursor movement ───────────────────────────────────────────────

    /// Move the primary cursor left by one character.
    ///
    /// If `extend` is true, the selection is extended; otherwise collapsed.
    pub fn move_left(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let new_head = if head.col > 0 {
            Position::new(head.line, head.col - 1)
        } else if head.line > 0 {
            // Wrap to end of previous line (excluding newline).
            let prev_line = head.line - 1;
            let len = self.line_len_no_newline(prev_line);
            Position::new(prev_line, len)
        } else {
            head
        };
        self.set_primary_head(new_head, extend);
    }

    /// Move the primary cursor right by one character.
    pub fn move_right(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let line_len = self.line_len_no_newline(head.line);
        let new_head = if head.col < line_len {
            Position::new(head.line, head.col + 1)
        } else if head.line + 1 < self.line_count() {
            Position::new(head.line + 1, 0)
        } else {
            head
        };
        self.set_primary_head(new_head, extend);
    }

    /// Move the primary cursor up by one line.
    pub fn move_up(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        if head.line == 0 {
            self.set_primary_head(Position::new(0, 0), extend);
            return;
        }
        let new_line = head.line - 1;
        let max_col = self.line_len_no_newline(new_line);
        let new_col = head.col.min(max_col);
        self.set_primary_head(Position::new(new_line, new_col), extend);
    }

    /// Move the primary cursor down by one line.
    pub fn move_down(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let last_line = self.line_count().saturating_sub(1);
        if head.line >= last_line {
            let len = self.line_len_no_newline(last_line);
            self.set_primary_head(Position::new(last_line, len), extend);
            return;
        }
        let new_line = head.line + 1;
        let max_col = self.line_len_no_newline(new_line);
        let new_col = head.col.min(max_col);
        self.set_primary_head(Position::new(new_line, new_col), extend);
    }

    /// Move cursor to start of current line.
    pub const fn move_line_start(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        self.set_primary_head(Position::new(head.line, 0), extend);
    }

    /// Move cursor to end of current line.
    pub fn move_line_end(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let len = self.line_len_no_newline(head.line);
        self.set_primary_head(Position::new(head.line, len), extend);
    }

    /// Move cursor to the start of the buffer.
    pub const fn move_buffer_start(&mut self, extend: bool) {
        self.set_primary_head(Position::new(0, 0), extend);
    }

    /// Move cursor to the end of the buffer.
    pub fn move_buffer_end(&mut self, extend: bool) {
        let last_line = self.line_count().saturating_sub(1);
        let len = self.line_len_no_newline(last_line);
        self.set_primary_head(Position::new(last_line, len), extend);
    }

    /// Move cursor left by one word.
    pub fn move_word_left(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let char_idx = self.pos_to_char(head);
        let new_idx = self.find_word_boundary_left(char_idx);
        let new_pos = self.char_to_pos(new_idx);
        self.set_primary_head(new_pos, extend);
    }

    /// Move cursor right by one word.
    pub fn move_word_right(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let char_idx = self.pos_to_char(head);
        let new_idx = self.find_word_boundary_right(char_idx);
        let new_pos = self.char_to_pos(new_idx);
        self.set_primary_head(new_pos, extend);
    }

    // ── File I/O ──────────────────────────────────────────────────────

    /// Save the buffer to its associated file path.
    ///
    /// # Errors
    /// Returns an error if the buffer has no file path or if writing fails.
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("buffer has no file path")?
            .clone();
        let text = self.rope.to_string();
        std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
        self.last_saved_revision = self.current_revision;
        Ok(())
    }

    /// Reload the buffer contents from disk.
    ///
    /// Resets undo/redo history and cursor position.
    ///
    /// # Errors
    /// Returns an error if the buffer has no file path or if reading fails.
    pub fn reload(&mut self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("buffer has no file path")?
            .clone();
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        self.rope = Rope::from_str(&content);
        self.undo_stack = UndoStack::new();
        self.redo_stack = UndoStack::new();
        self.cursor = CursorState::default();
        self.current_revision += 1;
        self.last_saved_revision = self.current_revision;
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────

    /// Line length excluding trailing newline characters.
    fn line_len_no_newline(&self, line_idx: usize) -> usize {
        if line_idx >= self.rope.len_lines() {
            return 0;
        }
        let line = self.rope.line(line_idx);
        let len = line.len_chars();
        let s = line.to_string();
        let trimmed = s.trim_end_matches(&['\n', '\r'][..]);
        len - (s.len() - trimmed.len())
    }

    /// Update the primary cursor head. If `extend` is false, anchor follows.
    const fn set_primary_head(&mut self, head: Position, extend: bool) {
        if extend {
            self.cursor.primary.head = head;
        } else {
            self.cursor.primary = Selection::cursor(head);
        }
    }

    /// Find the word boundary to the left of `char_idx`.
    fn find_word_boundary_left(&self, char_idx: usize) -> usize {
        if char_idx == 0 {
            return 0;
        }
        let mut idx = char_idx - 1;

        // Skip whitespace.
        while idx > 0 && self.char_at_idx(idx).is_whitespace() {
            idx -= 1;
        }
        // Skip word characters.
        while idx > 0 && !self.char_at_idx(idx - 1).is_whitespace() {
            idx -= 1;
        }
        idx
    }

    /// Find the word boundary to the right of `char_idx`.
    fn find_word_boundary_right(&self, char_idx: usize) -> usize {
        let max = self.rope.len_chars();
        if char_idx >= max {
            return max;
        }
        let mut idx = char_idx;

        // Skip current word characters.
        while idx < max && !self.char_at_idx(idx).is_whitespace() {
            idx += 1;
        }
        // Skip whitespace.
        while idx < max && self.char_at_idx(idx).is_whitespace() {
            idx += 1;
        }
        idx
    }

    /// Get the character at a given char index.
    fn char_at_idx(&self, idx: usize) -> char {
        self.rope.char(idx)
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    // ── Constructors & Accessors ──────────────────────────────────────

    #[test]
    fn new_buffer_is_empty() {
        let buf = TextBuffer::new();
        assert_eq!(buf.char_count(), 0);
        assert_eq!(buf.line_count(), 1); // rope always has at least 1 line
        assert!(!buf.is_dirty());
    }

    #[test]
    fn from_str_basic() {
        let buf = TextBuffer::from_text("hello\nworld\n");
        assert_eq!(buf.line_count(), 3); // 2 lines + trailing empty
        assert_eq!(buf.line(0).unwrap(), "hello\n");
        assert_eq!(buf.line(1).unwrap(), "world\n");
        assert_eq!(buf.line(2).unwrap(), "");
    }

    #[test]
    fn line_out_of_bounds() {
        let buf = TextBuffer::from_text("one\ntwo");
        assert!(buf.line(5).is_none());
    }

    #[test]
    fn text_roundtrip() {
        let original = "fn main() {\n    println!(\"hi\");\n}\n";
        let buf = TextBuffer::from_text(original);
        assert_eq!(buf.text(), original);
    }

    // ── Position conversion ───────────────────────────────────────────

    #[test]
    fn pos_to_char_and_back() {
        let buf = TextBuffer::from_text("abc\ndef\nghi");
        // (0,0) -> 0, (0,2) -> 2, (1,0) -> 4 (after 'abc\n'), etc.
        assert_eq!(buf.pos_to_char(Position::new(0, 0)), 0);
        assert_eq!(buf.pos_to_char(Position::new(0, 2)), 2);
        assert_eq!(buf.pos_to_char(Position::new(1, 0)), 4);
        assert_eq!(buf.pos_to_char(Position::new(2, 2)), 10);

        assert_eq!(buf.char_to_pos(0), Position::new(0, 0));
        assert_eq!(buf.char_to_pos(4), Position::new(1, 0));
        assert_eq!(buf.char_to_pos(10), Position::new(2, 2));
    }

    #[test]
    fn pos_clamps_out_of_bounds() {
        let buf = TextBuffer::from_text("ab\ncd");
        // Line 99 clamps to last line.
        let idx = buf.pos_to_char(Position::new(99, 0));
        assert_eq!(buf.char_to_pos(idx), Position::new(1, 0));
        // Col 99 on line 0 clamps to line length.
        let idx2 = buf.pos_to_char(Position::new(0, 99));
        // Line 0 is "ab\n", len_chars = 3
        assert_eq!(idx2, 3);
    }

    // ── Edit operations ───────────────────────────────────────────────

    #[test]
    fn insert_at_start() {
        let mut buf = TextBuffer::from_text("world");
        buf.insert(Position::new(0, 0), "hello ");
        assert_eq!(buf.text(), "hello world");
        assert!(buf.is_dirty());
    }

    #[test]
    fn insert_at_end() {
        let mut buf = TextBuffer::from_text("hello");
        buf.insert(Position::new(0, 5), " world");
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn insert_multiline() {
        let mut buf = TextBuffer::from_text("ac");
        buf.insert(Position::new(0, 1), "b\nd\ne");
        assert_eq!(buf.text(), "ab\nd\nec");
    }

    #[test]
    fn delete_single_char() {
        let mut buf = TextBuffer::from_text("hello");
        buf.delete(Position::new(0, 1), Position::new(0, 2));
        assert_eq!(buf.text(), "hllo");
    }

    #[test]
    fn delete_range() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.delete(Position::new(0, 5), Position::new(0, 11));
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn delete_multiline() {
        let mut buf = TextBuffer::from_text("line1\nline2\nline3");
        buf.delete(Position::new(0, 3), Position::new(2, 2));
        assert_eq!(buf.text(), "linne3");
    }

    #[test]
    fn replace_with_shorter() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.replace(Position::new(0, 0), Position::new(0, 5), "hi");
        assert_eq!(buf.text(), "hi world");
    }

    #[test]
    fn replace_with_longer() {
        let mut buf = TextBuffer::from_text("hi world");
        buf.replace(Position::new(0, 0), Position::new(0, 2), "hello");
        assert_eq!(buf.text(), "hello world");
    }

    // ── Undo / Redo ───────────────────────────────────────────────────

    #[test]
    fn undo_insert() {
        let mut buf = TextBuffer::from_text("hello");
        buf.insert(Position::new(0, 5), " world");
        assert_eq!(buf.text(), "hello world");

        assert!(buf.undo());
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn undo_delete() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.delete(Position::new(0, 5), Position::new(0, 11));
        assert_eq!(buf.text(), "hello");

        assert!(buf.undo());
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn redo_after_undo() {
        let mut buf = TextBuffer::from_text("hello");
        buf.insert(Position::new(0, 5), " world");
        buf.undo();
        assert_eq!(buf.text(), "hello");

        assert!(buf.redo());
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn undo_nothing_returns_false() {
        let mut buf = TextBuffer::new();
        assert!(!buf.undo());
    }

    #[test]
    fn redo_nothing_returns_false() {
        let mut buf = TextBuffer::new();
        assert!(!buf.redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = TextBuffer::from_text("hello");
        buf.insert(Position::new(0, 5), " world");
        buf.undo();
        // New edit should clear redo.
        buf.insert(Position::new(0, 5), "!");
        assert!(!buf.redo());
    }

    #[test]
    fn transaction_groups_ops() {
        let mut buf = TextBuffer::from_text("hello");
        buf.begin_transaction();
        buf.insert(Position::new(0, 5), " ");
        buf.insert(Position::new(0, 6), "world");
        buf.commit_transaction();

        assert_eq!(buf.text(), "hello world");

        // Single undo should revert the entire transaction.
        assert!(buf.undo());
        assert_eq!(buf.text(), "hello");
    }

    // ── Cursor movement ───────────────────────────────────────────────

    #[test]
    fn move_left_basic() {
        let mut buf = TextBuffer::from_text("hello");
        buf.cursor = CursorState::at(Position::new(0, 3));
        buf.move_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 2));
    }

    #[test]
    fn move_left_wraps_line() {
        let mut buf = TextBuffer::from_text("ab\ncd");
        buf.cursor = CursorState::at(Position::new(1, 0));
        buf.move_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 2));
    }

    #[test]
    fn move_left_at_start_stays() {
        let mut buf = TextBuffer::from_text("hello");
        buf.cursor = CursorState::at(Position::new(0, 0));
        buf.move_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 0));
    }

    #[test]
    fn move_right_basic() {
        let mut buf = TextBuffer::from_text("hello");
        buf.cursor = CursorState::at(Position::new(0, 2));
        buf.move_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 3));
    }

    #[test]
    fn move_right_wraps_line() {
        let mut buf = TextBuffer::from_text("ab\ncd");
        buf.cursor = CursorState::at(Position::new(0, 2));
        buf.move_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(1, 0));
    }

    #[test]
    fn move_up_down() {
        let mut buf = TextBuffer::from_text("abc\ndefgh\nij");
        buf.cursor = CursorState::at(Position::new(1, 4));
        buf.move_up(false);
        // Line 0 has 3 chars, col 4 clamps to 3.
        assert_eq!(buf.cursor.primary.head, Position::new(0, 3));

        buf.move_down(false);
        // Back to line 1, col 3 (clamped from 3, line 1 has 5 chars).
        assert_eq!(buf.cursor.primary.head, Position::new(1, 3));
    }

    #[test]
    fn move_line_start_end() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.cursor = CursorState::at(Position::new(0, 5));

        buf.move_line_start(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 0));

        buf.move_line_end(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 11));
    }

    #[test]
    fn move_buffer_start_end() {
        let mut buf = TextBuffer::from_text("line1\nline2\nline3");
        buf.cursor = CursorState::at(Position::new(1, 2));

        buf.move_buffer_start(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 0));

        buf.move_buffer_end(false);
        assert_eq!(buf.cursor.primary.head, Position::new(2, 5));
    }

    #[test]
    fn move_with_extend_creates_selection() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.cursor = CursorState::at(Position::new(0, 0));
        buf.move_right(true);
        buf.move_right(true);
        buf.move_right(true);

        assert_eq!(buf.cursor.primary.anchor, Position::new(0, 0));
        assert_eq!(buf.cursor.primary.head, Position::new(0, 3));
        assert!(!buf.cursor.primary.is_cursor());
    }

    #[test]
    fn move_word_right() {
        let mut buf = TextBuffer::from_text("hello world foo");
        buf.cursor = CursorState::at(Position::new(0, 0));
        buf.move_word_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 6));
    }

    #[test]
    fn move_word_left() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.cursor = CursorState::at(Position::new(0, 11));
        buf.move_word_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 6));
    }

    // ── Dirty flag ────────────────────────────────────────────────────

    #[test]
    fn dirty_after_edit() {
        let mut buf = TextBuffer::from_text("hello");
        assert!(!buf.is_dirty());
        buf.insert(Position::new(0, 0), "x");
        assert!(buf.is_dirty());
    }

    // ── File I/O ──────────────────────────────────────────────────────

    #[test]
    fn save_and_reload() {
        let dir = std::env::temp_dir().join("lune_test_save_reload");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.txt");

        let mut buf = TextBuffer::from_text("hello world");
        buf.file_path = Some(path.clone());
        buf.save().unwrap();
        assert!(!buf.is_dirty());

        // Modify externally.
        std::fs::write(&path, "modified content").unwrap();
        buf.reload().unwrap();
        assert_eq!(buf.text(), "modified content");
        assert!(!buf.is_dirty());

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_without_path_errors() {
        let mut buf = TextBuffer::from_text("hello");
        assert!(buf.save().is_err());
    }
}
