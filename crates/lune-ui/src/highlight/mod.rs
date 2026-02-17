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
    Box::new(NullHighlighter)
}
