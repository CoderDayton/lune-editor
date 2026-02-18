//! Property-based tests for `TextBuffer`.
//!
//! These tests use `proptest` to verify invariants that must hold for
//! arbitrary inputs:
//!
//! 1. Insert→undo restores original content.
//! 2. `pos_to_char ∘ char_to_pos = id` for valid char indices.
//! 3. Delete→undo restores original content.
//! 4. Multiple insert/undo cycles are idempotent.

use proptest::prelude::*;

use lune_core::buffer::TextBuffer;
use lune_core::position::Position;

// ── Strategies ────────────────────────────────────────────────────────

/// Generate arbitrary non-empty text (1–200 chars, printable ASCII + newlines).
fn arb_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::char::range('\n', '~'), 1..200)
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// Short insert text (1–20 chars, printable ASCII + newlines).
fn arb_insert_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::char::range(' ', '~'), 1..20)
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

// ── Property: insert then undo restores original ──────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn insert_undo_roundtrip(
        initial in arb_text(),
        insert_text in arb_insert_text(),
    ) {
        let mut buf = TextBuffer::from_text(&initial);
        let original = buf.text();

        // Insert at (0,0) — always valid.
        buf.insert(Position::new(0, 0), &insert_text);
        // Content should have changed (unless insert_text is empty, but
        // our strategy always generates non-empty).
        prop_assert_ne!(&buf.text(), &original);

        // Undo should restore original.
        let undone = buf.undo();
        prop_assert!(undone);
        prop_assert_eq!(buf.text(), original);
    }

    #[test]
    fn delete_undo_roundtrip(initial in arb_text()) {
        let mut buf = TextBuffer::from_text(&initial);
        let original = buf.text();
        let char_count = buf.char_count();

        if char_count >= 2 {
            // Delete first character.
            buf.delete(Position::new(0, 0), Position::new(0, 1));
            prop_assert_ne!(&buf.text(), &original);

            let undone = buf.undo();
            prop_assert!(undone);
            prop_assert_eq!(buf.text(), original);
        }
    }

    #[test]
    fn pos_char_roundtrip(initial in arb_text()) {
        let buf = TextBuffer::from_text(&initial);
        let total_chars = buf.char_count();

        // Test a sample of positions.
        if total_chars > 0 {
            for idx in [0, total_chars / 2, total_chars.saturating_sub(1)] {
                if idx < total_chars {
                    let pos = buf.char_to_pos(idx);
                    let back = buf.pos_to_char(pos);
                    prop_assert_eq!(back, idx, "pos_to_char(char_to_pos({})) != {}", idx, idx);
                }
            }
        }
    }

    #[test]
    fn multiple_insert_undo_is_clean(
        initial in arb_text(),
        inserts in prop::collection::vec(arb_insert_text(), 1..5),
    ) {
        let mut buf = TextBuffer::from_text(&initial);
        let original = buf.text();

        // Apply multiple inserts, each at (0,0).
        for text in &inserts {
            buf.insert(Position::new(0, 0), text);
        }

        // Undo all of them.
        for _ in &inserts {
            let undone = buf.undo();
            prop_assert!(undone);
        }

        prop_assert_eq!(buf.text(), original);
    }

    #[test]
    fn undo_redo_roundtrip(
        initial in arb_text(),
        insert_text in arb_insert_text(),
    ) {
        let mut buf = TextBuffer::from_text(&initial);

        buf.insert(Position::new(0, 0), &insert_text);
        let after_insert = buf.text();

        buf.undo();
        buf.redo();

        prop_assert_eq!(buf.text(), after_insert);
    }
}
