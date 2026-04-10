//! UI-agnostic syntax highlighting types.
//!
//! Defines the [`Highlighter`] trait and associated data structures that
//! describe *what* to highlight without specifying *how* to render it.
//! Concrete implementations live in `lune-ui`.

use std::ops::Range;

use smallvec::SmallVec;

use crate::buffer::TextBuffer;

// ── Highlight style categories ────────────────────────────────────────

/// Semantic category for a highlighted span.
///
/// Maps to a visual style via the theme layer in `lune-ui`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightStyle {
    /// Language keyword (`fn`, `let`, `if`, `for`, etc.).
    Keyword,
    /// Type name or annotation.
    Type,
    /// Function or method name.
    Function,
    /// String literal.
    String,
    /// Comment (line or block).
    Comment,
    /// Numeric literal.
    Number,
    /// Operator (`+`, `-`, `==`, etc.).
    Operator,
    /// Punctuation (brackets, commas, semicolons).
    Punctuation,
    /// Variable or parameter.
    Variable,
    /// Constant or enum variant.
    Constant,
    /// Attribute or decorator (`#[derive]`, `@decorator`).
    Attribute,
    /// Namespace or module path.
    Namespace,
    /// Error or invalid token.
    Error,
    /// Embedded content (e.g., code inside markdown).
    Embedded,
    /// Default / unstyled text.
    Default,
}

// ── Styled span ───────────────────────────────────────────────────────

/// A contiguous run of characters within a line sharing the same style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledSpan {
    /// Start column (0-based, inclusive).
    pub start_col: usize,
    /// End column (0-based, exclusive).
    pub end_col: usize,
    /// The highlight category.
    pub style: HighlightStyle,
}

impl StyledSpan {
    /// Create a new styled span.
    #[inline]
    #[must_use]
    pub const fn new(start_col: usize, end_col: usize, style: HighlightStyle) -> Self {
        Self {
            start_col,
            end_col,
            style,
        }
    }

    /// Width in columns.
    #[inline]
    #[must_use]
    pub const fn width(&self) -> usize {
        self.end_col - self.start_col
    }

    /// Whether this span is empty (zero width).
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start_col >= self.end_col
    }
}

// ── Highlighted line ──────────────────────────────────────────────────

/// Most lines have fewer than 8 styled spans; this threshold keeps them
/// entirely on the stack, avoiding a heap allocation per line per frame.
pub type SpanVec = SmallVec<[StyledSpan; 8]>;

/// A single line with its styled spans.
///
/// Spans are sorted by `start_col` and do not overlap.
/// Uses `SmallVec<[StyledSpan; 8]>` so lines with ≤8 spans incur zero
/// heap allocation (8 × 24 bytes = 192 bytes inline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightedLine {
    /// The 0-based line index in the buffer.
    pub line: usize,
    /// Styled spans, sorted by `start_col`, non-overlapping.
    pub spans: SpanVec,
}

impl HighlightedLine {
    /// Create a new highlighted line with no spans.
    #[inline]
    #[must_use]
    pub fn new(line: usize) -> Self {
        Self {
            line,
            spans: SpanVec::new(),
        }
    }

    /// Create a highlighted line from a vec of spans.
    #[inline]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // SmallVec has no const-compatible move
    pub fn with_spans(line: usize, spans: SpanVec) -> Self {
        Self { line, spans }
    }

    /// Whether this line has no highlight spans.
    #[inline]
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.spans.is_empty()
    }
}

// ── Highlighter trait ─────────────────────────────────────────────────

/// Trait for syntax highlighters.
///
/// Implementations parse buffer content and produce [`HighlightedLine`]
/// results for requested line ranges.
pub trait Highlighter: Send {
    /// Re-parse / update after a text change.
    ///
    /// - `buffer`: the current buffer contents.
    /// - `edit_range`: optional `(start_byte, old_end_byte)` for incremental
    ///   parsing. When `None`, triggers a full re-parse.
    fn update(&mut self, buffer: &TextBuffer, edit_range: Option<(usize, usize)>);

    /// Return styled spans for lines in the given range.
    ///
    /// The range is `[line_range.start, line_range.end)` (0-based).
    /// The returned slice contains one `HighlightedLine` per line in
    /// the range. The borrow is tied to `self`, so implementations are
    /// expected to maintain an internal cache that survives across
    /// calls — this lets the renderer walk visible lines without
    /// re-allocating on every frame.
    ///
    /// Takes `&mut self` so implementations can refresh/resize their
    /// internal cache lazily on demand.
    fn highlight_lines(&mut self, line_range: Range<usize>) -> &[HighlightedLine];
}

// ── Null highlighter ──────────────────────────────────────────────────

/// A no-op highlighter that produces empty spans for all lines.
///
/// Used as the default when no language-specific highlighter is available.
/// Maintains a tiny grow-only cache so the trait's slice-return contract
/// can be satisfied without allocating on every frame.
#[derive(Default)]
pub struct NullHighlighter {
    cache: Vec<HighlightedLine>,
}

impl Highlighter for NullHighlighter {
    fn update(&mut self, _buffer: &TextBuffer, _edit_range: Option<(usize, usize)>) {}

    fn highlight_lines(&mut self, line_range: Range<usize>) -> &[HighlightedLine] {
        // Grow the cache to cover the requested range, filling any new
        // slots with empty (plain) line entries. Since NullHighlighter
        // produces no spans, the cache only depends on the high-water
        // mark of the requested range.
        if line_range.end > self.cache.len() {
            let start = self.cache.len();
            self.cache.reserve(line_range.end - start);
            for i in start..line_range.end {
                self.cache.push(HighlightedLine::new(i));
            }
        }
        let end = line_range.end.min(self.cache.len());
        let start = line_range.start.min(end);
        &self.cache[start..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_span_basics() {
        let span = StyledSpan::new(0, 5, HighlightStyle::Keyword);
        assert_eq!(span.width(), 5);
        assert!(!span.is_empty());
        assert_eq!(span.style, HighlightStyle::Keyword);
    }

    #[test]
    fn styled_span_empty() {
        let span = StyledSpan::new(3, 3, HighlightStyle::Default);
        assert_eq!(span.width(), 0);
        assert!(span.is_empty());
    }

    #[test]
    fn highlighted_line_plain() {
        let line = HighlightedLine::new(0);
        assert!(line.is_plain());
        assert_eq!(line.line, 0);
    }

    #[test]
    fn highlighted_line_with_spans() {
        let spans: SpanVec = smallvec::smallvec![
            StyledSpan::new(0, 2, HighlightStyle::Keyword),
            StyledSpan::new(3, 8, HighlightStyle::Function),
        ];
        let line = HighlightedLine::with_spans(5, spans);
        assert!(!line.is_plain());
        assert_eq!(line.line, 5);
        assert_eq!(line.spans.len(), 2);
    }

    #[test]
    fn span_ordering() {
        let a = StyledSpan::new(0, 5, HighlightStyle::Keyword);
        let b = StyledSpan::new(6, 12, HighlightStyle::String);
        // Spans should be ordered by start_col.
        assert!(a.start_col < b.start_col);
        // And non-overlapping.
        assert!(a.end_col <= b.start_col);
    }

    #[test]
    fn null_highlighter_produces_empty_spans() {
        let mut hl = NullHighlighter::default();
        let buf = TextBuffer::from_text("fn main() {\n    println!(\"hi\");\n}\n");
        hl.update(&buf, None);
        let lines = hl.highlight_lines(0..3);
        assert_eq!(lines.len(), 3);
        for line in lines {
            assert!(line.is_plain());
        }
    }

    #[test]
    fn highlight_style_is_copy() {
        // Ensure HighlightStyle is Copy (important for performance).
        let s = HighlightStyle::Keyword;
        let s2 = s;
        assert_eq!(s, s2);
    }
}
