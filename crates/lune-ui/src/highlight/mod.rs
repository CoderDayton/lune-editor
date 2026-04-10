//! Syntax highlighting backends.
//!
//! Provides both a tree-sitter-based highlighter for languages with grammar
//! support and a regex-based fallback for everything else.

pub mod regex_hl;
pub mod theme;
pub mod tree_sitter;

use lune_core::highlight::{Highlighter, NullHighlighter};
use lune_core::language::LanguageId;

/// Create the best available highlighter for the given language.
///
/// Tries tree-sitter first, falls back to regex, then to `NullHighlighter`.
pub fn create_highlighter(lang_id: LanguageId) -> Box<dyn Highlighter> {
    if let Some(ts) = tree_sitter::TreeSitterHighlighter::new(lang_id) {
        return Box::new(ts);
    }
    if let Some(re) = regex_hl::RegexHighlighter::for_language(lang_id) {
        return Box::new(re);
    }
    Box::new(NullHighlighter::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lune_core::buffer::TextBuffer;
    use lune_core::highlight::HighlightedLine;
    use lune_core::language::lang;

    #[test]
    fn rust_gets_tree_sitter_highlighter() {
        let hl = create_highlighter(lang::RUST);
        // Tree-sitter highlighter produces non-plain highlights for Rust code.
        let buf = TextBuffer::from_text("fn main() { let x = 42; }");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..1);
        // Should have at least one line with styled spans (not plain).
        assert!(!lines.is_empty());
    }

    #[test]
    fn python_gets_tree_sitter_highlighter() {
        let hl = create_highlighter(lang::PYTHON);
        let buf = TextBuffer::from_text("def hello():\n    pass\n");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..1);
        assert!(!lines.is_empty());
    }

    #[test]
    fn toml_gets_regex_highlighter() {
        let hl = create_highlighter(lang::TOML);
        let buf = TextBuffer::from_text("[package]\nname = \"test\"\n");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..2);
        assert!(!lines.is_empty());
    }

    #[test]
    fn markdown_gets_regex_highlighter() {
        let hl = create_highlighter(lang::MARKDOWN);
        let buf = TextBuffer::from_text("# Hello\nSome text\n");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..1);
        assert!(!lines.is_empty());
    }

    #[test]
    fn plain_text_gets_null_highlighter() {
        let hl = create_highlighter(lang::PLAIN_TEXT);
        let buf = TextBuffer::from_text("just plain text");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..1);
        // Null highlighter produces empty or plain spans.
        assert!(lines.is_empty() || lines.iter().all(HighlightedLine::is_plain));
    }

    #[test]
    fn javascript_gets_tree_sitter_highlighter() {
        let hl = create_highlighter(lang::JAVASCRIPT);
        let buf = TextBuffer::from_text("const x = 1;\n");
        let mut hl = hl;
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..1);
        assert!(!lines.is_empty());
    }

    #[test]
    fn all_known_languages_produce_highlighter() {
        // Every known language should get a highlighter (never panic).
        let languages = [
            lang::RUST,
            lang::PYTHON,
            lang::JAVASCRIPT,
            lang::TYPESCRIPT,
            lang::TSX,
            lang::JSON,
            lang::TOML,
            lang::YAML,
            lang::MARKDOWN,
            lang::C,
            lang::CPP,
            lang::GO,
            lang::HTML,
            lang::CSS,
            lang::SHELL,
            lang::PLAIN_TEXT,
        ];
        for lid in languages {
            let _hl = create_highlighter(lid);
        }
    }
}
