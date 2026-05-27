//! Editor pane widget.
//!
//! Renders the active `TextBuffer` with line numbers, cursor highlighting,
//! selection rendering, and viewport scrolling. Handles mouse clicks for
//! cursor placement and scroll wheel for viewport adjustment.
//!
//! When syntax highlighting data is available, lines are rendered with
//! styled spans from the theme instead of plain text.

use crate::primitives::{
    Buffer, Line, Modifier, Rect, Scrollbar, ScrollbarOrientation, ScrollbarState, Span,
    StatefulWidget, Style, Stylize, Widget,
};

use smallvec::SmallVec;

use lune_core::highlight::{HighlightedLine, StyledSpan};
use lune_core::ports::GutterSnapshot;
use lune_core::prelude::*;

use crate::highlight::theme::SyntaxTheme;
use crate::theme::Theme;
use crate::vim::VimMode;

use unicode_width::UnicodeWidthChar;

// ── Unicode display-width helpers ─────────────────────────────────────
//
// `Position::col` / `ViewportState::left_col` are character columns —
// the unit the buffer's edit and cursor math is built on.  Terminals
// render in display *cells*, so wide chars (CJK, emoji) advance two
// cells per char and combining marks advance zero.  These helpers
// translate between the two coordinate systems at the render boundary,
// keeping the wider editor's char-based positions correct while making
// the screen output align to actual cell widths.

/// Display width of a single character.  Combining marks contribute 0,
/// wide CJK / emoji contribute 2, everything else 1.  Control chars
/// (which would otherwise return `None`) are mapped to 0 so they don't
/// throw off cell counts when they sneak through editor input.
#[inline]
fn char_cell_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Sum of display widths for the first `char_col` characters of `s`.
///
/// When `char_col` exceeds the number of chars in `s`, the chars beyond
/// the end contribute width 1 each — matching the buffer's convention
/// that the cursor can rest one column past the last char of a line.
fn chars_to_display_cols(s: &str, char_col: usize) -> usize {
    let mut cells = 0usize;
    let mut chars_seen = 0usize;
    for ch in s.chars() {
        if chars_seen == char_col {
            return cells;
        }
        cells += char_cell_width(ch);
        chars_seen += 1;
    }
    // `char_col` past EOL — pad with single-cell virtual columns.
    cells + (char_col - chars_seen)
}

// ── Viewport state ────────────────────────────────────────────────────

/// Tracks the visible viewport of the editor pane.
#[derive(Clone, Debug, Default)]
pub struct ViewportState {
    /// First visible line in the viewport (0-based).
    pub top_line: usize,
    /// Horizontal scroll offset (0-based column).
    pub left_col: usize,
    /// Cached line content for the visible viewport.
    pub line_cache: LineCache,
}

/// Viewport-scoped line content cache.
///
/// Caches the `String` result of `rope.line(idx).to_string()` for each visible
/// line. Invalidated when the buffer identity or revision changes, or when the
/// viewport scrolls. In the common case (cursor blink, no edit, no scroll),
/// this eliminates ~80 `String` heap allocations per frame.
#[derive(Clone, Debug, Default)]
pub struct LineCache {
    /// Cached line strings, indexed by `line_idx - top_line`.
    lines: Vec<String>,
    /// The `top_line` at which this cache was built.
    top_line: usize,
    /// Number of lines cached (= viewport height at cache time).
    count: usize,
    /// Buffer revision when the cache was built.
    revision: u64,
    /// Identity of the buffer the cache was built from. Distinct buffers can
    /// share a revision number (e.g. two freshly-opened files both at 0), so
    /// revision alone is not sufficient to detect a buffer switch.
    buffer_id: Option<BufferId>,
}

impl LineCache {
    /// Prepare the cache for a new frame with the given viewport parameters.
    ///
    /// If the buffer identity, revision, and viewport haven't changed, this is
    /// a no-op (O(1)). Otherwise, re-fetches all visible lines from the buffer.
    pub fn prepare(&mut self, top_line: usize, height: usize, buffer: &TextBuffer) {
        let revision = buffer.revision();
        let buffer_id = buffer.id;
        if self.buffer_id == Some(buffer_id)
            && revision == self.revision
            && top_line == self.top_line
            && height == self.count
        {
            return; // Cache is still valid.
        }

        self.lines.clear();
        self.lines.reserve(height);
        let total = buffer.line_count();
        for i in 0..height {
            let line_idx = top_line + i;
            if line_idx < total {
                self.lines.push(buffer.line(line_idx).unwrap_or_default());
            } else {
                self.lines.push(String::new());
            }
        }
        self.top_line = top_line;
        self.count = height;
        self.revision = revision;
        self.buffer_id = Some(buffer_id);
    }

    /// Get a cached line by absolute line index.
    /// Must be called after `prepare()`.
    ///
    /// Returns `""` when `line_idx` is outside the cached window —
    /// including below `top_line`, where naive subtraction would
    /// underflow in debug builds.
    #[inline]
    pub fn get(&self, line_idx: usize) -> &str {
        line_idx
            .checked_sub(self.top_line)
            .and_then(|i| self.lines.get(i))
            .map_or("", String::as_str)
    }
}

impl ViewportState {
    /// Ensure the cursor is visible within the viewport.
    pub fn scroll_to_cursor(
        &mut self,
        cursor_line: usize,
        cursor_col: usize,
        height: usize,
        width: usize,
    ) {
        // Vertical scrolling — use saturating math throughout so that
        // panes with `height < 4` (or `width < 6` below) don't underflow.
        let scroll_margin = 3.min(height / 4);

        if cursor_line < self.top_line + scroll_margin {
            self.top_line = cursor_line.saturating_sub(scroll_margin);
        } else if cursor_line >= self.top_line + height.saturating_sub(scroll_margin) {
            self.top_line =
                cursor_line.saturating_sub(height.saturating_sub(scroll_margin).saturating_sub(1));
        }

        // Horizontal scrolling.
        let h_margin = 5.min(width / 4);

        if cursor_col < self.left_col + h_margin {
            self.left_col = cursor_col.saturating_sub(h_margin);
        } else if cursor_col >= self.left_col + width.saturating_sub(h_margin) {
            self.left_col =
                cursor_col.saturating_sub(width.saturating_sub(h_margin).saturating_sub(1));
        }
    }

    /// Scroll up by N lines.
    pub const fn scroll_up(&mut self, n: usize) {
        self.top_line = self.top_line.saturating_sub(n);
    }

    /// Scroll down by N lines, clamped to total line count.
    pub fn scroll_down(&mut self, n: usize, total_lines: usize, viewport_height: usize) {
        let max_top = total_lines.saturating_sub(viewport_height.max(1));
        self.top_line = (self.top_line + n).min(max_top);
    }
}

// ── Gutter ────────────────────────────────────────────────────────────

/// Compute the gutter width (line numbers column) based on total line count.
#[must_use]
pub const fn gutter_width(total_lines: usize) -> u16 {
    // Number of digits + 1 for padding.
    let mut digits = 1u16;
    let mut n = total_lines;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits + 1 // +1 for right padding space
}

/// Width of the git gutter column (1 character).
const GIT_GUTTER_WIDTH: u16 = 1;
/// Width of the editor scrollbar column (1 character).
const SCROLLBAR_WIDTH: u16 = 1;
/// Minimal vertical scrollbar track symbol.
const SCROLLBAR_TRACK: &str = "│";
/// High-contrast scrollbar thumb symbol.
const SCROLLBAR_THUMB: &str = "█";

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the editor pane (git gutter + line numbers + buffer content + cursor).
///
/// When `highlighted` is `Some`, the lines are rendered with syntax-colored
/// spans. Otherwise, plain white text is used.
///
/// When `gutter_marks` is `Some`, a 1-character-wide git gutter column
/// is rendered to the left of the line numbers with colored markers.
///
#[allow(
    clippy::cast_possible_truncation,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub fn render_editor_pane(
    area: Rect,
    buf: &mut Buffer,
    text_buf: Option<&TextBuffer>,
    viewport: &mut ViewportState,
    follow_cursor: bool,
    vim_mode: VimMode,
    highlighted: Option<&[HighlightedLine]>,
    syntax_theme: &SyntaxTheme,
    gutter_marks: Option<&GutterSnapshot>,
    search_matches: Option<&lune_core::search::SearchState>,
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let Some(text_buf) = text_buf else {
        render_welcome(area, buf, theme);
        return;
    };

    let total_lines = text_buf.line_count();
    let gw = gutter_width(total_lines);
    let git_gw = if gutter_marks.is_some() {
        GIT_GUTTER_WIDTH
    } else {
        0
    };
    let viewport_height = area.height as usize;
    let total_gutter = git_gw + gw;
    let show_scrollbar =
        area.width > total_gutter + SCROLLBAR_WIDTH && total_lines > viewport_height;
    let scrollbar_width = if show_scrollbar { SCROLLBAR_WIDTH } else { 0 };
    let content_width = area.width.saturating_sub(total_gutter + scrollbar_width) as usize;

    // Keep viewport snapped to cursor only when requested by the caller.
    let cursor = &text_buf.cursor.primary.head;
    if follow_cursor {
        viewport.scroll_to_cursor(cursor.line, cursor.col, viewport_height, content_width);
    }

    // Prepare the line cache: re-fetches only if revision or viewport changed.
    viewport
        .line_cache
        .prepare(viewport.top_line, viewport_height, text_buf);

    let selection = {
        let sel = &text_buf.cursor.primary;
        if sel.head == sel.anchor {
            None
        } else {
            let (start, end) = sel.ordered();
            Some((start, end))
        }
    };
    let secondary_cursors: Vec<Position> = text_buf
        .cursor
        .secondary
        .iter()
        .map(|sel| sel.head)
        .collect();
    let secondary_selections: Vec<(Position, Position)> = text_buf
        .cursor
        .secondary
        .iter()
        .filter(|sel| !sel.is_cursor())
        .map(Selection::ordered)
        .collect();

    // Reusable format buffer for line numbers — avoids a `format!()` heap
    // allocation per visible line.
    let mut line_num_buf = String::with_capacity(16);

    for row in 0..viewport_height {
        let line_idx = viewport.top_line + row;
        let y = area.y + row as u16;
        clear_editor_row(area.x, y, area.width, buf, theme);

        if line_idx < total_lines {
            // Column offset accumulator — tracks how far right we are.
            let mut col_offset: u16 = 0;

            // Render git gutter mark (if active).
            if let Some(marks) = gutter_marks {
                render_git_gutter(area.x + col_offset, y, line_idx, marks, buf, theme);
                col_offset += git_gw;
            }

            render_line_number(
                area.x + col_offset,
                y,
                gw,
                line_idx,
                cursor.line == line_idx,
                &mut line_num_buf,
                buf,
                theme,
            );

            // Look up highlighted spans for this line.
            // PERF: binary search O(log n) — HighlightedLine entries are sorted by
            // line index (tree-sitter produces them in source order).
            let hl_line = highlighted.and_then(|lines| {
                lines
                    .binary_search_by_key(&line_idx, |hl| hl.line)
                    .ok()
                    .map(|i| &lines[i])
            });

            // Fetch the line from the cache (already prepared above).
            let cached_line = viewport.line_cache.get(line_idx);

            render_line_content(
                area.x + total_gutter,
                y,
                content_width,
                cached_line,
                line_idx,
                viewport.left_col,
                cursor,
                &secondary_cursors,
                &secondary_selections,
                vim_mode,
                selection.as_ref(),
                search_matches,
                hl_line,
                syntax_theme,
                buf,
                theme,
            );
        } else {
            // Tilde for lines past end of buffer.
            Line::from(Span::from("~").dim()).render(
                Rect::new(area.x, y, area.width.saturating_sub(scrollbar_width), 1),
                buf,
            );
        }
    }

    if show_scrollbar {
        render_vertical_scrollbar(area, total_lines, viewport, viewport_height, buf, theme);
    }
}

/// Clear a single editor row so stale text does not persist between frames.
fn clear_editor_row(x: u16, y: u16, width: u16, buf: &mut Buffer, theme: &Theme) {
    let clear_style = Style::new().fg(theme.fg).bg(theme.bg);
    for dx in 0..width {
        let cell = &mut buf[(x + dx, y)];
        cell.set_symbol(" ");
        cell.set_style(clear_style);
    }
}

/// Render a line number in the gutter.
///
/// Reuses `fmt_buf` across calls to avoid a `format!()` heap allocation per
/// visible line (typically ~80 allocations/frame eliminated).
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn render_line_number(
    x: u16,
    y: u16,
    gw: u16,
    line_idx: usize,
    is_current: bool,
    fmt_buf: &mut String,
    buf: &mut Buffer,
    theme: &Theme,
) {
    use std::fmt::Write;
    fmt_buf.clear();
    let _ = write!(
        fmt_buf,
        "{:>width$} ",
        line_idx + 1,
        width = (gw - 1) as usize
    );
    let span = if is_current {
        Span::styled(fmt_buf.as_str(), theme.editor_gutter_active)
    } else {
        Span::styled(fmt_buf.as_str(), theme.editor_gutter_inactive)
    };
    Line::from(span).render(Rect::new(x, y, gw, 1), buf);
}

/// Render the git gutter mark for a single line.
///
/// Displays a colored character:
/// - `│` green for added lines
/// - `│` yellow for modified lines
/// - `▾` red for deleted lines (at the line above the deletion)
fn render_git_gutter(
    x: u16,
    y: u16,
    line_idx: usize,
    marks: &GutterSnapshot,
    buf: &mut Buffer,
    theme: &Theme,
) {
    // `added` / `modified` / `deleted` are sorted `Vec<u32>` on the
    // port snapshot. Three `O(log n)` binary searches per visible line
    // — well under the cost of building a hashmap per frame.
    let Ok(line) = u32::try_from(line_idx) else {
        return;
    };
    let (ch, color) = if marks.added.binary_search(&line).is_ok() {
        ("│", theme.git_added)
    } else if marks.modified.binary_search(&line).is_ok() {
        ("│", theme.git_modified)
    } else if marks.deleted.binary_search(&line).is_ok() {
        ("▾", theme.git_deleted)
    } else {
        return;
    };
    let span = Span::styled(ch, Style::new().fg(color));
    Line::from(span).render(Rect::new(x, y, GIT_GUTTER_WIDTH, 1), buf);
}

/// Render the editor's vertical scrollbar.
fn render_vertical_scrollbar(
    area: Rect,
    total_lines: usize,
    viewport: &ViewportState,
    viewport_height: usize,
    buf: &mut Buffer,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let viewport_len = viewport_height.max(1);
    let max_top = total_lines.saturating_sub(viewport_len);
    // Represent the scroll domain as "top-line positions":
    // 0..=max_top. This makes thumb size proportional to visible/total and
    // ensures the thumb reaches the end at EOF.
    let scroll_domain_len = max_top.saturating_add(1).max(1);

    let mut state = ScrollbarState::new(scroll_domain_len)
        .position(viewport.top_line.min(max_top))
        .viewport_content_length(viewport_len);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some(SCROLLBAR_TRACK))
        .thumb_symbol(SCROLLBAR_THUMB)
        .track_style(Style::new().fg(theme.fg_dim))
        .thumb_style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD));

    StatefulWidget::render(scrollbar, area, buf, &mut state);
}

/// Extract a `&str` window into `s` starting at char column `start` whose
/// total display width does not exceed `width_cells`.
///
/// `start` is the leftmost *char* column (the unit the buffer uses); the
/// returned substring renders within `width_cells` terminal cells.  ASCII
/// retains its zero-walk fast path because `char == cell` everywhere.
///
/// Trailing chars whose remaining cell width would overflow the window
/// are dropped — the line renderer will pad with default-styled gap fill
/// rather than splitting a wide char across the viewport edge.
#[inline]
fn char_window(s: &str, start: usize, width_cells: usize) -> &str {
    if s.is_ascii() {
        let a = start.min(s.len());
        let b = (start + width_cells).min(s.len());
        return &s[a..b];
    }

    // Two-pass on the UTF-8 path: find the starting byte for char column
    // `start`, then walk widths from there until the cell budget runs out.
    let mut byte_start = s.len();
    let mut found_start = false;
    for (char_idx, (byte_idx, _)) in s.char_indices().enumerate() {
        if char_idx == start {
            byte_start = byte_idx;
            found_start = true;
            break;
        }
    }
    if !found_start {
        return "";
    }

    let mut acc = 0usize;
    let mut byte_end = byte_start;
    for (rel_byte_idx, ch) in s[byte_start..].char_indices() {
        let w = char_cell_width(ch);
        if acc + w > width_cells {
            break;
        }
        acc += w;
        byte_end = byte_start + rel_byte_idx + ch.len_utf8();
    }
    &s[byte_start..byte_end]
}

/// Render the text content of a single line with cursor, selection, and syntax highlighting.
///
/// `cached_line` is the pre-fetched line content from the `LineCache`,
/// avoiding a `rope.line().to_string()` heap allocation per line per frame.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn render_line_content(
    x: u16,
    y: u16,
    width: usize,
    cached_line: &str,
    line_idx: usize,
    left_col: usize,
    cursor: &Position,
    secondary_cursors: &[Position],
    secondary_selections: &[(Position, Position)],
    vim_mode: VimMode,
    selection: Option<&(Position, Position)>,
    search_matches: Option<&lune_core::search::SearchState>,
    hl_line: Option<&HighlightedLine>,
    theme: &SyntaxTheme,
    buf: &mut Buffer,
    ui_theme: &Theme,
) {
    let line_text = cached_line.trim_end_matches('\n').trim_end_matches('\r');

    // Render the text — either with syntax highlighting or plain.
    // PERF: char_window returns a &str slice (zero-alloc ASCII fast-path); the
    // styled path passes raw line_text + offsets so build_styled_line slices in-place.
    let rect = Rect::new(x, y, width as u16, 1);

    if let Some(hl) = hl_line {
        if hl.is_plain() {
            Line::from(char_window(line_text, left_col, width)).render(rect, buf);
        } else {
            let styled_spans = build_styled_line(line_text, left_col, width, &hl.spans, theme);
            Line::from(styled_spans).render(rect, buf);
        }
    } else {
        Line::from(char_window(line_text, left_col, width)).render(rect, buf);
    }

    // Apply search match highlighting.
    if let Some(search) = search_matches {
        apply_search_highlight(
            x, y, width, line_idx, left_col, search, buf, ui_theme, line_text,
        );
    }

    // Apply selection highlighting.
    if let Some((sel_start, sel_end)) = selection {
        apply_selection_highlight(
            x, y, width, line_idx, left_col, sel_start, sel_end, line_text, buf, ui_theme,
        );
    }
    for (sel_start, sel_end) in secondary_selections {
        apply_selection_highlight(
            x, y, width, line_idx, left_col, sel_start, sel_end, line_text, buf, ui_theme,
        );
    }

    // Render cursor.
    if cursor.line == line_idx {
        render_cursor(
            x, y, width, cursor, left_col, vim_mode, buf, ui_theme, line_text,
        );
    }

    for secondary in secondary_cursors {
        if secondary.line == line_idx {
            render_secondary_cursor(
                x, y, width, secondary, left_col, vim_mode, buf, ui_theme, line_text,
            );
        }
    }
}

/// Build a vector of ratatui `Span`s from highlight data, applying horizontal scroll.
///
/// Fills gaps between styled spans with `Default` style so the entire visible
/// width is covered.
fn build_styled_line<'a>(
    line_text: &'a str,
    left_col: usize,
    width: usize,
    spans: &[StyledSpan],
    theme: &SyntaxTheme,
) -> Vec<Span<'a>> {
    let right_col = left_col + width;

    // Build a char→byte offset lookup from char_indices, avoiding Vec<char> allocation.
    // PERF: SmallVec<128> avoids heap allocation for lines ≤ 127 chars (~90% of code).
    let char_byte_offsets: SmallVec<[usize; 128]> = line_text
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .chain(std::iter::once(line_text.len()))
        .collect();
    let total_cols = char_byte_offsets.len().saturating_sub(1);

    // Helper closure: convert a char column to a byte offset, clamped.
    let col_to_byte = |col: usize| -> usize {
        if col >= char_byte_offsets.len() {
            line_text.len()
        } else {
            char_byte_offsets[col]
        }
    };

    // Pre-allocate: spans.len() + 2 covers styled spans plus gap fills on both sides.
    let mut result: Vec<Span<'a>> = Vec::with_capacity(spans.len() + 2);
    let mut pos = left_col;

    for span in spans {
        // Skip spans entirely before the viewport.
        if span.end_col <= left_col {
            continue;
        }
        // Stop if past the viewport.
        if span.start_col >= right_col {
            break;
        }

        let span_start = span.start_col.max(left_col);
        let span_end = span.end_col.min(right_col).min(total_cols);

        if span_start > span_end {
            continue;
        }

        // Fill gap with default style — slice the original str directly.
        if span_start > pos {
            let gap_end = span_start.min(total_cols);
            if gap_end > pos {
                let byte_start = col_to_byte(pos);
                let byte_end = col_to_byte(gap_end);
                result.push(Span::from(&line_text[byte_start..byte_end]));
            }
        }

        // Add the styled span — slice the original str directly.
        if span_end > span_start {
            let byte_start = col_to_byte(span_start);
            let byte_end = col_to_byte(span_end);
            result.push(Span::styled(
                &line_text[byte_start..byte_end],
                theme.resolve(span.style),
            ));
        }

        pos = span_end;
    }

    // Fill remaining visible area with default style.
    let remaining_end = right_col.min(total_cols);
    if pos < remaining_end {
        let byte_start = col_to_byte(pos);
        let byte_end = col_to_byte(remaining_end);
        result.push(Span::from(&line_text[byte_start..byte_end]));
    }

    result
}

/// Convert a char-based cursor column to its on-screen display column,
/// relative to the viewport's left edge.
///
/// Returns `None` when the cursor sits left of the viewport (positions
/// where chars `0..left_col` cannot all be measured because the char
/// column is left of the viewport entirely).
fn cursor_screen_col(line_text: &str, cursor_col: usize, left_col: usize) -> Option<usize> {
    if cursor_col < left_col {
        return None;
    }
    let cursor_display = chars_to_display_cols(line_text, cursor_col);
    let left_display = chars_to_display_cols(line_text, left_col);
    cursor_display.checked_sub(left_display)
}

/// Render the cursor on a line cell.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_arguments)]
fn render_cursor(
    x: u16,
    y: u16,
    width: usize,
    cursor: &Position,
    left_col: usize,
    vim_mode: VimMode,
    buf: &mut Buffer,
    theme: &Theme,
    line_text: &str,
) {
    let Some(screen_col) = cursor_screen_col(line_text, cursor.col, left_col) else {
        return;
    };
    if screen_col < width {
        let cx = x + screen_col as u16;
        let cell = &mut buf[(cx, y)];

        match vim_mode {
            VimMode::Normal | VimMode::Visual | VimMode::VisualLine | VimMode::Command => {
                // Block cursor: reverse the cell.
                cell.set_style(theme.editor_cursor_normal);
            }
            VimMode::Insert => {
                // Line cursor: underline the cell.
                cell.set_style(theme.editor_cursor_insert);
            }
        }
    }
}

/// Render a secondary cursor on a line cell.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_arguments)]
fn render_secondary_cursor(
    x: u16,
    y: u16,
    width: usize,
    cursor: &Position,
    left_col: usize,
    vim_mode: VimMode,
    buf: &mut Buffer,
    theme: &Theme,
    line_text: &str,
) {
    let Some(screen_col) = cursor_screen_col(line_text, cursor.col, left_col) else {
        return;
    };
    if screen_col >= width {
        return;
    }

    let cx = x + screen_col as u16;
    let cell = &mut buf[(cx, y)];
    let style = match vim_mode {
        VimMode::Normal | VimMode::Visual | VimMode::VisualLine | VimMode::Command => Style::new()
            .fg(theme.bg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
        VimMode::Insert => Style::new()
            .fg(theme.accent)
            .add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
    };
    cell.set_style(style);
}

/// Paint background-only style onto every screen cell whose underlying
/// *char column* falls in `range_start_char..range_end_char`.
///
/// Single forward walk of `line_text` — accumulates display column and
/// resolves `left_col`'s display offset along the way, so the cost is
/// `O(line_chars + range_width)` regardless of how many ranges the
/// caller paints in succession.  Replaces the previous `O(N²)`
/// implementation that called `chars_to_display_cols` (`O(n)`) and
/// `chars().nth()` (`O(n)`) per visited column.
///
/// `apply_cell` is invoked once per cell that should be painted; the
/// caller chooses whether to `set_style`, `set_bg`, etc.
///
/// Trailing virtual columns past EOL are treated as single-cell — the
/// same convention `chars_to_display_cols` uses — so a selection that
/// extends past the last char still paints the empty-line padding.
#[inline]
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn paint_char_range<F>(
    line_text: &str,
    range_start_char: usize,
    range_end_char: usize,
    left_col: usize,
    width: usize,
    x: u16,
    y: u16,
    buf: &mut Buffer,
    mut apply_cell: F,
) where
    F: FnMut(&mut Buffer, u16, u16),
{
    if range_end_char <= range_start_char || width == 0 {
        return;
    }

    let mut chars_seen: usize = 0;
    let mut display_col: usize = 0;
    let mut left_display: usize = 0;
    let mut left_known = left_col == 0;

    for ch in line_text.chars() {
        if !left_known && chars_seen == left_col {
            left_display = display_col;
            left_known = true;
        }
        if chars_seen >= range_end_char {
            return;
        }
        let w = char_cell_width(ch);
        if left_known && chars_seen >= range_start_char {
            let screen_col = display_col - left_display;
            if screen_col >= width {
                return;
            }
            let paint_w = w.max(1);
            for offset in 0..paint_w {
                let sc = screen_col + offset;
                if sc >= width {
                    break;
                }
                apply_cell(buf, x + sc as u16, y);
            }
        }
        display_col += w;
        chars_seen += 1;
    }

    // Past EOL — pad with single-cell virtual columns so highlights that
    // extend beyond the last char (multi-line selection, search match
    // covering a trailing newline) still paint the empty cells.
    if !left_known {
        // `left_col` is itself past EOL; map it onto the virtual columns.
        // `left_known` doesn't need updating — the past-EOL loop below
        // doesn't read it.
        left_display = display_col + (left_col - chars_seen);
    }
    while chars_seen < range_end_char {
        if chars_seen >= range_start_char && display_col >= left_display {
            let screen_col = display_col - left_display;
            if screen_col >= width {
                return;
            }
            apply_cell(buf, x + screen_col as u16, y);
        }
        display_col += 1;
        chars_seen += 1;
    }
}

/// Apply selection highlighting to a line.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn apply_selection_highlight(
    x: u16,
    y: u16,
    width: usize,
    line_idx: usize,
    left_col: usize,
    sel_start: &Position,
    sel_end: &Position,
    line_text: &str,
    buf: &mut Buffer,
    theme: &Theme,
) {
    // Determine the selected column range on this line.
    let (line_sel_start, line_sel_end) = if line_idx < sel_start.line || line_idx > sel_end.line {
        return; // Line not in selection.
    } else if line_idx == sel_start.line && line_idx == sel_end.line {
        (sel_start.col, sel_end.col)
    } else if line_idx == sel_start.line {
        (sel_start.col, line_text.len())
    } else if line_idx == sel_end.line {
        (0, sel_end.col)
    } else {
        (0, line_text.len())
    };

    let sel_style = Style::new().bg(theme.selection_bg);
    paint_char_range(
        line_text,
        line_sel_start,
        line_sel_end,
        left_col,
        width,
        x,
        y,
        buf,
        |buf, cx, cy| {
            buf[(cx, cy)].set_style(sel_style);
        },
    );
}

/// Apply search match highlighting to a line.
///
/// Like [`apply_selection_highlight`], we iterate the char columns of the
/// match and paint every display cell those chars occupy — so wide CJK /
/// emoji glyphs get their full 2-cell highlight, and combining marks
/// don't smear the highlight to the wrong cell.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn apply_search_highlight(
    x: u16,
    y: u16,
    width: usize,
    line_idx: usize,
    left_col: usize,
    search: &lune_core::search::SearchState,
    buf: &mut Buffer,
    theme: &Theme,
    line_text: &str,
) {
    let current_idx = search.current_match;
    // Cache once per line — `chars().count()` would be O(n) per match
    // otherwise, and a single line can hold many matches.
    let mut line_char_count: Option<usize> = None;

    for (i, &(start, end)) in search.matches.iter().enumerate() {
        if end.line < line_idx || start.line > line_idx {
            continue;
        }

        let line_start_col = if start.line == line_idx { start.col } else { 0 };
        let line_end_col = if end.line == line_idx {
            end.col
        } else {
            *line_char_count.get_or_insert_with(|| line_text.chars().count())
        };

        if line_end_col <= line_start_col {
            continue;
        }

        let bg = if current_idx == Some(i) {
            theme.search_current_bg
        } else {
            theme.search_match_bg
        };
        paint_char_range(
            line_text,
            line_start_col,
            line_end_col,
            left_col,
            width,
            x,
            y,
            buf,
            |buf, cx, cy| {
                buf[(cx, cy)].set_bg(bg);
            },
        );
    }
}

/// Render the welcome screen when no buffer is open.
#[allow(clippy::cast_possible_truncation)]
fn render_welcome(area: Rect, buf: &mut Buffer, theme: &Theme) {
    let messages = [
        "Lune Editor",
        "",
        "Open a file to get started",
        "",
        "Ctrl+P  Command Palette",
        "Ctrl+B  Toggle File Tree",
        "Ctrl+`  Toggle Agents Tab",
        "Ctrl+Q  Quit",
    ];

    let start_y = area.y + area.height.saturating_sub(messages.len() as u16) / 2;

    for (i, msg) in messages.iter().enumerate() {
        let y = start_y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let x = area.x + area.width.saturating_sub(msg.len() as u16) / 2;
        let span = if i == 0 {
            Span::styled(*msg, theme.welcome_title)
        } else {
            Span::styled(*msg, theme.welcome_text)
        };
        Line::from(span).render(Rect::new(x, y, msg.len() as u16, 1), buf);
    }
}

/// Map a mouse click position to a buffer position, accounting for
/// gutter width (git gutter + line numbers) and viewport offset.
#[must_use]
pub const fn is_on_scrollbar(
    click_x: u16,
    click_y: u16,
    area: Rect,
    total_lines: usize,
    has_git_gutter: bool,
) -> bool {
    if click_x < area.x
        || click_y < area.y
        || click_x >= area.x + area.width
        || click_y >= area.y + area.height
    {
        return false;
    }

    let gw = gutter_width(total_lines);
    let git_gw = if has_git_gutter { GIT_GUTTER_WIDTH } else { 0 };
    let total_gutter = git_gw + gw;
    let has_scrollbar =
        area.width > total_gutter + SCROLLBAR_WIDTH && total_lines > area.height as usize;
    has_scrollbar && click_x >= area.x + area.width.saturating_sub(SCROLLBAR_WIDTH)
}

/// Convert a scrollbar row hit/drag position to a viewport top-line.
///
/// Returns `None` when the content does not overflow the viewport.
#[must_use]
pub fn scrollbar_row_to_top_line(row: u16, area: Rect, total_lines: usize) -> Option<usize> {
    let viewport_height = area.height as usize;
    let max_top = total_lines.saturating_sub(viewport_height.max(1));
    if max_top == 0 || area.height == 0 {
        return None;
    }

    let min_y = area.y;
    let max_y = area.y + area.height.saturating_sub(1);
    let clamped = row.clamp(min_y, max_y);
    let rel = (clamped - area.y) as usize;
    let denom = area.height.saturating_sub(1) as usize;
    if denom == 0 {
        return Some(0);
    }
    Some((rel * max_top + denom / 2) / denom)
}

/// Map a mouse click position to a buffer position, accounting for
/// gutter width (git gutter + line numbers) and viewport offset.
#[must_use]
pub const fn click_to_position(
    click_x: u16,
    click_y: u16,
    area: Rect,
    viewport: &ViewportState,
    total_lines: usize,
    has_git_gutter: bool,
) -> Option<Position> {
    // Check bounds.
    if click_x < area.x
        || click_y < area.y
        || click_x >= area.x + area.width
        || click_y >= area.y + area.height
    {
        return None;
    }

    let gw = gutter_width(total_lines);
    let git_gw = if has_git_gutter { GIT_GUTTER_WIDTH } else { 0 };
    let total_gutter = git_gw + gw;
    // Ignore clicks on the scrollbar column.
    if is_on_scrollbar(click_x, click_y, area, total_lines, has_git_gutter) {
        return None;
    }

    // Check if click is in gutter area.
    if click_x < area.x + total_gutter {
        return None;
    }

    let col = (click_x - area.x - total_gutter) as usize + viewport.left_col;
    let line = (click_y - area.y) as usize + viewport.top_line;

    Some(Position::new(line, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_cache_invalidates_on_buffer_switch() {
        // Two distinct buffers with the same shape (revision 0, same line count)
        // but different content. Switching between them must not serve stale
        // lines from the previous buffer.
        let buf_a = TextBuffer::from_text("alpha\nbeta\ngamma");
        let buf_b = TextBuffer::from_text("delta\nepsilon\nzeta");
        assert_ne!(buf_a.id, buf_b.id);
        assert_eq!(buf_a.revision(), buf_b.revision());

        let mut cache = LineCache::default();
        cache.prepare(0, 3, &buf_a);
        assert_eq!(cache.get(0).trim_end(), "alpha");

        cache.prepare(0, 3, &buf_b);
        assert_eq!(cache.get(0).trim_end(), "delta");
        assert_eq!(cache.get(1).trim_end(), "epsilon");
        assert_eq!(cache.get(2).trim_end(), "zeta");
    }

    #[test]
    fn chars_to_display_cols_ascii() {
        assert_eq!(chars_to_display_cols("hello", 0), 0);
        assert_eq!(chars_to_display_cols("hello", 3), 3);
        assert_eq!(chars_to_display_cols("hello", 5), 5);
        // Past end of string: each phantom column counts as 1 cell.
        assert_eq!(chars_to_display_cols("hello", 7), 7);
    }

    #[test]
    fn chars_to_display_cols_cjk() {
        // Three wide CJK chars: 6 cells total.
        let s = "你好世";
        assert_eq!(s.chars().count(), 3);
        assert_eq!(chars_to_display_cols(s, 0), 0);
        assert_eq!(chars_to_display_cols(s, 1), 2);
        assert_eq!(chars_to_display_cols(s, 2), 4);
        assert_eq!(chars_to_display_cols(s, 3), 6);
    }

    #[test]
    fn chars_to_display_cols_combining_mark_is_zero_width() {
        // 'e' + combining acute accent: visible as one glyph but two chars.
        // The combining mark contributes 0 cells.
        let s = "e\u{0301}f";
        assert_eq!(chars_to_display_cols(s, 0), 0);
        assert_eq!(chars_to_display_cols(s, 1), 1);
        assert_eq!(chars_to_display_cols(s, 2), 1);
        assert_eq!(chars_to_display_cols(s, 3), 2);
    }

    #[test]
    fn char_window_ascii_unchanged() {
        assert_eq!(char_window("hello world", 0, 5), "hello");
        assert_eq!(char_window("hello world", 6, 5), "world");
    }

    #[test]
    fn char_window_cjk_fits_in_cells() {
        let s = "abc你好def";
        // Window starts at char 0 with 4 cells: a(1)+b(1)+c(1)+你(2)=5 — too
        // wide. The window must stop after 'c' so the wide char isn't split.
        assert_eq!(char_window(s, 0, 4), "abc");
        // 5 cells fits "abc你".
        assert_eq!(char_window(s, 0, 5), "abc你");
    }

    #[test]
    fn cursor_screen_col_ascii() {
        assert_eq!(cursor_screen_col("hello", 3, 0), Some(3));
        assert_eq!(cursor_screen_col("hello", 3, 1), Some(2));
        assert_eq!(cursor_screen_col("hello", 0, 2), None);
    }

    #[test]
    fn cursor_screen_col_after_wide_char() {
        // After "你" (wide, 2 cells), char col 1 is at display col 2.
        let line = "你好";
        assert_eq!(cursor_screen_col(line, 1, 0), Some(2));
        assert_eq!(cursor_screen_col(line, 2, 0), Some(4));
    }

    // ── paint_char_range parity tests ─────────────────────────────────
    //
    // These pin the post-refactor forward-walk implementation to the
    // exact pixel pattern the previous O(N²) version produced, so the
    // selection/search highlight visual output cannot regress silently.

    /// Capture which `(x, y)` cells `paint_char_range` paints, as a
    /// sorted Vec for stable comparisons.
    fn capture_painted_cells(
        line: &str,
        range_start: usize,
        range_end: usize,
        left_col: usize,
        width: usize,
    ) -> Vec<u16> {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        paint_char_range(
            line,
            range_start,
            range_end,
            left_col,
            width,
            0,
            0,
            &mut buf,
            |buf, cx, cy| {
                buf[(cx, cy)].set_symbol("X");
            },
        );
        (0..area.width)
            .filter(|x| buf[(*x, 0)].symbol() == "X")
            .collect()
    }

    #[test]
    fn paint_char_range_ascii_full_line() {
        let cells = capture_painted_cells("hello", 0, 5, 0, 10);
        assert_eq!(cells, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn paint_char_range_ascii_with_scroll() {
        // left_col = 2 shifts char positions left by 2 cells.
        let cells = capture_painted_cells("hello world", 4, 9, 2, 10);
        // chars 4..9 = "o wor"; screen cols start at (display 4 - 2) = 2.
        assert_eq!(cells, vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn paint_char_range_wide_char_covers_two_cells() {
        // "a你b" — '你' is wide (2 cells).  Painting char range 1..2
        // covers BOTH cells of '你'.
        let cells = capture_painted_cells("a你b", 1, 2, 0, 10);
        assert_eq!(cells, vec![1, 2]);
    }

    #[test]
    fn paint_char_range_combining_mark_paints_one_cell() {
        // Combining mark (zero width) is forced to one cell so the
        // highlight isn't invisible.  Range covers only the mark.
        let cells = capture_painted_cells("e\u{0301}f", 1, 2, 0, 10);
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn paint_char_range_past_eol_pads_virtual_cells() {
        // Range extends past the last char; each virtual column gets
        // one cell of paint, up to the width budget.
        let cells = capture_painted_cells("ab", 0, 6, 0, 5);
        // chars 0..2 paint cells 0,1; virtual chars 2..5 paint cells 2,3,4.
        assert_eq!(cells, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn paint_char_range_width_clamps_wide_char_at_edge() {
        // The wide char would overflow the viewport — its second cell
        // is dropped; the first cell is still painted.
        let cells = capture_painted_cells("a你b", 0, 3, 0, 2);
        // a → cell 0; first half of 你 → cell 1; second half clipped.
        assert_eq!(cells, vec![0, 1]);
    }

    #[test]
    fn paint_char_range_empty_range_paints_nothing() {
        assert!(capture_painted_cells("hello", 3, 3, 0, 10).is_empty());
        assert!(capture_painted_cells("hello", 5, 2, 0, 10).is_empty());
    }

    #[test]
    fn paint_char_range_zero_width_paints_nothing() {
        assert!(capture_painted_cells("hello", 0, 5, 0, 0).is_empty());
    }

    #[test]
    fn paint_char_range_left_col_past_eol() {
        // left_col is itself past the end of the line.  Selection that
        // also lives past EOL should paint into the virtual columns
        // starting from the viewport's left edge.
        let cells = capture_painted_cells("ab", 4, 7, 4, 5);
        // virtual chars 4..7 mapped past left_col=4 → screen cols 0,1,2.
        assert_eq!(cells, vec![0, 1, 2]);
    }

    #[test]
    fn gutter_width_small() {
        assert_eq!(gutter_width(1), 2); // "1 "
        assert_eq!(gutter_width(9), 2); // "9 "
        assert_eq!(gutter_width(10), 3); // "10 "
        assert_eq!(gutter_width(99), 3); // "99 "
        assert_eq!(gutter_width(100), 4); // "100 "
        assert_eq!(gutter_width(1000), 5);
    }

    #[test]
    fn viewport_scroll_to_cursor_vertical() {
        let mut vp = ViewportState::default();
        // viewport height 20, cursor at line 25 => should scroll down.
        vp.scroll_to_cursor(25, 0, 20, 80);
        assert!(vp.top_line > 0);
        assert!(25 >= vp.top_line && 25 < vp.top_line + 20);
    }

    #[test]
    fn viewport_scroll_to_cursor_horizontal() {
        let mut vp = ViewportState::default();
        // content width 80, cursor at col 100 => should scroll right.
        vp.scroll_to_cursor(0, 100, 20, 80);
        assert!(vp.left_col > 0);
        assert!(100 >= vp.left_col && 100 < vp.left_col + 80);
    }

    #[test]
    fn viewport_scroll_up() {
        let mut vp = ViewportState {
            top_line: 10,
            left_col: 0,
            ..ViewportState::default()
        };
        vp.scroll_up(5);
        assert_eq!(vp.top_line, 5);
        vp.scroll_up(100);
        assert_eq!(vp.top_line, 0);
    }

    #[test]
    fn viewport_scroll_down() {
        let mut vp = ViewportState::default();
        vp.scroll_down(10, 100, 20);
        assert_eq!(vp.top_line, 10);
        vp.scroll_down(200, 100, 20);
        assert_eq!(vp.top_line, 80); // max = 100 - 20 (last full screen)
    }

    #[test]
    fn click_to_position_in_gutter() {
        let area = Rect::new(0, 0, 80, 24);
        let vp = ViewportState::default();
        // Gutter for 100 lines = 4 cols ("100 "). Click at x=2 is in gutter.
        let pos = click_to_position(2, 5, area, &vp, 100, false);
        assert!(pos.is_none());
    }

    #[test]
    fn click_to_position_in_content() {
        let area = Rect::new(0, 0, 80, 24);
        let vp = ViewportState::default();
        // Gutter for 100 lines = 4 cols. Click at x=10 => col = 10-0-4 = 6.
        let pos = click_to_position(10, 5, area, &vp, 100, false);
        assert_eq!(pos, Some(Position::new(5, 6)));
    }

    #[test]
    fn click_to_position_with_scroll() {
        let area = Rect::new(0, 0, 80, 24);
        let vp = ViewportState {
            top_line: 50,
            left_col: 10,
            ..ViewportState::default()
        };
        let gw = gutter_width(200);
        let pos = click_to_position(gw + 5, 3, area, &vp, 200, false);
        assert_eq!(pos, Some(Position::new(53, 15))); // line: 3+50, col: 5+10
    }

    #[test]
    fn click_out_of_bounds() {
        let area = Rect::new(10, 5, 60, 20);
        let vp = ViewportState::default();
        // Click outside area.
        assert!(click_to_position(5, 10, area, &vp, 50, false).is_none());
        assert!(click_to_position(80, 10, area, &vp, 50, false).is_none());
    }

    #[test]
    fn click_on_scrollbar_returns_none() {
        let area = Rect::new(0, 0, 80, 24);
        let vp = ViewportState::default();
        assert!(click_to_position(79, 6, area, &vp, 200, false).is_none());
    }

    #[test]
    fn is_on_scrollbar_only_when_overflowing() {
        let area = Rect::new(0, 0, 80, 24);
        assert!(is_on_scrollbar(79, 6, area, 200, false));
        assert!(!is_on_scrollbar(79, 6, area, 10, false));
    }

    #[test]
    fn scrollbar_row_to_top_line_maps_full_range() {
        let area = Rect::new(0, 0, 80, 20);
        // max_top = 100 - 20 = 80
        assert_eq!(scrollbar_row_to_top_line(0, area, 100), Some(0));
        assert_eq!(scrollbar_row_to_top_line(19, area, 100), Some(80));
    }

    #[test]
    fn render_editor_pane_draws_secondary_cursor_with_accent_style() {
        let area = Rect::new(0, 0, 20, 3);
        let mut render_buf = Buffer::empty(area);
        let mut viewport = ViewportState::default();
        let mut text_buf = TextBuffer::from_text("alpha");
        text_buf.cursor = CursorState::at(Position::new(0, 0));
        assert!(text_buf.toggle_secondary_cursor(Position::new(0, 2)));

        let theme = Theme::dark();
        render_editor_pane(
            area,
            &mut render_buf,
            Some(&text_buf),
            &mut viewport,
            false,
            VimMode::Normal,
            None,
            &SyntaxTheme::dark(),
            None,
            None,
            &theme,
        );

        let cell = &render_buf[(4, 0)];
        assert_eq!(cell.style().bg, Some(theme.accent));
        assert_eq!(cell.style().fg, Some(theme.bg));
    }
}
