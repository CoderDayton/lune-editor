//! Git panel widget.
//!
//! Renders staged and unstaged file changes in the right sidebar.
//! Supports keyboard/mouse navigation, staging (`s`), unstaging (`u`),
//! discarding (`d`), opening diff view (`Enter`), and committing (`c`).

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style, Stylize};
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;

use lune_core::workspace::FileStatus;
use lune_git::{GitFileStatus, GitStatus};

use crate::theme::Theme;

/// State of the git panel widget.
#[derive(Clone, Debug)]
pub struct GitPanelState {
    /// Cached git status snapshot.
    pub status: Option<GitStatus>,
    /// Flattened list of entries for rendering/navigation.
    entries: Vec<PanelEntry>,
    /// Currently selected index in the entries list.
    pub selected: usize,
    /// Scroll offset.
    pub scroll: usize,
}

/// A single entry in the git panel (section header or file).
#[derive(Clone, Debug)]
enum PanelEntry {
    /// Section header ("Staged Changes", "Changes").
    Header(String),
    /// A file entry.
    File {
        file: GitFileStatus,
        /// Index into `status.files` for the original entry.
        _file_index: usize,
    },
}

impl Default for GitPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPanelState {
    /// Create an empty git panel state.
    pub const fn new() -> Self {
        Self {
            status: None,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }

    /// Update the panel with a new git status snapshot.
    pub fn update_status(&mut self, status: GitStatus) {
        self.entries = build_entries(&status);
        self.status = Some(status);

        // Clamp selection.
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
        // Skip to first file if header is selected.
        self.skip_headers_forward();
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
            self.skip_headers_backward();
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected < self.entries.len().saturating_sub(1) {
            self.selected += 1;
            self.skip_headers_forward();
        }
    }

    /// Get the currently selected file entry (if any).
    pub fn selected_file(&self) -> Option<&GitFileStatus> {
        self.entries.get(self.selected).and_then(|e| match e {
            PanelEntry::File { file, .. } => Some(file),
            PanelEntry::Header(_) => None,
        })
    }

    /// Whether the selected file is staged.
    pub fn selected_is_staged(&self) -> Option<bool> {
        self.selected_file().map(|f| f.staged)
    }

    /// Number of entries (including headers).
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn skip_headers_forward(&mut self) {
        while self.selected < self.entries.len() {
            if matches!(self.entries[self.selected], PanelEntry::File { .. }) {
                break;
            }
            if self.selected < self.entries.len() - 1 {
                self.selected += 1;
            } else {
                break;
            }
        }
    }

    fn skip_headers_backward(&mut self) {
        while self.selected > 0 {
            if matches!(self.entries[self.selected], PanelEntry::File { .. }) {
                return;
            }
            self.selected -= 1;
        }
        // If we reached index 0 and it's a header, move forward to the first file.
        if matches!(self.entries.get(self.selected), Some(PanelEntry::Header(_))) {
            self.skip_headers_forward();
        }
    }
}

/// Build the flattened entry list from a `GitStatus`.
fn build_entries(status: &GitStatus) -> Vec<PanelEntry> {
    let mut entries = Vec::new();

    let staged: Vec<(usize, &GitFileStatus)> = status
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.staged)
        .collect();

    let unstaged: Vec<(usize, &GitFileStatus)> = status
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.staged && f.status != FileStatus::Ignored)
        .collect();

    if !staged.is_empty() {
        entries.push(PanelEntry::Header(format!(
            "Staged Changes ({})",
            staged.len()
        )));
        for (idx, file) in staged {
            entries.push(PanelEntry::File {
                file: file.clone(),
                _file_index: idx,
            });
        }
    }

    if !unstaged.is_empty() {
        entries.push(PanelEntry::Header(format!("Changes ({})", unstaged.len())));
        for (idx, file) in unstaged {
            entries.push(PanelEntry::File {
                file: file.clone(),
                _file_index: idx,
            });
        }
    }

    entries
}

/// Render the git panel.
#[allow(clippy::cast_possible_truncation)]
pub fn render_git_panel(
    area: Rect,
    buf: &mut Buffer,
    state: &mut GitPanelState,
    is_focused: bool,
    theme: &Theme,
) {
    if area.height == 0 || area.width < 2 {
        return;
    }

    let accent = if is_focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };

    // Reserve the leftmost column for the border separator.
    let content_x = area.x + 1;
    let content_width = area.width - 1;

    // Draw a left border line with rounded corners for visual separation from the editor pane.
    let border_style = Style::default().fg(accent);
    let tl_str = theme.border_chars.top_left.to_string();
    let bl_str = theme.border_chars.bottom_left.to_string();
    let v_str = theme.border_chars.vertical.to_string();
    let last_y = area.y + area.height - 1;
    for y in area.y..area.y + area.height {
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            let sym = if y == area.y {
                &tl_str
            } else if y == last_y {
                &bl_str
            } else {
                &v_str
            };
            cell.set_symbol(sym);
            cell.set_style(border_style);
        }
    }

    // Title bar — accent color when focused.
    let title = " SOURCE CONTROL";
    let title_style = if is_focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(Span::styled(title, title_style))
        .render(Rect::new(content_x, area.y, content_width, 1), buf);

    if state.status.is_none() || state.entries.is_empty() {
        if area.height > 1 {
            Line::from(Span::from(" No changes").dim())
                .render(Rect::new(content_x, area.y + 1, content_width, 1), buf);
        }
        return;
    }

    let list_area_height = (area.height - 1) as usize; // -1 for title

    // Ensure selected entry is visible.
    if state.selected < state.scroll {
        state.scroll = state.selected;
    } else if state.selected >= state.scroll + list_area_height {
        state.scroll = state.selected - list_area_height + 1;
    }

    for row in 0..list_area_height {
        let entry_idx = state.scroll + row;
        let y = area.y + 1 + row as u16;

        if entry_idx >= state.entries.len() {
            break;
        }

        let is_selected = entry_idx == state.selected;
        let entry = &state.entries[entry_idx];

        match entry {
            PanelEntry::Header(text) => {
                let span = Span::from(format!(" {text}")).bold();
                Line::from(span).render(Rect::new(content_x, y, content_width, 1), buf);
            }
            PanelEntry::File { file, .. } => {
                render_file_entry(content_x, y, content_width, file, is_selected, buf, theme);
            }
        }
    }
}

/// Render a single file entry in the git panel.
#[allow(clippy::cast_possible_truncation)]
fn render_file_entry(
    x: u16,
    y: u16,
    width: u16,
    file: &GitFileStatus,
    is_selected: bool,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let (icon, color) = status_icon_color(file.status, theme);
    let path_str = file.path.to_string_lossy();

    // Format: " M path/to/file.rs"
    let icon_span = Span::styled(format!(" {icon} "), Style::new().fg(color));
    let path_span = if is_selected {
        Span::styled(
            path_str.to_string(),
            Style::new().add_modifier(Modifier::REVERSED),
        )
    } else {
        Span::from(path_str.to_string())
    };

    let line = Line::from(vec![icon_span, path_span]);
    line.render(Rect::new(x, y, width, 1), buf);
}

/// Get the status icon character and color for a `FileStatus`.
const fn status_icon_color(status: FileStatus, theme: &Theme) -> (char, Color) {
    match status {
        FileStatus::Modified => ('M', theme.git_modified),
        FileStatus::Added => ('A', theme.git_added),
        FileStatus::Deleted => ('D', theme.git_deleted),
        FileStatus::Renamed => ('R', theme.git_renamed),
        FileStatus::Untracked => ('U', theme.git_untracked),
        FileStatus::Conflicted => ('C', theme.git_conflicted),
        FileStatus::Ignored => ('I', theme.git_ignored),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::theme::Theme;

    fn make_status() -> GitStatus {
        GitStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            files: vec![
                GitFileStatus {
                    path: PathBuf::from("staged.rs"),
                    status: FileStatus::Modified,
                    staged: true,
                },
                GitFileStatus {
                    path: PathBuf::from("unstaged.rs"),
                    status: FileStatus::Modified,
                    staged: false,
                },
                GitFileStatus {
                    path: PathBuf::from("new_file.rs"),
                    status: FileStatus::Untracked,
                    staged: false,
                },
            ],
        }
    }

    #[test]
    fn panel_state_default_is_empty() {
        let state = GitPanelState::new();
        assert!(state.status.is_none());
        assert_eq!(state.entry_count(), 0);
    }

    #[test]
    fn update_status_builds_entries() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        // Should have: "Staged Changes (1)" header + 1 file + "Changes (2)" header + 2 files = 5
        assert_eq!(state.entry_count(), 5);
    }

    #[test]
    fn selected_file_skips_headers() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        // After update, selected should land on first file (skipping header).
        let file = state.selected_file();
        assert!(file.is_some());
        assert!(file.unwrap().staged);
    }

    #[test]
    fn navigation_up_down() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        let initial = state.selected;
        state.select_next();
        assert!(state.selected >= initial);

        // Navigate to first file.
        state.selected = 1;
        state.select_prev();
        // Should still be on a file (skip header backward).
        let file = state.selected_file();
        assert!(file.is_some());
    }

    #[test]
    fn selected_is_staged_check() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        // First file entry should be staged.
        assert_eq!(state.selected_is_staged(), Some(true));

        // Navigate past staged section to unstaged.
        state.select_next();
        state.select_next();
        // Now should be on an unstaged file (possibly after the "Changes" header).
        if let Some(staged) = state.selected_is_staged() {
            assert!(!staged);
        }
    }

    #[test]
    fn render_does_not_panic() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_git_panel(area, &mut buf, &mut state, false, &theme);
    }

    #[test]
    fn render_empty_does_not_panic() {
        let mut state = GitPanelState::new();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_git_panel(area, &mut buf, &mut state, false, &theme);
    }

    #[test]
    fn status_icon_colors() {
        let theme = Theme::dark();
        let (icon, _) = status_icon_color(FileStatus::Modified, &theme);
        assert_eq!(icon, 'M');
        let (icon, _) = status_icon_color(FileStatus::Added, &theme);
        assert_eq!(icon, 'A');
        let (icon, _) = status_icon_color(FileStatus::Deleted, &theme);
        assert_eq!(icon, 'D');
    }
}
