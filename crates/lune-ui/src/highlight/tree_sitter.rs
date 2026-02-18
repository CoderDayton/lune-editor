//! Tree-sitter-based syntax highlighter.
//!
//! Uses the [`tree_sitter_highlight`] crate to parse source code and produce
//! [`HighlightedLine`] spans that map to [`HighlightStyle`] categories.

use std::ops::Range;

use lune_core::highlight::{HighlightStyle, HighlightedLine, Highlighter, StyledSpan};
use lune_core::language::{LanguageId, lang};
use lune_core::prelude::TextBuffer;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent};

// ── Recognized highlight names ────────────────────────────────────────

/// The ordered list of highlight names we recognize.
///
/// The index in this array corresponds to the `Highlight(usize)` value
/// returned by `tree_sitter_highlight`. We map each index to a
/// [`HighlightStyle`] via [`HIGHLIGHT_STYLES`].
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",             // 0
    "comment",               // 1
    "constant",              // 2
    "constructor",           // 3
    "embedded",              // 4
    "escape",                // 5
    "function",              // 6
    "keyword",               // 7
    "number",                // 8
    "operator",              // 9
    "property",              // 10
    "punctuation",           // 11
    "string",                // 12
    "type",                  // 13
    "variable",              // 14
    "error",                 // 15
    "constant.builtin",      // 16
    "function.builtin",      // 17
    "type.builtin",          // 18
    "variable.builtin",      // 19
    "variable.parameter",    // 20
    "comment.documentation", // 21
    "string.special",        // 22
    "punctuation.bracket",   // 23
    "punctuation.delimiter", // 24
    "punctuation.special",   // 25
    "function.method",       // 26
    "function.macro",        // 27
    "label",                 // 28
];

/// Maps each index in [`HIGHLIGHT_NAMES`] to a [`HighlightStyle`].
const HIGHLIGHT_STYLES: &[HighlightStyle] = &[
    HighlightStyle::Attribute,   // 0  attribute
    HighlightStyle::Comment,     // 1  comment
    HighlightStyle::Constant,    // 2  constant
    HighlightStyle::Function,    // 3  constructor
    HighlightStyle::Embedded,    // 4  embedded
    HighlightStyle::String,      // 5  escape (render like string)
    HighlightStyle::Function,    // 6  function
    HighlightStyle::Keyword,     // 7  keyword
    HighlightStyle::Number,      // 8  number
    HighlightStyle::Operator,    // 9  operator
    HighlightStyle::Variable,    // 10 property
    HighlightStyle::Punctuation, // 11 punctuation
    HighlightStyle::String,      // 12 string
    HighlightStyle::Type,        // 13 type
    HighlightStyle::Variable,    // 14 variable
    HighlightStyle::Error,       // 15 error
    HighlightStyle::Constant,    // 16 constant.builtin
    HighlightStyle::Function,    // 17 function.builtin
    HighlightStyle::Type,        // 18 type.builtin
    HighlightStyle::Variable,    // 19 variable.builtin
    HighlightStyle::Variable,    // 20 variable.parameter
    HighlightStyle::Comment,     // 21 comment.documentation
    HighlightStyle::String,      // 22 string.special
    HighlightStyle::Punctuation, // 23 punctuation.bracket
    HighlightStyle::Punctuation, // 24 punctuation.delimiter
    HighlightStyle::Punctuation, // 25 punctuation.special
    HighlightStyle::Function,    // 26 function.method
    HighlightStyle::Attribute,   // 27 function.macro
    HighlightStyle::Variable,    // 28 label
];

// ── Language configuration loader ─────────────────────────────────────

/// Load the `HighlightConfiguration` for a given language.
///
/// Returns `None` if the language has no tree-sitter grammar support.
#[allow(clippy::too_many_lines)]
fn load_highlight_config(lang_id: LanguageId) -> Option<HighlightConfiguration> {
    let (language_fn, highlights, injections, locals, name) = match lang_id {
        l if l == lang::RUST => (
            tree_sitter_rust::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-rust-0.24.0/queries/highlights.scm"
            )),
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-rust-0.24.0/queries/injections.scm"
            )),
            "",
            "rust",
        ),
        l if l == lang::PYTHON => (
            tree_sitter_python::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-python-0.25.0/queries/highlights.scm"
            )),
            "",
            "",
            "python",
        ),
        l if l == lang::JAVASCRIPT || l == lang::JSX => (
            tree_sitter_javascript::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-javascript-0.25.0/queries/highlights.scm"
            )),
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-javascript-0.25.0/queries/injections.scm"
            )),
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-javascript-0.25.0/queries/locals.scm"
            )),
            "javascript",
        ),
        l if l == lang::TYPESCRIPT => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-typescript-0.23.2/queries/highlights.scm"
            )),
            "",
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-typescript-0.23.2/queries/locals.scm"
            )),
            "typescript",
        ),
        l if l == lang::TSX => (
            tree_sitter_typescript::LANGUAGE_TSX,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-typescript-0.23.2/queries/highlights.scm"
            )),
            "",
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-typescript-0.23.2/queries/locals.scm"
            )),
            "tsx",
        ),
        l if l == lang::JSON => (
            tree_sitter_json::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-json-0.24.8/queries/highlights.scm"
            )),
            "",
            "",
            "json",
        ),
        l if l == lang::C => (
            tree_sitter_c::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-c-0.24.1/queries/highlights.scm"
            )),
            "",
            "",
            "c",
        ),
        l if l == lang::GO => (
            tree_sitter_go::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-go-0.25.0/queries/highlights.scm"
            )),
            "",
            "",
            "go",
        ),
        l if l == lang::HTML => (
            tree_sitter_html::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-html-0.23.2/queries/highlights.scm"
            )),
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-html-0.23.2/queries/injections.scm"
            )),
            "",
            "html",
        ),
        l if l == lang::CSS => (
            tree_sitter_css::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-css-0.25.0/queries/highlights.scm"
            )),
            "",
            "",
            "css",
        ),
        l if l == lang::SHELL => (
            tree_sitter_bash::LANGUAGE,
            include_str!(concat!(
                env!("CARGO_HOME"),
                "/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-bash-0.25.1/queries/highlights.scm"
            )),
            "",
            "",
            "bash",
        ),
        _ => return None,
    };

    let language = tree_sitter::Language::from(language_fn);
    let mut config =
        HighlightConfiguration::new(language, name, highlights, injections, locals).ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

// ── Tree-sitter highlighter ───────────────────────────────────────────

/// Syntax highlighter backed by tree-sitter.
///
/// Parses the full buffer on creation, then supports incremental updates.
/// Line byte offsets are cached and recomputed only on `update()`.
///
/// Maintains a generation-stamped highlight cache so that `highlight_lines`
/// can return the previous result when the buffer hasn't changed and the
/// requested range is contained within the cached range.
pub struct TreeSitterHighlighter {
    /// The highlight configuration for this language.
    config: HighlightConfiguration,
    /// Cached source text (UTF-8 bytes) for the last parse.
    source: Vec<u8>,
    /// Cached byte offset of each line start (recomputed on `update()`).
    line_byte_offsets: Vec<usize>,
    /// Language identifier.
    language_id: LanguageId,
    /// Reusable tree-sitter highlighter instance (avoids re-allocation each
    /// call to `highlight_lines`).
    ts_highlighter: tree_sitter_highlight::Highlighter,
    /// Monotonic generation counter, incremented on every `update()`.
    generation: u64,
    /// Cached highlight result from the last `highlight_lines()` call.
    cached_result: Vec<HighlightedLine>,
    /// The generation at which the cache was computed.
    cached_generation: u64,
    /// The line range that the cache covers.
    cached_range: Range<usize>,
}

impl TreeSitterHighlighter {
    /// Create a new tree-sitter highlighter for the given language.
    ///
    /// Returns `None` if no tree-sitter grammar is available for this language.
    pub fn new(language_id: LanguageId) -> Option<Self> {
        let config = load_highlight_config(language_id)?;
        Some(Self {
            config,
            source: Vec::new(),
            line_byte_offsets: vec![0],
            language_id,
            ts_highlighter: tree_sitter_highlight::Highlighter::new(),
            generation: 0,
            cached_result: Vec::new(),
            cached_generation: u64::MAX, // force miss on first call
            cached_range: 0..0,
        })
    }

    /// The language this highlighter handles.
    pub const fn language_id(&self) -> LanguageId {
        self.language_id
    }
}

impl Highlighter for TreeSitterHighlighter {
    fn update(&mut self, buffer: &TextBuffer, _edit_range: Option<(usize, usize)>) {
        // Bump generation to invalidate the highlight cache.
        self.generation = self.generation.wrapping_add(1);

        // Re-cache source bytes and line offsets.
        // Build Vec<u8> directly from rope chunks to avoid the intermediate
        // String allocation that `buffer.text().into_bytes()` would incur.
        let rope = buffer.rope();
        let byte_len = rope.len_bytes();
        self.source.clear();
        self.source.reserve(byte_len);
        for chunk in rope.chunks() {
            self.source.extend_from_slice(chunk.as_bytes());
        }
        self.line_byte_offsets = compute_line_byte_offsets(&self.source);
    }

    fn highlight_lines(&mut self, line_range: Range<usize>) -> Vec<HighlightedLine> {
        if self.source.is_empty() {
            return line_range.map(HighlightedLine::new).collect();
        }

        let total_lines = self.line_byte_offsets.len();

        // Clamp range.
        let start_line = line_range.start.min(total_lines);
        let end_line = line_range.end.min(total_lines);

        if start_line >= end_line {
            return Vec::new();
        }

        // Cache hit: same generation and requested range is within the cached range.
        if self.cached_generation == self.generation
            && start_line >= self.cached_range.start
            && end_line <= self.cached_range.end
        {
            let offset = start_line - self.cached_range.start;
            let len = end_line - start_line;
            return self.cached_result[offset..offset + len].to_vec();
        }

        // Cache miss — recompute.
        // Reuse the cached `ts_highlighter` instance to avoid re-allocating
        // its internal buffers on every call.
        let Ok(events) = self
            .ts_highlighter
            .highlight(&self.config, &self.source, None, |_| None)
        else {
            return (start_line..end_line).map(HighlightedLine::new).collect();
        };

        // Convert HighlightEvents to per-line styled spans.
        let mut result: Vec<HighlightedLine> =
            (start_line..end_line).map(HighlightedLine::new).collect();

        let mut style_stack: Vec<HighlightStyle> = Vec::new();

        for event in events {
            let Ok(event) = event else { break };

            match event {
                HighlightEvent::HighlightStart(h) => {
                    let style = HIGHLIGHT_STYLES
                        .get(h.0)
                        .copied()
                        .unwrap_or(HighlightStyle::Default);
                    style_stack.push(style);
                }
                HighlightEvent::HighlightEnd => {
                    style_stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    let current_style = style_stack.last().copied();
                    if let Some(style) = current_style {
                        add_spans_for_byte_range(
                            start..end,
                            style,
                            &self.line_byte_offsets,
                            start_line..end_line,
                            &mut result,
                        );
                    }
                }
            }
        }

        // Store result in cache for next frame.
        self.cached_result.clone_from(&result);
        self.cached_generation = self.generation;
        self.cached_range = start_line..end_line;

        result
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Compute the byte offset of the start of each line.
fn compute_line_byte_offsets(source: &[u8]) -> Vec<usize> {
    let mut offsets = vec![0];
    for (i, &byte) in source.iter().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Given a byte range and a style, add spans to the appropriate lines in `result`.
fn add_spans_for_byte_range(
    byte_range: Range<usize>,
    style: HighlightStyle,
    line_byte_offsets: &[usize],
    view_lines: Range<usize>,
    result: &mut [HighlightedLine],
) {
    let (byte_start, byte_end) = (byte_range.start, byte_range.end);
    let (view_start_line, view_end_line) = (view_lines.start, view_lines.end);
    if byte_start >= byte_end {
        return;
    }

    // Find which line the byte_start is on.
    let start_line = match line_byte_offsets.binary_search(&byte_start) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };

    // Walk from start_line forward, adding spans.
    for line_idx in start_line..view_end_line {
        if line_idx >= line_byte_offsets.len() {
            break;
        }

        let line_start_byte = line_byte_offsets[line_idx];
        let line_end_byte = line_byte_offsets
            .get(line_idx + 1)
            .copied()
            .unwrap_or(usize::MAX);

        // Check if this line is past the highlight range.
        if line_start_byte >= byte_end {
            break;
        }

        // Skip lines before the view range.
        if line_idx < view_start_line {
            continue;
        }

        // Compute column offsets within this line.
        let span_start_byte = byte_start.max(line_start_byte);
        let span_end_byte = byte_end.min(line_end_byte);

        if span_start_byte >= span_end_byte {
            continue;
        }

        let start_col = span_start_byte - line_start_byte;
        let end_col = span_end_byte - line_start_byte;

        let result_idx = line_idx - view_start_line;
        if result_idx < result.len() {
            result[result_idx]
                .spans
                .push(StyledSpan::new(start_col, end_col, style));
        }
    }
}

/// Check if a language has tree-sitter support.
///
/// Derived from the same match arms as [`load_highlight_config`] to stay
/// in sync without maintaining a duplicate list.
pub fn has_tree_sitter_support(lang_id: LanguageId) -> bool {
    // The set of supported languages is defined by load_highlight_config's
    // match arms. We keep a const array here derived from that same set.
    const SUPPORTED: &[LanguageId] = &[
        lang::RUST,
        lang::PYTHON,
        lang::JAVASCRIPT,
        lang::JSX,
        lang::TYPESCRIPT,
        lang::TSX,
        lang::JSON,
        lang::C,
        lang::GO,
        lang::HTML,
        lang::CSS,
        lang::SHELL,
    ];
    SUPPORTED.contains(&lang_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lune_core::highlight::HighlightStyle;

    #[test]
    fn highlight_rust_snippet() {
        let source = "fn main() {\n    let x = 42;\n}\n";
        let buf = TextBuffer::from_text(source);
        let mut hl = TreeSitterHighlighter::new(lang::RUST).expect("Rust grammar should load");
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..3);
        assert_eq!(result.len(), 3);

        // Line 0: "fn main() {" — should have at least a keyword span for "fn".
        assert!(
            !result[0].is_plain(),
            "first line of Rust code should have highlights"
        );
        // Find the keyword span (should be at col 0..2 for "fn").
        let has_keyword = result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Keyword && s.start_col == 0);
        assert!(has_keyword, "should have keyword highlight for 'fn'");
    }

    #[test]
    fn highlight_python_snippet() {
        let source = "def hello():\n    print(\"world\")\n";
        let buf = TextBuffer::from_text(source);
        let mut hl = TreeSitterHighlighter::new(lang::PYTHON).expect("Python grammar should load");
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..2);
        assert_eq!(result.len(), 2);

        // Line 0 should have keyword "def".
        assert!(!result[0].is_plain());
        let has_keyword = result[0]
            .spans
            .iter()
            .any(|s| s.style == HighlightStyle::Keyword);
        assert!(has_keyword, "should have keyword highlight for 'def'");
    }

    #[test]
    fn highlight_javascript_snippet() {
        let source = "function greet(name) {\n  return \"Hello \" + name;\n}\n";
        let buf = TextBuffer::from_text(source);
        let mut hl = TreeSitterHighlighter::new(lang::JAVASCRIPT).expect("JS grammar should load");
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..3);
        assert_eq!(result.len(), 3);

        // Line 0 should have at least a keyword and function highlight.
        assert!(!result[0].is_plain());
    }

    #[test]
    fn unsupported_language_returns_none() {
        assert!(TreeSitterHighlighter::new(lang::MARKDOWN).is_none());
        assert!(TreeSitterHighlighter::new(lang::TOML).is_none());
        assert!(TreeSitterHighlighter::new(lang::PLAIN_TEXT).is_none());
    }

    #[test]
    fn empty_buffer_produces_empty_spans() {
        let buf = TextBuffer::from_text("");
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();
        hl.update(&buf, None);

        let result = hl.highlight_lines(0..1);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_plain());
    }

    #[test]
    fn line_byte_offsets_correct() {
        let source = b"abc\ndef\n";
        let offsets = compute_line_byte_offsets(source);
        assert_eq!(offsets, vec![0, 4, 8]);
    }

    #[test]
    fn has_ts_support_check() {
        assert!(has_tree_sitter_support(lang::RUST));
        assert!(has_tree_sitter_support(lang::PYTHON));
        assert!(!has_tree_sitter_support(lang::MARKDOWN));
        assert!(!has_tree_sitter_support(lang::TOML));
    }

    #[test]
    fn update_and_highlight_performance() {
        use std::fmt::Write;

        // Generate a ~500-line Rust file.
        let mut source = String::new();
        for i in 0..500 {
            let _ = writeln!(
                source,
                "fn func_{i}(x: i32) -> i32 {{ let y = x + {i}; y * 2 }}"
            );
        }
        let buf = TextBuffer::from_text(&source);
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();

        // Measure update + highlight for a viewport of 50 lines.
        let start = std::time::Instant::now();
        hl.update(&buf, None);
        let _ = hl.highlight_lines(200..250);
        let elapsed = start.elapsed();

        // Should complete well under 100ms (target is <5ms for typical edits).
        assert!(
            elapsed.as_millis() < 100,
            "highlight took {elapsed:?}, expected <100ms"
        );
    }
}
