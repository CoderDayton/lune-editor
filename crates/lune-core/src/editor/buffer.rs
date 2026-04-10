//! Text buffer backed by a rope data structure.
//!
//! [`TextBuffer`] is the fundamental editing primitive. It wraps a
//! [`ropey::Rope`] and provides position-aware insert/delete/replace
//! operations, undo/redo, cursor management, and dirty tracking.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ropey::Rope;
use uuid::Uuid;

use crate::position::{CursorState, Position, Selection};
use crate::undo::{
    EditOp, RevisionId, Transaction, UndoStack, UndoState, end_position_after_insert,
};

use std::sync::Arc;

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

    /// Signed distance from the save point in the undo history.
    ///
    /// - `0` means the buffer content matches the last saved state.
    /// - Positive: N forward edits past save.
    /// - Negative: N undos past save.
    ///
    /// When a new edit is made while `save_distance < 0` (i.e. the redo
    /// stack has been forked), the save point is unreachable and
    /// `save_point_lost` is set to `true`.
    save_distance: isize,
    /// Set to `true` when the undo history has been forked past the save
    /// point, making it impossible to return to the saved state via
    /// undo/redo alone.  Cleared on save or reload.
    save_point_lost: bool,

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
            save_distance: 0,
            save_point_lost: false,
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
    /// Uses `Rope::from_reader` to stream directly from disk, avoiding
    /// an intermediate full-file `String` allocation.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub fn from_file(path: &Path) -> Result<Self> {
        let file =
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let rope = Rope::from_reader(BufReader::new(file))
            .with_context(|| format!("reading {}", path.display()))?;
        let mut buf = Self {
            rope,
            ..Self::new()
        };
        buf.file_path = Some(path.to_path_buf());
        Ok(buf)
    }

    // ── Accessors ─────────────────────────────────────────────────────

    /// Number of lines in the buffer (always >= 1).
    #[inline]
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Total number of characters in the buffer.
    #[inline]
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
    #[inline]
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

    /// Extract text between two positions as an owned `String`.
    ///
    /// Positions are ordered internally — callers may pass them in any order.
    #[must_use]
    pub fn text_range(&self, start: Position, end: Position) -> String {
        let s = self.pos_to_char(start);
        let e = self.pos_to_char(end);
        if s <= e {
            self.rope.slice(s..e).to_string()
        } else {
            self.rope.slice(e..s).to_string()
        }
    }

    /// Get a reference to the underlying rope.
    #[must_use]
    pub const fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Whether the buffer has been modified since last save.
    ///
    /// Uses a signed distance from the save point in the undo history.
    /// Undoing all changes back to the saved state correctly reports
    /// the buffer as clean (`save_distance == 0`).  If the undo history
    /// is forked (new edit after undo past save), the save point is
    /// unreachable and the buffer stays dirty until the next save.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.save_point_lost || self.save_distance != 0
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
    #[inline]
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
    #[inline]
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
        self.check_save_point_before_edit();
        self.save_distance += 1;

        let text: Arc<str> = Arc::from(text);
        let op = EditOp::Insert {
            pos,
            text: Arc::clone(&text),
        };

        // Move cursor to end of inserted text.
        let end = end_position_after_insert(pos, &text);
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

        let deleted_text: Arc<str> = Arc::from(self.rope.slice(start_idx..end_idx).to_string());
        self.rope.remove(start_idx..end_idx);
        self.current_revision += 1;
        self.check_save_point_before_edit();
        self.save_distance += 1;

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

    /// Insert the same text at every cursor in the current cursor set.
    ///
    /// When the primary selection is active and there are no secondary
    /// cursors, this behaves like a normal replace-selection edit.
    ///
    /// Returns `false` when the cursor set cannot be edited safely as a
    /// multi-cursor operation.
    pub fn insert_at_cursor_set(&mut self, text: &str) -> bool {
        self.cursor.normalize_secondary();
        let mut targets = self.selection_targets();
        if targets.is_empty() {
            return true;
        }

        let was_in_transaction = self.pending_ops.is_some();
        if !was_in_transaction {
            self.begin_transaction();
        }

        targets.sort_by_key(|&(_, start, end)| (start, end));
        targets.reverse();
        let mut resulting_positions = vec![Position::default(); targets.len()];

        for (idx, start, end) in targets {
            if start != end {
                self.delete(start, end);
            }
            self.insert(start, text);
            resulting_positions[idx] = end_position_after_insert(start, text);
        }

        self.set_cursor_positions(&resulting_positions);

        if !was_in_transaction {
            self.commit_transaction();
        }
        true
    }

    /// Delete one character backward at every cursor in the current cursor
    /// set, or delete the active primary selection when there is only one.
    ///
    /// Returns `false` when the cursor set cannot be edited safely as a
    /// multi-cursor operation.
    pub fn backspace_cursor_set(&mut self) -> bool {
        self.cursor.normalize_secondary();
        let mut deletions: Vec<(usize, Position, Position)> = self
            .selection_targets()
            .into_iter()
            .filter_map(|(idx, start, end)| {
                if start == end {
                    self.backward_delete_range(start)
                        .map(|(del_start, del_end)| (idx, del_start, del_end))
                } else {
                    Some((idx, start, end))
                }
            })
            .collect();
        if deletions.is_empty() {
            return true;
        }

        let was_in_transaction = self.pending_ops.is_some();
        if !was_in_transaction {
            self.begin_transaction();
        }

        deletions.sort_by_key(|&(_, start, end)| (start, end));
        deletions.reverse();
        let mut resulting_positions = self.selection_positions();

        for (idx, start, end) in deletions {
            self.delete(start, end);
            resulting_positions[idx] = start;
        }

        self.set_cursor_positions(&resulting_positions);

        if !was_in_transaction {
            self.commit_transaction();
        }
        true
    }

    /// Delete one character forward at every cursor in the current cursor
    /// set, or delete the active primary selection when there is only one.
    ///
    /// Returns `false` when the cursor set cannot be edited safely as a
    /// multi-cursor operation.
    pub fn delete_cursor_set(&mut self) -> bool {
        self.cursor.normalize_secondary();
        let mut deletions: Vec<(usize, Position, Position)> = self
            .selection_targets()
            .into_iter()
            .filter_map(|(idx, start, end)| {
                if start == end {
                    self.forward_delete_range(start)
                        .map(|(del_start, del_end)| (idx, del_start, del_end))
                } else {
                    Some((idx, start, end))
                }
            })
            .collect();
        if deletions.is_empty() {
            return true;
        }

        let was_in_transaction = self.pending_ops.is_some();
        if !was_in_transaction {
            self.begin_transaction();
        }

        deletions.sort_by_key(|&(_, start, end)| (start, end));
        deletions.reverse();
        let mut resulting_positions = self.selection_positions();

        for (idx, start, end) in deletions {
            self.delete(start, end);
            resulting_positions[idx] = start;
        }

        self.set_cursor_positions(&resulting_positions);

        if !was_in_transaction {
            self.commit_transaction();
        }
        true
    }

    /// Add or remove a secondary cursor at `pos`.
    ///
    /// The position is clamped to the nearest valid cursor location.
    pub fn toggle_secondary_cursor(&mut self, pos: Position) -> bool {
        self.cursor.normalize_secondary_cursors();
        let clamped = self.clamp_position(pos);
        self.cursor.toggle_secondary_cursor(clamped)
    }

    /// Clear all secondary cursors.
    pub fn clear_secondary_cursors(&mut self) {
        self.cursor.clear_secondary();
    }

    /// Replace the current cursor state with a rectangular block selection.
    pub fn set_block_selection(&mut self, anchor: Position, head: Position) {
        let start_line = anchor.line.min(head.line);
        let end_line = anchor.line.max(head.line);
        let start_col = anchor.col.min(head.col);
        let end_col = anchor.col.max(head.col);

        let mut selections = Vec::with_capacity(end_line.saturating_sub(start_line) + 1);
        for line in start_line..=end_line {
            let line_len = self.line_len_no_newline(line);
            let a = Position::new(line, start_col.min(line_len));
            let b = Position::new(line, end_col.min(line_len));
            selections.push(Selection::new(a, b));
        }

        let Some(primary) = selections.first().cloned() else {
            return;
        };
        self.cursor = CursorState {
            primary,
            secondary: selections.into_iter().skip(1).collect(),
        };
        self.cursor.normalize_secondary();
    }

    /// Add a secondary cursor above the current top-most cursor.
    pub fn add_cursor_above(&mut self) -> bool {
        self.cursor.normalize_secondary_cursors();
        let Some(positions) = self.cursor.cursor_positions() else {
            return false;
        };
        let Some(&topmost) = positions.iter().min() else {
            return false;
        };
        if topmost.line == 0 {
            return false;
        }

        let target_line = topmost.line - 1;
        let target = Position::new(target_line, topmost.col.min(self.line_len_no_newline(target_line)));
        self.cursor.add_secondary_cursor(target)
    }

    /// Add a secondary cursor below the current bottom-most cursor.
    pub fn add_cursor_below(&mut self) -> bool {
        self.cursor.normalize_secondary_cursors();
        let Some(positions) = self.cursor.cursor_positions() else {
            return false;
        };
        let Some(&bottommost) = positions.iter().max() else {
            return false;
        };
        if bottommost.line + 1 >= self.line_count() {
            return false;
        }

        let target_line = bottommost.line + 1;
        let target = Position::new(
            target_line,
            bottommost.col.min(self.line_len_no_newline(target_line)),
        );
        self.cursor.add_secondary_cursor(target)
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
        // Move one step closer to (or past) the save point.
        self.save_distance -= 1;
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
        // Move one step away from the save point (back toward where we were).
        self.save_distance += 1;
        self.undo_stack.push(txn);
        true
    }

    /// Check whether making a new edit will fork the undo history past the
    /// save point.  Must be called BEFORE incrementing `save_distance`.
    ///
    /// If the redo stack is non-empty and we are currently behind or at the
    /// save point (`save_distance <= 0`), clearing the redo stack (which
    /// happens when a new edit is recorded) will remove the only path back
    /// to the saved state — so we mark the save point as lost.
    fn check_save_point_before_edit(&mut self) {
        if !self.save_point_lost && !self.redo_stack.is_empty() && self.save_distance < 0 {
            self.save_point_lost = true;
        }
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

    // ── Undo state persistence ────────────────────────────────────────

    /// Extract a serializable snapshot of the undo/redo history.
    ///
    /// Up to `max_entries` transactions are captured from each stack.
    /// The returned `UndoState` includes a content hash so that
    /// `restore_undo_state` can reject stale snapshots.
    #[must_use]
    pub fn extract_undo_state(&self, max_entries: usize) -> UndoState {
        let take = |stack: &UndoStack| -> Vec<Transaction> {
            let entries = stack.entries();
            let skip = entries.len().saturating_sub(max_entries);
            entries.iter().skip(skip).cloned().collect()
        };
        UndoState {
            undo_entries: take(&self.undo_stack),
            redo_entries: take(&self.redo_stack),
            content_hash: self.content_hash(),
        }
    }

    /// Restore undo/redo history from a persisted snapshot.
    ///
    /// Returns `true` if the content hash matches and the state was
    /// restored, `false` if the buffer contents have changed since the
    /// snapshot was taken (in which case nothing is modified).
    pub fn restore_undo_state(&mut self, state: UndoState) -> bool {
        if state.content_hash != self.content_hash() {
            return false;
        }
        self.undo_stack.replace(state.undo_entries.into());
        self.redo_stack.replace(state.redo_entries.into());
        true
    }

    /// FNV-1a hash of the full buffer content for mismatch detection.
    fn content_hash(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for chunk in self.rope.chunks() {
            for byte in chunk.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
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

    /// Move cursor to the smart "home" position on the current line.
    ///
    /// The first press moves to the first non-whitespace character.
    /// If the cursor is already there, it moves to column 0.
    pub fn move_line_home(&mut self, extend: bool) {
        let head = self.cursor.primary.head;
        let text_col = self.first_non_whitespace_col(head.line);
        let target_col = if head.col == text_col { 0 } else { text_col };
        self.set_primary_head(Position::new(head.line, target_col), extend);
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
    /// Uses `rope.write_to()` to stream directly to disk, avoiding
    /// an intermediate full-buffer `String` allocation.
    ///
    /// # Errors
    /// Returns an error if the buffer has no file path or if writing fails.
    pub fn save(&mut self) -> Result<()> {
        let path = self.file_path.as_ref().context("buffer has no file path")?;
        let file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        let mut writer = std::io::BufWriter::new(file);
        self.rope
            .write_to(&mut writer)
            .with_context(|| format!("writing {}", path.display()))?;
        // Reset save-point tracking: we are now at the save point.
        self.save_distance = 0;
        self.save_point_lost = false;
        Ok(())
    }

    /// Reload the buffer contents from disk.
    ///
    /// Resets undo/redo history and cursor position. Streams from disk
    /// via `Rope::from_reader` to avoid a full-file `String` allocation.
    ///
    /// # Errors
    /// Returns an error if the buffer has no file path or if reading fails.
    pub fn reload(&mut self) -> Result<()> {
        let path = self.file_path.as_ref().context("buffer has no file path")?;
        let file =
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        self.rope = Rope::from_reader(BufReader::new(file))
            .with_context(|| format!("reading {}", path.display()))?;
        self.undo_stack = UndoStack::new();
        self.redo_stack = UndoStack::new();
        self.cursor = CursorState::default();
        self.current_revision += 1;
        // Reload = fresh save point.
        self.save_distance = 0;
        self.save_point_lost = false;
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────

    /// Line length excluding trailing newline characters.
    ///
    /// Avoids allocating a `String` by counting trailing `\n`/`\r` chars
    /// directly from the rope slice.  Used for cursor column clamping
    /// (the newline is not a valid cursor position).
    #[inline]
    #[must_use]
    pub fn line_len_no_newline(&self, line_idx: usize) -> usize {
        if line_idx >= self.rope.len_lines() {
            return 0;
        }
        let line = self.rope.line(line_idx);
        let len = line.len_chars();
        let mut trailing = 0;
        for ch in line.chars_at(len).reversed() {
            if ch == '\n' || ch == '\r' {
                trailing += 1;
            } else {
                break;
            }
        }
        len - trailing
    }

    /// Column of the first non-whitespace character on the line.
    ///
    /// Returns the line length when the line is blank or contains only
    /// whitespace before the newline.
    #[must_use]
    pub fn first_non_whitespace_col(&self, line_idx: usize) -> usize {
        if line_idx >= self.rope.len_lines() {
            return 0;
        }

        for (col, ch) in self.rope.line(line_idx).chars().enumerate() {
            if matches!(ch, '\n' | '\r') {
                break;
            }
            if !ch.is_whitespace() {
                return col;
            }
        }
        0
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
        let mut idx = char_idx;

        while idx > 0 && matches!(Self::char_kind(self.char_at_idx(idx - 1)), CharKind::Whitespace) {
            idx -= 1;
        }
        if idx == 0 {
            return 0;
        }

        let kind = Self::char_kind(self.char_at_idx(idx - 1));
        while idx > 0 && Self::char_kind(self.char_at_idx(idx - 1)) == kind {
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

        let kind = Self::char_kind(self.char_at_idx(idx));
        while idx < max && Self::char_kind(self.char_at_idx(idx)) == kind {
            idx += 1;
        }
        idx
    }

    /// Get the character at a given char index.
    fn char_at_idx(&self, idx: usize) -> char {
        self.rope.char(idx)
    }

    fn selection_targets(&self) -> Vec<(usize, Position, Position)> {
        std::iter::once(&self.cursor.primary)
            .chain(self.cursor.secondary.iter())
            .enumerate()
            .map(|(idx, sel)| {
                let (start, end) = if sel.is_cursor() {
                    (sel.head, sel.head)
                } else {
                    sel.ordered()
                };
                (idx, start, end)
            })
            .collect()
    }

    fn selection_positions(&self) -> Vec<Position> {
        std::iter::once(&self.cursor.primary)
            .chain(self.cursor.secondary.iter())
            .map(|sel| if sel.is_cursor() { sel.head } else { sel.ordered().0 })
            .collect()
    }

    fn char_kind(ch: char) -> CharKind {
        if ch.is_whitespace() {
            CharKind::Whitespace
        } else if ch.is_alphanumeric() || ch == '_' {
            CharKind::Word
        } else {
            CharKind::Punctuation
        }
    }

    fn backward_delete_range(&self, pos: Position) -> Option<(Position, Position)> {
        if pos.col > 0 {
            Some((Position::new(pos.line, pos.col - 1), pos))
        } else if pos.line > 0 {
            let prev_line = pos.line - 1;
            let prev_len = self.line_len_no_newline(prev_line);
            Some((Position::new(prev_line, prev_len), pos))
        } else {
            None
        }
    }

    fn forward_delete_range(&self, pos: Position) -> Option<(Position, Position)> {
        let line_len = self.line_len_no_newline(pos.line);
        if pos.col < line_len {
            Some((pos, Position::new(pos.line, pos.col + 1)))
        } else if pos.line + 1 < self.line_count() {
            Some((pos, Position::new(pos.line + 1, 0)))
        } else {
            None
        }
    }

    fn clamp_position(&self, pos: Position) -> Position {
        let line = pos.line.min(self.line_count().saturating_sub(1));
        Position::new(line, pos.col.min(self.line_len_no_newline(line)))
    }

    fn set_cursor_positions(&mut self, positions: &[Position]) {
        let Some((&primary, secondary)) = positions.split_first() else {
            self.cursor = CursorState::default();
            return;
        };

        self.cursor = CursorState {
            primary: Selection::cursor(primary),
            secondary: secondary.iter().copied().map(Selection::cursor).collect(),
        };
        self.cursor.normalize_secondary_cursors();
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharKind {
    Whitespace,
    Word,
    Punctuation,
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

    #[test]
    fn insert_at_cursor_set_applies_to_all_cursors() {
        let mut buf = TextBuffer::from_text("abc\ndef");
        buf.cursor = CursorState::at(Position::new(0, 1));
        assert!(buf.toggle_secondary_cursor(Position::new(1, 1)));

        assert!(buf.insert_at_cursor_set("!"));
        assert_eq!(buf.text(), "a!bc\nd!ef");
        assert_eq!(buf.cursor.primary.head, Position::new(0, 2));
        assert_eq!(
            buf.cursor.secondary,
            vec![Selection::cursor(Position::new(1, 2))]
        );
    }

    #[test]
    fn backspace_cursor_set_deletes_at_all_cursors() {
        let mut buf = TextBuffer::from_text("abc\ndef");
        buf.cursor = CursorState::at(Position::new(0, 2));
        assert!(buf.toggle_secondary_cursor(Position::new(1, 2)));

        assert!(buf.backspace_cursor_set());
        assert_eq!(buf.text(), "ac\ndf");
        assert_eq!(buf.cursor.primary.head, Position::new(0, 1));
        assert_eq!(
            buf.cursor.secondary,
            vec![Selection::cursor(Position::new(1, 1))]
        );
    }

    #[test]
    fn delete_cursor_set_deletes_forward_at_all_cursors() {
        let mut buf = TextBuffer::from_text("abc\ndef");
        buf.cursor = CursorState::at(Position::new(0, 1));
        assert!(buf.toggle_secondary_cursor(Position::new(1, 1)));

        assert!(buf.delete_cursor_set());
        assert_eq!(buf.text(), "ac\ndf");
        assert_eq!(buf.cursor.primary.head, Position::new(0, 1));
        assert_eq!(
            buf.cursor.secondary,
            vec![Selection::cursor(Position::new(1, 1))]
        );
    }

    #[test]
    fn insert_at_cursor_set_replaces_block_selection_ranges() {
        let mut buf = TextBuffer::from_text("alpha\nbeta\ngamma");
        buf.set_block_selection(Position::new(0, 1), Position::new(2, 3));

        assert!(buf.insert_at_cursor_set("Z"));
        assert_eq!(buf.text(), "aZha\nbZa\ngZma");
        assert_eq!(buf.cursor.primary.head, Position::new(0, 2));
        assert_eq!(
            buf.cursor.secondary,
            vec![
                Selection::cursor(Position::new(1, 2)),
                Selection::cursor(Position::new(2, 2)),
            ]
        );
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
    fn move_line_home_toggles_between_indent_and_start() {
        let mut buf = TextBuffer::from_text("    hello");
        buf.cursor = CursorState::at(Position::new(0, 7));

        buf.move_line_home(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 4));

        buf.move_line_home(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 0));
    }

    #[test]
    fn move_line_home_on_blank_line_stays_at_start() {
        let mut buf = TextBuffer::from_text("    \nnext");
        buf.cursor = CursorState::at(Position::new(0, 3));

        buf.move_line_home(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 0));
    }

    #[test]
    fn move_line_home_with_extend_keeps_anchor() {
        let mut buf = TextBuffer::from_text("    hello");
        buf.cursor = CursorState::at(Position::new(0, 8));

        buf.move_line_home(true);
        assert_eq!(buf.cursor.primary.anchor, Position::new(0, 8));
        assert_eq!(buf.cursor.primary.head, Position::new(0, 4));
    }

    #[test]
    fn add_cursor_above_and_below_clamp_columns() {
        let mut buf = TextBuffer::from_text("wide\nx\nthree");
        buf.cursor = CursorState::at(Position::new(1, 1));

        assert!(buf.add_cursor_above());
        assert!(buf.add_cursor_below());
        assert_eq!(
            buf.cursor.secondary,
            vec![
                Selection::cursor(Position::new(0, 1)),
                Selection::cursor(Position::new(2, 1)),
            ]
        );
    }

    #[test]
    fn set_block_selection_creates_one_selection_per_line() {
        let mut buf = TextBuffer::from_text("alpha\nbeta\ngamma");

        buf.set_block_selection(Position::new(0, 1), Position::new(2, 3));

        assert_eq!(
            buf.cursor.primary,
            Selection::new(Position::new(0, 1), Position::new(0, 3))
        );
        assert_eq!(
            buf.cursor.secondary,
            vec![
                Selection::new(Position::new(1, 1), Position::new(1, 3)),
                Selection::new(Position::new(2, 1), Position::new(2, 3)),
            ]
        );
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
        assert_eq!(buf.cursor.primary.head, Position::new(0, 5));
    }

    #[test]
    fn move_word_left() {
        let mut buf = TextBuffer::from_text("hello world");
        buf.cursor = CursorState::at(Position::new(0, 11));
        buf.move_word_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 6));
    }

    #[test]
    fn move_word_respects_punctuation_boundaries() {
        let mut buf = TextBuffer::from_text("foo.bar");
        buf.cursor = CursorState::at(Position::new(0, 0));

        buf.move_word_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 3));

        buf.move_word_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 4));

        buf.move_word_right(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 7));

        buf.move_word_left(false);
        assert_eq!(buf.cursor.primary.head, Position::new(0, 4));
    }

    // ── Dirty flag ────────────────────────────────────────────────────

    #[test]
    fn dirty_after_edit() {
        let mut buf = TextBuffer::from_text("hello");
        assert!(!buf.is_dirty());
        buf.insert(Position::new(0, 0), "x");
        assert!(buf.is_dirty());
    }

    #[test]
    fn undo_back_to_saved_is_clean() {
        let mut buf = TextBuffer::from_text("hello");
        assert!(!buf.is_dirty());
        buf.insert(Position::new(0, 5), " world");
        assert!(buf.is_dirty());
        // Undo the insert — content is back to "hello".
        assert!(buf.undo());
        assert!(
            !buf.is_dirty(),
            "buffer should be clean after undoing all edits"
        );
    }

    #[test]
    fn undo_redo_returns_to_dirty() {
        let mut buf = TextBuffer::from_text("hello");
        buf.insert(Position::new(0, 5), "!");
        assert!(buf.is_dirty());
        assert!(buf.undo());
        assert!(!buf.is_dirty());
        // Redo re-applies — dirty again.
        assert!(buf.redo());
        assert!(buf.is_dirty());
    }

    #[test]
    fn multiple_undos_back_to_saved() {
        let mut buf = TextBuffer::from_text("a");
        buf.insert(Position::new(0, 1), "b");
        buf.insert(Position::new(0, 2), "c");
        assert!(buf.is_dirty());
        assert!(buf.undo()); // remove "c"
        assert!(buf.is_dirty()); // still dirty ("ab" != "a")
        assert!(buf.undo()); // remove "b"
        assert!(!buf.is_dirty()); // clean ("a" == "a")
    }

    #[test]
    fn undo_to_saved_then_new_edit_is_still_trackable() {
        // Undo back to save point, then make a different edit.
        // Since the save point is at distance 0 and the new edit brings
        // us to distance 1, undoing the new edit returns to 0 (clean).
        let mut buf = TextBuffer::from_text("a");
        buf.insert(Position::new(0, 1), "b");
        assert!(buf.undo()); // back to "a" — clean
        assert!(!buf.is_dirty());
        buf.insert(Position::new(0, 1), "x"); // "ax"
        assert!(buf.is_dirty());
        assert!(buf.undo()); // back to "a" — clean
        assert!(!buf.is_dirty());
    }

    #[test]
    fn new_edit_after_undo_past_save_loses_save_point() {
        // Save at "ab", then undo past save to "a", then make a new edit.
        // The redo stack (containing "b") is cleared, making the save
        // point ("ab") unreachable.
        let dir = std::env::temp_dir().join("lune_test_fork");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("fork_test.txt");
        std::fs::write(&path, "ab").unwrap();

        let mut buf = TextBuffer::from_file(&path).unwrap();
        assert!(!buf.is_dirty()); // "ab" on disk = clean
        // Undo is empty, so insert then save to establish save point.
        buf.insert(Position::new(0, 2), "c"); // "abc"
        buf.save().unwrap(); // save point at "abc"
        assert!(!buf.is_dirty());

        // Now undo past the save point.
        assert!(buf.undo()); // "ab" — save_distance = -1
        assert!(buf.is_dirty()); // we're behind the save point
        // Make a new edit — forks history, redo stack cleared.
        buf.insert(Position::new(0, 2), "x"); // "abx"
        assert!(buf.is_dirty());
        // Undo the "x" — content is "ab", save_distance = -1 again,
        // but save_point_lost = true because redo stack was forked.
        assert!(buf.undo());
        assert!(
            buf.is_dirty(),
            "save point should be lost after history fork"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_resets_dirty_tracking() {
        let dir = std::env::temp_dir().join("lune_test_save_dirty");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("dirty_test.txt");
        std::fs::write(&path, "original").unwrap();

        let mut buf = TextBuffer::from_file(&path).unwrap();
        buf.insert(Position::new(0, 0), "X");
        assert!(buf.is_dirty());
        buf.save().unwrap();
        assert!(!buf.is_dirty());
        // Edit after save, then undo — should be clean again.
        buf.insert(Position::new(0, 0), "Y");
        assert!(buf.is_dirty());
        assert!(buf.undo());
        assert!(!buf.is_dirty());

        let _ = std::fs::remove_dir_all(&dir);
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
