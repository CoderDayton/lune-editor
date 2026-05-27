//! Diff view widget.
//!
//! Renders a unified or side-by-side diff for a single file.
//! Supports navigation between hunks and scrolling.

use crate::primitives::{Buffer, Line, Modifier, Rect, Span, Style, Stylize, Widget};

use lune_git::diff::{DiffLineKind, FileDiff};

use crate::theme::Theme;

/// Display mode for the diff view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffViewMode {
    /// Unified diff (single pane with +/- lines).
    #[default]
    Unified,
    /// Side-by-side diff (old on left, new on right).
    SideBySide,
}

/// State of the diff view widget.
#[derive(Clone, Debug, Default)]
pub struct DiffViewState {
    /// The file diff to display.
    pub diff: Option<FileDiff>,
    /// Display mode.
    pub mode: DiffViewMode,
    /// Scroll offset (line index in flattened view).
    pub scroll: usize,
    /// Current hunk index for hunk-to-hunk navigation.
    pub current_hunk: usize,
}

impl DiffViewState {
    /// Set the diff to display.
    pub fn set_diff(&mut self, diff: FileDiff) {
        self.scroll = 0;
        self.current_hunk = 0;
        self.diff = Some(diff);
    }

    /// Clear the diff view.
    pub fn clear(&mut self) {
        self.diff = None;
        self.scroll = 0;
        self.current_hunk = 0;
    }

    /// Toggle between unified and side-by-side modes.
    pub const fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DiffViewMode::Unified => DiffViewMode::SideBySide,
            DiffViewMode::SideBySide => DiffViewMode::Unified,
        };
    }

    /// Jump to the next hunk.
    pub fn next_hunk(&mut self) {
        if let Some(diff) = &self.diff {
            if self.current_hunk < diff.hunks.len().saturating_sub(1) {
                self.current_hunk += 1;
                self.scroll_to_current_hunk();
            }
        }
    }

    /// Jump to the previous hunk.
    pub fn prev_hunk(&mut self) {
        if self.current_hunk > 0 {
            self.current_hunk -= 1;
            self.scroll_to_current_hunk();
        }
    }

    /// Scroll the view so the current hunk header is visible.
    fn scroll_to_current_hunk(&mut self) {
        let Some(diff) = &self.diff else { return };
        let mut line = 0usize;
        for (i, hunk) in diff.hunks.iter().enumerate() {
            if i == self.current_hunk {
                self.scroll = line;
                return;
            }
            line += 1 + hunk.lines.len(); // +1 for hunk header
        }
    }

    /// Total number of flattened lines (headers + diff lines).
    fn total_lines(&self) -> usize {
        self.diff
            .as_ref()
            .map_or(0, |d| d.hunks.iter().map(|h| 1 + h.lines.len()).sum())
    }

    /// Scroll up by N lines.
    pub const fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    /// Scroll down by N lines.
    pub fn scroll_down(&mut self, n: usize, viewport_height: usize) {
        let max = self.total_lines().saturating_sub(viewport_height);
        self.scroll = (self.scroll + n).min(max);
    }

    /// Get the current hunk and the file path, if available.
    #[must_use]
    pub fn current_hunk_data(&self) -> Option<(&std::path::Path, &lune_git::diff::DiffHunk)> {
        let diff = self.diff.as_ref()?;
        let hunk = diff.hunks.get(self.current_hunk)?;
        Some((diff.path.as_path(), hunk))
    }
}

/// Render the diff view in unified mode.
#[allow(clippy::cast_possible_truncation)]
pub fn render_diff_view(area: Rect, buf: &mut Buffer, state: &DiffViewState, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let Some(diff) = &state.diff else {
        Line::from(Span::from(" No diff to display").dim())
            .render(Rect::new(area.x, area.y, area.width, 1), buf);
        return;
    };

    // Title.
    let title = format!(" {} ", diff.path.display());
    Line::from(Span::from(title).bold().fg(theme.fg))
        .render(Rect::new(area.x, area.y, area.width, 1), buf);

    let view_area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);

    match state.mode {
        DiffViewMode::Unified => render_unified(view_area, diff, state, buf, theme),
        DiffViewMode::SideBySide => render_side_by_side(view_area, diff, state, buf, theme),
    }
}

/// Render unified diff.
///
/// Iterates directly over hunks/lines, counting logical rows to find the
/// scroll offset, then renders at most `height` rows — no intermediate Vec.
#[allow(clippy::cast_possible_truncation)]
fn render_unified(
    area: Rect,
    diff: &FileDiff,
    state: &DiffViewState,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let (x, start_y, width, height) = (area.x, area.y, area.width, area.height as usize);
    let scroll = state.scroll;

    // Logical row counter across all hunks.
    let mut logical_row: usize = 0;
    let mut rendered: usize = 0;

    'outer: for (hunk_idx, hunk) in diff.hunks.iter().enumerate() {
        // --- hunk header row ---
        if logical_row >= scroll {
            let row = logical_row - scroll;
            if row >= height {
                break;
            }
            let header_style = if hunk_idx == state.current_hunk {
                Style::new()
                    .fg(theme.diff_hunk_fg)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::new().fg(theme.diff_hunk_fg)
            };
            let label = format!(" {}", hunk.header);
            let truncated: String = label.chars().take(width as usize).collect();
            Line::from(Span::styled(truncated, header_style))
                .render(Rect::new(x, start_y + row as u16, width, 1), buf);
            rendered += 1;
        }
        logical_row += 1;

        // --- diff lines ---
        for line in &hunk.lines {
            if logical_row >= scroll {
                let row = logical_row - scroll;
                if row >= height {
                    break 'outer;
                }
                let (prefix, style) = match line.kind {
                    DiffLineKind::Addition => (
                        "+",
                        Style::new().fg(theme.diff_add_fg).bg(theme.diff_add_bg),
                    ),
                    DiffLineKind::Deletion => (
                        "-",
                        Style::new().fg(theme.diff_del_fg).bg(theme.diff_del_bg),
                    ),
                    DiffLineKind::Context => (" ", Style::default()),
                };
                let text = format!("{prefix}{}", line.content.trim_end_matches('\n'));
                let truncated: String = text.chars().take(width as usize).collect();
                Line::from(Span::styled(truncated, style))
                    .render(Rect::new(x, start_y + row as u16, width, 1), buf);
                rendered += 1;
            }
            logical_row += 1;
        }
    }
    let _ = rendered; // used for clarity
}

/// Render side-by-side diff.
///
/// Builds paired rows on-the-fly within a small window buffer rather than
/// pre-allocating the entire flattened list. Only rows in the visible window
/// (scroll..scroll+height) are materialised.
#[allow(clippy::cast_possible_truncation)]
fn render_side_by_side(
    area: Rect,
    diff: &FileDiff,
    state: &DiffViewState,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let (x, start_y, width, height) = (area.x, area.y, area.width, area.height as usize);
    if height == 0 {
        return;
    }
    let half_width = (width / 2) as usize;
    let left_x = x;
    let right_x = x + half_width as u16;
    let scroll = state.scroll;

    // We materialise only the visible window.
    // paired_window[i] = (left, right) for logical row (scroll + i).
    // Capacity: at most `height` rows.
    let mut paired_window: Vec<(Option<String>, Option<String>)> = Vec::with_capacity(height);

    // Walk through all logical rows until we have filled the window.
    let mut logical_row: usize = 0;
    // We may need to back-fill the right side of a deletion row, so we track
    // whether the last window entry is an unpaired deletion.
    let window_end = scroll + height;

    for hunk in &diff.hunks {
        // Hunk header row.
        if logical_row >= window_end {
            break;
        }
        if logical_row >= scroll {
            paired_window.push((Some(hunk.header.clone()), Some(hunk.header.clone())));
        }
        logical_row += 1;

        for line in &hunk.lines {
            if logical_row >= window_end {
                break;
            }
            let in_window = logical_row >= scroll;
            let content = line.content.trim_end_matches('\n');
            match line.kind {
                DiffLineKind::Context => {
                    if in_window {
                        paired_window.push((Some(content.to_owned()), Some(content.to_owned())));
                    }
                    logical_row += 1;
                }
                DiffLineKind::Deletion => {
                    if in_window {
                        paired_window.push((Some(content.to_owned()), None));
                    }
                    logical_row += 1;
                }
                DiffLineKind::Addition => {
                    // Try to pair with the last unpaired deletion in the window.
                    let paired = if in_window {
                        if let Some(last) = paired_window.last_mut() {
                            if last.1.is_none() {
                                last.1 = Some(content.to_owned());
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !paired {
                        if in_window {
                            paired_window.push((None, Some(content.to_owned())));
                        }
                        logical_row += 1;
                    }
                    // If paired, we don't advance logical_row (the row was already counted).
                }
            }
        }
    }

    // Render the collected window rows.
    for (row, (left, right)) in paired_window.iter().enumerate() {
        let y = start_y + row as u16;

        // Left side.
        if let Some(text) = left {
            let truncated: String = text.chars().take(half_width.saturating_sub(1)).collect();
            let style = Style::new().fg(theme.diff_del_fg);
            Line::from(Span::styled(truncated, style))
                .render(Rect::new(left_x, y, half_width as u16, 1), buf);
        }

        // Separator.
        if half_width as u16 > 0 {
            let sep_x = x + half_width as u16 - 1;
            if sep_x < x + width {
                Line::from(Span::from("│").dim()).render(Rect::new(sep_x, y, 1, 1), buf);
            }
        }

        // Right side.
        if let Some(text) = right {
            let truncated: String = text.chars().take(half_width.saturating_sub(1)).collect();
            let style = Style::new().fg(theme.diff_add_fg);
            Line::from(Span::styled(truncated, style))
                .render(Rect::new(right_x, y, half_width as u16, 1), buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use lune_git::diff::{DiffHunk, DiffLine};
    use std::path::PathBuf;

    fn make_diff() -> FileDiff {
        FileDiff {
            path: PathBuf::from("test.rs"),
            hunks: vec![DiffHunk {
                header: "@@ -1,3 +1,4 @@".to_owned(),
                old_start: 1,
                old_count: 3,
                new_start: 1,
                new_count: 4,
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "fn main() {\n".to_owned(),
                        old_lineno: Some(1),
                        new_lineno: Some(1),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Deletion,
                        content: "    println!(\"old\");\n".to_owned(),
                        old_lineno: Some(2),
                        new_lineno: None,
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Addition,
                        content: "    println!(\"new\");\n".to_owned(),
                        old_lineno: None,
                        new_lineno: Some(2),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Addition,
                        content: "    println!(\"extra\");\n".to_owned(),
                        old_lineno: None,
                        new_lineno: Some(3),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "}\n".to_owned(),
                        old_lineno: Some(3),
                        new_lineno: Some(4),
                        no_newline_eof: false,
                    },
                ],
            }],
        }
    }

    #[test]
    fn state_default_is_empty() {
        let state = DiffViewState::default();
        assert!(state.diff.is_none());
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn set_and_clear_diff() {
        let mut state = DiffViewState::default();
        state.set_diff(make_diff());
        assert!(state.diff.is_some());
        state.clear();
        assert!(state.diff.is_none());
    }

    #[test]
    fn toggle_mode() {
        let mut state = DiffViewState::default();
        assert_eq!(state.mode, DiffViewMode::Unified);
        state.toggle_mode();
        assert_eq!(state.mode, DiffViewMode::SideBySide);
        state.toggle_mode();
        assert_eq!(state.mode, DiffViewMode::Unified);
    }

    #[test]
    fn hunk_navigation() {
        let mut state = DiffViewState::default();
        let mut diff = make_diff();
        // Add a second hunk.
        diff.hunks.push(DiffHunk {
            header: "@@ -10,2 +11,3 @@".to_owned(),
            old_start: 10,
            old_count: 2,
            new_start: 11,
            new_count: 3,
            lines: vec![DiffLine {
                kind: DiffLineKind::Addition,
                content: "new line\n".to_owned(),
                old_lineno: None,
                new_lineno: Some(12),
                no_newline_eof: false,
            }],
        });
        state.set_diff(diff);

        assert_eq!(state.current_hunk, 0);
        state.next_hunk();
        assert_eq!(state.current_hunk, 1);
        state.next_hunk(); // Should not go past last.
        assert_eq!(state.current_hunk, 1);
        state.prev_hunk();
        assert_eq!(state.current_hunk, 0);
        state.prev_hunk(); // Should not go below 0.
        assert_eq!(state.current_hunk, 0);
    }

    #[test]
    fn scroll_up_down() {
        let mut state = DiffViewState::default();
        state.set_diff(make_diff());

        state.scroll_down(3, 5);
        assert!(state.scroll > 0);
        state.scroll_up(1);
        // scroll should decrease (or stay at 0).
    }

    #[test]
    fn render_unified_does_not_panic() {
        let mut state = DiffViewState::default();
        state.set_diff(make_diff());

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_diff_view(area, &mut buf, &state, &Theme::dark());
    }

    #[test]
    fn render_side_by_side_does_not_panic() {
        let mut state = DiffViewState::default();
        state.set_diff(make_diff());
        state.mode = DiffViewMode::SideBySide;

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_diff_view(area, &mut buf, &state, &Theme::dark());
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = DiffViewState::default();
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_diff_view(area, &mut buf, &state, &Theme::dark());
    }
}
