//! Property-based tests for search and replace.
//!
//! Key invariants:
//! 1. Searching for text that exists always finds it.
//! 2. After `replace_all`, re-searching yields 0 matches (when
//!    replacement doesn't contain the query).
//! 3. Case-insensitive search finds at least as many matches as
//!    case-sensitive search.

use proptest::prelude::*;

use lune_core::buffer::TextBuffer;

// ── Strategies ────────────────────────────────────────────────────────

/// Short ASCII query (1–5 chars, lowercase letters only).
fn arb_query() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z]{1,5}").unwrap()
}

/// Text that definitely contains the query at least once.
fn text_containing_query() -> impl Strategy<Value = (String, String)> {
    arb_query().prop_flat_map(|query| {
        let q = query;
        // Build text = prefix + query + suffix.
        (
            prop::string::string_regex("[a-zA-Z ]{0,30}").unwrap(),
            prop::string::string_regex("[a-zA-Z ]{0,30}").unwrap(),
        )
            .prop_map(move |(prefix, suffix)| (format!("{prefix}{q}{suffix}"), q.clone()))
    })
}

// ── Properties ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn search_finds_known_substring((text, query) in text_containing_query()) {
        let buf = TextBuffer::from_text(&text);
        let state = buf.search(&query, false);
        prop_assert!(
            state.has_matches(),
            "Expected to find '{}' in '{}'",
            query,
            text
        );
    }

    #[test]
    fn replace_all_eliminates_matches((text, query) in text_containing_query()) {
        let mut buf = TextBuffer::from_text(&text);
        let state = buf.search(&query, false);

        if state.has_matches() {
            // Replace with digits — guaranteed not to match [a-z] query.
            let replacement = "99999";
            let new_state = buf.replace_all(&state, replacement);
            prop_assert_eq!(
                new_state.match_count(),
                0,
                "Expected 0 matches after replace_all, got {}",
                new_state.match_count()
            );
        }
    }

    #[test]
    fn case_insensitive_finds_at_least_as_many(
        text in "[a-zA-Z ]{5,50}",
        query in arb_query(),
    ) {
        let buf = TextBuffer::from_text(&text);
        let sensitive = buf.search(&query, true);
        let insensitive = buf.search(&query, false);

        prop_assert!(
            insensitive.match_count() >= sensitive.match_count(),
            "Case-insensitive ({}) < case-sensitive ({}) for query '{}' in '{}'",
            insensitive.match_count(),
            sensitive.match_count(),
            query,
            text,
        );
    }

    #[test]
    fn empty_query_returns_no_matches(text in "[a-zA-Z]{1,30}") {
        let buf = TextBuffer::from_text(&text);
        let state = buf.search("", true);
        prop_assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn replace_all_is_undoable((text, query) in text_containing_query()) {
        let mut buf = TextBuffer::from_text(&text);
        let original = buf.text();
        let state = buf.search(&query, false);

        if state.has_matches() {
            buf.replace_all(&state, "X");
            let undone = buf.undo();
            prop_assert!(undone);
            prop_assert_eq!(buf.text(), original);
        }
    }
}
