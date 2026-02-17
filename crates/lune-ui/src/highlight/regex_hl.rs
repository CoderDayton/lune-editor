//! Regex-based fallback syntax highlighter.
//!
//! Provides basic highlighting for languages without tree-sitter grammar
//! support (e.g., TOML, Markdown, YAML). Uses ordered regex rules with
//! first-match priority.

use std::ops::Range;

use lune_core::highlight::{HighlightStyle, HighlightedLine, Highlighter, StyledSpan};
use lune_core::language::{lang, LanguageId};
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
        })
    }

    /// Highlight a single line of text, producing non-overlapping spans.
    fn highlight_line(&self, line_text: &str) -> Vec<StyledSpan> {
        // Track which columns are already claimed.
        let len = line_text.len();
        let mut claimed = vec![false; len];
        let mut spans = Vec::new();

        for rule in &self.rules {
            for mat in rule.pattern.find_iter(line_text) {
                let start = mat.start();
                let end = mat.end();

                // Skip if any part of this match is already claimed.
                if claimed[start..end].iter().any(|&c| c) {
                    continue;
                }

                // Claim the range.
                for slot in &mut claimed[start..end] {
                    *slot = true;
                }
                spans.push(StyledSpan::new(start, end, rule.style));
            }
        }

        // Sort by start column.
        spans.sort_by_key(|s| s.start_col);
        spans
    }
}

impl Highlighter for RegexHighlighter {
    fn update(&mut self, buffer: &TextBuffer, _edit_range: Option<(usize, usize)>) {
        self.lines.clear();
        for i in 0..buffer.line_count() {
            self.lines.push(buffer.line(i).unwrap_or_default());
        }
    }

    fn highlight_lines(&self, line_range: Range<usize>) -> Vec<HighlightedLine> {
        let start = line_range.start.min(self.lines.len());
        let end = line_range.end.min(self.lines.len());

        (start..end)
            .map(|i| {
                let text = &self.lines[i];
                let line_no_newline = text.trim_end_matches('\n').trim_end_matches('\r');
                let spans = self.highlight_line(line_no_newline);
                HighlightedLine::with_spans(i, spans)
            })
            .collect()
    }
}

// ── Rule builders ─────────────────────────────────────────────────────

/// Build regex rules for a language.
fn build_rules(lang_id: LanguageId) -> Vec<Rule> {
    if lang_id == lang::TOML {
        toml_rules()
    } else if lang_id == lang::YAML {
        yaml_rules()
    } else if lang_id == lang::MARKDOWN {
        markdown_rules()
    } else {
        // Generic fallback rules for any language.
        generic_rules()
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
        rule(r"```[\s\S]*?```", HighlightStyle::String),
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
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..2);
        assert_eq!(result.len(), 2);

        // First line should be a comment.
        assert!(!result[0].is_plain());
        assert!(result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Comment));
    }

    #[test]
    fn toml_highlights_string_and_key() {
        let buf = TextBuffer::from_text("[package]\nname = \"lune\"\n");
        let mut hl = RegexHighlighter::for_language(lang::TOML).unwrap();
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..2);

        // Line 0: [package] should be Namespace.
        assert!(result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Namespace));

        // Line 1: "lune" should be String.
        assert!(result[1]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::String));
    }

    #[test]
    fn markdown_highlights_heading() {
        let buf = TextBuffer::from_text("# Hello World\nSome text\n");
        let mut hl = RegexHighlighter::for_language(lang::MARKDOWN).unwrap();
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..2);
        assert!(result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Keyword));
    }

    #[test]
    fn generic_rules_highlight_comments_and_strings() {
        let buf = TextBuffer::from_text("// a comment\nlet x = \"hello\";\n");
        let mut hl = RegexHighlighter::for_language(lang::LUA).unwrap(); // uses generic rules
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..2);
        assert!(result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Comment));
        assert!(result[1]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::String));
    }

    #[test]
    fn spans_are_non_overlapping() {
        let buf = TextBuffer::from_text("x = \"hello\" # comment\n");
        let mut hl = RegexHighlighter::for_language(lang::TOML).unwrap();
        hl.update(&buf, None);

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
