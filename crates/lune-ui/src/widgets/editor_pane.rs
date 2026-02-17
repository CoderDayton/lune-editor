//! Editor pane widget.
//!
//! Renders the active `TextBuffer` with line numbers, cursor highlighting,
//! selection rendering, and viewport scrolling. Handles mouse clicks for
//! cursor placement and scroll wheel for viewport adjustment.
//!
//! When syntax highlighting data is available, lines are rendered with
//! styled spans from the theme instead of plain text.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Style, Stylize};
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;

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
        // Allow scrolling beyond the last screenful so content can reach the
        // top of the viewport, minus a small margin to always show one line.
        let min_visible = 1.min(viewport_height);
        let max_top = total_lines.saturating_sub(min_visible);
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

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the editor pane (git gutter + line numbers + buffer content + cursor).
///
/// When `highlighted` is `Some`, the lines are rendered with syntax-colored
/// spans. Otherwise, plain white text is used.
///
/// When `gutter_marks` is `Some`, a 1-character-wide git gutter column
/// is rendered to the left of the line numbers with colored markers.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
pub fn render_editor_pane(
    area: Rect,
    buf: &mut Buffer,
    text_buf: Option<&TextBuffer>,
    viewport: &mut ViewportState,
    vim_mode: VimMode,
    highlighted: Option<&[HighlightedLine]>,
    syntax_theme: &SyntaxTheme,
    gutter_marks: Option<&GutterMarks>,
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
    let total_gutter = gw + git_gw;
    let content_width = area.width.saturating_sub(total_gutter) as usize;
    let viewport_height = area.height as usize;

    // Scroll viewport to keep cursor visible.
    let cursor = &text_buf.cursor.primary.head;
    viewport.scroll_to_cursor(cursor.line, cursor.col, viewport_height, content_width);

    let selection = {
        let sel = &text_buf.cursor.primary;
        if sel.head == sel.anchor {
            None
        } else {
            let (start, end) = sel.ordered();
            Some((start, end))
        }
    };

    for row in 0..viewport_height {
        let line_idx = viewport.top_line + row;
        let y = area.y + row as u16;

        if line_idx < total_lines {
            // Render git gutter mark (if active).
            if let Some(marks) = gutter_marks {
                render_git_gutter(area.x, y, line_idx, marks, buf, theme);
            }

            render_line_number(
                area.x + git_gw,
                y,
                gw,
                line_idx,
                cursor.line == line_idx,
                buf,
                theme,
            );

            // Look up highlighted spans for this line.
            let hl_line = highlighted.and_then(|lines| lines.iter().find(|hl| hl.line == line_idx));

            render_line_content(
                area.x + total_gutter,
                y,
                content_width,
                text_buf,
                line_idx,
                viewport.left_col,
                cursor,
                vim_mode,
                selection.as_ref(),
                hl_line,
                syntax_theme,
                buf,
                theme,
            );
        } else {
            // Tilde for lines past end of buffer.
            Line::from(Span::from("~").dim()).render(Rect::new(area.x, y, area.width, 1), buf);
        }
    }
}

/// Render a line number in the gutter.
#[allow(clippy::cast_possible_truncation)]
fn render_line_number(
    x: u16,
    y: u16,
    gw: u16,
    line_idx: usize,
    is_current: bool,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let num_str = format!("{:>width$} ", line_idx + 1, width = (gw - 1) as usize);
    let span = if is_current {
        Span::styled(num_str, theme.editor_gutter_active)
    } else {
        Span::styled(num_str, theme.editor_gutter_inactive)
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

/// Render the text content of a single line with cursor, selection, and syntax highlighting.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn render_line_content(
    x: u16,
    y: u16,
    width: usize,
    text_buf: &TextBuffer,
    line_idx: usize,
    left_col: usize,
    cursor: &Position,
    vim_mode: VimMode,
    selection: Option<&(Position, Position)>,
    hl_line: Option<&HighlightedLine>,
    theme: &SyntaxTheme,
    buf: &mut Buffer,
    ui_theme: &Theme,
) {
    let line_owned = text_buf.line(line_idx).unwrap_or_default();
    let line_text = line_owned.trim_end_matches('\n').trim_end_matches('\r');

    // Apply horizontal scroll.
    let visible_text: String = line_text.chars().skip(left_col).take(width).collect();

    // Render the text — either with syntax highlighting or plain.
    let rect = Rect::new(x, y, width as u16, 1);

    if let Some(hl) = hl_line {
        if hl.is_plain() {
            Line::from(visible_text.as_str()).render(rect, buf);
        } else {
            let styled_spans = build_styled_line(line_text, left_col, width, &hl.spans, theme);
            Line::from(styled_spans).render(rect, buf);
        }
    } else {
        Line::from(visible_text.as_str()).render(rect, buf);
    }

    // Apply selection highlighting.
    if let Some((sel_start, sel_end)) = selection {
        apply_selection_highlight(
            x, y, width, line_idx, left_col, sel_start, sel_end, line_text, buf, ui_theme,
        );
    }

    // Render cursor.
    if cursor.line == line_idx {
        render_cursor(x, y, width, cursor, left_col, vim_mode, buf, ui_theme);
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
    let chars: Vec<char> = line_text.chars().collect();
    let total_cols = chars.len();
    let mut result: Vec<Span<'a>> = Vec::new();
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

        // Fill gap with default style.
        if span_start > pos {
            let gap_end = span_start.min(total_cols);
            if gap_end > pos {
                let text: String = chars[pos..gap_end].iter().collect();
                result.push(Span::from(text));
            }
        }

        // Add the styled span.
        if span_end > span_start {
            let text: String = chars[span_start..span_end].iter().collect();
            result.push(Span::styled(text, theme.resolve(span.style)));
        }

        pos = span_end;
    }

    // Fill remaining visible area with default style.
    let remaining_end = right_col.min(total_cols);
    if pos < remaining_end {
        let text: String = chars[pos..remaining_end].iter().collect();
        result.push(Span::from(text));
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
        "Ctrl+`  Toggle AI Panel",
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
    let total_gutter = gw + git_gw;

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
        assert_eq!(vp.top_line, 99); // max = 100 - 1 (last line at top)
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
}
