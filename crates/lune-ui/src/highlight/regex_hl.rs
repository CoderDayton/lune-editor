//! Regex-based fallback syntax highlighter.
//!
//! Provides basic highlighting for languages without tree-sitter grammar
//! support (e.g., TOML, Markdown, YAML). Uses ordered regex rules with
//! first-match priority.

use std::ops::Range;

use lune_core::highlight::{
    BufferEdit, HighlightStyle, HighlightedLine, Highlighter, SpanVec, StyledSpan,
};
use lune_core::language::{LanguageId, lang};
use lune_core::prelude::TextBuffer;
use regex::Regex;

// ── Rule ──────────────────────────────────────────────────────────────

/// A single regex highlight rule.
struct Rule {
    pattern: Regex,
    style: HighlightStyle,
}

// ── Regex highlighter ─────────────────────────────────────────────────

/// A regex-based syntax highlighter.
///
/// Applies ordered rules per-line. First match wins for overlapping regions.
pub struct RegexHighlighter {
    rules: Vec<Rule>,
    /// Cached source lines from the last `update()`.
    lines: Vec<String>,
    /// Pre-computed highlighted spans for every line in `lines`. Kept in
    /// lockstep with `lines` by `update()` so `highlight_lines` is a
    /// cheap slice rather than a per-frame regex scan.
    cached: Vec<HighlightedLine>,
}

impl RegexHighlighter {
    /// Create a regex highlighter for the given language.
    ///
    /// Always returns `Some` — uses language-specific or generic fallback rules.
    pub fn for_language(lang_id: LanguageId) -> Option<Self> {
        let rules = build_rules(lang_id);
        Some(Self {
            rules,
            lines: Vec::new(),
            cached: Vec::new(),
        })
    }

    /// Build a `HighlightedLine` for a single cached line index.
    fn build_cached_line(&self, idx: usize) -> HighlightedLine {
        let text = &self.lines[idx];
        let line_no_newline = text.trim_end_matches('\n').trim_end_matches('\r');
        let spans = self.highlight_line(line_no_newline);
        HighlightedLine::with_spans(idx, spans)
    }

    /// Highlight a single line of text, producing non-overlapping spans.
    fn highlight_line(&self, line_text: &str) -> SpanVec {
        let mut spans = SpanVec::new();

        for rule in &self.rules {
            for mat in rule.pattern.find_iter(line_text) {
                let start = mat.start();
                let end = mat.end();

                // Skip if any part of this match overlaps an already-claimed span.
                let overlaps = spans.iter().any(|s| start < s.end_col && end > s.start_col);

                if !overlaps {
                    spans.push(StyledSpan::new(start, end, rule.style));
                }
            }
        }

        // Sort by start column.
        spans.sort_by_key(|s| s.start_col);
        spans
    }
}

impl Highlighter for RegexHighlighter {
    fn update(&mut self, buffer: &TextBuffer, _edits: &[BufferEdit]) {
        let new_count = buffer.line_count();
        let old_count = self.lines.len();

        if new_count == old_count {
            // Same line count: only replace lines whose content changed and
            // re-highlight just those lines. This avoids re-allocating
            // Strings and re-running every regex rule over every line on
            // each keystroke (the common case during single-line edits).
            for i in 0..new_count {
                let new_line = buffer.line(i).unwrap_or_default();
                if self.lines[i] != new_line {
                    self.lines[i] = new_line;
                    self.cached[i] = self.build_cached_line(i);
                }
            }
        } else {
            // Line count changed — full rebuild (lines were added/removed).
            self.lines.clear();
            self.lines.reserve(new_count);
            for i in 0..new_count {
                self.lines.push(buffer.line(i).unwrap_or_default());
            }

            // Rebuild the entire highlight cache in lock-step with `lines`
            // so `highlight_lines` is a cheap slice on every frame.
            self.cached.clear();
            self.cached.reserve(self.lines.len());
            for i in 0..self.lines.len() {
                self.cached.push(self.build_cached_line(i));
            }
        }
    }

    fn highlight_lines(&mut self, line_range: Range<usize>) -> &[HighlightedLine] {
        let end = line_range.end.min(self.cached.len());
        let start = line_range.start.min(end);
        &self.cached[start..end]
    }
}

// ── Rule builders ─────────────────────────────────────────────────────

/// Build regex rules for a language.
fn build_rules(lang_id: LanguageId) -> Vec<Rule> {
    match lang_id {
        l if l == lang::TOML => toml_rules(),
        l if l == lang::YAML => yaml_rules(),
        l if l == lang::MARKDOWN => markdown_rules(),
        _ => generic_rules(),
    }
}

fn toml_rules() -> Vec<Rule> {
    vec![
        rule(r"#[^\n]*", HighlightStyle::Comment),
        rule(r#""(?:[^"\\]|\\.)*""#, HighlightStyle::String),
        rule(r"'(?:[^'\\]|\\.)*'", HighlightStyle::String),
        rule(r"\b(true|false)\b", HighlightStyle::Constant),
        rule(r"\b\d[\d_.]*\b", HighlightStyle::Number),
        rule(r"\[[\w.\-]+\]", HighlightStyle::Namespace),
        rule(r"[\w\-]+\s*=", HighlightStyle::Variable),
    ]
}

fn yaml_rules() -> Vec<Rule> {
    vec![
        rule(r"#[^\n]*", HighlightStyle::Comment),
        rule(r#""(?:[^"\\]|\\.)*""#, HighlightStyle::String),
        rule(r"'(?:[^'\\]|\\.)*'", HighlightStyle::String),
        rule(r"\b(true|false|null|yes|no)\b", HighlightStyle::Constant),
        rule(r"\b\d[\d_.]*\b", HighlightStyle::Number),
        rule(r"^[\w\-]+:", HighlightStyle::Variable),
    ]
}

fn markdown_rules() -> Vec<Rule> {
    vec![
        rule(r"^#{1,6}\s.*", HighlightStyle::Keyword),
        rule(r"`[^`]+`", HighlightStyle::String),
        rule(r"^\s*```[\w-]*\s*$", HighlightStyle::String),
        rule(r"\*\*[^*]+\*\*", HighlightStyle::Keyword),
        rule(r"\*[^*]+\*", HighlightStyle::Attribute),
        rule(r"\[([^\]]+)\]\([^)]+\)", HighlightStyle::Function),
        rule(r"^>\s.*", HighlightStyle::Comment),
        rule(r"^[-*+]\s", HighlightStyle::Punctuation),
    ]
}

fn generic_rules() -> Vec<Rule> {
    vec![
        // Line comments: //, #, --
        rule(r"//[^\n]*", HighlightStyle::Comment),
        rule(r"#[^\n]*", HighlightStyle::Comment),
        rule(r"--[^\n]*", HighlightStyle::Comment),
        // Block comments (single line only in regex mode).
        rule(r"/\*.*?\*/", HighlightStyle::Comment),
        // Strings.
        rule(r#""(?:[^"\\]|\\.)*""#, HighlightStyle::String),
        rule(r"'(?:[^'\\]|\\.)*'", HighlightStyle::String),
        // Numbers.
        rule(r"\b\d[\d_.eE]*\b", HighlightStyle::Number),
        rule(r"\b0x[\da-fA-F_]+\b", HighlightStyle::Number),
    ]
}

/// Helper to construct a `Rule` from a pattern string.
///
/// Panics if the pattern is invalid (only called with known-good literals).
fn rule(pattern: &str, style: HighlightStyle) -> Rule {
    Rule {
        pattern: Regex::new(pattern).expect("invalid regex rule"),
        style,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_highlights_comment() {
        let buf = TextBuffer::from_text("# This is a comment\nkey = \"value\"\n");
        let mut hl = RegexHighlighter::for_language(lang::TOML).unwrap();
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..2);
        assert_eq!(result.len(), 2);

        // First line should be a comment.
        assert!(!result[0].is_plain());
        assert!(
            result[0]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::Comment)
        );
    }

    #[test]
    fn toml_highlights_string_and_key() {
        let buf = TextBuffer::from_text("[package]\nname = \"lune\"\n");
        let mut hl = RegexHighlighter::for_language(lang::TOML).unwrap();
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..2);

        // Line 0: [package] should be Namespace.
        assert!(
            result[0]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::Namespace)
        );

        // Line 1: "lune" should be String.
        assert!(
            result[1]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::String)
        );
    }

    #[test]
    fn markdown_highlights_heading() {
        let buf = TextBuffer::from_text("# Hello World\nSome text\n");
        let mut hl = RegexHighlighter::for_language(lang::MARKDOWN).unwrap();
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..2);
        assert!(
            result[0]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::Keyword)
        );
    }

    #[test]
    fn generic_rules_highlight_comments_and_strings() {
        let buf = TextBuffer::from_text("// a comment\nlet x = \"hello\";\n");
        let mut hl = RegexHighlighter::for_language(lang::LUA).unwrap(); // uses generic rules
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..2);
        assert!(
            result[0]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::Comment)
        );
        assert!(
            result[1]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::String)
        );
    }

    #[test]
    fn markdown_fence_line_gets_styled() {
        let buf = TextBuffer::from_text("```rust\nlet x = 1;\n```\n");
        let mut hl = RegexHighlighter::for_language(lang::MARKDOWN).unwrap();
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..3);
        // Opening fence line carries a non-default style.
        assert!(!result[0].is_plain());
        assert!(
            result[0]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::String)
        );
        // Closing fence line too.
        assert!(
            result[2]
                .spans
                .iter()
                .any(|s| s.style == HighlightStyle::String)
        );
    }

    #[test]
    fn update_single_line_change_leaves_others_intact() {
        let buf = TextBuffer::from_text("# title\nplain text\nmore text\n");
        let mut hl = RegexHighlighter::for_language(lang::MARKDOWN).unwrap();
        hl.update(&buf, &[]);

        // Snapshot the spans of all lines before the edit.
        let before: Vec<Vec<StyledSpan>> = hl
            .highlight_lines(0..3)
            .iter()
            .map(|l| l.spans.to_vec())
            .collect();

        // Re-parse with the edited buffer. The regex highlighter ignores
        // `edits` and fully re-parses, so unchanged lines must still
        // produce identical spans.
        let buf2 = TextBuffer::from_text("# title\nedited text\nmore text\n");
        hl.update(&buf2, &[]);

        let after: Vec<Vec<StyledSpan>> = hl
            .highlight_lines(0..3)
            .iter()
            .map(|l| l.spans.to_vec())
            .collect();

        // Lines 0 and 2 are byte-identical; only line 1 may differ.
        assert_eq!(before[0], after[0]);
        assert_eq!(before[2], after[2]);
    }

    #[test]
    fn spans_are_non_overlapping() {
        let buf = TextBuffer::from_text("x = \"hello\" # comment\n");
        let mut hl = RegexHighlighter::for_language(lang::TOML).unwrap();
        hl.update(&buf, &[]);

        let result = hl.highlight_lines(0..1);
        let spans = &result[0].spans;

        // Verify no overlaps.
        for window in spans.windows(2) {
            assert!(
                window[0].end_col <= window[1].start_col,
                "spans overlap: {:?} and {:?}",
                window[0],
                window[1]
            );
        }
    }
}
