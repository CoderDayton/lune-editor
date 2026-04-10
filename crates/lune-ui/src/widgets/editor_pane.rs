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
use lune_core::prelude::*;
use lune_git::GutterMarks;

use crate::highlight::theme::SyntaxTheme;
use crate::theme::Theme;
use crate::vim::VimMode;

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
    #[inline]
    pub fn get(&self, line_idx: usize) -> &str {
        let idx = line_idx - self.top_line;
        if idx < self.lines.len() {
            &self.lines[idx]
        } else {
            ""
        }
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
        // Vertical scrolling.
        let scroll_margin = 3.min(height / 4);

        if cursor_line < self.top_line + scroll_margin {
            self.top_line = cursor_line.saturating_sub(scroll_margin);
        } else if cursor_line >= self.top_line + height - scroll_margin {
            self.top_line = cursor_line.saturating_sub(height - scroll_margin - 1);
        }

        // Horizontal scrolling.
        let h_margin = 5.min(width / 4);

        if cursor_col < self.left_col + h_margin {
            self.left_col = cursor_col.saturating_sub(h_margin);
        } else if cursor_col >= self.left_col + width - h_margin {
            self.left_col = cursor_col.saturating_sub(width - h_margin - 1);
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
    gutter_marks: Option<&GutterMarks>,
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
    marks: &GutterMarks,
    buf: &mut Buffer,
    theme: &Theme,
) {
    if let Some(mark) = marks.get(line_idx) {
        let (ch, color) = match mark {
            lune_git::GutterMark::Added => ("│", theme.git_added),
            lune_git::GutterMark::Modified => ("│", theme.git_modified),
            lune_git::GutterMark::Deleted => ("▾", theme.git_deleted),
        };
        let span = Span::styled(ch, Style::new().fg(color));
        Line::from(span).render(Rect::new(x, y, GIT_GUTTER_WIDTH, 1), buf);
    }
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

/// Extract a `&str` window into `s` starting at char offset `start` for `width` chars.
///
/// Zero-allocation ASCII fast-path (O(1) pointer arithmetic); correct UTF-8 fallback.
#[inline]
fn char_window(s: &str, start: usize, width: usize) -> &str {
    if s.is_ascii() {
        // ASCII: every byte is a char boundary — direct byte slice.
        let a = start.min(s.len());
        let b = (start + width).min(s.len());
        &s[a..b]
    } else {
        // UTF-8: walk char_indices to find byte boundaries.
        let mut start_byte = s.len();
        let mut end_byte = s.len();
        for (col, (byte_idx, _)) in s.char_indices().enumerate() {
            if col == start {
                start_byte = byte_idx;
            }
            if col == start + width {
                end_byte = byte_idx;
                break;
            }
        }
        if start_byte <= end_byte {
            &s[start_byte..end_byte]
        } else {
            ""
        }
    }
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
        apply_search_highlight(x, y, width, line_idx, left_col, search, buf, ui_theme);
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
        render_cursor(x, y, width, cursor, left_col, vim_mode, buf, ui_theme);
    }

    for secondary in secondary_cursors {
        if secondary.line == line_idx {
            render_secondary_cursor(x, y, width, secondary, left_col, vim_mode, buf, ui_theme);
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
) {
    let cursor_screen_col = cursor.col.saturating_sub(left_col);
    if cursor_screen_col < width {
        let cx = x + cursor_screen_col as u16;
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
) {
    let cursor_screen_col = cursor.col.saturating_sub(left_col);
    if cursor_screen_col >= width {
        return;
    }

    let cx = x + cursor_screen_col as u16;
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

    for col in line_sel_start..line_sel_end {
        if col < left_col {
            continue;
        }
        let screen_col = col - left_col;
        if screen_col >= width {
            break;
        }
        let cx = x + screen_col as u16;
        buf[(cx, y)].set_style(sel_style);
    }
}

/// Apply search match highlighting to a line.
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
) {
    let current_idx = search.current_match;

    for (i, &(start, end)) in search.matches.iter().enumerate() {
        // Only highlight matches that touch this line.
        if end.line < line_idx || start.line > line_idx {
            continue;
        }

        let col_start = if start.line == line_idx {
            start.col.saturating_sub(left_col)
        } else {
            0
        };
        let col_end = if end.line == line_idx {
            end.col.saturating_sub(left_col)
        } else {
            width
        };

        if col_start >= width || col_end == 0 || col_start >= col_end {
            continue;
        }

        let is_current = current_idx == Some(i);
        let bg = if is_current {
            theme.search_current_bg
        } else {
            theme.search_match_bg
        };

        for col in col_start..col_end.min(width) {
            let cell_x = x + col as u16;
            let cell = &mut buf[(cell_x, y)];
            cell.set_bg(bg);
        }
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
