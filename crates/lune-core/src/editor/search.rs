//! Search and replace within a text buffer.

use crate::buffer::TextBuffer;
use crate::position::Position;

/// Describes a search configuration and its results.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    /// The query string.
    pub query: String,
    /// Whether the search is case-sensitive.
    pub case_sensitive: bool,
    /// Whether the query is a regex pattern.
    pub regex: bool,
    /// All match ranges found in the buffer, as `(start, end)` positions.
    pub matches: Vec<(Position, Position)>,
    /// Index into `matches` of the currently highlighted match.
    pub current_match: Option<usize>,
}

impl SearchState {
    /// Number of matches found.
    #[must_use]
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Whether there are any matches.
    #[must_use]
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }
}

impl TextBuffer {
    /// Search the buffer for `query` and populate the search state.
    ///
    /// Returns a `SearchState` with all matches. For now, only plain-text
    /// search is implemented (regex support is a TODO).
    #[must_use]
    pub fn search(&self, query: &str, case_sensitive: bool) -> SearchState {
        let mut state = SearchState {
            query: query.to_string(),
            case_sensitive,
            regex: false,
            matches: Vec::new(),
            current_match: None,
        };

        if query.is_empty() {
            return state;
        }

        // Allocate the text once (unavoidable — we need a contiguous &str
        // for `str::find`). Avoid the previous redundant `.clone()`.
        let text = self.text();
        let (search_text, search_query);

        if case_sensitive {
            // Borrow `text` directly — no clone needed.
            search_text = std::borrow::Cow::Borrowed(text.as_str());
            search_query = std::borrow::Cow::Borrowed(query);
        } else {
            search_text = std::borrow::Cow::Owned(text.to_lowercase());
            search_query = std::borrow::Cow::Owned(query.to_lowercase());
        }

        // Maintain a running char count to avoid the O(n*m) re-scan from
        // byte 0 on every match.  We track the char count up to the
        // current `byte_offset` and only count newly-advanced bytes.
        let mut byte_offset = 0;
        let mut char_offset = 0;

        while let Some(found) = search_text[byte_offset..].find(&*search_query) {
            let match_start_byte = byte_offset + found;
            let match_end_byte = match_start_byte + search_query.len();

            // Count chars only in the gap since last position — O(n) total.
            char_offset += text[byte_offset..match_start_byte].chars().count();
            let start_char = char_offset;
            let match_chars = text[match_start_byte..match_end_byte].chars().count();
            let end_char = start_char + match_chars;

            let start_pos = self.char_to_pos(start_char);
            let end_pos = self.char_to_pos(end_char);

            state.matches.push((start_pos, end_pos));

            // Advance running counters to match end.
            char_offset = end_char;
            byte_offset = match_end_byte;
        }

        if !state.matches.is_empty() {
            state.current_match = Some(0);
        }

        state
    }

    /// Advance to the next match in the search state.
    #[must_use]
    pub fn search_next(state: &SearchState) -> Option<usize> {
        let current = state.current_match?;
        if state.matches.is_empty() {
            return None;
        }
        Some((current + 1) % state.matches.len())
    }

    /// Go to the previous match in the search state.
    #[must_use]
    pub fn search_prev(state: &SearchState) -> Option<usize> {
        let current = state.current_match?;
        if state.matches.is_empty() {
            return None;
        }
        if current == 0 {
            Some(state.matches.len() - 1)
        } else {
            Some(current - 1)
        }
    }

    /// Replace the current match with `replacement`.
    ///
    /// Returns the updated search state (matches are recalculated).
    pub fn replace_current(&mut self, state: &SearchState, replacement: &str) -> SearchState {
        if let Some(idx) = state.current_match {
            if let Some(&(start, end)) = state.matches.get(idx) {
                self.replace(start, end, replacement);
            }
        }
        // Recalculate matches after replacement.
        self.search(&state.query, state.case_sensitive)
    }

    /// Replace all matches with `replacement` as a single transaction.
    ///
    /// Returns the updated search state (should be empty if all replaced).
    pub fn replace_all(&mut self, state: &SearchState, replacement: &str) -> SearchState {
        if state.matches.is_empty() {
            return state.clone();
        }

        self.begin_transaction();

        // Replace in reverse order so positions don't shift.
        for &(start, end) in state.matches.iter().rev() {
            self.replace(start, end, replacement);
        }

        self.commit_transaction();

        // Recalculate.
        self.search(&state.query, state.case_sensitive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_basic() {
        let buf = TextBuffer::from_text("hello world hello");
        let state = buf.search("hello", true);
        assert_eq!(state.match_count(), 2);
        assert_eq!(state.matches[0].0, Position::new(0, 0));
        assert_eq!(state.matches[0].1, Position::new(0, 5));
        assert_eq!(state.matches[1].0, Position::new(0, 12));
        assert_eq!(state.matches[1].1, Position::new(0, 17));
    }

    #[test]
    fn search_case_insensitive() {
        let buf = TextBuffer::from_text("Hello HELLO hello");
        let state = buf.search("hello", false);
        assert_eq!(state.match_count(), 3);
    }

    #[test]
    fn search_case_sensitive() {
        let buf = TextBuffer::from_text("Hello HELLO hello");
        let state = buf.search("hello", true);
        assert_eq!(state.match_count(), 1);
        assert_eq!(state.matches[0].0, Position::new(0, 12));
    }

    #[test]
    fn search_empty_query() {
        let buf = TextBuffer::from_text("hello");
        let state = buf.search("", true);
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn search_no_match() {
        let buf = TextBuffer::from_text("hello world");
        let state = buf.search("xyz", true);
        assert_eq!(state.match_count(), 0);
        assert!(state.current_match.is_none());
    }

    #[test]
    fn search_multiline() {
        let buf = TextBuffer::from_text("foo\nbar\nfoo\nbaz");
        let state = buf.search("foo", true);
        assert_eq!(state.match_count(), 2);
        assert_eq!(state.matches[0].0, Position::new(0, 0));
        assert_eq!(state.matches[1].0, Position::new(2, 0));
    }

    #[test]
    fn search_next_prev() {
        let buf = TextBuffer::from_text("aaa");
        let state = buf.search("a", true);
        assert_eq!(state.match_count(), 3);
        assert_eq!(state.current_match, Some(0));

        let next = TextBuffer::search_next(&state).unwrap();
        assert_eq!(next, 1);

        let mut state2 = state.clone();
        state2.current_match = Some(2);
        let next2 = TextBuffer::search_next(&state2).unwrap();
        assert_eq!(next2, 0); // wraps

        let prev = TextBuffer::search_prev(&state).unwrap();
        assert_eq!(prev, 2); // wraps backward
    }

    #[test]
    fn replace_current_match() {
        let mut buf = TextBuffer::from_text("hello world hello");
        let state = buf.search("hello", true);
        let new_state = buf.replace_current(&state, "hi");
        assert_eq!(buf.text(), "hi world hello");
        assert_eq!(new_state.match_count(), 1);
    }

    #[test]
    fn replace_all_matches() {
        let mut buf = TextBuffer::from_text("hello world hello");
        let state = buf.search("hello", true);
        let new_state = buf.replace_all(&state, "hi");
        assert_eq!(buf.text(), "hi world hi");
        assert_eq!(new_state.match_count(), 0);
    }

    #[test]
    fn replace_all_can_undo() {
        let mut buf = TextBuffer::from_text("aaa bbb aaa");
        let state = buf.search("aaa", true);
        buf.replace_all(&state, "x");
        assert_eq!(buf.text(), "x bbb x");

        // Single undo should revert all replacements.
        assert!(buf.undo());
        assert_eq!(buf.text(), "aaa bbb aaa");
    }
}
