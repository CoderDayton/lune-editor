//! Property-based tests for the diff engine.
//!
//! Key invariants:
//! 1. Identical inputs → no hunks.
//! 2. Swapping old/new flips hunk kinds (Insert↔Delete).
//! 3. Incremental diff produces same results as full diff.
//! 4. Inline highlights on identical strings → empty.

use lune_core::ropey::Rope;
use proptest::prelude::*;

use lune_core::diff::{compute_diff, compute_diff_str, compute_inline_highlights, LiveHunkKind};

// ── Strategies ────────────────────────────────────────────────────────

/// Generate multi-line text (1–10 lines, each 0–40 chars).
fn arb_multiline() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::string::string_regex("[a-zA-Z0-9 ]{0,40}").unwrap(),
        1..10,
    )
    .prop_map(|lines| lines.join("\n"))
}

// ── Properties ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn identical_inputs_no_hunks(text in arb_multiline()) {
        let rope = Rope::from_str(&text);
        let hunks = compute_diff(&rope, &rope);
        prop_assert!(
            hunks.is_empty(),
            "Expected no hunks for identical inputs, got {}",
            hunks.len()
        );
    }

    #[test]
    fn identical_str_no_hunks(text in arb_multiline()) {
        let hunks = compute_diff_str(&text, &text);
        prop_assert!(
            hunks.is_empty(),
            "Expected no hunks for identical string inputs, got {}",
            hunks.len()
        );
    }

    #[test]
    fn swap_flips_hunk_kinds(
        old in arb_multiline(),
        new in arb_multiline(),
    ) {
        let hunks_forward = compute_diff_str(&old, &new);
        let hunks_reverse = compute_diff_str(&new, &old);

        // The number of non-modification hunks may differ due to context
        // merging, but insertions in forward should correspond to deletions
        // in reverse and vice versa.
        let fwd_inserts = hunks_forward
            .iter()
            .filter(|h| h.kind == LiveHunkKind::Insertion)
            .count();
        let fwd_deletes = hunks_forward
            .iter()
            .filter(|h| h.kind == LiveHunkKind::Deletion)
            .count();
        let rev_inserts = hunks_reverse
            .iter()
            .filter(|h| h.kind == LiveHunkKind::Insertion)
            .count();
        let rev_deletes = hunks_reverse
            .iter()
            .filter(|h| h.kind == LiveHunkKind::Deletion)
            .count();

        prop_assert_eq!(
            fwd_inserts, rev_deletes,
            "Forward insertions ({}) != reverse deletions ({})",
            fwd_inserts, rev_deletes
        );
        prop_assert_eq!(
            fwd_deletes, rev_inserts,
            "Forward deletions ({}) != reverse insertions ({})",
            fwd_deletes, rev_inserts
        );
    }

    #[test]
    fn hunk_ids_sequential(
        old in arb_multiline(),
        new in arb_multiline(),
    ) {
        let hunks = compute_diff_str(&old, &new);
        for (i, hunk) in hunks.iter().enumerate() {
            prop_assert_eq!(
                hunk.id, i,
                "Hunk {} has id {}, expected {}",
                i, hunk.id, i
            );
        }
    }

    #[test]
    fn inline_highlights_identical_is_empty(line in "[a-zA-Z0-9 ]{0,40}") {
        let highlights = compute_inline_highlights(&line, &line);
        prop_assert!(
            highlights.is_empty(),
            "Expected no inline highlights for identical lines, got {}",
            highlights.len()
        );
    }

    #[test]
    fn diff_str_matches_diff_rope(
        old in arb_multiline(),
        new in arb_multiline(),
    ) {
        let hunks_str = compute_diff_str(&old, &new);
        let old_rope = Rope::from_str(&old);
        let new_rope = Rope::from_str(&new);
        let hunks_rope = compute_diff(&old_rope, &new_rope);

        // Both paths should produce the same number of hunks with the same kinds.
        prop_assert_eq!(
            hunks_str.len(),
            hunks_rope.len(),
            "str ({}) vs rope ({}) hunk count mismatch",
            hunks_str.len(),
            hunks_rope.len()
        );
        for (s, r) in hunks_str.iter().zip(hunks_rope.iter()) {
            prop_assert_eq!(s.kind, r.kind, "Hunk kind mismatch at id {}", s.id);
            prop_assert_eq!(s.old_range.clone(), r.old_range.clone(), "old_range mismatch at id {}", s.id);
            prop_assert_eq!(s.new_range.clone(), r.new_range.clone(), "new_range mismatch at id {}", s.id);
        }
    }
}
