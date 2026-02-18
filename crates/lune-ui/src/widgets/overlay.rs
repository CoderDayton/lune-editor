//! Overlay system (command palette, find/replace, file picker, notifications).
//!
//! Overlays render on top of the main layout and capture keyboard input
//! when active. The command palette provides fuzzy-filtered command search.
//! The file picker provides interactive directory browsing.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::widgets::{Block, BorderType, Borders, Clear};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Style, Stylize};
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;

use crate::event::AppCommand;
use crate::theme::Theme;
use lune_core::language::lang;

// ── Overlay kinds ─────────────────────────────────────────────────────

/// The type of overlay currently displayed.
#[derive(Clone, Debug)]
pub enum OverlayKind {
    /// Command palette with fuzzy search.
    CommandPalette,
    /// Find/replace dialog.
    FindReplace,
    /// Confirm dialog (destructive actions).
    ConfirmDialog {
        /// The message to display.
        message: String,
        /// The command to execute on confirmation.
        on_confirm: AppCommand,
    },
    /// Interactive file/directory picker.
    FilePicker,
}

// ── Overlay state ─────────────────────────────────────────────────────

/// Top-level overlay state.
#[derive(Clone, Debug, Default)]
pub struct OverlayState {
    /// The currently active overlay, if any.
    pub active: Option<OverlayKind>,
    /// Command palette state (persisted across open/close for history).
    pub command_palette: CommandPaletteState,
    /// File picker state (persisted across open/close).
    pub file_picker: FilePickerState,
    /// Active notifications (toast messages).
    pub notifications: Vec<Notification>,
}

impl OverlayState {
    /// Whether any overlay is capturing input.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Open the command palette.
    pub fn open_command_palette(&mut self) {
        self.command_palette.input.clear();
        self.command_palette.selected = 0;
        self.command_palette.scroll_offset = 0;
        self.command_palette.ensure_commands_cached();
        // Reuse allocation; input was just cleared so update_filter will copy all commands.
        self.command_palette.update_filter();
        self.active = Some(OverlayKind::CommandPalette);
    }

    /// Open the file picker at the given directory.
    pub fn open_file_picker(&mut self, start_dir: &Path) {
        self.file_picker.open(start_dir);
        self.active = Some(OverlayKind::FilePicker);
    }

    /// Close whatever overlay is open.
    pub fn close(&mut self) {
        self.active = None;
    }

    /// Open a confirmation dialog.
    pub fn open_confirm(&mut self, message: impl Into<String>, on_confirm: AppCommand) {
        self.active = Some(OverlayKind::ConfirmDialog {
            message: message.into(),
            on_confirm,
        });
    }

    /// Push a notification toast.
    pub fn notify(&mut self, message: impl Into<String>, level: NotificationLevel) {
        self.notifications.push(Notification {
            message: message.into(),
            level,
            created: Instant::now(),
        });
    }

    /// Remove expired notifications (older than 4 seconds).
    pub fn prune_notifications(&mut self) {
        let now = Instant::now();
        self.notifications
            .retain(|n| now.duration_since(n.created).as_secs() < 4);
    }
}

// ── Command palette ───────────────────────────────────────────────────

/// A command that can appear in the command palette.
#[derive(Clone, Debug)]
pub struct PaletteCommand {
    /// Display name.
    pub label: String,
    /// Pre-computed lowercase label for filtering.
    label_lower: String,
    /// The command to execute.
    pub command: AppCommand,
}

/// State for the command palette overlay.
#[derive(Clone, Debug, Default)]
pub struct CommandPaletteState {
    /// User's search input.
    pub input: String,
    /// Index of the currently selected command.
    pub selected: usize,
    /// Scroll offset for the visible list window.
    pub scroll_offset: usize,
    /// Filtered list of commands matching the input.
    pub filtered_commands: Vec<PaletteCommand>,
    /// Cached full command list (built once, reused across filter calls).
    all_commands: Vec<PaletteCommand>,
}

impl CommandPaletteState {
    /// Ensure the cached command list is populated.
    fn ensure_commands_cached(&mut self) {
        if self.all_commands.is_empty() {
            self.all_commands = all_palette_commands();
        }
    }

    /// Update the filtered command list based on current input.
    pub fn update_filter(&mut self) {
        self.ensure_commands_cached();
        let query = self.input.to_lowercase();

        // Reuse the existing Vec allocation where possible.
        self.filtered_commands.clear();
        if query.is_empty() {
            self.filtered_commands
                .extend(self.all_commands.iter().cloned());
        } else {
            self.filtered_commands.extend(
                self.all_commands
                    .iter()
                    .filter(|cmd| cmd.label_lower.contains(&query))
                    .cloned(),
            );
        }

        // Clamp selection.
        if self.filtered_commands.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered_commands.len() - 1);
        }
        self.scroll_offset = self.scroll_offset.min(self.selected);
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if !self.filtered_commands.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered_commands.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.filtered_commands.is_empty() {
            self.selected = (self.selected + 1) % self.filtered_commands.len();
        }
    }

    /// Adjust `scroll_offset` so `selected` is visible within `visible_rows`.
    pub const fn ensure_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected + 1 - visible_rows;
        }
    }

    /// Get the currently selected command.
    #[must_use]
    pub fn selected_command(&self) -> Option<&AppCommand> {
        self.filtered_commands
            .get(self.selected)
            .map(|c| &c.command)
    }

    /// Feed a character into the input.
    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
        self.update_filter();
    }

    /// Delete the last character from the input.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

/// Zero-allocation ASCII case-insensitive string comparison.
///
/// Avoids the two `to_lowercase()` allocations that a naïve comparator would
/// incur on every call during a sort.
fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().map(|c| c.to_ascii_lowercase());
    let mut bi = b.chars().map(|c| c.to_ascii_lowercase());
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) => match ac.cmp(&bc) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            },
        }
    }
}

/// Helper to build a `PaletteCommand` with pre-computed lowercase label.
fn palette_cmd(label: &str, command: AppCommand) -> PaletteCommand {
    PaletteCommand {
        label_lower: label.to_lowercase(),
        label: label.to_string(),
        command,
    }
}

/// Build the full list of palette commands.
fn all_palette_commands() -> Vec<PaletteCommand> {
    let mut cmds = vec![
        palette_cmd("Save", AppCommand::Save),
        palette_cmd("Save All", AppCommand::SaveAll),
        palette_cmd("Open File", AppCommand::OpenFilePicker),
        palette_cmd("Close Tab", AppCommand::CloseTab),
        palette_cmd("Next Tab", AppCommand::NextTab),
        palette_cmd("Previous Tab", AppCommand::PrevTab),
        palette_cmd("Toggle File Tree", AppCommand::ToggleFileTree),
        palette_cmd("Toggle AI Panel", AppCommand::ToggleAiPanel),
        palette_cmd("Toggle Git Panel", AppCommand::ToggleGitPanel),
        palette_cmd("Undo", AppCommand::Undo),
        palette_cmd("Redo", AppCommand::Redo),
        palette_cmd("Find", AppCommand::Find),
        palette_cmd("Find and Replace", AppCommand::Replace),
        palette_cmd("Quit", AppCommand::Quit),
    ];

    // Language change commands.
    let languages = [
        ("Rust", lang::RUST),
        ("Python", lang::PYTHON),
        ("JavaScript", lang::JAVASCRIPT),
        ("TypeScript", lang::TYPESCRIPT),
        ("TSX", lang::TSX),
        ("JSON", lang::JSON),
        ("TOML", lang::TOML),
        ("YAML", lang::YAML),
        ("Markdown", lang::MARKDOWN),
        ("C", lang::C),
        ("C++", lang::CPP),
        ("Go", lang::GO),
        ("HTML", lang::HTML),
        ("CSS", lang::CSS),
        ("Shell", lang::SHELL),
        ("Plain Text", lang::PLAIN_TEXT),
    ];

    for (name, lid) in languages {
        let label = format!("Change Language: {name}");
        cmds.push(palette_cmd(&label, AppCommand::ChangeLanguage(lid)));
    }

    cmds.sort_by(|a, b| a.label_lower.cmp(&b.label_lower));
    cmds
}

// ── Notifications ─────────────────────────────────────────────────────

// ── File picker ───────────────────────────────────────────────────────

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
    pub fn select_prev(&mut self) {
        if !self.filtered_entries.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered_entries.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
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

// ── Notifications ─────────────────────────────────────────────────────

/// Severity level for notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationLevel {
    /// Informational message.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// A toast notification message.
#[derive(Clone, Debug)]
pub struct Notification {
    /// The message text.
    pub message: String,
    /// Severity level.
    pub level: NotificationLevel,
    /// When the notification was created.
    pub created: Instant,
}

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the active overlay on top of the main layout.
#[allow(clippy::cast_possible_truncation)]
pub fn render_overlay(area: Rect, buf: &mut Buffer, overlay: &mut OverlayState, theme: &Theme) {
    // Render notifications (bottom-right toasts).
    render_notifications(area, buf, &overlay.notifications, theme);

    // Render the active overlay.
    match &overlay.active {
        Some(OverlayKind::CommandPalette) => {
            render_command_palette(area, buf, &mut overlay.command_palette, theme);
        }
        Some(OverlayKind::FindReplace) => {
            // Placeholder — will be implemented with search integration.
            render_centered_popup(area, buf, "Find & Replace", &["(Coming soon)"], theme);
        }
        Some(OverlayKind::ConfirmDialog { message, .. }) => {
            render_centered_popup(
                area,
                buf,
                "Confirm",
                &[message, "", "Press Enter to confirm, Esc to cancel"],
                theme,
            );
        }
        Some(OverlayKind::FilePicker) => {
            render_file_picker(area, buf, &overlay.file_picker, theme);
        }
        None => {}
    }
}

/// Render a popup frame with a title, input line, and separator. Returns the inner
/// area below the separator, or `None` if the popup is too small.
#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
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
    let popup_x = area.x + (area.width - popup_w) / 2;
    let popup_y = area.y + (area.height - popup_h) / 4;

    let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
    Clear.render(popup_rect, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(popup_rect);
    block.render(popup_rect, buf);

    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    // Input line.
    let input_line = format!("> {input}");
    Line::from(Span::from(input_line).bold())
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Separator.
    if inner.height > 1 {
        let sep = "─".repeat(inner.width as usize);
        Line::from(Span::from(sep).dim())
            .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }

    let list_start_y = inner.y + 2;
    Some((inner, list_start_y))
}

/// Render the command palette popup.
#[allow(clippy::cast_possible_truncation)]
fn render_command_palette(
    area: Rect,
    buf: &mut Buffer,
    state: &mut CommandPaletteState,
    theme: &Theme,
) {
    let Some((inner, list_start_y)) = render_popup_frame(
        area,
        buf,
        " Command Palette ",
        &state.input,
        60,
        40,
        30,
        8,
        theme,
    ) else {
        return;
    };

    let list_height = inner.height.saturating_sub(2) as usize;
    state.ensure_visible(list_height);

    for (vi, i) in (state.scroll_offset..).take(list_height).enumerate() {
        if i >= state.filtered_commands.len() {
            break;
        }
        let y = list_start_y + vi as u16;
        if y >= inner.y + inner.height {
            break;
        }

        let cmd = &state.filtered_commands[i];
        let label = format!("  {}", cmd.label);
        let span = if i == state.selected {
            Span::styled(label, theme.overlay_selected)
        } else {
            Span::from(label)
        };
        Line::from(span).render(Rect::new(inner.x, y, inner.width, 1), buf);
    }
}

/// Render the file picker popup.
#[allow(clippy::cast_possible_truncation)]
fn render_file_picker(area: Rect, buf: &mut Buffer, state: &FilePickerState, theme: &Theme) {
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

/// Truncate a path display string to fit within `max_len` characters.
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

/// Render a generic centered popup with a title and message lines.
#[allow(clippy::cast_possible_truncation)]
fn render_centered_popup(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    messages: &[&str],
    theme: &Theme,
) {
    let popup_w = (area.width * 50 / 100).max(20).min(area.width);
    let popup_h = (messages.len() as u16 + 4).min(area.height);
    let popup_x = area.x + (area.width - popup_w) / 2;
    let popup_y = area.y + (area.height - popup_h) / 2;

    let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
    Clear.render(popup_rect, buf);

    let title_str = format!(" {title} ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title_str)
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(popup_rect);
    block.render(popup_rect, buf);

    for (i, msg) in messages.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.y + inner.height {
            break;
        }
        Line::from(Span::from(*msg).dim()).render(
            Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
            buf,
        );
    }
}

/// Render toast notifications in the bottom-right corner.
#[allow(clippy::cast_possible_truncation)]
fn render_notifications(
    area: Rect,
    buf: &mut Buffer,
    notifications: &[Notification],
    theme: &Theme,
) {
    if notifications.is_empty() {
        return;
    }

    let max_width: u16 = 40;
    let mut y = area.y + area.height;

    for notif in notifications.iter().rev().take(5) {
        if y < area.y + 2 {
            break;
        }

        let msg_width = (notif.message.len() as u16 + 4).min(max_width);
        let x = area.x + area.width - msg_width;
        y = y.saturating_sub(2);

        let rect = Rect::new(x, y, msg_width, 1);
        Clear.render(rect, buf);

        let style = match notif.level {
            NotificationLevel::Info => Style::new().fg(theme.notif_info),
            NotificationLevel::Warning => Style::new().fg(theme.notif_warn),
            NotificationLevel::Error => Style::new().fg(theme.notif_error),
        };

        let text = format!(
            " {} ",
            &notif.message[..notif.message.len().min((msg_width - 2) as usize)]
        );
        Line::from(Span::from(text).style(style)).render(rect, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_palette() -> CommandPaletteState {
        let all = all_palette_commands();
        CommandPaletteState {
            filtered_commands: all.clone(),
            all_commands: all,
            ..Default::default()
        }
    }

    #[test]
    fn palette_filter_empty() {
        let mut cp = make_palette();
        cp.update_filter();
        assert!(!cp.filtered_commands.is_empty());
    }

    #[test]
    fn palette_filter_narrows() {
        let mut cp = make_palette();
        cp.type_char('s');
        cp.type_char('a');
        cp.type_char('v');
        // Should match "Save" and "Save All".
        assert!(cp.filtered_commands.len() >= 2);
        assert!(cp
            .filtered_commands
            .iter()
            .all(|c| c.label.to_lowercase().contains("sav")));
    }

    #[test]
    fn palette_select_wrap() {
        let mut cp = make_palette();
        let count = cp.filtered_commands.len();
        cp.selected = 0;
        cp.select_prev();
        assert_eq!(cp.selected, count - 1);
        cp.select_next();
        assert_eq!(cp.selected, 0);
    }

    #[test]
    fn palette_backspace() {
        let mut cp = make_palette();
        cp.type_char('x');
        cp.type_char('y');
        cp.type_char('z');
        assert!(cp.filtered_commands.is_empty());
        cp.backspace();
        cp.backspace();
        cp.backspace();
        assert!(!cp.filtered_commands.is_empty());
    }

    #[test]
    fn notification_prune() {
        let mut overlay = OverlayState::default();
        overlay.notify("test", NotificationLevel::Info);
        assert_eq!(overlay.notifications.len(), 1);
        overlay.prune_notifications();
        // Should still be there (just created).
        assert_eq!(overlay.notifications.len(), 1);
    }

    #[test]
    fn overlay_open_close() {
        let mut overlay = OverlayState::default();
        assert!(!overlay.is_active());
        overlay.open_command_palette();
        assert!(overlay.is_active());
        overlay.close();
        assert!(!overlay.is_active());
    }

    // ── File picker tests ─────────────────────────────────────────

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

    #[test]
    fn palette_includes_open_file_command() {
        let cmds = all_palette_commands();
        assert!(cmds.iter().any(|c| c.label == "Open File"));
    }
}
