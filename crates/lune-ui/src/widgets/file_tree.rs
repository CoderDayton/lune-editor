//! File tree widget — renders and interacts with the workspace tree.
//!
//! Displays the workspace directory tree in the left sidebar with:
//! - Indented entries (directories first, alphabetical)
//! - Expand/collapse indicators for directories
//! - Git status color suffixes
//! - Selection highlight bar
//! - Scrolling for large trees

use std::path::Path;

use crate::primitives::{
    Borders, Buffer, Line, Modifier, Rect, Scrollbar, ScrollbarOrientation, ScrollbarState, Span,
    StatefulWidget, Style, Widget, symbols,
};

use lune_core::workspace::{DirEntry, EntryKind, FileStatus, Workspace, flatten_tree};

use crate::theme::Theme;
use crate::widgets::panel::{panel_block, panel_title};

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
    /// Last visible content height, recorded by [`Self::ensure_visible`]
    /// so page-wise navigation jumps by a full screenful.
    page_height: usize,
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
            page_height: 0,
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

    /// Jump selection to the first entry.
    pub const fn select_first(&mut self) {
        self.selected = 0;
    }

    /// Jump selection to the last entry.
    pub const fn select_last(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
    }

    /// Move selection up by one screenful, keeping one row of overlap.
    pub const fn page_up(&mut self) {
        self.select_prev(self.page_step());
    }

    /// Move selection down by one screenful, keeping one row of overlap.
    pub fn page_down(&mut self) {
        self.select_next(self.page_step());
    }

    /// Rows moved per page jump: a screenful minus one row of overlap, so
    /// the entry at the fold stays on screen for context. At least 1.
    const fn page_step(&self) -> usize {
        match self.page_height.saturating_sub(1) {
            0 => 1,
            step => step,
        }
    }

    /// Move selection to the directory enclosing the current entry — the
    /// nearest preceding entry at a shallower depth. No-op for a
    /// top-level entry (nothing encloses it).
    pub fn select_parent(&mut self) {
        let Some(&(depth, _)) = self.entries.get(self.selected) else {
            return;
        };
        if let Some(idx) = self.entries[..self.selected]
            .iter()
            .rposition(|(d, _)| *d < depth)
        {
            self.selected = idx;
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
        self.page_height = visible_height;
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
    /// After calling this, you should call `refresh` and then `select_by_path`.
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
    pub const fn hit_test(&self, row: u16, area: Rect) -> Option<usize> {
        // Block with Borders::ALL: top border (with title) at area.y,
        // content starts at area.y + 1, bottom border at area.y + height - 1.
        if row <= area.y || row >= area.y + area.height.saturating_sub(1) {
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

    let block = panel_block(theme, is_focused, Borders::ALL).title(panel_title(
        workspace_name,
        theme,
        is_focused,
    ));
    let content_area = block.inner(area);
    block.render(area, buf);
    let content_width = content_area.width;

    if content_area.height == 0 {
        return;
    }

    let content_height = content_area.height as usize;
    state.ensure_visible(content_height);

    // Empty-state hint: a blank panel reads as "broken"; show a muted
    // centered cue instead.
    if state.entries.is_empty() {
        let hint = "No files";
        let hint_w = hint.chars().count() as u16;
        if hint_w <= content_area.width {
            let x = content_area.x + (content_area.width - hint_w) / 2;
            let y = content_area.y + content_area.height / 2;
            Line::from(Span::styled(hint, Style::default().fg(theme.fg_muted)))
                .render(Rect::new(x, y, hint_w, 1), buf);
        }
        return;
    }

    // Reserve the last interior column for a scrollbar when the tree is
    // taller than the panel, so there is a visible cue that more entries
    // exist above or below the fold.
    let show_scrollbar = state.entries.len() > content_height;
    let list_width = content_width.saturating_sub(u16::from(show_scrollbar));

    let visible_entries = state
        .entries
        .iter()
        .skip(state.scroll_offset)
        .take(content_height);

    for (i, (depth, entry)) in visible_entries.enumerate() {
        let y = content_area.y + i as u16;
        if y >= content_area.y + content_area.height {
            break;
        }

        let is_selected = state.scroll_offset + i == state.selected;
        let line_area = Rect::new(content_area.x, y, list_width, 1);

        render_entry(
            line_area,
            buf,
            entry,
            *depth,
            is_selected,
            is_focused,
            &state.config,
            theme,
        );
    }

    if show_scrollbar {
        render_tree_scrollbar(content_area, state, content_height, buf, theme);
    }
}

/// Render a vertical scrollbar in the rightmost column of `area`. The
/// scroll domain is expressed as scroll-offset positions so the thumb is
/// proportional to visible/total and reaches the end at the last row.
fn render_tree_scrollbar(
    area: Rect,
    state: &FileTreeState,
    viewport_height: usize,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let viewport_len = viewport_height.max(1);
    let max_top = state.entries.len().saturating_sub(viewport_len);
    let scroll_domain_len = max_top.saturating_add(1).max(1);

    let mut sb_state = ScrollbarState::new(scroll_domain_len)
        .position(state.scroll_offset.min(max_top))
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

/// Render a single file tree entry.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn render_entry(
    area: Rect,
    buf: &mut Buffer,
    entry: &DirEntry,
    depth: usize,
    is_selected: bool,
    is_focused: bool,
    config: &FileTreeConfig,
    theme: &Theme,
) {
    if area.width == 0 {
        return;
    }

    let indent = build_indent_guides(depth, config.indent_size);

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

    // Derive git-related display data in a single lookup: the styled name
    // colour and a single-char status marker pinned to the right edge.
    let (name_style, git_marker) = entry.git_status.map_or((base_name_style, None), |status| {
        let (marker, color) = match status {
            FileStatus::Modified => ("M", theme.git_modified),
            FileStatus::Added => ("A", theme.git_added),
            FileStatus::Deleted => ("D", theme.git_deleted),
            FileStatus::Renamed => ("R", theme.git_renamed),
            FileStatus::Untracked => ("?", theme.git_untracked),
            FileStatus::Conflicted => ("!", theme.git_conflicted),
            FileStatus::Ignored => ("I", theme.git_ignored),
        };
        (base_name_style.fg(color), Some((marker, color)))
    });

    // Truncate the name so a long filename can never push the git marker
    // (the key at-a-glance signal) off the right edge. Reserve 2 cells on
    // the right for the marker plus a 1-cell gap when a marker is shown.
    // Optional file-type icon, rendered between the prefix and the name.
    let icon_part = if config.icons {
        format!("{} ", icon_for(entry))
    } else {
        String::new()
    };
    let icon_cells = unicode_width::UnicodeWidthStr::width(icon_part.as_str());

    let reserved = if git_marker.is_some() { 2 } else { 0 };
    let indent_cells = depth * config.indent_size as usize;
    let prefix_cells = 2; // every prefix ("▼ ", "  ", "@ ") is 2 cells wide.
    let name_budget =
        (area.width as usize).saturating_sub(indent_cells + prefix_cells + icon_cells + reserved);
    let display_name = truncate_to_cells(&entry.name, name_budget);

    let spans = vec![
        Span::styled(indent, Style::default().fg(theme.fg_dim)),
        Span::raw(prefix),
        Span::styled(icon_part, name_style),
        Span::styled(display_name, name_style),
    ];

    let bg = if is_selected {
        if is_focused {
            theme.tree_selected_bg
        } else {
            // Dim the selection bar when the panel isn't focused so the
            // active pane is unambiguous.
            crate::style::color::blend(theme.bg, theme.tree_selected_bg, 0.5)
        }
    } else {
        theme.bg
    };

    Line::from(spans).render(area, buf);

    // Right-align the git marker in the final column.
    if let Some((marker, color)) = git_marker {
        let mx = area.x + area.width.saturating_sub(1);
        Line::from(Span::styled(marker, Style::default().fg(color)))
            .render(Rect::new(mx, area.y, 1, 1), buf);
    }

    // Apply selection background over rendered cells (single pass).
    if is_selected {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }
    }
}

/// Nerd-font icon for an entry, used only when [`FileTreeConfig::icons`]
/// is enabled. Directories and symlinks get a fixed glyph; files are
/// keyed off their extension with a generic fallback.
fn icon_for(entry: &DirEntry) -> &'static str {
    match entry.kind {
        EntryKind::Directory { expanded } => {
            if expanded {
                "\u{f07c}" // open folder
            } else {
                "\u{f07b}" // closed folder
            }
        }
        EntryKind::Symlink => "\u{f481}", // link
        EntryKind::File => icon_for_extension(&entry.name),
    }
}

/// Map a filename's extension to a nerd-font glyph.
fn icon_for_extension(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "\u{e7a8}",                             // rust
        "md" | "markdown" => "\u{f48a}",                // markdown
        "toml" | "yaml" | "yml" | "json" => "\u{e615}", // config
        "js" | "jsx" | "ts" | "tsx" => "\u{e74e}",      // js/ts
        "py" => "\u{e73c}",                             // python
        "lock" => "\u{f023}",                           // lock
        _ => "\u{f15b}",                                // generic file
    }
}

/// Build the indent prefix with vertical guide rails, one `│` per
/// nesting level followed by `indent_size - 1` spaces. Returns an empty
/// string for top-level (depth 0) entries.
fn build_indent_guides(depth: usize, indent_size: u16) -> String {
    if depth == 0 {
        return String::new();
    }
    let pad = (indent_size as usize).saturating_sub(1);
    let mut s = String::with_capacity(depth * indent_size as usize);
    for _ in 0..depth {
        s.push('│');
        for _ in 0..pad {
            s.push(' ');
        }
    }
    s
}

/// Truncate `s` to at most `max` display cells, appending `…` when cut.
/// Mirrors the editor pane's helper; kept local so the file-tree widget
/// stays self-contained.
fn truncate_to_cells(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if max == 0 {
        return String::new();
    }
    let mut width = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w > max {
            while width > max - 1 {
                if let Some(c) = out.pop() {
                    width -= UnicodeWidthChar::width(c).unwrap_or(0);
                } else {
                    break;
                }
            }
            out.push('…');
            return out;
        }
        out.push(ch);
        width += w;
    }
    out
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

        // Verify the Block title is rendered in the top border row.
        let top_row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            top_row.contains("my-project"),
            "Expected 'my-project' in top border row: {top_row:?}"
        );
    }

    #[test]
    fn overflowing_tree_renders_scrollbar() {
        let mut state = FileTreeState::new();
        state.entries = (0..30)
            .map(|i| {
                (
                    0usize,
                    DirEntry {
                        path: PathBuf::from(format!("/ws/f{i}.rs")),
                        name: format!("f{i}.rs"),
                        kind: EntryKind::File,
                        git_status: None,
                    },
                )
            })
            .collect();

        let area = Rect::new(0, 0, 24, 8); // content height 6 < 30 entries
        let theme = Theme::dark();
        let mut buf = Buffer::empty(area);
        render_file_tree(area, &mut buf, &mut state, "p", true, &theme);

        // Scrollbar sits in the last interior column (x = width - 2; the
        // right border is at width - 1).
        let sb_x = area.width - 2;
        let col: String = (1..area.height - 1)
            .filter_map(|y| buf.cell((sb_x, y)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            col.contains('█'),
            "an overflowing tree should render a scrollbar thumb: {col:?}"
        );
    }

    #[test]
    fn nested_entries_render_guide_rails() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries(); // index 1 = main.rs at depth 1
        let area = Rect::new(0, 0, 30, 8);
        let theme = Theme::dark();
        let mut buf = Buffer::empty(area);
        render_file_tree(area, &mut buf, &mut state, "p", true, &theme);

        // main.rs is the second content row → y = 2 (inside the top border).
        // Its first interior indent column (x = 1) should carry a rail,
        // not a blank space. (Scanning the whole row would falsely match
        // the block's own border `│`.)
        let guide = buf.cell((1, 2)).unwrap().symbol().to_string();
        assert_eq!(
            guide, "│",
            "a depth-1 entry should render a vertical guide rail in its indent"
        );
    }

    #[test]
    fn long_name_keeps_git_marker_visible() {
        let mut state = FileTreeState::new();
        state.entries = vec![(
            0usize,
            DirEntry {
                path: PathBuf::from("/ws/a_very_long_file_name_that_overflows.rs"),
                name: "a_very_long_file_name_that_overflows.rs".to_string(),
                kind: EntryKind::File,
                git_status: Some(FileStatus::Modified),
            },
        )];

        let area = Rect::new(0, 0, 20, 4); // inner width 18
        let theme = Theme::dark();
        let mut buf = Buffer::empty(area);
        render_file_tree(area, &mut buf, &mut state, "p", true, &theme);

        // First content row is y = 1 (inside the top border).
        let row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            row.contains('M'),
            "git marker must stay visible despite a long name: {row:?}"
        );
        assert!(
            row.contains('…'),
            "an overflowing name should be ellipsis-truncated: {row:?}"
        );
    }

    #[test]
    fn unfocused_selection_bar_is_dimmer() {
        let area = Rect::new(0, 0, 30, 8);
        let theme = Theme::dark();

        // Selected row is content row 0 (src) → y = 1, inside the top border.
        let mut s1 = FileTreeState::new();
        s1.entries = make_test_entries();
        let mut focused = Buffer::empty(area);
        render_file_tree(area, &mut focused, &mut s1, "p", true, &theme);

        let mut s2 = FileTreeState::new();
        s2.entries = make_test_entries();
        let mut unfocused = Buffer::empty(area);
        render_file_tree(area, &mut unfocused, &mut s2, "p", false, &theme);

        let fbg = focused.cell((2, 1)).unwrap().bg;
        let ubg = unfocused.cell((2, 1)).unwrap().bg;
        assert_eq!(
            fbg, theme.tree_selected_bg,
            "focused selection uses the full selection bg"
        );
        assert_ne!(
            ubg, fbg,
            "unfocused selection bar should be dimmer than focused"
        );
    }

    #[test]
    fn empty_tree_shows_hint() {
        let mut state = FileTreeState::new();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);
        let theme = Theme::dark();
        render_file_tree(area, &mut buf, &mut state, "proj", true, &theme);

        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("No files"),
            "empty tree should render a hint, got: {text:?}"
        );
    }

    #[test]
    fn icon_for_distinguishes_kinds_and_extensions() {
        let dir = DirEntry {
            path: PathBuf::from("/d"),
            name: "d".into(),
            kind: EntryKind::Directory { expanded: false },
            git_status: None,
        };
        let rs = DirEntry {
            path: PathBuf::from("/a.rs"),
            name: "a.rs".into(),
            kind: EntryKind::File,
            git_status: None,
        };
        let md = DirEntry {
            path: PathBuf::from("/b.md"),
            name: "b.md".into(),
            kind: EntryKind::File,
            git_status: None,
        };
        assert!(!icon_for(&dir).is_empty(), "every entry has an icon");
        assert_ne!(
            icon_for(&dir),
            icon_for(&rs),
            "directory and file icons differ"
        );
        assert_ne!(
            icon_for(&rs),
            icon_for(&md),
            "different file extensions map to different icons"
        );
    }

    #[test]
    fn icons_render_when_enabled() {
        let entry = DirEntry {
            path: PathBuf::from("/ws/main.rs"),
            name: "main.rs".into(),
            kind: EntryKind::File,
            git_status: None,
        };
        let expected = icon_for(&entry); // &'static str, independent of `entry`

        let mut state = FileTreeState::new();
        state.config.icons = true;
        state.entries = vec![(0usize, entry)];

        let area = Rect::new(0, 0, 30, 4);
        let theme = Theme::dark();
        let mut buf = Buffer::empty(area);
        render_file_tree(area, &mut buf, &mut state, "p", true, &theme);

        let row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            row.contains(expected),
            "the file-type icon should render when config.icons is set: {row:?}"
        );
    }

    #[test]
    fn icons_and_git_marker_coexist() {
        let entry = DirEntry {
            path: PathBuf::from("/ws/main.rs"),
            name: "main.rs".into(),
            kind: EntryKind::File,
            git_status: Some(FileStatus::Modified),
        };
        let icon = icon_for(&entry); // &'static str, independent of `entry`

        let mut state = FileTreeState::new();
        state.config.icons = true;
        state.entries = vec![(0usize, entry)];

        let area = Rect::new(0, 0, 20, 4); // inner width 18
        let theme = Theme::dark();
        let mut buf = Buffer::empty(area);
        render_file_tree(area, &mut buf, &mut state, "p", true, &theme);

        let row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            row.contains(icon),
            "icon should render alongside a git marker: {row:?}"
        );
        // The marker is pinned to the final interior column (just inside the
        // right border), regardless of the icon column ahead of the name.
        let marker = buf.cell((area.width - 2, 1)).unwrap().symbol().to_string();
        assert_eq!(
            marker, "M",
            "git marker stays in the final content column: {row:?}"
        );
    }

    #[test]
    fn config_default() {
        let config = FileTreeConfig::default();
        assert_eq!(config.indent_size, 2);
        assert!(!config.icons);
        assert!(config.sort_dirs_first);
    }

    #[test]
    fn select_first_and_last() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries(); // 5 entries
        state.selected = 2;

        state.select_first();
        assert_eq!(state.selected, 0);

        state.select_last();
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn page_nav_uses_recorded_height() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries(); // 5 entries
        // ensure_visible records the page height the next page jump uses.
        state.ensure_visible(3);

        state.select_first();
        state.page_down();
        assert_eq!(
            state.selected, 2,
            "page down jumps by a screenful minus overlap"
        );

        state.page_up();
        assert_eq!(
            state.selected, 0,
            "page up jumps back by a screenful minus overlap"
        );
    }

    #[test]
    fn select_parent_moves_to_enclosing_dir() {
        let mut state = FileTreeState::new();
        state.entries = make_test_entries();

        // lib.rs (index 2, depth 1) → parent is src (index 0, depth 0).
        state.selected = 2;
        state.select_parent();
        assert_eq!(state.selected, 0);

        // A top-level entry has no parent: selection stays put.
        state.selected = 4;
        state.select_parent();
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn nav_methods_safe_on_empty() {
        let mut state = FileTreeState::new();
        assert!(state.entries.is_empty());

        state.select_first();
        state.select_last();
        state.page_down();
        state.page_up();
        assert_eq!(state.selected, 0, "navigation on empty tree stays at 0");
    }
}
