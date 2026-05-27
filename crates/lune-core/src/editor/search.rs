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
    /// Plain-text only; the case-insensitive path uses Unicode lowercasing
    /// (`char::to_lowercase`) and tracks the byte-to-char mapping back to
    /// the original buffer so that variable-width lowerings such as
    /// `ß → ss`, `İ → i\u{307}`, and `\u{01C4} → \u{01C6}` produce
    /// match ranges that align with rope char boundaries.
    #[must_use]
    pub fn search(&self, query: &str, case_sensitive: bool) -> SearchState {
        let mut state = SearchState {
            query: query.to_string(),
            case_sensitive,
            matches: Vec::new(),
            current_match: None,
        };

        if query.is_empty() {
            return state;
        }

        // Allocate the buffer text once (unavoidable — we need a contiguous
        // &str for `str::find`).
        let text = self.text();

        if case_sensitive {
            self.find_matches_exact(&text, query, &mut state);
        } else {
            self.find_matches_case_insensitive(&text, query, &mut state);
        }

        if !state.matches.is_empty() {
            state.current_match = Some(0);
        }

        state
    }

    /// Case-sensitive scan: bytes in the haystack alias bytes in the
    /// original `text`, so we can count chars over the gap on the
    /// original directly and accumulate `char_offset` in O(n) total.
    fn find_matches_exact(&self, text: &str, query: &str, state: &mut SearchState) {
        let mut byte_offset = 0;
        let mut char_offset = 0;

        while let Some(found) = text[byte_offset..].find(query) {
            let match_start_byte = byte_offset + found;
            let match_end_byte = match_start_byte + query.len();

            char_offset += text[byte_offset..match_start_byte].chars().count();
            let start_char = char_offset;
            let match_chars = text[match_start_byte..match_end_byte].chars().count();
            let end_char = start_char + match_chars;

            state
                .matches
                .push((self.char_to_pos(start_char), self.char_to_pos(end_char)));

            char_offset = end_char;
            byte_offset = match_end_byte;
        }
    }

    /// Case-insensitive scan: build a lowercased haystack alongside a
    /// `lower-byte → original-char` map so that match byte offsets in the
    /// lowercased string can be translated back to char positions in the
    /// original buffer even when `char::to_lowercase` widens a char
    /// (e.g. `ß → ss`, `İ → i\u{307}`).  Match ranges round outward to
    /// cover every original char that contributed.
    fn find_matches_case_insensitive(&self, text: &str, query: &str, state: &mut SearchState) {
        // Pre-size to `text.len()`; the lowered form is typically the same
        // length or slightly larger.
        let mut lower = String::with_capacity(text.len());
        let mut byte_to_char: Vec<usize> = Vec::with_capacity(text.len());
        for (char_idx, ch) in text.chars().enumerate() {
            for lc in ch.to_lowercase() {
                let lc_bytes = lc.len_utf8();
                for _ in 0..lc_bytes {
                    byte_to_char.push(char_idx);
                }
                lower.push(lc);
            }
        }

        let lower_query = query.to_lowercase();
        // `query` was non-empty (caller checked), but lowercasing some
        // unusual sequences could in theory yield empty output.  Bail
        // defensively rather than risk an infinite loop on `find("")`.
        if lower_query.is_empty() {
            return;
        }

        let mut byte_offset = 0;
        while let Some(found) = lower[byte_offset..].find(&lower_query) {
            let match_start_byte = byte_offset + found;
            let match_end_byte = match_start_byte + lower_query.len();

            // Round outward: the start is the first contributing char,
            // the end is one past the last contributing char.
            let start_char = byte_to_char[match_start_byte];
            let end_char = byte_to_char[match_end_byte - 1] + 1;

            state
                .matches
                .push((self.char_to_pos(start_char), self.char_to_pos(end_char)));

            byte_offset = match_end_byte;
        }
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

    #[test]
    fn search_case_insensitive_capital_i_dot_widens() {
        // `İ` (U+0130, 2 UTF-8 bytes) lowercases to `i\u{307}` (3 UTF-8
        // bytes).  The lowercased haystack has a different byte length
        // than the original, so the old code would mis-map byte offsets
        // and could panic on a non-char-boundary slice.  The match range
        // must align with original-char boundaries.
        let buf = TextBuffer::from_text("İstanbul");
        let state = buf.search("i", false);
        assert_eq!(state.match_count(), 1);
        // chars: İ(0), s(1), t(2), a(3), n(4), b(5), u(6), l(7).
        assert_eq!(state.matches[0].0, Position::new(0, 0));
        assert_eq!(state.matches[0].1, Position::new(0, 1));
    }

    #[test]
    fn search_case_insensitive_widening_rounds_outward() {
        // A match in the lowered form that straddles an `İ → i\u{307}`
        // expansion should round outward to cover every original char
        // that contributed.
        let buf = TextBuffer::from_text("AİB");
        let state = buf.search("ai", false);
        assert_eq!(state.match_count(), 1);
        // chars: A(0), İ(1), B(2) — match covers A and İ.
        assert_eq!(state.matches[0].0, Position::new(0, 0));
        assert_eq!(state.matches[0].1, Position::new(0, 2));
    }

    #[test]
    fn search_case_insensitive_titlecase_digraph() {
        // `Ǆ` (U+01C4) lowercases to `ǆ` (U+01C6) — same byte length, but
        // a different codepoint.  The match must still land on the single
        // original char.
        let buf = TextBuffer::from_text("xǄy");
        let state = buf.search("ǆ", false);
        assert_eq!(state.match_count(), 1);
        assert_eq!(state.matches[0].0, Position::new(0, 1));
        assert_eq!(state.matches[0].1, Position::new(0, 2));
    }
}
