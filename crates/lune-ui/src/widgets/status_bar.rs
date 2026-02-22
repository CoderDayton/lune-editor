//! Status bar widget.
//!
//! Renders a single-row status bar at the bottom of the editor showing:
//! - Left: vim mode indicator, file path, dirty indicator
//! - Center: cursor position (Ln/Col)
//! - Right: git branch, encoding, file type, AI status

use crate::primitives::{Buffer, Line, Rect, Span, Widget};

use crate::theme::Theme;
use crate::vim::VimMode;

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
}

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the status bar.
#[allow(clippy::cast_possible_truncation)]
pub fn render_status_bar(area: Rect, buf: &mut Buffer, status: &StatusLineState, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Always show Normal/Insert indicator so users know when typing is
    // blocked.  The full mode set (VISUAL, V-LINE, COMMAND) is only
    // reachable when vim keybindings are enabled.
    let mode_label: &str = mode_string(status.mode);
    let dirty_mark = if status.dirty { " [+]" } else { "" };

    let left_text = if status.message.is_empty() {
        format!("{}{dirty_mark}", status.file_path)
    } else {
        status.message.clone()
    };

    let cursor_text = if status.cursor_line > 0 {
        format!("Ln {}, Col {}", status.cursor_line, status.cursor_col)
    } else {
        String::new()
    };

    // Build right-side segments.
    let mut right_parts: Vec<&str> = Vec::new();
    if !status.git_branch.is_empty() {
        right_parts.push(&status.git_branch);
    }
    if !status.encoding.is_empty() {
        right_parts.push(status.encoding);
    }
    if !status.file_type.is_empty() {
        right_parts.push(&status.file_type);
    }
    if !status.ai_status.is_empty() {
        right_parts.push(&status.ai_status);
    }
    let right_text = right_parts.join(" │ ");

    // Calculate spacing.
    let mode_width = mode_label.len() as u16;
    let separator = if mode_label.is_empty() { "" } else { " " };
    let left_width = left_text.len() as u16;
    let cursor_width = cursor_text.len() as u16;
    let right_width = right_text.len() as u16;

    let fixed_width =
        mode_width + separator.len() as u16 + left_width + cursor_width + 1 + right_width;
    let padding = area.width.saturating_sub(fixed_width) as usize;

    // Split padding: put cursor info roughly centered.
    let left_pad = padding / 2;
    let right_pad = padding - left_pad;

    let mut spans = Vec::with_capacity(7);
    if !mode_label.is_empty() {
        spans.push(Span::styled(mode_label, theme.status_mode));
        spans.push(Span::from(" "));
    }
    spans.push(Span::from(left_text));
    spans.push(Span::from(" ".repeat(left_pad)));
    spans.push(Span::from(cursor_text));
    spans.push(Span::from(" ".repeat(right_pad)));
    spans.push(Span::styled(right_text, theme.status_info));

    Line::from(spans).style(theme.status_bg).render(area, buf);
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
}
