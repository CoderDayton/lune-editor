//! Status bar widget.
//!
//! Renders a single-row status bar at the bottom of the editor showing:
//! - Left: vim mode, file path, cursor position, file type, line ending,
//!   and encoding — clustered together, micro-editor style
//! - Right: git branch and AI status

use crate::primitives::{Buffer, Line, Rect, Span, StatefulWidget, Widget};

use crate::theme::Theme;
use crate::vim::VimMode;

use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Hard cap for displayed file-path characters in the status bar.
///
/// This prevents extremely long absolute paths from dominating the full line.
const MAX_STATUS_PATH_CHARS: usize = 120;
/// Fixed width for the git branch segment.
const BRANCH_SEGMENT_WIDTH: usize = 16;
/// Fixed width for the AI status segment.
const AI_SEGMENT_WIDTH: usize = 12;
/// Separator drawn between status segments.
const SEG_SEP: &str = " │ ";
/// Brand badge shown in the prefix slot on the empty welcome bar — rendered
/// like the vim mode badge, sharing its accent background (`status_brand`,
/// which matches `status_mode` by default). The surrounding spaces give it
/// the same padded-box look as ` NORMAL `.
const BRAND_BADGE: &str = " Lune Editor ";
/// Left-cluster hint shown when no buffer is open, so the status bar reads
/// as a discoverable welcome line instead of a blank strip. Chords mirror
/// the default keymap (see `runtime/keybindings.rs`).
const EMPTY_STATE_HINT: &str = "C-o open file │ C-p palette │ C-b explorer";

// ── Status line state ─────────────────────────────────────────────────

/// Collected state for the status bar (built from `AppState` each frame).
///
/// Uses `&'static str` for the encoding field to avoid a per-frame heap
/// allocation for a constant value.
// A flat per-frame view-model: grouping these independent display flags
// into sub-structs would only add indirection.
#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct StatusLineState {
    /// Vim mode label ("NORMAL", "INSERT", "VISUAL", etc.).
    pub mode: VimMode,
    /// Whether vim keybindings are enabled. When `false`, the editor has no
    /// modal state and the mode indicator is hidden entirely.
    pub vim_enabled: bool,
    /// File path display string (or empty).
    pub file_path: String,
    /// Whether the current buffer is dirty.
    pub dirty: bool,
    /// Whether an editable buffer is currently open. Drives the welcome /
    /// empty state: when `false` the bar shows the brand badge and hint
    /// instead of mode + path. Kept distinct from `cursor_line` so the
    /// empty state never depends on the cursor happening to read as 0.
    pub has_buffer: bool,
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
    /// When `true`, the AI segment renders a live spinner instead of a
    /// static glyph — used while an AI session is starting or actively
    /// streaming output so the status bar reflects ongoing work.
    pub ai_busy: bool,
    /// When `true`, the git branch segment renders with a leading spinner
    /// glyph — set during async fetch/push/clone operations.
    pub git_busy: bool,
}

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the status bar.
///
/// Pass `throbber` to drive spinner glyphs in the AI / git segments when
/// `status.ai_busy` or `status.git_busy` are set. The state is advanced
/// once per render by the caller — see `render_editor_tab`.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
pub fn render_status_bar(
    area: Rect,
    buf: &mut Buffer,
    status: &StatusLineState,
    theme: &Theme,
    throbber: &mut ThrobberState,
) {
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

    // Prefix badge: the vim mode while editing a buffer, otherwise the brand
    // badge on the empty welcome bar. They never co-occur — no buffer means
    // no mode — so a single slot renders whichever applies. With vim disabled
    // and a buffer open the slot is empty (no modal state to show).
    let no_buffer = !status.has_buffer;
    let (badge_label, badge_style): (&str, _) = if status.vim_enabled && !no_buffer {
        (mode_string(status.mode), theme.status_mode)
    } else if no_buffer {
        (BRAND_BADGE, theme.status_brand)
    } else {
        ("", theme.status_mode)
    };
    let dirty_mark = if status.dirty { " [+]" } else { "" };

    // ── Left cluster (micro-editor style) ─────────────────────────────
    // `path (Ln,Col) │ ft:type │ LF │ UTF-8`.  A transient message takes
    // priority over the file path; the meta tail only shows when a buffer
    // is open.
    let primary = if !status.message.is_empty() {
        status.message.clone()
    } else if no_buffer {
        // No buffer open: show the welcome hint so the bar is never blank.
        EMPTY_STATE_HINT.to_string()
    } else {
        let path = truncate_path_with_ellipsis(&status.file_path, MAX_STATUS_PATH_CHARS);
        format!("{path}{dirty_mark}")
    };

    let mut left_text = primary;
    if status.has_buffer {
        let mut pos = format!(" ({},{})", status.cursor_line, status.cursor_col);
        if status.selection_chars > 0 {
            use std::fmt::Write as _;
            let _ = write!(pos, " {} sel", status.selection_chars);
        }
        left_text.push_str(&pos);

        let mut meta: Vec<String> = Vec::with_capacity(3);
        if !status.file_type.trim().is_empty() {
            meta.push(format!("ft:{}", status.file_type.to_lowercase()));
        }
        if !status.line_ending.is_empty() {
            meta.push(status.line_ending.to_string());
        }
        if !status.encoding.trim().is_empty() {
            meta.push(status.encoding.to_string());
        }
        if !meta.is_empty() {
            left_text.push_str(SEG_SEP);
            left_text.push_str(&meta.join(SEG_SEP));
        }
    }

    // ── Right cluster: git branch and AI status ───────────────────────
    let mut right_segments = Vec::with_capacity(2);
    if !status.git_branch.trim().is_empty() {
        // Reserve 2 leading cells for the throbber when the git op is
        // busy: the BRAILLE_SIX_DOUBLE throbber renders as a glyph plus a
        // trailing cell, so a single space would let it eat the first
        // character of the branch name (verified by snapshot).
        let branch = if status.git_busy {
            format!("  {}", status.git_branch)
        } else {
            status.git_branch.clone()
        };
        right_segments.push(fixed_segment(&branch, BRANCH_SEGMENT_WIDTH, false));
    }
    if !status.ai_status.trim().is_empty() {
        // Reserve 2 leading cells for the throbber when busy: it renders
        // as a glyph plus a trailing cell, so a single space would let it
        // eat the first character of the AI status label.
        let ai_label = if status.ai_busy {
            format!("  {}", status.ai_status)
        } else {
            status.ai_status.clone()
        };
        right_segments.push(fixed_segment(&ai_label, AI_SEGMENT_WIDTH, false));
    }
    // In command-line mode, replace left content with the cmdline and hide right segments.
    if let Some(cmdline) = &status.vim_cmdline {
        left_text = format!(":{cmdline}");
        right_segments.clear();
    }

    let right_text = right_segments.join(SEG_SEP);

    // Calculate spacing and clamp left text to available width.
    //
    // Widths MUST be terminal display columns, not Rust `char` counts —
    // CJK / wide-emoji glyphs occupy two cells but a single `char`, so
    // `.chars().count()` underestimates the painted width and lets the
    // right segments overflow the line. See lune `unicode_width` use in
    // `editor_pane.rs` for the same invariant.
    let badge_width = display_width(badge_label);
    let separator = if badge_label.is_empty() { "" } else { " " };
    let right_width = display_width(&right_text);
    let prefix_width = badge_width + display_width(separator);
    // Keep at least one spacer column before right-side info.
    let max_left = line_area
        .width
        .saturating_sub(prefix_width + right_width + 1) as usize;
    left_text = truncate_with_ellipsis(&left_text, max_left);
    let left_width = display_width(&left_text);
    let spacer_width = line_area
        .width
        .saturating_sub(prefix_width + left_width + right_width);

    let mut spans = Vec::with_capacity(7);
    if !badge_label.is_empty() {
        spans.push(Span::styled(badge_label, badge_style));
        spans.push(Span::from(" "));
    }
    spans.push(Span::from(left_text));
    spans.push(Span::from(" ".repeat(spacer_width as usize)));
    if !right_text.is_empty() {
        spans.push(Span::styled(right_text.clone(), theme.status_info));
    }

    Line::from(spans)
        .style(theme.status_bg)
        .render(line_area, buf);

    // Overlay live spinner glyphs over the first cell of the AI and git
    // segments while their async work is in flight.  The Line above has
    // already painted the segment label; the Throbber widget rewrites a
    // single cell with the current step glyph from `BRAILLE_SIX_DOUBLE`.
    //
    // Segment screen-X positions are computed relative to the start of the
    // joined right_text, which begins at `right_x` below.  Each segment is
    // a `fixed_segment` (left-aligned, padded to its declared width) and
    // segments are joined by " | " (3 cells).
    if (status.ai_busy || status.git_busy) && !right_text.is_empty() {
        // Each segment is a `fixed_segment` left-padded to its declared
        // width; segments are joined by `" | "` (3 cells). We re-derive
        // the cumulative left edge of the git and AI segments here so a
        // spinner can be overlaid on the first cell of each.
        // Git is the first right segment now; AI follows it, separated
        // by SEG_SEP (3 cells).  Cursor and meta info live on the left.
        let right_x = line_area.x + prefix_width + left_width + spacer_width;
        let git_x = right_x;
        let ai_x = if status.git_branch.trim().is_empty() {
            right_x
        } else {
            right_x.saturating_add(BRANCH_SEGMENT_WIDTH as u16 + 3)
        };

        let spinner = Throbber::default()
            .throbber_set(BRAILLE_SIX_DOUBLE)
            .throbber_style(theme.status_info)
            .use_type(WhichUse::Spin);
        let right_edge = line_area.x + line_area.width;
        if status.git_busy && !status.git_branch.trim().is_empty() && git_x < right_edge {
            let r = Rect::new(git_x, line_area.y, 1, 1);
            StatefulWidget::render(spinner.clone(), r, buf, throbber);
        }
        if status.ai_busy && !status.ai_status.trim().is_empty() && ai_x < right_edge {
            let r = Rect::new(ai_x, line_area.y, 1, 1);
            StatefulWidget::render(spinner, r, buf, throbber);
        }
    }
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

/// Display width (terminal columns) of `s`, capped to `u16::MAX`.
///
/// Wide characters (CJK, wide emoji) occupy two columns; zero-width
/// combiners contribute zero. Using char count here breaks layout for
/// any non-ASCII glyph in a branch name, file path, or AI status string.
fn display_width(s: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(s)).unwrap_or(u16::MAX)
}

/// Truncate a string to at most `max_cols` terminal columns, appending
/// `...` when truncation occurs.
///
/// Operates on display width, not char count, so wide glyphs don't blow
/// past the column budget. Grapheme boundaries are approximated by
/// `char`s — combining marks are emitted with their base if there is
/// room for the base, dropped otherwise (acceptable for status text).
fn truncate_with_ellipsis(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    if UnicodeWidthStr::width(s) <= max_cols {
        return s.to_string();
    }

    if max_cols <= 3 {
        return ".".repeat(max_cols);
    }

    let keep = max_cols - 3;
    let mut out = String::with_capacity(s.len().min(keep * 4) + 3);
    let mut width = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > keep {
            break;
        }
        width += cw;
        out.push(ch);
    }
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
///
/// Pads to `width` terminal columns using explicit spaces — `format!`'s
/// `width$` specifier counts Unicode scalars, not display columns, which
/// over-pads wide glyphs (e.g. `format!("{:<4}", "中")` produces 5 cells
/// of painted text). After truncation the input is guaranteed to fit
/// within `width` columns; this only adds the missing spaces.
fn fixed_segment(text: &str, width: usize, right_align: bool) -> String {
    let trimmed = truncate_with_ellipsis(text, width);
    let trimmed_cols = UnicodeWidthStr::width(trimmed.as_str());
    let pad = width.saturating_sub(trimmed_cols);
    let mut out = String::with_capacity(trimmed.len() + pad);
    if right_align {
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str(&trimmed);
    } else {
        out.push_str(&trimmed);
        for _ in 0..pad {
            out.push(' ');
        }
    }
    out
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

    #[test]
    fn truncate_respects_display_width_for_wide_chars() {
        // Each CJK char occupies 2 terminal columns. Budget = 10 cols,
        // keep = 7 cols → 3 wide chars (6 cols) then `...` (3 cols) = 9 cols.
        // A naive char-count truncation would emit 7 wide chars = 14 cols.
        let out = truncate_with_ellipsis("中文字符宽度测试", 10);
        assert!(
            UnicodeWidthStr::width(out.as_str()) <= 10,
            "wide-char truncation exceeded budget: {out:?} = {} cols",
            UnicodeWidthStr::width(out.as_str())
        );
        assert!(out.ends_with("..."));
    }

    #[test]
    fn fixed_segment_pads_to_display_columns_for_wide_chars() {
        // "中" is 2 cols + 14 spaces = 16 cols, not 1 + 15 = 16 chars.
        let out = fixed_segment("中", 16, false);
        assert_eq!(
            UnicodeWidthStr::width(out.as_str()),
            16,
            "fixed_segment must pad to display columns, got {out:?}"
        );
    }

    #[test]
    fn fixed_segment_ascii_padding_unchanged() {
        let out = fixed_segment("abc", 8, false);
        assert_eq!(out, "abc     ");
        assert_eq!(UnicodeWidthStr::width(out.as_str()), 8);
    }

    #[test]
    fn display_width_handles_wide_and_zero_width() {
        assert_eq!(display_width(""), 0);
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中"), 2);
        // Combining acute over `e` is zero-width.
        assert_eq!(display_width("e\u{0301}"), 1);
    }
}
