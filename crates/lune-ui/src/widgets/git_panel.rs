//! Git panel widget.
//!
//! Renders staged and unstaged file changes in the right sidebar.
//! Supports keyboard/mouse navigation, staging (`s`), unstaging (`u`),
//! discarding (`d`), opening diff view (`Enter`), and committing (`c`).

use crate::primitives::{
    Borders, Buffer, Color, Line, Modifier, Rect, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Span, StatefulWidget, Style, Widget, symbols,
};

use std::path::Path;
use std::sync::Arc;

use lune_core::ports::{FileEntry, FileState, StatusSnapshot};

use crate::theme::Theme;
use crate::widgets::diff_view::DiffViewState;
use crate::widgets::panel::{panel_block, panel_title};

/// State of the git panel widget.
#[derive(Clone, Debug)]
pub struct GitPanelState {
    /// Most recently published git status snapshot (shared, Arc-cloned).
    pub status: Option<Arc<StatusSnapshot>>,
    /// Flattened list of entries for rendering/navigation.
    entries: Vec<PanelEntry>,
    /// Currently selected index in the entries list.
    pub selected: usize,
    /// Scroll offset.
    pub scroll: usize,
    /// Diff view state for the currently selected file.
    pub diff_view: DiffViewState,
    /// List viewport height from the last render, for page navigation.
    page_height: usize,
    /// List region from the last render, for hit-testing mouse clicks.
    /// `None` until the entry list is drawn (e.g. empty/clean state).
    list_area: Option<Rect>,
}

/// A single entry in the git panel (section header or file).
#[derive(Clone, Debug)]
enum PanelEntry {
    /// Section header ("Staged Changes", "Changes").
    Header(String),
    /// A file entry.
    File {
        file: FileEntry,
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
    pub fn new() -> Self {
        Self {
            status: None,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            diff_view: DiffViewState::default(),
            page_height: 0,
            list_area: None,
        }
    }

    /// Update the panel with a new git status snapshot.
    pub fn update_status(&mut self, status: Arc<StatusSnapshot>) {
        // Remember the path under the cursor so selection sticks to the
        // same file after the list is rebuilt and reordered (e.g. a
        // staged file hops from "Changes" to "Staged Changes").
        let prev_path = self.selected_file().map(|f| f.path.clone());

        self.entries = build_entries(&status);
        self.status = Some(status);

        // Re-anchor to the previously selected path when it still exists;
        // otherwise clamp the index into range.
        if let Some(idx) = prev_path.as_deref().and_then(|p| self.index_of_path(p)) {
            self.selected = idx;
        } else if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
        // Skip to first file if a header is selected.
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
    pub fn selected_file(&self) -> Option<&FileEntry> {
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

    /// Whether the entry at `idx` is a file (as opposed to a section
    /// header). Headers are not selectable, so a click landing on one
    /// must not move the selection.
    #[must_use]
    pub fn entry_is_file(&self, idx: usize) -> bool {
        matches!(self.entries.get(idx), Some(PanelEntry::File { .. }))
    }

    /// Map a terminal row to the entry index drawn there, using the list
    /// region recorded on the last render. Returns `None` for rows
    /// outside the list (branch header, footer, padding) or past the
    /// last entry.
    #[must_use]
    pub fn hit_test(&self, row: u16) -> Option<usize> {
        let area = self.list_area?;
        if row < area.y || row >= area.y + area.height {
            return None;
        }
        let idx = self.scroll + (row - area.y) as usize;
        (idx < self.entries.len()).then_some(idx)
    }

    /// Jump to the first file entry.
    pub fn select_first(&mut self) {
        self.selected = 0;
        self.skip_headers_forward();
    }

    /// Jump to the last file entry.
    pub fn select_last(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.skip_headers_backward();
    }

    /// Move selection up by one screenful (keeping one row of overlap).
    pub fn page_up(&mut self) {
        let step = self.page_step();
        self.selected = self.selected.saturating_sub(step);
        self.skip_headers_backward();
    }

    /// Move selection down by one screenful (keeping one row of overlap).
    pub fn page_down(&mut self) {
        let step = self.page_step();
        let last = self.entries.len().saturating_sub(1);
        self.selected = (self.selected + step).min(last);
        self.skip_headers_forward();
    }

    /// Rows moved per page jump: a screenful minus one row of overlap.
    fn page_step(&self) -> usize {
        self.page_height.saturating_sub(1).max(1)
    }

    /// Find the entry index of a file by repo-relative path.
    fn index_of_path(&self, path: &Path) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| matches!(e, PanelEntry::File { file, .. } if file.path == path))
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

/// Build the flattened entry list from a `StatusSnapshot`.
fn build_entries(status: &StatusSnapshot) -> Vec<PanelEntry> {
    let mut entries = Vec::new();

    let staged: Vec<(usize, &FileEntry)> = status
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.staged)
        .collect();

    let unstaged: Vec<(usize, &FileEntry)> = status
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.staged)
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
    // Cleared here and only set once the entry list is actually drawn,
    // so a stale rect can't make clicks select rows on an empty panel.
    state.list_area = None;

    let block = panel_block(theme, is_focused, Borders::ALL).title(panel_title(
        "SOURCE CONTROL",
        theme,
        is_focused,
    ));
    let content_area = block.inner(area);
    block.render(area, buf);

    if content_area.height == 0 || content_area.width == 0 {
        return;
    }

    let mut body = content_area;

    // Branch / ahead-behind summary on the first interior row.
    if let Some(status) = state.status.as_deref() {
        render_branch_header(Rect::new(body.x, body.y, body.width, 1), buf, status, theme);
        body = Rect::new(body.x, body.y + 1, body.width, body.height - 1);
    }

    // Reserve a footer row for key hints when a repo is present and the
    // panel is tall enough to spare the row.
    if state.status.is_some() && body.height >= 3 {
        let footer = Rect::new(body.x, body.y + body.height - 1, body.width, 1);
        render_footer_hints(footer, buf, theme);
        body = Rect::new(body.x, body.y, body.width, body.height - 1);
    }

    // Empty / clean placeholder.
    if state.status.is_none() || state.entries.is_empty() {
        render_empty_state(body, buf, state.status.is_some(), theme);
        return;
    }

    let list_height = body.height as usize;
    state.page_height = list_height;

    // Ensure selected entry is visible.
    if state.selected < state.scroll {
        state.scroll = state.selected;
    } else if state.selected >= state.scroll + list_height {
        state.scroll = state.selected - list_height + 1;
    }

    // Reserve the rightmost column for a scrollbar when entries overflow.
    let show_scrollbar = state.entries.len() > list_height;
    let list_width = body.width.saturating_sub(u16::from(show_scrollbar));

    // Record the list region so mouse clicks can be mapped back to
    // entries (`hit_test`). Includes the scrollbar column, which is fine
    // since hit-testing only uses the row.
    state.list_area = Some(body);

    for row in 0..list_height {
        let entry_idx = state.scroll + row;
        if entry_idx >= state.entries.len() {
            break;
        }
        let y = body.y + row as u16;
        let is_selected = entry_idx == state.selected;
        let line_area = Rect::new(body.x, y, list_width, 1);

        match &state.entries[entry_idx] {
            PanelEntry::Header(text) => {
                Line::from(Span::styled(
                    format!(" {text}"),
                    Style::new().fg(theme.fg_muted).add_modifier(Modifier::BOLD),
                ))
                .render(line_area, buf);
            }
            PanelEntry::File { file, .. } => {
                render_file_entry(line_area, buf, file, is_selected, is_focused, theme);
            }
        }
    }

    if show_scrollbar {
        render_scrollbar(body, state, list_height, buf, theme);
    }
}

/// Render the branch name and ahead/behind counts on one row.
fn render_branch_header(area: Rect, buf: &mut Buffer, status: &StatusSnapshot, theme: &Theme) {
    let branch = status.branch.as_deref().unwrap_or("(detached)");
    let mut spans = vec![
        Span::styled(" \u{2387} ", Style::new().fg(theme.fg_muted)),
        Span::styled(
            branch.to_string(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ];
    if status.ahead > 0 {
        spans.push(Span::styled(
            format!(" \u{2191}{}", status.ahead),
            Style::new().fg(theme.git_added),
        ));
    }
    if status.behind > 0 {
        spans.push(Span::styled(
            format!(" \u{2193}{}", status.behind),
            Style::new().fg(theme.git_deleted),
        ));
    }
    Line::from(spans).render(area, buf);
}

/// Render the key-hint footer row.
fn render_footer_hints(area: Rect, buf: &mut Buffer, theme: &Theme) {
    // Pick the widest legend that fits so the row never truncates mid-word.
    const FULL: &str = " s stage  u unstage  c commit  d discard";
    const SHORT: &str = " s/u stage  c commit  d discard";
    const TINY: &str = " s u c d";
    let width = area.width as usize;
    let text = if width >= FULL.chars().count() {
        FULL
    } else if width >= SHORT.chars().count() {
        SHORT
    } else {
        TINY
    };
    Line::from(Span::styled(text, Style::new().fg(theme.fg_dim))).render(area, buf);
}

/// Render the empty / clean placeholder, centered in `area`.
#[allow(clippy::cast_possible_truncation)]
fn render_empty_state(area: Rect, buf: &mut Buffer, in_repo: bool, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let (text, color) = if in_repo {
        ("\u{2713} Working tree clean", theme.git_added)
    } else {
        ("Not a git repository", theme.fg_muted)
    };
    let w = text.chars().count() as u16;
    if w > area.width {
        return;
    }
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height / 2;
    Line::from(Span::styled(text, Style::new().fg(color))).render(Rect::new(x, y, w, 1), buf);
}

/// Render the list scrollbar in the rightmost column of `area`.
fn render_scrollbar(
    area: Rect,
    state: &GitPanelState,
    viewport_height: usize,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let viewport_len = viewport_height.max(1);
    let max_top = state.entries.len().saturating_sub(viewport_len);
    let scroll_domain_len = max_top.saturating_add(1).max(1);

    let mut sb_state = ScrollbarState::new(scroll_domain_len)
        .position(state.scroll.min(max_top))
        .viewport_content_length(viewport_len);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some(symbols::line::VERTICAL))
        .thumb_symbol(symbols::block::FULL)
        .track_style(Style::new().fg(theme.fg_dim))
        .thumb_style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD));

    StatefulWidget::render(scrollbar, area, buf, &mut sb_state);
}

/// Render a single file entry in the git panel.
fn render_file_entry(
    area: Rect,
    buf: &mut Buffer,
    file: &FileEntry,
    is_selected: bool,
    is_focused: bool,
    theme: &Theme,
) {
    if area.width == 0 {
        return;
    }

    let (icon, color) = status_icon_color(file.state, theme);
    let path_str = file.path.to_string_lossy();

    // Split "dir/sub/name.rs" so the directory prefix can be dimmed and
    // the filename stays bright for quicker scanning.
    let (dir, name) = path_str.rfind('/').map_or_else(
        || ("", path_str.as_ref()),
        |i| (&path_str[..=i], &path_str[i + 1..]),
    );

    let mut spans = vec![Span::styled(format!(" {icon} "), Style::new().fg(color))];
    if !dir.is_empty() {
        spans.push(Span::styled(dir.to_string(), Style::new().fg(theme.fg_dim)));
    }
    spans.push(Span::styled(name.to_string(), Style::new().fg(theme.fg)));
    Line::from(spans).render(area, buf);

    // Paint a full-row selection bar, dimmed when the panel is unfocused
    // so the active pane is unambiguous (mirrors the file tree).
    if is_selected {
        let bg = if is_focused {
            theme.tree_selected_bg
        } else {
            crate::style::color::blend(theme.bg, theme.tree_selected_bg, 0.5)
        };
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }
    }
}

/// Get the status icon character and color for a port [`FileState`].
const fn status_icon_color(state: FileState, theme: &Theme) -> (char, Color) {
    match state {
        FileState::Modified => ('M', theme.git_modified),
        FileState::Added => ('A', theme.git_added),
        FileState::Deleted => ('D', theme.git_deleted),
        FileState::Untracked => ('U', theme.git_untracked),
        FileState::Conflicted => ('C', theme.git_conflicted),
        FileState::Clean => ('·', theme.git_ignored),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::theme::Theme;

    fn make_status() -> Arc<StatusSnapshot> {
        Arc::new(StatusSnapshot {
            branch: Some("main".to_owned()),
            files: vec![
                FileEntry {
                    path: PathBuf::from("staged.rs"),
                    state: FileState::Modified,
                    staged: true,
                },
                FileEntry {
                    path: PathBuf::from("unstaged.rs"),
                    state: FileState::Modified,
                    staged: false,
                },
                FileEntry {
                    path: PathBuf::from("new_file.rs"),
                    state: FileState::Untracked,
                    staged: false,
                },
            ],
            ..Default::default()
        })
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
    fn hit_test_maps_rows_to_entries_after_render() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        // Borders::ALL inset (1) + branch-header row (1) put the first
        // list row at y=2; a footer row is reserved at the bottom.
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_git_panel(area, &mut buf, &mut state, false, &theme);

        // entries: [Header, staged.rs, Header, unstaged.rs, new_file.rs]
        assert_eq!(state.hit_test(2), Some(0), "first list row");
        assert!(!state.entry_is_file(0), "row 0 is a section header");
        assert_eq!(state.hit_test(3), Some(1));
        assert!(state.entry_is_file(1), "staged.rs is a file");
        assert_eq!(state.hit_test(6), Some(4), "last file row");
        assert!(state.entry_is_file(4));

        // Branch-header row, the row past the last entry, and the footer
        // are all outside the list.
        assert_eq!(state.hit_test(1), None, "branch-header row");
        assert_eq!(state.hit_test(7), None, "past last entry");
    }

    #[test]
    fn hit_test_is_none_on_empty_panel() {
        let mut state = GitPanelState::new();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_git_panel(area, &mut buf, &mut state, false, &theme);
        assert_eq!(state.hit_test(3), None);
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
        let (icon, _) = status_icon_color(FileState::Modified, &theme);
        assert_eq!(icon, 'M');
        let (icon, _) = status_icon_color(FileState::Added, &theme);
        assert_eq!(icon, 'A');
        let (icon, _) = status_icon_color(FileState::Deleted, &theme);
        assert_eq!(icon, 'D');
    }

    /// Build a snapshot where every file is staged (used to make the
    /// previously-unstaged file hop sections).
    fn all_staged_status() -> Arc<StatusSnapshot> {
        Arc::new(StatusSnapshot {
            branch: Some("main".to_owned()),
            files: vec![
                FileEntry {
                    path: PathBuf::from("staged.rs"),
                    state: FileState::Modified,
                    staged: true,
                },
                FileEntry {
                    path: PathBuf::from("unstaged.rs"),
                    state: FileState::Modified,
                    staged: true,
                },
                FileEntry {
                    path: PathBuf::from("new_file.rs"),
                    state: FileState::Untracked,
                    staged: true,
                },
            ],
            ..Default::default()
        })
    }

    #[test]
    fn select_first_and_last_land_on_files() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        state.select_first();
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("staged.rs"))
        );

        state.select_last();
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("new_file.rs"))
        );
    }

    #[test]
    fn page_up_down_stay_on_files_and_clamp() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        state.select_first();
        state.page_down();
        assert!(state.selected_file().is_some());
        state.page_up();
        assert!(state.selected_file().is_some());

        // Paging past the end clamps to the last file.
        for _ in 0..10 {
            state.page_down();
        }
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("new_file.rs"))
        );
    }

    #[test]
    fn update_status_reanchors_selection_to_same_path() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());

        // Move selection onto the unstaged file.
        state.select_last();
        state.select_prev();
        // Walk to "unstaged.rs" deterministically.
        state.select_first();
        while state
            .selected_file()
            .is_some_and(|f| f.path != Path::new("unstaged.rs"))
        {
            let before = state.selected;
            state.select_next();
            if state.selected == before {
                break;
            }
        }
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("unstaged.rs"))
        );

        // After staging it, the file hops to the Staged section; selection
        // must follow the path, not the old index.
        state.update_status(all_staged_status());
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("unstaged.rs"))
        );
    }

    #[test]
    fn update_status_clamps_when_selected_path_removed() {
        let mut state = GitPanelState::new();
        state.update_status(make_status());
        state.select_last();

        // Rebuild with only the first file remaining.
        let shrunk = Arc::new(StatusSnapshot {
            branch: Some("main".to_owned()),
            files: vec![FileEntry {
                path: PathBuf::from("staged.rs"),
                state: FileState::Modified,
                staged: true,
            }],
            ..Default::default()
        });
        state.update_status(shrunk);

        // Selection must remain on a valid file entry.
        assert_eq!(
            state.selected_file().map(|f| f.path.clone()),
            Some(PathBuf::from("staged.rs"))
        );
    }

    #[test]
    fn selection_bar_bg_differs_with_focus() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 10);

        let render = |focused: bool| {
            let mut state = GitPanelState::new();
            state.update_status(make_status());
            let mut buf = Buffer::empty(area);
            render_git_panel(area, &mut buf, &mut state, focused, &theme);
            buf
        };

        let focused = render(true);
        let unfocused = render(false);

        // The focus-dimmed selection bar is the only styling difference, so
        // at least one cell background must differ between the two renders.
        let differs = (area.x..area.x + area.width).any(|x| {
            (area.y..area.y + area.height)
                .any(|y| focused.cell((x, y)).map(|c| c.bg) != unfocused.cell((x, y)).map(|c| c.bg))
        });
        assert!(
            differs,
            "focused and unfocused selection bars should differ in background"
        );
    }
}
