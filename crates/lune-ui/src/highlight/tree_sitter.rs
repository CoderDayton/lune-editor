//! Tree-sitter-based syntax highlighter.
//!
//! Uses the vendored [`lune_ts_highlight`] crate to parse source code and produce
//! [`HighlightedLine`] spans that map to [`HighlightStyle`] categories.

use std::ops::Range;

use lune_core::highlight::{BufferEdit, HighlightStyle, HighlightedLine, Highlighter, StyledSpan};
use lune_core::language::{LanguageId, lang};
use lune_core::prelude::TextBuffer;
use lune_ts_highlight::{HighlightConfiguration, HighlightEvent};

// ── Recognized highlight names ────────────────────────────────────────

/// The ordered list of highlight names we recognize.
///
/// The index in this array corresponds to the `Highlight(usize)` value
/// returned by `lune_ts_highlight`. We map each index to a
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
/// Each [`update`](Highlighter::update) re-caches the full buffer source and
/// its line byte offsets and bumps a generation counter. The first
/// [`highlight_lines`](Highlighter::highlight_lines) call in a new generation
/// re-runs tree-sitter over the whole source and stores a per-line highlight
/// result; later calls in the same generation slice that cached result, so
/// scrolling the viewport never re-parses.
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
    ts_highlighter: lune_ts_highlight::Highlighter,
    /// Monotonic generation counter, incremented on every `update()`.
    generation: u64,
    /// Full-file highlight result for `cached_generation`, sliced per request.
    cached_result: Vec<HighlightedLine>,
    /// The generation at which `cached_result` was computed.
    cached_generation: u64,
    /// Edits accumulated across `update()` calls since the last full
    /// reparse, in tree-sitter's native form. Fed to
    /// `highlight_incremental` so the parser can reuse the previous tree,
    /// then cleared once the highlight events have been consumed. Empty =
    /// full reparse.
    pending_edits: Vec<tree_sitter::InputEdit>,
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
            ts_highlighter: lune_ts_highlight::Highlighter::new(),
            generation: 0,
            cached_result: Vec::new(),
            cached_generation: u64::MAX, // force miss on first call
            pending_edits: Vec::new(),
        })
    }

    /// The language this highlighter handles.
    pub const fn language_id(&self) -> LanguageId {
        self.language_id
    }
}

impl Highlighter for TreeSitterHighlighter {
    fn update(&mut self, buffer: &TextBuffer, edits: &[BufferEdit]) {
        // Bump generation to invalidate the highlight cache.
        self.generation = self.generation.wrapping_add(1);

        // Accumulate the byte+point deltas in tree-sitter's native form so
        // the next reparse can reuse the previous tree. Multiple `update`
        // calls may land before a rebuild; the deltas are already in
        // successive coordinate spaces, so we just append. An empty `edits`
        // slice contributes nothing and (if `pending_edits` is still empty)
        // yields a full reparse.
        self.pending_edits
            .extend(edits.iter().map(|e| tree_sitter::InputEdit {
                start_byte: e.start_byte,
                old_end_byte: e.old_end_byte,
                new_end_byte: e.new_end_byte,
                start_position: tree_sitter::Point::new(e.start_point.0, e.start_point.1),
                old_end_position: tree_sitter::Point::new(e.old_end_point.0, e.old_end_point.1),
                new_end_position: tree_sitter::Point::new(e.new_end_point.0, e.new_end_point.1),
            }));

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

    fn highlight_lines(&mut self, line_range: Range<usize>) -> &[HighlightedLine] {
        let total_lines = self.line_byte_offsets.len();

        if self.source.is_empty() || total_lines == 0 {
            // Fast path: empty buffer. Lazily grow a plain-line cache
            // big enough to satisfy the request.
            self.ensure_empty_cache(line_range.end);
            let end = line_range.end.min(self.cached_result.len());
            let start = line_range.start.min(end);
            return &self.cached_result[start..end];
        }

        // The cache stores the full file's highlight result for the
        // current generation. Any per-frame request is served by slicing
        // from that. Invalidated only when the buffer is edited (the
        // generation bumps via `update`), not when the viewport scrolls.
        if self.cached_generation != self.generation {
            self.rebuild_full_cache(total_lines);
        }

        let end_line = line_range.end.min(self.cached_result.len());
        let start_line = line_range.start.min(end_line);
        &self.cached_result[start_line..end_line]
    }
}

impl TreeSitterHighlighter {
    /// Ensure the cache covers at least the requested line count with
    /// plain (unstyled) entries. Used on the empty-buffer fast path to
    /// honour the trait's slice-return contract without allocating on
    /// every frame.
    fn ensure_empty_cache(&mut self, line_count: usize) {
        if line_count > self.cached_result.len() {
            let start = self.cached_result.len();
            self.cached_result.reserve(line_count - start);
            for i in start..line_count {
                self.cached_result.push(HighlightedLine::new(i));
            }
        }
        self.cached_generation = self.generation;
    }

    /// Re-run tree-sitter over the entire source and rebuild
    /// `cached_result` as a per-line vector. Called at most once per
    /// buffer edit, not per frame.
    fn rebuild_full_cache(&mut self, total_lines: usize) {
        let mut result: Vec<HighlightedLine> = (0..total_lines).map(HighlightedLine::new).collect();

        // Reuse the cached `ts_highlighter` instance to avoid re-allocating
        // its internal buffers on every edit. `pending_edits` lets the
        // parser reuse the previous tree; an empty slice forces a full
        // reparse (identical result).
        let Ok(events) = self.ts_highlighter.highlight_incremental(
            &self.config,
            &self.source,
            &self.pending_edits,
            None,
            |_| None,
        ) else {
            // Reparse failed: drop the now-unusable edit hints so the next
            // rebuild reparses the freshly cached source from scratch.
            self.pending_edits.clear();
            self.cached_result = result;
            self.cached_generation = self.generation;
            return;
        };

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
                    if let Some(style) = style_stack.last().copied() {
                        add_spans_for_byte_range(
                            start..end,
                            style,
                            &self.line_byte_offsets,
                            &mut result,
                        );
                    }
                }
            }
        }

        // The `events` iterator (which borrowed `ts_highlighter` and held the
        // accumulated edits applied to the parser) is now dropped, so it is
        // safe to clear the consumed edits. The next reparse will be a full
        // one unless new edits arrive.
        self.pending_edits.clear();

        self.cached_result = result;
        self.cached_generation = self.generation;
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

/// Distribute a styled byte range across the lines it covers, pushing one
/// [`StyledSpan`] per line into `result`.
///
/// `result` is indexed by absolute line number and holds one entry per line
/// in `line_byte_offsets`.
fn add_spans_for_byte_range(
    byte_range: Range<usize>,
    style: HighlightStyle,
    line_byte_offsets: &[usize],
    result: &mut [HighlightedLine],
) {
    let (byte_start, byte_end) = (byte_range.start, byte_range.end);
    if byte_start >= byte_end {
        return;
    }

    // Find which line `byte_start` falls on.
    let start_line = match line_byte_offsets.binary_search(&byte_start) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };

    // Walk forward from there, clipping the range to each line.
    for line_idx in start_line..line_byte_offsets.len() {
        let line_start_byte = line_byte_offsets[line_idx];

        // Once a line starts past the range end, no later line can overlap.
        if line_start_byte >= byte_end {
            break;
        }

        let line_end_byte = line_byte_offsets
            .get(line_idx + 1)
            .copied()
            .unwrap_or(usize::MAX);

        let span_start_byte = byte_start.max(line_start_byte);
        let span_end_byte = byte_end.min(line_end_byte);
        if span_start_byte >= span_end_byte {
            continue;
        }

        let start_col = span_start_byte - line_start_byte;
        let end_col = span_end_byte - line_start_byte;

        if line_idx < result.len() {
            result[line_idx]
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
        hl.update(&buf, &[]);

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
        hl.update(&buf, &[]);

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
        hl.update(&buf, &[]);

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
        hl.update(&buf, &[]);

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
        hl.update(&buf, &[]);
        let _ = hl.highlight_lines(200..250);
        let elapsed = start.elapsed();

        // Should complete well under 100ms (target is <5ms for typical edits).
        assert!(
            elapsed.as_millis() < 100,
            "highlight took {elapsed:?}, expected <100ms"
        );
    }

    // ── Incremental == full reparse equality ──────────────────────────

    /// Snapshot per-line spans for `[0, line_count)`.
    fn snapshot_spans(hl: &mut TreeSitterHighlighter, line_count: usize) -> Vec<Vec<StyledSpan>> {
        hl.highlight_lines(0..line_count)
            .iter()
            .map(|l| l.spans.to_vec())
            .collect()
    }

    /// Spans produced by a fresh full reparse of `buf`.
    fn full_reparse_spans(buf: &TextBuffer, line_count: usize) -> Vec<Vec<StyledSpan>> {
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();
        hl.update(buf, &[]);
        snapshot_spans(&mut hl, line_count)
    }

    #[test]
    fn incremental_insert_matches_full_reparse() {
        use lune_core::prelude::Position;

        let mut buf = TextBuffer::from_text("fn main() {\n    let x = 1;\n}\n");
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();

        // Prime with a full parse.
        hl.update(&buf, &[]);
        let _ = snapshot_spans(&mut hl, buf.line_count());

        // Real edit: insert a new statement on a new line after `let x = 1;`.
        buf.insert(Position::new(1, 14), "\n    let y = x + 2;");
        let edits = buf.take_pending_edits();
        assert!(!edits.is_empty(), "insert must produce a pending edit");

        hl.update(&buf, &edits);
        let incremental = snapshot_spans(&mut hl, buf.line_count());

        let full = full_reparse_spans(&buf, buf.line_count());
        assert_eq!(
            incremental, full,
            "incremental insert must match a full reparse per line"
        );
    }

    #[test]
    fn incremental_delete_matches_full_reparse() {
        use lune_core::prelude::Position;

        let mut buf = TextBuffer::from_text("fn main() {\n    let x = 1;\n    let y = 2;\n}\n");
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();

        hl.update(&buf, &[]);
        let _ = snapshot_spans(&mut hl, buf.line_count());

        // Delete the entire "    let y = 2;\n" line (line 2).
        buf.delete(Position::new(2, 0), Position::new(3, 0));
        let edits = buf.take_pending_edits();
        assert!(!edits.is_empty(), "delete must produce a pending edit");

        hl.update(&buf, &edits);
        let incremental = snapshot_spans(&mut hl, buf.line_count());

        let full = full_reparse_spans(&buf, buf.line_count());
        assert_eq!(
            incremental, full,
            "incremental delete must match a full reparse per line"
        );
    }

    #[test]
    fn incremental_multi_edit_sequence_matches_full_reparse() {
        use lune_core::prelude::Position;

        let mut buf = TextBuffer::from_text("fn main() {\n    let x = 1;\n}\n");
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();

        hl.update(&buf, &[]);
        let _ = snapshot_spans(&mut hl, buf.line_count());

        // Two edits before a single rebuild: rename `x` -> `value`, then
        // insert a trailing comment. `replace` yields a delete + insert
        // pair, exercising multi-edit accumulation in `pending_edits`.
        buf.replace(Position::new(1, 8), Position::new(1, 9), "value");
        buf.insert(Position::new(1, 18), " // c");
        let edits = buf.take_pending_edits();
        assert!(edits.len() >= 2, "expected accumulated edits");

        hl.update(&buf, &edits);
        let incremental = snapshot_spans(&mut hl, buf.line_count());

        let full = full_reparse_spans(&buf, buf.line_count());
        assert_eq!(
            incremental, full,
            "incremental multi-edit must match a full reparse per line"
        );
    }

    #[test]
    fn incremental_large_file_matches_full_and_is_bounded() {
        use lune_core::prelude::Position;
        use std::fmt::Write;

        // ~1500-line Rust file.
        let mut src = String::new();
        for i in 0..1500 {
            let _ = writeln!(
                src,
                "fn func_{i}(x: i32) -> i32 {{ let y = x + {i}; y * 2 }}"
            );
        }
        let mut buf = TextBuffer::from_text(&src);
        let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();
        hl.update(&buf, &[]);
        let _ = snapshot_spans(&mut hl, buf.line_count());

        // A small edit near the end of the file.
        buf.insert(Position::new(1499, 0), "// note\n");
        let edits = buf.take_pending_edits();
        assert!(!edits.is_empty());

        let t0 = std::time::Instant::now();
        hl.update(&buf, &edits);
        let incremental = snapshot_spans(&mut hl, buf.line_count());
        let incremental_elapsed = t0.elapsed();

        let t1 = std::time::Instant::now();
        let full = full_reparse_spans(&buf, buf.line_count());
        let full_elapsed = t1.elapsed();

        assert_eq!(
            incremental, full,
            "incremental update on a large file must match a full reparse"
        );
        eprintln!("large-file update: incremental={incremental_elapsed:?} full={full_elapsed:?}");
        // Absolute bound (non-flaky); relative speedup is reported, not asserted.
        assert!(
            incremental_elapsed.as_millis() < 200,
            "incremental update took {incremental_elapsed:?}, expected <200ms"
        );
    }

    // ── Cost-split measurement (Stage 1) ──────────────────────────────
    //
    // Decomposes one incremental edit's highlight cost into parse / query-walk
    // / span-build, to learn how much of the ~10x target is reachable and
    // whether limiting the query range alone suffices or a per-line cache
    // splice is also required. Mirrors `rebuild_full_cache` (the production
    // rebuild path) but times each sub-step. `#[ignore]`d: it is a
    // measurement, not a pass/fail assertion, and prints with `--nocapture`.
    //
    //   cargo test -p lune-ui \
    //     highlight::tree_sitter::tests::measure_incremental_cost_split \
    //     -- --ignored --nocapture
    #[test]
    #[ignore = "measurement; run with --ignored --nocapture"]
    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        clippy::similar_names
    )]
    fn measure_incremental_cost_split() {
        use lune_core::prelude::Position;
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        const REPS: usize = 30;
        const WARMUP: usize = 8;
        const BLOCKS: usize = 90; // ~21 lines/block => ~1890 lines

        // A varied Rust file: doc comments, attributes, structs, generics,
        // string literals, the `format!` macro, methods and operators — so the
        // highlight query has realistic work, not a single repeated token.
        fn gen_source(blocks: usize) -> String {
            const TEMPLATE: &str = r#"/// Documentation for `Widget$I`, a sample container type.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget$I {
    name: String,
    count: usize,
    tags: Vec<String>,
}

impl Widget$I {
    /// Create a new widget named `name`.
    pub fn new(name: &str) -> Self {
        // initialize with default tags
        Self { name: name.to_string(), count: $I, tags: vec!["alpha".into(), "beta".into()] }
    }

    pub fn describe(&self) -> String {
        format!("widget {} holds {} items", self.name, self.count)
    }
}

"#;
            let mut s = String::with_capacity(TEMPLATE.len() * blocks);
            for i in 0..blocks {
                s.push_str(&TEMPLATE.replace("$I", &i.to_string()));
            }
            s
        }

        // Build a highlighter that has completed a full parse (so `prev_root`
        // is populated and the next reparse is incremental), then apply a
        // single one-character edit in the middle of the file and stage it.
        // Returns the primed highlighter and the post-edit line count.
        // The priming cost is intentionally OUTSIDE every timer.
        fn prime(src: &str) -> (TreeSitterHighlighter, usize) {
            let mut buf = TextBuffer::from_text(src);
            let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();
            hl.update(&buf, &[]);
            let total = buf.line_count();
            let _ = hl.highlight_lines(0..total); // full parse -> prev_root set
            let mid = total / 2;
            buf.insert(Position::new(mid, 0), "z");
            let edits = buf.take_pending_edits();
            assert!(!edits.is_empty(), "insert must produce a pending edit");
            hl.update(&buf, &edits); // stage incremental edit
            (hl, buf.line_count())
        }

        let src = gen_source(BLOCKS);
        let total_lines = TextBuffer::from_text(&src).line_count();

        // ── Bucket: setup (incremental parse) + consume_full (query + add_spans).
        // Both come from one primed run: the parse happens eagerly inside the
        // `highlight_incremental` call; the query walk is lazy and unfolds while
        // draining the event iterator (exactly as in `rebuild_full_cache`).
        let mut setup_s: Vec<Duration> = Vec::new();
        let mut consume_full_s: Vec<Duration> = Vec::new();
        for rep in 0..(WARMUP + REPS) {
            let (mut hl, total) = prime(&src);

            let t = Instant::now();
            let events = hl
                .ts_highlighter
                .highlight_incremental(&hl.config, &hl.source, &hl.pending_edits, None, |_| None)
                .expect("incremental highlight");
            let setup = t.elapsed();

            // Allocate the result vec BEFORE the consume timer (the allocation
            // is measured in its own bucket).
            let mut result: Vec<HighlightedLine> = (0..total).map(HighlightedLine::new).collect();
            let mut style_stack: Vec<HighlightStyle> = Vec::new();

            let t = Instant::now();
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
                        if let Some(style) = style_stack.last().copied() {
                            add_spans_for_byte_range(
                                start..end,
                                style,
                                &hl.line_byte_offsets,
                                &mut result,
                            );
                        }
                    }
                }
            }
            let consume = t.elapsed();
            black_box(&result);

            if rep >= WARMUP {
                setup_s.push(setup);
                consume_full_s.push(consume);
            }
        }

        // ── Bucket: consume_noop (query walk only). Identical loop, but the
        // `Source` arm black-boxes its arguments instead of distributing spans.
        // Isolates the cursor walk / event generation from the span build.
        let mut consume_noop_s: Vec<Duration> = Vec::new();
        for rep in 0..(WARMUP + REPS) {
            let (mut hl, _total) = prime(&src);
            let events = hl
                .ts_highlighter
                .highlight_incremental(&hl.config, &hl.source, &hl.pending_edits, None, |_| None)
                .expect("incremental highlight");
            let mut style_stack: Vec<HighlightStyle> = Vec::new();

            let t = Instant::now();
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
                        if let Some(style) = style_stack.last().copied() {
                            black_box((start, end, style));
                        }
                    }
                }
            }
            let consume = t.elapsed();
            if rep >= WARMUP {
                consume_noop_s.push(consume);
            }
        }

        // ── Bucket: alloc (the full per-line result vector).
        let mut alloc_s: Vec<Duration> = Vec::new();
        for rep in 0..(WARMUP + REPS) {
            let t = Instant::now();
            let v: Vec<HighlightedLine> = (0..total_lines).map(HighlightedLine::new).collect();
            let a = t.elapsed();
            black_box(&v);
            if rep >= WARMUP {
                alloc_s.push(a);
            }
        }

        // ── End-to-end: full reparse (T_full). Fresh highlighter => prev_root
        // is None => `highlight_lines` triggers a full rebuild.
        let mut tfull_s: Vec<Duration> = Vec::new();
        for rep in 0..(WARMUP + REPS) {
            let buf = TextBuffer::from_text(&src);
            let mut hl = TreeSitterHighlighter::new(lang::RUST).unwrap();
            hl.update(&buf, &[]);
            let total = buf.line_count();
            let t = Instant::now();
            let _ = hl.highlight_lines(0..total);
            let e = t.elapsed();
            if rep >= WARMUP {
                tfull_s.push(e);
            }
        }

        // ── End-to-end: incremental edit (T_inc). The real production path.
        let mut tinc_s: Vec<Duration> = Vec::new();
        for rep in 0..(WARMUP + REPS) {
            let (mut hl, total) = prime(&src);
            let t = Instant::now();
            let _ = hl.highlight_lines(0..total);
            let e = t.elapsed();
            if rep >= WARMUP {
                tinc_s.push(e);
            }
        }

        // ── Reduce. `min` is the cleanest estimator of true cost (least
        // perturbed by the scheduler); `mean` is printed alongside for context.
        let mean = |xs: &[Duration]| -> f64 {
            xs.iter().map(Duration::as_secs_f64).sum::<f64>() / xs.len() as f64
        };
        let min = |xs: &[Duration]| -> f64 {
            xs.iter()
                .map(Duration::as_secs_f64)
                .fold(f64::INFINITY, f64::min)
        };
        let us = |s: f64| s * 1.0e6; // seconds -> microseconds

        let parse_inc = min(&setup_s);
        let c_q = min(&consume_noop_s);
        let add_spans = (min(&consume_full_s) - c_q).max(0.0);
        let alloc = min(&alloc_s);
        let c_s = add_spans + alloc;
        let t_full = min(&tfull_s);
        let t_inc = min(&tinc_s);

        let parse_full = (t_full - c_q - c_s).max(0.0);
        let f_parse = parse_full / t_full;
        let f_q = c_q / t_full;
        let f_s = c_s / t_full;

        let observed_speedup = t_full / t_inc;
        let decomp_t_inc = parse_inc + c_q + c_s; // should ~ t_inc
        let ceil_query_only = t_full / (parse_inc + c_s);
        let ceil_query_plus_splice = t_full / parse_inc.max(1.0e-9);

        eprintln!("\n=== incremental highlight cost split ===");
        eprintln!("file: {total_lines} lines, {} bytes", src.len());
        eprintln!("samples: {REPS} (after {WARMUP} warmup), reduced by min\n");

        eprintln!("end-to-end (min / mean us):");
        eprintln!(
            "  T_full (full reparse)   : {:8.1} / {:8.1}",
            us(t_full),
            us(mean(&tfull_s))
        );
        eprintln!(
            "  T_inc  (1-char edit)    : {:8.1} / {:8.1}",
            us(t_inc),
            us(mean(&tinc_s))
        );
        eprintln!("  observed speedup        : {observed_speedup:6.2}x\n");

        eprintln!("decomposed buckets (min us):");
        eprintln!("  parse_inc (setup)       : {:8.1}", us(parse_inc));
        eprintln!("  query walk (C_q)        : {:8.1}", us(c_q));
        eprintln!("  add_spans               : {:8.1}", us(add_spans));
        eprintln!("  alloc                   : {:8.1}", us(alloc));
        eprintln!("  span build (C_s)        : {:8.1}", us(c_s));
        eprintln!(
            "  parse_full (derived)    : {:8.1}   [= T_full - C_q - C_s]\n",
            us(parse_full)
        );

        eprintln!("fractions of T_full:");
        eprintln!("  f_parse                 : {:5.1}%", f_parse * 100.0);
        eprintln!("  f_q (query)             : {:5.1}%", f_q * 100.0);
        eprintln!("  f_s (span)              : {:5.1}%", f_s * 100.0);
        eprintln!(
            "  check: parse_inc+C_q+C_s = {:.1}us vs measured T_inc {:.1}us\n",
            us(decomp_t_inc),
            us(t_inc)
        );

        eprintln!("predicted speedup ceilings vs T_full:");
        eprintln!("  range-limit query only  : {ceil_query_only:6.2}x   [removes C_q]");
        eprintln!(
            "  range-limit + splice    : {ceil_query_plus_splice:6.2}x   [removes C_q and C_s; optimistic, ignores fixed overhead]\n"
        );

        // Keep results observable; assert only a trivially-true sanity bound so
        // the test never flakes on timing.
        assert!(t_full > 0.0 && t_inc > 0.0);
    }
}
