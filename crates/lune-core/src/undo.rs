//! Undo/redo transaction system.
//!
//! Each edit operation is captured as an [`EditOp`]. Multiple ops can be
//! grouped into a [`Transaction`] which is treated as a single undo step.

use crate::position::{CursorState, Position};

/// Monotonically increasing revision identifier.
pub type RevisionId = u64;

/// A single atomic edit operation that can be applied or reversed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditOp {
    /// Text was inserted at a position.
    Insert {
        /// Where the text was inserted.
        pos: Position,
        /// The text that was inserted.
        text: String,
    },
    /// Text was deleted from a range.
    Delete {
        /// Start of the deleted range.
        start: Position,
        /// End of the deleted range.
        end: Position,
        /// The text that was deleted (needed for undo).
        deleted_text: String,
    },
}

impl EditOp {
    /// Produce the inverse operation (for undo).
    #[must_use]
    pub fn inverse(&self) -> Self {
        match self {
            Self::Insert { pos, text } => {
                // To invert an insert, we need to figure out the end position.
                // We calculate it from the inserted text.
                let end = end_position_after_insert(*pos, text);
                Self::Delete {
                    start: *pos,
                    end,
                    deleted_text: text.clone(),
                }
            }
            Self::Delete {
                start,
                deleted_text,
                ..
            } => Self::Insert {
                pos: *start,
                text: deleted_text.clone(),
            },
        }
    }
}

/// Calculate the position after inserting `text` at `pos`.
#[must_use]
pub fn end_position_after_insert(pos: Position, text: &str) -> Position {
    let newline_count = text.chars().filter(|&c| c == '\n').count();
    if newline_count == 0 {
        Position::new(pos.line, pos.col + text.chars().count())
    } else {
        let last_line_chars = text
            .rsplit_once('\n')
            .map_or_else(|| text.chars().count(), |(_, after)| after.chars().count());
        Position::new(pos.line + newline_count, last_line_chars)
    }
}

/// A group of edit operations that form a single undo step.
#[derive(Clone, Debug)]
pub struct Transaction {
    /// The revision this transaction produced.
    pub revision: RevisionId,
    /// The operations in this transaction, in order.
    pub ops: Vec<EditOp>,
    /// Cursor state before this transaction.
    pub cursor_before: CursorState,
    /// Cursor state after this transaction.
    pub cursor_after: CursorState,
}

/// A bounded stack of transactions for undo or redo history.
#[derive(Debug)]
pub struct UndoStack {
    entries: Vec<Transaction>,
    max_entries: usize,
}

impl UndoStack {
    /// Maximum entries in the undo stack.
    const DEFAULT_MAX: usize = 10_000;

    /// Create a new undo stack with the default capacity.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: Self::DEFAULT_MAX,
        }
    }

    /// Create a new undo stack with a specified maximum size.
    #[must_use]
    pub const fn with_max(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
        }
    }

    /// Push a transaction onto the stack.
    ///
    /// If the stack exceeds `max_entries`, the oldest entry is discarded.
    pub fn push(&mut self, transaction: Transaction) {
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(transaction);
    }

    /// Pop the most recent transaction from the stack.
    pub fn pop(&mut self) -> Option<Transaction> {
        self.entries.pop()
    }

    /// Clear the stack entirely.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries in the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the stack is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    #[test]
    fn edit_op_inverse_insert() {
        let op = EditOp::Insert {
            pos: Position::new(0, 0),
            text: "hello".to_string(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            EditOp::Delete {
                start: Position::new(0, 0),
                end: Position::new(0, 5),
                deleted_text: "hello".to_string(),
            }
        );
    }

    #[test]
    fn edit_op_inverse_insert_multiline() {
        let op = EditOp::Insert {
            pos: Position::new(1, 3),
            text: "ab\ncd\nef".to_string(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            EditOp::Delete {
                start: Position::new(1, 3),
                end: Position::new(3, 2),
                deleted_text: "ab\ncd\nef".to_string(),
            }
        );
    }

    #[test]
    fn edit_op_inverse_delete() {
        let op = EditOp::Delete {
            start: Position::new(0, 2),
            end: Position::new(0, 5),
            deleted_text: "llo".to_string(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            EditOp::Insert {
                pos: Position::new(0, 2),
                text: "llo".to_string(),
            }
        );
    }

    #[test]
    fn undo_stack_push_pop() {
        let mut stack = UndoStack::new();
        assert!(stack.is_empty());

        let txn = Transaction {
            revision: 1,
            ops: vec![EditOp::Insert {
                pos: Position::new(0, 0),
                text: "a".to_string(),
            }],
            cursor_before: CursorState::default(),
            cursor_after: CursorState::default(),
        };
        stack.push(txn);
        assert_eq!(stack.len(), 1);

        let popped = stack.pop().unwrap();
        assert_eq!(popped.revision, 1);
        assert!(stack.is_empty());
    }

    #[test]
    fn undo_stack_respects_max() {
        let mut stack = UndoStack::with_max(3);
        for i in 0..5 {
            stack.push(Transaction {
                revision: i,
                ops: vec![],
                cursor_before: CursorState::default(),
                cursor_after: CursorState::default(),
            });
        }
        assert_eq!(stack.len(), 3);
        // Oldest entries should have been discarded.
        let t = stack.pop().unwrap();
        assert_eq!(t.revision, 4);
    }

    #[test]
    fn end_position_single_line() {
        let pos = Position::new(2, 5);
        let end = end_position_after_insert(pos, "abc");
        assert_eq!(end, Position::new(2, 8));
    }

    #[test]
    fn end_position_multiline() {
        let pos = Position::new(0, 3);
        let end = end_position_after_insert(pos, "a\nbcd\ne");
        assert_eq!(end, Position::new(2, 1));
    }
}
