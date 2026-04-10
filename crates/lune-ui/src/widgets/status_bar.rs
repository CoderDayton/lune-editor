//! Status bar widget.
//!
//! Renders a single-row status bar at the bottom of the editor showing:
//! - Left: vim mode indicator, file path, dirty indicator
//! - Center: cursor position (Ln/Col)
//! - Right: git branch, encoding, file type, AI status

use crate::primitives::{Buffer, Line, Rect, Span, Widget};

use crate::theme::Theme;
use crate::vim::VimMode;

/// Hard cap for displayed file-path characters in the status bar.
///
/// This prevents extremely long absolute paths from dominating the full line.
const MAX_STATUS_PATH_CHARS: usize = 120;
/// Fixed width for the cursor segment (`Ln X, Col Y` + optional selection).
const CURSOR_SEGMENT_WIDTH: usize = 24;
/// Fixed width for the line ending segment.
const LINE_ENDING_SEGMENT_WIDTH: usize = 4;
/// Fixed width for the git branch segment.
const BRANCH_SEGMENT_WIDTH: usize = 16;
/// Fixed width for the encoding segment.
const ENCODING_SEGMENT_WIDTH: usize = 7;
/// Fixed width for the file type segment.
const FILETYPE_SEGMENT_WIDTH: usize = 8;
/// Fixed width for the AI status segment.
const AI_SEGMENT_WIDTH: usize = 12;

// ── Status line state ─────────────────────────────────────────────────

/// Collected state for the status bar (built from `AppState` each frame).
///
/// Uses `&'static str` for the encoding field to avoid a per-frame heap
/// allocation for a constant value.
#[derive(Clone, Debug, Default)]
pub struct StatusLineState {
    /// Vim mode label ("NORMAL", "INSERT", "VISUAL", etc.).
    pub mode: VimMode,
    /// File path display string (or empty).
    pub file_path: String,
    /// Whether the current buffer is dirty.
    pub dirty: bool,
    /// Cursor position: line number (1-based).
    pub cursor_line: usize,
    /// Cursor position: column number (1-based).
    pub cursor_col: usize,
    /// Git branch name (empty if not in a repo).
    pub git_branch: String,
    /// File encoding — always a static string (e.g. `"UTF-8"`).
    pub encoding: &'static str,
    /// AI status string (e.g., "AI: Connected").
    pub ai_status: String,
    /// File type label (e.g., "Rust", "Markdown").
    pub file_type: String,
    /// Transient status message (takes priority over file path).
    pub message: String,
    /// Number of characters in the active selection (0 = no selection).
    pub selection_chars: usize,
    /// Line ending style (e.g., "LF", "CRLF").
    pub line_ending: &'static str,
    /// Vim command-line buffer (`Some(text)` when in Command mode).
    pub vim_cmdline: Option<String>,
}

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the status bar.
#[allow(clippy::cast_possible_truncation)]
pub fn render_status_bar(area: Rect, buf: &mut Buffer, status: &StatusLineState, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Render a single row anchored to the bottom of the allocated area. We
    // previously pinned to `buf.area().height - 1`, which assumed the buffer
    // area matched the layout exactly — a fragile assumption that could leave
    // a stale strip above the actual status row when the two disagreed.
    let y = area.y + area.height.saturating_sub(1);
    let line_area = Rect::new(area.x, y, area.width, 1);
    // Clear the full row first to avoid stale text artifacts.
    for dx in 0..line_area.width {
        let cell = &mut buf[(line_area.x + dx, line_area.y)];
        cell.set_symbol(" ");
        cell.set_style(theme.status_bg);
    }

    // Always show Normal/Insert indicator so users know when typing is
    // blocked.  The full mode set (VISUAL, V-LINE, COMMAND) is only
    // reachable when vim keybindings are enabled.
    let mode_label: &str = mode_string(status.mode);
    let dirty_mark = if status.dirty { " [+]" } else { "" };

    let mut left_text = if status.message.is_empty() {
        let path = truncate_path_with_ellipsis(&status.file_path, MAX_STATUS_PATH_CHARS);
        format!("{path}{dirty_mark}")
    } else {
        status.message.clone()
    };

    // Build right-side segments using fixed widths so core components keep
    // stable positions regardless of left-path length.
    let cursor_text = if status.cursor_line > 0 {
        if status.selection_chars > 0 {
            format!(
                "Ln {}, Col {} ({} sel)",
                status.cursor_line, status.cursor_col, status.selection_chars
            )
        } else {
            format!("Ln {}, Col {}", status.cursor_line, status.cursor_col)
        }
    } else {
        String::new()
    };
    let mut right_segments = Vec::with_capacity(6);
    if !cursor_text.is_empty() {
        right_segments.push(fixed_segment(&cursor_text, CURSOR_SEGMENT_WIDTH, true));
    }
    if !status.git_branch.trim().is_empty() {
        right_segments.push(fixed_segment(
            &status.git_branch,
            BRANCH_SEGMENT_WIDTH,
            false,
        ));
    }
    if !status.encoding.trim().is_empty() {
        right_segments.push(fixed_segment(
            status.encoding,
            ENCODING_SEGMENT_WIDTH,
            false,
        ));
    }
    if !status.line_ending.is_empty() {
        right_segments.push(fixed_segment(
            status.line_ending,
            LINE_ENDING_SEGMENT_WIDTH,
            false,
        ));
    }
    if !status.file_type.trim().is_empty() {
        right_segments.push(fixed_segment(
            &status.file_type,
            FILETYPE_SEGMENT_WIDTH,
            false,
        ));
    }
    if !status.ai_status.trim().is_empty() {
        right_segments.push(fixed_segment(&status.ai_status, AI_SEGMENT_WIDTH, false));
    }
    // In command-line mode, replace left content with the cmdline and hide right segments.
    if let Some(cmdline) = &status.vim_cmdline {
        left_text = format!(":{cmdline}");
        right_segments.clear();
    }

    let right_text = right_segments.join(" | ");

    // Calculate spacing and clamp left text to available width.
    let mode_width = mode_label.chars().count() as u16;
    let separator = if mode_label.is_empty() { "" } else { " " };
    let right_width = right_text.chars().count() as u16;
    let prefix_width = mode_width + separator.chars().count() as u16;
    // Keep at least one spacer column before right-side info.
    let max_left = line_area
        .width
        .saturating_sub(prefix_width + right_width + 1) as usize;
    left_text = truncate_with_ellipsis(&left_text, max_left);
    let left_width = left_text.chars().count() as u16;
    let spacer_width = line_area
        .width
        .saturating_sub(prefix_width + left_width + right_width);

    let mut spans = Vec::with_capacity(7);
    if !mode_label.is_empty() {
        spans.push(Span::styled(mode_label, theme.status_mode));
        spans.push(Span::from(" "));
    }
    spans.push(Span::from(left_text));
    spans.push(Span::from(" ".repeat(spacer_width as usize)));
    if !right_text.is_empty() {
        spans.push(Span::styled(right_text, theme.status_info));
    }

    Line::from(spans)
        .style(theme.status_bg)
        .render(line_area, buf);
}

/// Convert a `VimMode` to its display label.
const fn mode_string(mode: VimMode) -> &'static str {
    match mode {
        VimMode::Normal => " NORMAL ",
        VimMode::Insert => " INSERT ",
        VimMode::Visual => " VISUAL ",
        VimMode::VisualLine => " V-LINE ",
        VimMode::Command => " COMMAND ",
    }
}

/// Truncate a generic string to `max_chars`, appending `...` when truncated.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let keep = max_chars - 3;
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// Truncate a path-like string to `max_chars`, preserving the head.
///
/// Example: `"lune-editor/crates/lune-ui/src/runtime/app.rs"` ->
/// `"lune-editor/crates/lune-ui/src/..."`.
fn truncate_path_with_ellipsis(path: &str, max_chars: usize) -> String {
    truncate_with_ellipsis(path, max_chars)
}

/// Format a fixed-width status segment with truncation.
fn fixed_segment(text: &str, width: usize, right_align: bool) -> String {
    let trimmed = truncate_with_ellipsis(text, width);
    if right_align {
        format!("{trimmed:>width$}")
    } else {
        format!("{trimmed:<width$}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_labels() {
        assert_eq!(mode_string(VimMode::Normal), " NORMAL ");
        assert_eq!(mode_string(VimMode::Insert), " INSERT ");
        assert_eq!(mode_string(VimMode::Visual), " VISUAL ");
        assert_eq!(mode_string(VimMode::VisualLine), " V-LINE ");
        assert_eq!(mode_string(VimMode::Command), " COMMAND ");
    }

    #[test]
    fn status_default() {
        let s = StatusLineState::default();
        assert!(s.file_path.is_empty());
        assert!(!s.dirty);
        assert_eq!(s.cursor_line, 0);
    }

    #[test]
    fn truncate_path_preserves_head() {
        let path = "lune-editor/crates/lune-ui/src/runtime/app.rs";
        let out = truncate_path_with_ellipsis(path, 18);
        assert!(out.starts_with("lune-editor/"));
        assert!(out.ends_with("..."));
        assert!(out.chars().count() <= 18);
    }

    #[test]
    fn truncate_generic_adds_ellipsis() {
        let out = truncate_with_ellipsis("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(out, "abcdefg...");
    }
}
