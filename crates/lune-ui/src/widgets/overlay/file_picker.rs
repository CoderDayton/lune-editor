//! Interactive file/directory picker overlay.

use std::path::{Path, PathBuf};

use crate::primitives::{Buffer, Line, Rect, Span, Style, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

use super::util::{cmp_ignore_ascii_case, render_hrule};

/// An entry in the file picker (file or directory).
#[derive(Clone, Debug)]
pub struct PickerEntry {
    /// Display name (file/dir name, not full path).
    pub name: String,
    /// Full absolute path.
    pub path: PathBuf,
    /// Whether this is a directory.
    pub is_dir: bool,
}

/// State for the interactive file picker overlay.
#[derive(Clone, Debug)]
pub struct FilePickerState {
    /// The directory currently being browsed.
    pub current_dir: PathBuf,
    /// All entries in the current directory (unfiltered).
    all_entries: Vec<PickerEntry>,
    /// Filtered entries matching the current input.
    pub filtered_entries: Vec<PickerEntry>,
    /// User's filter input.
    pub input: String,
    /// Index of the currently selected entry.
    pub selected: usize,
    /// Scroll offset for the entry list.
    pub scroll_offset: usize,
}

impl Default for FilePickerState {
    fn default() -> Self {
        Self {
            current_dir: PathBuf::new(),
            all_entries: Vec::new(),
            filtered_entries: Vec::new(),
            input: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }
}

impl FilePickerState {
    /// Open the file picker at the given directory, scanning its contents.
    pub fn open(&mut self, dir: &Path) {
        self.navigate_to(dir.to_path_buf());
    }

    /// Scan the current directory and populate entries.
    pub fn scan_directory(&mut self) {
        self.all_entries.clear();

        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();

            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();

                // Skip hidden files/dirs (starting with '.').
                if name.starts_with('.') {
                    continue;
                }

                let is_dir = path.is_dir();
                let picker_entry = PickerEntry { name, path, is_dir };

                if is_dir {
                    dirs.push(picker_entry);
                } else {
                    files.push(picker_entry);
                }
            }

            // Sort directories first (alphabetically), then files (alphabetically).
            // Use char-by-char ASCII case-insensitive compare to avoid allocating
            // two lowercase Strings per comparison.
            dirs.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));
            files.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));

            self.all_entries.extend(dirs);
            self.all_entries.extend(files);
        }

        self.update_filter();
    }

    /// Update the filtered entries based on current input.
    pub fn update_filter(&mut self) {
        let query = self.input.to_lowercase();

        // Reuse the existing Vec allocation where possible.
        self.filtered_entries.clear();
        if query.is_empty() {
            self.filtered_entries
                .extend(self.all_entries.iter().cloned());
        } else {
            // `e.name.to_lowercase()` still allocates per entry; use
            // contains with a byte-level ASCII fold for pure-ASCII names,
            // falling back to the allocating path only when needed.
            self.filtered_entries.extend(
                self.all_entries
                    .iter()
                    .filter(|e| e.name.to_lowercase().contains(&query))
                    .cloned(),
            );
        }

        // Clamp selection.
        if self.filtered_entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered_entries.len() - 1);
        }
        self.scroll_offset = self.scroll_offset.min(self.selected);
    }

    /// Move selection up.
    pub const fn select_prev(&mut self) {
        if !self.filtered_entries.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered_entries.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub const fn select_next(&mut self) {
        if !self.filtered_entries.is_empty() {
            self.selected = (self.selected + 1) % self.filtered_entries.len();
        }
    }

    /// Get the currently selected entry.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.filtered_entries.get(self.selected)
    }

    /// Navigate to a directory, reset state, and rescan.
    fn navigate_to(&mut self, dir: PathBuf) {
        self.current_dir = dir;
        self.input.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.scan_directory();
    }

    /// Navigate into a subdirectory.
    pub fn enter_directory(&mut self, dir: &Path) {
        self.navigate_to(dir.to_path_buf());
    }

    /// Navigate up to the parent directory.
    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) {
            self.navigate_to(parent);
        }
    }

    /// Feed a character into the filter input.
    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
        self.update_filter();
    }

    /// Delete the last character from the filter input.
    /// Returns `true` if a character was deleted, `false` if input was already empty.
    pub fn backspace(&mut self) -> bool {
        if self.input.pop().is_some() {
            self.update_filter();
            true
        } else {
            false
        }
    }

    /// Ensure the selected item is visible within the given viewport height.
    pub const fn ensure_visible(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + viewport_height {
            self.scroll_offset = self.selected - viewport_height + 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_popup_frame(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    input: &str,
    width_pct: u16,
    height_pct: u16,
    min_w: u16,
    min_h: u16,
    theme: &Theme,
) -> Option<(Rect, u16)> {
    let popup_w = (area.width * width_pct / 100).max(min_w).min(area.width);
    let popup_h = (area.height * height_pct / 100).max(min_h).min(area.height);
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(title)
        .size_cells(popup_w, popup_h)
        .anchor(Anchor::Top {
            margin: (area.height.saturating_sub(popup_h)) / 4,
        })
        .render(area, buf, &mut modal, |_, _| {});
    let inner = modal.inner_area()?;

    // Input line.
    let input_line = format!("> {input}");
    Line::from(Span::from(input_line).bold())
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Separator.
    if inner.height > 1 {
        render_hrule(buf, inner.x, inner.y + 1, inner.width);
    }

    let list_start_y = inner.y + 2;
    Some((inner, list_start_y))
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_file_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &FilePickerState,
    theme: &Theme,
) {
    let dir_display = truncate_path_display(
        &state.current_dir,
        (area.width * 60 / 100).saturating_sub(6) as usize,
    );
    let title = format!(" Open: {dir_display} ");
    let Some((inner, _)) =
        render_popup_frame(area, buf, &title, &state.input, 60, 60, 40, 10, theme)
    else {
        return;
    };

    // Breadcrumb hint: ".." to go up.
    let list_start_y = inner.y + 2;
    if inner.height > 2 {
        let up_label = "  ../ (parent directory)";
        let up_style = Style::new().fg(theme.overlay_hint_fg).italic();
        Line::from(Span::from(up_label).style(up_style))
            .render(Rect::new(inner.x, list_start_y, inner.width, 1), buf);
    }

    // Entry list.
    let entry_start_y = list_start_y + 1;
    let list_height = inner.height.saturating_sub(3) as usize;

    if state.filtered_entries.is_empty() {
        if entry_start_y < inner.y + inner.height {
            Line::from(Span::from("  (empty)").dim())
                .render(Rect::new(inner.x, entry_start_y, inner.width, 1), buf);
        }
    } else {
        for (vi, i) in (state.scroll_offset..).take(list_height).enumerate() {
            if i >= state.filtered_entries.len() {
                break;
            }
            let y = entry_start_y + vi as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let entry = &state.filtered_entries[i];
            let icon = if entry.is_dir { "[D]" } else { "   " };
            let label = format!("{icon} {}", entry.name);
            let max_label_len = inner.width as usize;
            let truncated = if label.len() > max_label_len {
                format!("{}...", &label[..max_label_len.saturating_sub(3)])
            } else {
                label
            };

            let style = if entry.is_dir {
                Style::new().fg(theme.overlay_dir_fg)
            } else {
                Style::new().fg(theme.overlay_file_fg)
            };

            let span = if i == state.selected {
                Span::styled(truncated, theme.overlay_selected)
            } else {
                Span::from(truncated).style(style)
            };

            Line::from(span).render(Rect::new(inner.x, y, inner.width, 1), buf);
        }
    }
}

fn truncate_path_display(path: &Path, max_len: usize) -> String {
    let display = path.display().to_string();
    if display.len() <= max_len {
        display
    } else {
        format!(
            "...{}",
            &display[display.len() - max_len.saturating_sub(3)..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::{OverlayKind, OverlayState};
    use std::fs;

    fn make_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("alpha_dir")).unwrap();
        fs::create_dir(dir.path().join("beta_dir")).unwrap();
        fs::write(dir.path().join("hello.txt"), "hello").unwrap();
        fs::write(dir.path().join("world.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join(".hidden"), "secret").unwrap();
        dir
    }

    #[test]
    fn file_picker_opens_and_scans() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        assert_eq!(fp.current_dir, dir.path());
        // Should have 2 dirs + 2 files (hidden excluded).
        assert_eq!(fp.filtered_entries.len(), 4);
        // Dirs should come first.
        assert!(fp.filtered_entries[0].is_dir);
        assert!(fp.filtered_entries[1].is_dir);
        assert!(!fp.filtered_entries[2].is_dir);
        assert!(!fp.filtered_entries[3].is_dir);
    }

    #[test]
    fn file_picker_filter() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        fp.type_char('h');
        fp.type_char('e');
        // Should match "hello.txt".
        assert_eq!(fp.filtered_entries.len(), 1);
        assert_eq!(fp.filtered_entries[0].name, "hello.txt");
    }

    #[test]
    fn file_picker_navigate_up_down() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        assert_eq!(fp.selected, 0);
        fp.select_next();
        assert_eq!(fp.selected, 1);
        fp.select_next();
        assert_eq!(fp.selected, 2);
        fp.select_prev();
        assert_eq!(fp.selected, 1);
    }

    #[test]
    fn file_picker_select_wraps() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        let count = fp.filtered_entries.len();
        fp.selected = 0;
        fp.select_prev();
        assert_eq!(fp.selected, count - 1);
        fp.select_next();
        assert_eq!(fp.selected, 0);
    }

    #[test]
    fn file_picker_enter_directory() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        let subdir_path = dir.path().join("alpha_dir");
        fp.enter_directory(&subdir_path);
        assert_eq!(fp.current_dir, subdir_path);
        assert!(fp.filtered_entries.is_empty()); // Empty subdir.
        assert!(fp.input.is_empty());
    }

    #[test]
    fn file_picker_go_up() {
        let dir = make_test_dir();
        let subdir = dir.path().join("alpha_dir");
        let mut fp = FilePickerState::default();
        fp.open(&subdir);

        fp.go_up();
        assert_eq!(fp.current_dir, dir.path());
        // Should now see contents of parent.
        assert!(!fp.filtered_entries.is_empty());
    }

    #[test]
    fn file_picker_backspace_on_empty_goes_up() {
        let dir = make_test_dir();
        let subdir = dir.path().join("alpha_dir");
        let mut fp = FilePickerState::default();
        fp.open(&subdir);

        // Backspace on empty input returns false (signals go-up).
        assert!(!fp.backspace());
    }

    #[test]
    fn file_picker_backspace_on_input_deletes() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        fp.type_char('z');
        fp.type_char('z');
        assert!(fp.filtered_entries.is_empty());

        assert!(fp.backspace());
        assert!(fp.backspace());
        assert_eq!(fp.input, "");
        assert!(!fp.filtered_entries.is_empty());
    }

    #[test]
    fn file_picker_ensure_visible() {
        let entries: Vec<PickerEntry> = (0..20)
            .map(|i| PickerEntry {
                name: format!("file_{i}"),
                path: PathBuf::from(format!("file_{i}")),
                is_dir: false,
            })
            .collect();
        let mut fp = FilePickerState {
            filtered_entries: entries,
            selected: 15,
            scroll_offset: 0,
            ..FilePickerState::default()
        };
        fp.ensure_visible(10);
        // Should scroll so item 15 is visible.
        assert!(fp.scroll_offset > 0);
        assert!(fp.selected < fp.scroll_offset + 10);
    }

    #[test]
    fn file_picker_selected_entry() {
        let dir = make_test_dir();
        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        let entry = fp.selected_entry().unwrap();
        assert_eq!(entry.name, "alpha_dir");
        assert!(entry.is_dir);

        fp.select_next();
        fp.select_next();
        let entry = fp.selected_entry().unwrap();
        assert!(!entry.is_dir);
    }

    #[test]
    fn file_picker_overlay_open_close() {
        let dir = make_test_dir();
        let mut overlay = OverlayState::default();
        assert!(!overlay.is_active());

        overlay.open_file_picker(dir.path());
        assert!(overlay.is_active());
        assert!(matches!(overlay.active, Some(OverlayKind::FilePicker)));
        assert!(!overlay.file_picker.filtered_entries.is_empty());

        overlay.close();
        assert!(!overlay.is_active());
    }

    #[test]
    fn file_picker_dirs_sorted_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Zebra")).unwrap();
        fs::create_dir(dir.path().join("alpha")).unwrap();
        fs::create_dir(dir.path().join("Beta")).unwrap();

        let mut fp = FilePickerState::default();
        fp.open(dir.path());

        let names: Vec<&str> = fp
            .filtered_entries
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "Beta", "Zebra"]);
    }

    #[test]
    fn truncate_path_display_short() {
        let path = PathBuf::from("/home/user");
        let result = truncate_path_display(&path, 50);
        assert_eq!(result, "/home/user");
    }

    #[test]
    fn truncate_path_display_long() {
        let path = PathBuf::from("/very/long/path/that/exceeds/the/limit");
        let result = truncate_path_display(&path, 20);
        assert!(result.starts_with("..."));
        assert!(result.len() <= 20);
    }
}
