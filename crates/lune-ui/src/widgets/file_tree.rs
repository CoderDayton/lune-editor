//! File tree widget — renders and interacts with the workspace tree.
//!
//! Displays the workspace directory tree in the left sidebar with:
//! - Indented entries (directories first, alphabetical)
//! - Expand/collapse indicators for directories
//! - Git status color suffixes
//! - Selection highlight bar
//! - Scrolling for large trees

use std::path::Path;

use crate::primitives::{Block, Borders, Buffer, Line, Modifier, Rect, Span, Style, Widget};

use lune_core::workspace::{DirEntry, EntryKind, FileStatus, Workspace, flatten_tree};

use crate::theme::Theme;

/// Configuration for file tree rendering.
#[derive(Clone, Debug)]
pub struct FileTreeConfig {
    /// Spaces per nesting level.
    pub indent_size: u16,
    /// Whether to show type icons (nerd font).
    pub icons: bool,
    /// Sort directories before files.
    pub sort_dirs_first: bool,
}

impl Default for FileTreeConfig {
    fn default() -> Self {
        Self {
            indent_size: 2,
            icons: false,
            sort_dirs_first: true,
        }
    }
}

/// State for the file tree widget.
#[derive(Debug)]
pub struct FileTreeState {
    /// Flattened list of (depth, entry) for rendering.
    pub entries: Vec<(usize, DirEntry)>,
    /// Currently selected index in the flattened list.
    pub selected: usize,
    /// Scroll offset (first visible row).
    pub scroll_offset: usize,
    /// Display configuration.
    pub config: FileTreeConfig,
}

impl FileTreeState {
    /// Create a new file tree state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            config: FileTreeConfig::default(),
        }
    }

    /// Refresh the flattened entry list from the workspace.
    ///
    /// # Errors
    /// Returns an error if the workspace cannot read directories.
    pub fn refresh(&mut self, workspace: &mut Workspace) -> anyhow::Result<()> {
        self.entries = flatten_tree(workspace)?;
        // Clamp selected index.
        if self.entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.entries.len() - 1);
        }
        Ok(())
    }

    /// Move selection up by `n`.
    pub const fn select_prev(&mut self, n: usize) {
        self.selected = self.selected.saturating_sub(n);
    }

    /// Move selection down by `n`.
    pub fn select_next(&mut self, n: usize) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + n).min(self.entries.len() - 1);
        }
    }

    /// Get the currently selected entry.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&(usize, DirEntry)> {
        self.entries.get(self.selected)
    }

    /// Get the path of the currently selected entry.
    #[must_use]
    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_entry().map(|(_, entry)| entry.path.as_path())
    }

    /// Whether the selected entry is a directory.
    #[must_use]
    pub fn selected_is_dir(&self) -> bool {
        self.selected_entry()
            .is_some_and(|(_, entry)| matches!(entry.kind, EntryKind::Directory { .. }))
    }

    /// Whether the selected entry is a file.
    #[must_use]
    pub fn selected_is_file(&self) -> bool {
        self.selected_entry()
            .is_some_and(|(_, entry)| matches!(entry.kind, EntryKind::File))
    }

    /// Ensure the selected item is visible given the panel height.
    pub const fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        }
    }

    /// Find the index of an entry by path.
    #[must_use]
    pub fn find_by_path(&self, path: &Path) -> Option<usize> {
        self.entries.iter().position(|(_, e)| e.path == path)
    }

    /// Select an entry by path, scrolling to it if needed.
    /// Returns `true` if the entry was found and selected.
    pub fn select_by_path(&mut self, path: &Path, visible_height: usize) -> bool {
        if let Some(idx) = self.find_by_path(path) {
            self.selected = idx;
            self.ensure_visible(visible_height);
            true
        } else {
            false
        }
    }

    /// Reveal a path by expanding all ancestor directories.
    ///
    /// After calling this, you should call [`refresh`] and then [`select_by_path`].
    ///
    /// # Errors
    /// Returns an error if ancestor directories cannot be listed.
    pub fn reveal_path(&mut self, path: &Path, workspace: &mut Workspace) -> anyhow::Result<()> {
        let root = workspace.root().to_path_buf();

        // Collect ancestors from root down to the path.
        let mut ancestors = Vec::new();
        let mut current = path.to_path_buf();
        while current != root && current.starts_with(&root) {
            if let Some(parent) = current.parent() {
                ancestors.push(current.clone());
                current = parent.to_path_buf();
            } else {
                break;
            }
        }
        ancestors.reverse();

        // Expand each ancestor directory.
        for ancestor in &ancestors {
            if ancestor.is_dir() {
                // Ensure the parent is listed so the entry exists.
                if let Some(parent) = ancestor.parent() {
                    let _ = workspace.list_dir(parent);
                }
                workspace.set_expanded(ancestor, true);
            }
        }

        Ok(())
    }

    /// Hit test: given a mouse click position and the render area,
    /// return the index of the clicked entry.
    #[must_use]
    pub fn hit_test(&self, row: u16, area: Rect) -> Option<usize> {
        if row < area.y + 1 {
            // Clicked on header row.
            return None;
        }
        let rel_row = (row - area.y - 1) as usize;
        let idx = self.scroll_offset + rel_row;
        if idx < self.entries.len() {
            Some(idx)
        } else {
            None
        }
    }
}

impl Default for FileTreeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the file tree into a buffer region.
///
/// Layout:
/// ```text
/// ┌ EXPLORER ──────────┐
/// │ ▼ src               │
/// │   main.rs         M │
/// │   lib.rs            │
/// │ ▶ tests             │
/// │ Cargo.toml        ? │
/// └────────────────────┘
/// ```
#[allow(clippy::cast_possible_truncation)]
pub fn render_file_tree(
    area: Rect,
    buf: &mut Buffer,
    state: &mut FileTreeState,
    workspace_name: &str,
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

    // Use a ratatui Block with a right border for the panel separator.
    let border_style = Style::default().fg(accent);
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border_style);
    let content_area = block.inner(area);
    block.render(area, buf);
    let content_width = content_area.width;

    // Header row — accent color when focused.
    let header = format!(" {workspace_name}");
    let header_style = if is_focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(Span::styled(header, header_style)).render(
        Rect::new(content_area.x, content_area.y, content_width, 1),
        buf,
    );

    if area.height < 2 {
        return;
    }

    let content_height = (area.height - 1) as usize;
    state.ensure_visible(content_height);

    let visible_entries = state
        .entries
        .iter()
        .skip(state.scroll_offset)
        .take(content_height);

    for (i, (depth, entry)) in visible_entries.enumerate() {
        let y = content_area.y + 1 + i as u16;
        if y >= content_area.y + content_area.height {
            break;
        }

        let is_selected = state.scroll_offset + i == state.selected;
        let line_area = Rect::new(content_area.x, y, content_width, 1);

        render_entry(
            line_area,
            buf,
            entry,
            *depth,
            is_selected,
            &state.config,
            theme,
        );
    }
}

/// Render a single file tree entry.
#[allow(clippy::cast_possible_truncation)]
fn render_entry(
    area: Rect,
    buf: &mut Buffer,
    entry: &DirEntry,
    depth: usize,
    is_selected: bool,
    config: &FileTreeConfig,
    theme: &Theme,
) {
    if area.width == 0 {
        return;
    }

    let indent = " ".repeat(depth * config.indent_size as usize);

    let (prefix, base_name_style) = match &entry.kind {
        EntryKind::Directory { expanded } => {
            let arrow = if *expanded { "▼ " } else { "▶ " };
            (
                arrow,
                Style::default()
                    .fg(theme.tree_dir_fg)
                    .add_modifier(Modifier::BOLD),
            )
        }
        EntryKind::File => ("  ", Style::default().fg(theme.tree_file_fg)),
        EntryKind::Symlink => ("@ ", Style::default().fg(theme.tree_symlink_fg)),
    };

    // Derive git-related display data in a single lookup.
    let (name_style, git_suffix, git_color) =
        entry
            .git_status
            .map_or((base_name_style, "", theme.fg_dim), |status| {
                let (suffix, color) = match status {
                    FileStatus::Modified => (" M", theme.git_modified),
                    FileStatus::Added => (" A", theme.git_added),
                    FileStatus::Deleted => (" D", theme.git_deleted),
                    FileStatus::Renamed => (" R", theme.git_renamed),
                    FileStatus::Untracked => (" ?", theme.git_untracked),
                    FileStatus::Conflicted => (" !", theme.git_conflicted),
                    FileStatus::Ignored => (" I", theme.git_ignored),
                };
                (base_name_style.fg(color), suffix, color)
            });

    let mut spans = vec![
        Span::raw(indent),
        Span::raw(prefix),
        Span::styled(&entry.name, name_style),
    ];

    if !git_suffix.is_empty() {
        spans.push(Span::styled(git_suffix, Style::default().fg(git_color)));
    }

    let bg = if is_selected {
        theme.tree_selected_bg
    } else {
        theme.bg
    };

    Line::from(spans).render(area, buf);

    // Apply selection background over rendered cells (single pass).
    if is_selected {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn make_test_entries() -> Vec<(usize, DirEntry)> {
        vec![
            (
                0,
                DirEntry {
                    path: PathBuf::from("/ws/src"),
                    name: "src".to_string(),
                    kind: EntryKind::Directory { expanded: true },
                    git_status: None,
                },
            ),
            (
                1,
                DirEntry {
                    path: PathBuf::from("/ws/src/main.rs"),
                    name: "main.rs".to_string(),
                    kind: EntryKind::File,
                    git_status: Some(FileStatus::Modified),
                },
            ),
            (
                1,
                DirEntry {
                    path: PathBuf::from("/ws/src/lib.rs"),
                    name: "lib.rs".to_string(),
                    kind: EntryKind::File,
                    git_status: None,
                },
            ),
            (
                0,
                DirEntry {
                    path: PathBuf::from("/ws/tests"),
                    name: "tests".to_string(),
                    kind: EntryKind::Directory { expanded: false },
                    git_status: None,
                },
            ),
            (
                0,
                DirEntry {
                    path: PathBuf::from("/ws/Cargo.toml"),
                    name: "Cargo.toml".to_string(),
                    kind: EntryKind::File,
                    git_status: Some(FileStatus::Untracked),
                },
            ),
        ]
    }

    #[test]
    fn state_select_prev_next() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();

        assert_eq!(state.selected, 0);
        state.select_next(1);
        assert_eq!(state.selected, 1);
        state.select_next(1);
        assert_eq!(state.selected, 2);
        state.select_prev(1);
        assert_eq!(state.selected, 1);

        // Can't go below 0.
        state.select_prev(100);
        assert_eq!(state.selected, 0);

        // Can't go past last.
        state.select_next(100);
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn selected_entry_info() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();

        // Index 0 is a directory.
        assert!(state.selected_is_dir());
        assert!(!state.selected_is_file());
        assert_eq!(state.selected_path(), Some(Path::new("/ws/src")));

        // Index 1 is a file.
        state.selected = 1;
        assert!(!state.selected_is_dir());
        assert!(state.selected_is_file());
    }

    #[test]
    fn ensure_visible_scrolls() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();
        state.selected = 4;

        // With height 3, selected(4) should force scroll_offset up.
        state.ensure_visible(3);
        assert_eq!(state.scroll_offset, 2);

        // With selected 0, should scroll back.
        state.selected = 0;
        state.ensure_visible(3);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn find_and_select_by_path() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();

        let found = state.select_by_path(Path::new("/ws/src/lib.rs"), 10);
        assert!(found);
        assert_eq!(state.selected, 2);

        let not_found = state.select_by_path(Path::new("/ws/nonexistent"), 10);
        assert!(!not_found);
    }

    #[test]
    fn hit_test_maps_row_to_index() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();
        let area = Rect::new(0, 0, 30, 10);

        // Row 0 is the header.
        assert!(state.hit_test(0, area).is_none());

        // Row 1 should be entry 0.
        assert_eq!(state.hit_test(1, area), Some(0));
        // Row 2 → entry 1.
        assert_eq!(state.hit_test(2, area), Some(1));
        // Row 6 → entry 5 (beyond entries).
        assert!(state.hit_test(6, area).is_none());
    }

    #[test]
    fn hit_test_with_scroll() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();
        state.scroll_offset = 2;
        let area = Rect::new(0, 0, 30, 10);

        // Row 1 → scroll_offset(2) + 0 = entry 2.
        assert_eq!(state.hit_test(1, area), Some(2));
    }

    #[test]
    fn render_does_not_panic_on_empty() {
        let mut state = FileTreeState::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        let theme = Theme::dark();
        render_file_tree(
            Rect::new(0, 0, 30, 10),
            &mut buf,
            &mut state,
            "project",
            false,
            &theme,
        );
    }

    #[test]
    fn render_does_not_panic_on_zero_area() {
        let mut state = FileTreeState::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        let theme = Theme::dark();
        render_file_tree(Rect::ZERO, &mut buf, &mut state, "project", false, &theme);
    }

    #[test]
    fn render_with_entries() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();

        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_file_tree(area, &mut buf, &mut state, "my-project", true, &theme);

        // Verify header is rendered.
        let header_cell = buf.cell((1, 0)).expect("cell should exist");
        assert_eq!(header_cell.symbol(), "m"); // " my-project" starts at col 1
    }

    #[test]
    fn config_default() {
        let config = FileTreeConfig::default();
        assert_eq!(config.indent_size, 2);
        assert!(!config.icons);
        assert!(config.sort_dirs_first);
    }
}
