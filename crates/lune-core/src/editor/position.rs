//! Cursor position and selection types.
//!
//! All positions are 0-based: line 0 is the first line, col 0 is the first
//! byte offset within that line.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// A position in a text buffer, identified by line and column.
///
/// Both `line` and `col` are 0-based. `col` represents a character (char)
/// offset within the line, not a byte offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    /// 0-based line index.
    pub line: usize,
    /// 0-based character offset within the line.
    pub col: usize,
}

impl Position {
    /// Create a new position.
    #[inline]
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

impl Ord for Position {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.line.cmp(&other.line).then(self.col.cmp(&other.col))
    }
}

impl PartialOrd for Position {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A selection in a text buffer, defined by an anchor and a head (cursor).
///
/// When `anchor == head`, the selection is collapsed to a cursor (no selected
/// text). The anchor is where the selection started; the head is where the
/// cursor currently sits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Where the selection started.
    pub anchor: Position,
    /// Where the cursor currently is.
    pub head: Position,
}

impl Selection {
    /// Create a collapsed selection (cursor) at the given position.
    #[inline]
    #[must_use]
    pub const fn cursor(pos: Position) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// Create a selection spanning from `anchor` to `head`.
    #[inline]
    #[must_use]
    pub const fn new(anchor: Position, head: Position) -> Self {
        Self { anchor, head }
    }

    /// Returns `true` if the selection is collapsed (no text selected).
    #[inline]
    #[must_use]
    pub fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    /// Returns the selection bounds in document order: `(start, end)` where
    /// `start <= end`.
    #[inline]
    #[must_use]
    pub fn ordered(&self) -> (Position, Position) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Returns `true` if `pos` falls within this selection (inclusive of
    /// start, exclusive of end).
    #[inline]
    #[must_use]
    pub fn contains(&self, pos: Position) -> bool {
        let (start, end) = self.ordered();
        pos >= start && pos < end
    }

    /// Returns `true` if this selection is a collapsed cursor at `pos`.
    #[inline]
    #[must_use]
    pub fn is_cursor_at(&self, pos: Position) -> bool {
        self.is_cursor() && self.head == pos
    }
}

/// The full cursor state for a buffer, supporting a primary selection and
/// optional secondary cursors for multi-cursor editing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    /// The primary selection / cursor.
    pub primary: Selection,
    /// Additional cursors for multi-cursor editing (future).
    pub secondary: Vec<Selection>,
}

impl CursorState {
    /// Create a cursor state with a single cursor at the given position.
    #[inline]
    #[must_use]
    pub const fn at(pos: Position) -> Self {
        Self {
            primary: Selection::cursor(pos),
            secondary: Vec::new(),
        }
    }

    /// Create a cursor state from a primary selection.
    #[inline]
    #[must_use]
    pub const fn from_selection(sel: Selection) -> Self {
        Self {
            primary: sel,
            secondary: Vec::new(),
        }
    }

    /// Returns `true` when the primary and all secondary selections are
    /// collapsed cursors.
    #[must_use]
    pub fn all_cursors(&self) -> bool {
        self.primary.is_cursor() && self.secondary.iter().all(Selection::is_cursor)
    }

    /// Returns all cursor positions in primary-then-secondary order when the
    /// whole cursor set is collapsed.
    #[must_use]
    pub fn cursor_positions(&self) -> Option<Vec<Position>> {
        if !self.all_cursors() {
            return None;
        }

        let mut positions = Vec::with_capacity(self.secondary.len() + 1);
        positions.push(self.primary.head);
        positions.extend(self.secondary.iter().map(|sel| sel.head));
        Some(positions)
    }

    /// Remove duplicate secondary cursors and normalize them in document order.
    ///
    /// The primary cursor remains authoritative; any secondary at the same
    /// position is discarded.
    pub fn normalize_secondary(&mut self) {
        self.secondary.sort_by_key(Selection::ordered);
        self.secondary
            .dedup_by(|a, b| a.anchor == b.anchor && a.head == b.head);
        self.secondary
            .retain(|sel| sel.anchor != self.primary.anchor || sel.head != self.primary.head);
    }

    /// Remove duplicate secondary cursors and normalize them in document order.
    ///
    /// The primary cursor remains authoritative; any secondary at the same
    /// position is discarded.
    pub fn normalize_secondary_cursors(&mut self) {
        self.secondary.retain(Selection::is_cursor);
        self.normalize_secondary();
        self.secondary.retain(|sel| sel.head != self.primary.head);
    }

    /// Add a secondary cursor if it does not already exist.
    ///
    /// Returns `false` when the cursor set is not fully collapsed or when the
    /// target matches the primary cursor.
    pub fn add_secondary_cursor(&mut self, pos: Position) -> bool {
        if !self.all_cursors() || self.primary.head == pos {
            return false;
        }
        if self.secondary.iter().any(|sel| sel.is_cursor_at(pos)) {
            return false;
        }
        self.secondary.push(Selection::cursor(pos));
        self.normalize_secondary_cursors();
        true
    }

    /// Remove the secondary cursor at `pos`, if any.
    pub fn remove_secondary_cursor(&mut self, pos: Position) -> bool {
        let len_before = self.secondary.len();
        self.secondary.retain(|sel| !sel.is_cursor_at(pos));
        len_before != self.secondary.len()
    }

    /// Toggle a secondary cursor at `pos`.
    ///
    /// Returns `false` when the cursor set is not fully collapsed or when the
    /// position is the primary cursor.
    pub fn toggle_secondary_cursor(&mut self, pos: Position) -> bool {
        if !self.all_cursors() || self.primary.head == pos {
            return false;
        }
        if self.remove_secondary_cursor(pos) {
            true
        } else {
            self.add_secondary_cursor(pos)
        }
    }

    /// Drop all secondary cursors.
    pub fn clear_secondary(&mut self) {
        self.secondary.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_ordering() {
        let a = Position::new(0, 0);
        let b = Position::new(0, 5);
        let c = Position::new(1, 0);
        let d = Position::new(1, 3);

        assert!(a < b);
        assert!(b < c);
        assert!(c < d);
        assert!(a < d);
        assert_eq!(a, Position::new(0, 0));
    }

    #[test]
    fn selection_is_cursor() {
        let sel = Selection::cursor(Position::new(1, 2));
        assert!(sel.is_cursor());

        let sel2 = Selection::new(Position::new(0, 0), Position::new(1, 0));
        assert!(!sel2.is_cursor());
    }

    #[test]
    fn selection_ordered() {
        // Forward selection.
        let sel = Selection::new(Position::new(0, 0), Position::new(1, 5));
        let (start, end) = sel.ordered();
        assert_eq!(start, Position::new(0, 0));
        assert_eq!(end, Position::new(1, 5));

        // Backward selection.
        let sel2 = Selection::new(Position::new(1, 5), Position::new(0, 0));
        let (start2, end2) = sel2.ordered();
        assert_eq!(start2, Position::new(0, 0));
        assert_eq!(end2, Position::new(1, 5));
    }

    #[test]
    fn selection_contains() {
        let sel = Selection::new(Position::new(0, 2), Position::new(0, 8));

        assert!(!sel.contains(Position::new(0, 0)));
        assert!(!sel.contains(Position::new(0, 1)));
        assert!(sel.contains(Position::new(0, 2)));
        assert!(sel.contains(Position::new(0, 5)));
        assert!(sel.contains(Position::new(0, 7)));
        // End is exclusive.
        assert!(!sel.contains(Position::new(0, 8)));
        assert!(!sel.contains(Position::new(0, 9)));
    }

    #[test]
    fn selection_contains_multiline() {
        let sel = Selection::new(Position::new(1, 3), Position::new(3, 2));

        assert!(!sel.contains(Position::new(0, 0)));
        assert!(!sel.contains(Position::new(1, 2)));
        assert!(sel.contains(Position::new(1, 3)));
        assert!(sel.contains(Position::new(2, 0)));
        assert!(sel.contains(Position::new(3, 1)));
        assert!(!sel.contains(Position::new(3, 2)));
        assert!(!sel.contains(Position::new(4, 0)));
    }

    #[test]
    fn cursor_collapsed_contains_nothing() {
        let sel = Selection::cursor(Position::new(1, 1));
        assert!(!sel.contains(Position::new(1, 1)));
    }

    #[test]
    fn cursor_state_defaults() {
        let cs = CursorState::at(Position::new(0, 0));
        assert!(cs.primary.is_cursor());
        assert!(cs.secondary.is_empty());
    }

    #[test]
    fn normalize_secondary_cursors_sorts_and_dedupes() {
        let mut cs = CursorState::at(Position::new(1, 2));
        cs.secondary = vec![
            Selection::cursor(Position::new(2, 0)),
            Selection::cursor(Position::new(1, 2)),
            Selection::cursor(Position::new(0, 4)),
            Selection::cursor(Position::new(2, 0)),
        ];

        cs.normalize_secondary_cursors();

        assert_eq!(
            cs.secondary,
            vec![
                Selection::cursor(Position::new(0, 4)),
                Selection::cursor(Position::new(2, 0)),
            ]
        );
    }

    #[test]
    fn normalize_secondary_preserves_non_cursor_selections() {
        let mut cs = CursorState::at(Position::new(0, 0));
        cs.secondary = vec![
            Selection::new(Position::new(2, 1), Position::new(2, 3)),
            Selection::new(Position::new(1, 0), Position::new(1, 2)),
        ];

        cs.normalize_secondary();

        assert_eq!(
            cs.secondary,
            vec![
                Selection::new(Position::new(1, 0), Position::new(1, 2)),
                Selection::new(Position::new(2, 1), Position::new(2, 3)),
            ]
        );
    }

    #[test]
    fn toggle_secondary_cursor_adds_and_removes() {
        let mut cs = CursorState::at(Position::new(0, 0));

        assert!(cs.toggle_secondary_cursor(Position::new(1, 3)));
        assert_eq!(cs.secondary, vec![Selection::cursor(Position::new(1, 3))]);

        assert!(cs.toggle_secondary_cursor(Position::new(1, 3)));
        assert!(cs.secondary.is_empty());
    }

    #[test]
    fn add_secondary_cursor_rejects_primary_or_selection_state() {
        let mut with_selection =
            CursorState::from_selection(Selection::new(Position::new(0, 0), Position::new(0, 2)));
        assert!(!with_selection.add_secondary_cursor(Position::new(1, 0)));

        let mut at = CursorState::at(Position::new(0, 1));
        assert!(!at.add_secondary_cursor(Position::new(0, 1)));
        assert!(at.secondary.is_empty());
    }
}
