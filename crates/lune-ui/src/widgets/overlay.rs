//! Overlay system (command palette, find/replace, file picker, notifications).
//!
//! Overlays render on top of the main layout and capture keyboard input
//! when active. The command palette provides fuzzy-filtered command search.
//! The file picker provides interactive directory browsing.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::primitives::{
    Block, BorderType, Borders, Buffer, Clear, Line, Rect, Span, Style, Stylize, Widget,
};

use crate::effects::blend_toward;
use crate::event::AppCommand;
use crate::primitives::Color;
use crate::theme::Theme;
use lune_ai::session::AiClientKind;
use lune_core::language::{lang, LanguageId};

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
    /// AI client picker (choose which client to launch).
    AiClientPicker,
    /// Inline text input dialog (new file, rename, etc.).
    InputDialog,
    /// Language selector (fuzzy-filtered list of all known languages).
    LanguagePicker,
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
    /// AI client picker state.
    pub ai_client_picker: AiClientPickerState,
    /// Input dialog state (for file operations).
    pub input_dialog: Option<InputDialogState>,
    /// Find/replace bar state.
    pub find_replace: FindReplaceState,
    /// Language picker state.
    pub language_picker: LanguagePickerState,
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

    /// Open the AI client picker overlay.
    ///
    /// Re-scans PATH for available clients each time it opens.
    pub fn open_ai_client_picker(&mut self) {
        self.ai_client_picker = AiClientPickerState::scan_available();
        self.active = Some(OverlayKind::AiClientPicker);
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

    /// Open an input dialog.
    pub fn open_input_dialog(&mut self, state: InputDialogState) {
        self.input_dialog = Some(state);
        self.active = Some(OverlayKind::InputDialog);
    }

    /// Open find bar (no replace row).
    pub fn open_find(&mut self) {
        self.find_replace.show_replace = false;
        self.find_replace.active_field = FindReplaceField::Find;
        self.active = Some(OverlayKind::FindReplace);
    }

    /// Open find and replace bar.
    pub fn open_find_replace(&mut self) {
        self.find_replace.show_replace = true;
        self.find_replace.active_field = FindReplaceField::Find;
        self.active = Some(OverlayKind::FindReplace);
    }

    /// Open the language picker loaded with the given language list.
    pub fn open_language_picker(&mut self, languages: Vec<LanguageId>) {
        self.language_picker = LanguagePickerState::new(languages);
        self.active = Some(OverlayKind::LanguagePicker);
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
        palette_cmd("Show Editor", AppCommand::ShowEditorTab),
        palette_cmd("Show Agents", AppCommand::ShowAgentsTab),
        palette_cmd("Toggle File Tree", AppCommand::ToggleFileTree),
        palette_cmd("Close AI Session", AppCommand::AiCloseSession),
        palette_cmd("Next AI Session", AppCommand::AiNextSession),
        palette_cmd("Previous AI Session", AppCommand::AiPrevSession),
        palette_cmd("Toggle Git Panel", AppCommand::ToggleGitPanel),
        palette_cmd("Stage Hunk", AppCommand::GitStageHunk),
        palette_cmd("Unstage Hunk", AppCommand::GitUnstageHunk),
        palette_cmd("Discard Hunk", AppCommand::GitDiscardHunk),
        palette_cmd("Undo", AppCommand::Undo),
        palette_cmd("Redo", AppCommand::Redo),
        palette_cmd("Find", AppCommand::Find),
        palette_cmd("Find and Replace", AppCommand::Replace),
        palette_cmd("Quit", AppCommand::Quit),
        palette_cmd("Select Language", AppCommand::OpenLanguagePicker),
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

// ── AI client picker ──────────────────────────────────────────────────

/// Metadata for a known AI CLI client.
struct KnownClient {
    label: &'static str,
    command: &'static str,
    /// Terminal color used for the colored bullet.
    color: Color,
}

/// The full catalog of known AI CLI clients.
const KNOWN_CLIENTS: &[KnownClient] = &[
    KnownClient {
        label: "Claude Code",
        command: "claude",
        color: Color::Rgb(215, 150, 60), // amber
    },
    KnownClient {
        label: "OpenCode",
        command: "opencode",
        color: Color::Rgb(80, 180, 255), // sky blue
    },
    KnownClient {
        label: "Gemini",
        command: "gemini",
        color: Color::Rgb(66, 200, 140), // teal-green
    },
    KnownClient {
        label: "Kilo Code",
        command: "kilo",
        color: Color::Rgb(255, 100, 100), // coral red
    },
    KnownClient {
        label: "Cline",
        command: "cline",
        color: Color::Rgb(160, 110, 255), // violet
    },
    KnownClient {
        label: "Qwen Code",
        command: "qwen",
        color: Color::Rgb(60, 210, 200), // cyan
    },
];

/// Returns `true` if `cmd` is found as an executable file on `$PATH`.
fn is_command_available(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path_var| std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file()))
}

/// An entry in the AI client picker (only installed clients appear).
#[derive(Clone, Debug)]
pub struct AiClientEntry {
    /// Display name.
    pub label: String,
    /// CLI command.
    pub command: String,
    /// Accent color for the colored bullet.
    pub color: Color,
    /// The client kind to spawn.
    pub kind: AiClientKind,
}

/// State for the AI client picker overlay.
#[derive(Clone, Debug, Default)]
pub struct AiClientPickerState {
    /// Installed clients found on PATH.
    pub entries: Vec<AiClientEntry>,
    /// Currently highlighted index.
    pub selected: usize,
}

impl AiClientPickerState {
    /// Scan PATH for available clients and return a ready state.
    ///
    /// Always appends a "System Shell" entry at the end.
    #[must_use]
    pub fn scan_available() -> Self {
        let mut entries: Vec<AiClientEntry> = KNOWN_CLIENTS
            .iter()
            .filter(|c| is_command_available(c.command))
            .map(|c| AiClientEntry {
                label: c.label.to_string(),
                command: c.command.to_string(),
                color: c.color,
                kind: if c.command == "claude" {
                    AiClientKind::ClaudeCode
                } else {
                    AiClientKind::Custom {
                        name: c.label.to_string(),
                        command: c.command.to_string(),
                    }
                },
            })
            .collect();

        // Always append a system shell entry.
        let shell_cmd = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        entries.push(AiClientEntry {
            label: "System Shell".to_string(),
            command: shell_cmd,
            color: Color::Rgb(120, 200, 120),
            kind: AiClientKind::Shell,
        });

        Self {
            entries,
            selected: 0,
        }
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if !self.entries.is_empty() {
            self.selected = if self.selected == 0 {
                self.entries.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1) % self.entries.len();
        }
    }

    /// Get the client kind for the currently selected entry.
    #[must_use]
    pub fn selected_kind(&self) -> Option<AiClientKind> {
        self.entries.get(self.selected).map(|e| e.kind.clone())
    }
}

// ── Language picker ───────────────────────────────────────────────────

/// State for the language selector overlay.
#[derive(Clone, Debug, Default)]
pub struct LanguagePickerState {
    /// All available language IDs (sorted alphabetically, deduplicated).
    pub all_languages: Vec<LanguageId>,
    /// Currently displayed (filtered) subset.
    pub filtered: Vec<LanguageId>,
    /// Highlighted index into `filtered`.
    pub selected: usize,
    /// Filter input string.
    pub input: String,
    /// Scroll offset for the visible list window.
    pub scroll_offset: usize,
}

impl LanguagePickerState {
    /// Build from a list of language IDs (sorts and deduplicates).
    #[must_use]
    pub fn new(mut languages: Vec<LanguageId>) -> Self {
        languages.sort_by_key(|l| l.0);
        languages.dedup();
        let filtered = languages.clone();
        Self {
            all_languages: languages,
            filtered,
            selected: 0,
            input: String::new(),
            scroll_offset: 0,
        }
    }

    /// Re-filter `all_languages` by `input` (case-insensitive substring).
    pub fn update_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = if query.is_empty() {
            self.all_languages.clone()
        } else {
            self.all_languages
                .iter()
                .filter(|l| l.0.to_lowercase().contains(&query))
                .copied()
                .collect()
        };
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Move selection down (wraps).
    pub fn select_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.filtered.len();
        self.ensure_visible(10);
    }

    /// Move selection up (wraps).
    pub fn select_prev(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.filtered.len() - 1);
        self.ensure_visible(10);
    }

    /// Scroll so that `selected` is within a window of `list_height` rows.
    const fn ensure_visible(&mut self, list_height: usize) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + list_height {
            self.scroll_offset = self.selected + 1 - list_height;
        }
    }

    /// The currently selected language, if any.
    #[must_use]
    pub fn selected_lang(&self) -> Option<LanguageId> {
        self.filtered.get(self.selected).copied()
    }

    /// Append a character to the filter input.
    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.update_filter();
    }

    /// Remove the last character from the filter input.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

// ── Input dialog ──────────────────────────────────────────────────────

/// Action to perform when an input dialog is confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputDialogAction {
    /// Create a new file in the given parent directory.
    CreateFile { parent: PathBuf },
    /// Create a new directory in the given parent directory.
    CreateDir { parent: PathBuf },
    /// Rename an entry (from is the current path).
    Rename { from: PathBuf },
    /// Commit staged changes with the entered message.
    CommitMessage,
}

/// State for the inline input dialog overlay.
#[derive(Clone, Debug)]
pub struct InputDialogState {
    /// Dialog title (e.g. "New File", "Rename").
    pub title: String,
    /// Current input text.
    pub input: String,
    /// Cursor position within the input (byte offset).
    pub cursor_pos: usize,
    /// Hint text shown when input is empty.
    pub hint: String,
    /// The action to perform on confirm.
    pub action: InputDialogAction,
}

impl InputDialogState {
    /// Create a new input dialog state.
    pub fn new(title: impl Into<String>, hint: impl Into<String>, action: InputDialogAction) -> Self {
        Self {
            title: title.into(),
            input: String::new(),
            cursor_pos: 0,
            hint: hint.into(),
            action,
        }
    }

    /// Create with pre-filled input text (e.g. for rename).
    #[must_use]
    pub fn with_input(mut self, input: impl Into<String>) -> Self {
        self.input = input.into();
        self.cursor_pos = self.input.len();
        self
    }

    /// Type a character at the cursor position.
    pub fn type_char(&mut self, ch: char) {
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            // Find the previous character boundary.
            let prev = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            self.input.drain(prev..self.cursor_pos);
            self.cursor_pos = prev;
        }
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map_or(self.input.len(), |(i, _)| self.cursor_pos + i);
            self.input.drain(self.cursor_pos..next);
        }
    }

    /// Move cursor left by one character.
    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
        }
    }

    /// Move cursor right by one character.
    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map_or(self.input.len(), |(i, _)| self.cursor_pos + i);
        }
    }

    /// Move cursor to the start.
    pub const fn home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to the end.
    pub fn end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Validate the input. Returns an error message if invalid, None if OK.
    pub fn validate(&self) -> Option<&'static str> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return Some("Input cannot be empty");
        }
        // Path separator check only applies to file/dir operations.
        if !matches!(self.action, InputDialogAction::CommitMessage)
            && (trimmed.contains('/') || trimmed.contains('\\'))
        {
            return Some("Name cannot contain path separators");
        }
        None
    }
}

// ── Find/Replace ──────────────────────────────────────────────────────

/// Which field is active in the find/replace bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindReplaceField {
    #[default]
    Find,
    Replace,
}

/// State for the find/replace overlay bar.
#[derive(Clone, Debug, Default)]
pub struct FindReplaceState {
    /// Current find input.
    pub find_input: String,
    /// Current replace input.
    pub replace_input: String,
    /// Which field is active.
    pub active_field: FindReplaceField,
    /// Whether search is case-sensitive.
    pub case_sensitive: bool,
    /// Whether to show the replace row.
    pub show_replace: bool,
    /// Cached search results from the active buffer.
    pub search_state: lune_core::search::SearchState,
}

impl FindReplaceState {
    /// Type a character into the active field.
    pub fn type_char(&mut self, ch: char) {
        match self.active_field {
            FindReplaceField::Find => self.find_input.push(ch),
            FindReplaceField::Replace => self.replace_input.push(ch),
        }
    }

    /// Delete the last character from the active field.
    pub fn backspace(&mut self) {
        match self.active_field {
            FindReplaceField::Find => { self.find_input.pop(); }
            FindReplaceField::Replace => { self.replace_input.pop(); }
        }
    }

    /// Toggle between find and replace fields.
    pub const fn toggle_field(&mut self) {
        self.active_field = match self.active_field {
            FindReplaceField::Find => {
                if self.show_replace { FindReplaceField::Replace } else { FindReplaceField::Find }
            }
            FindReplaceField::Replace => FindReplaceField::Find,
        };
    }

    /// Toggle case sensitivity.
    pub const fn toggle_case(&mut self) {
        self.case_sensitive = !self.case_sensitive;
    }

    /// Format the match count display.
    #[must_use]
    pub fn match_display(&self) -> String {
        let count = self.search_state.match_count();
        if self.find_input.is_empty() {
            String::new()
        } else if count == 0 {
            "No results".to_string()
        } else if let Some(idx) = self.search_state.current_match {
            format!("{} of {count}", idx + 1)
        } else {
            format!("{count} results")
        }
    }
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

/// Notification TTL in seconds.
const NOTIFICATION_TTL_SECS: f32 = 4.0;
/// Seconds before expiry when fade-out begins.
const NOTIFICATION_FADE_SECS: f32 = 1.0;

impl Notification {
    /// Remaining vitality: 1.0 when fresh, fading to 0.0 during the
    /// final second before expiry.
    #[must_use]
    pub fn vitality(&self) -> f32 {
        let elapsed = self.created.elapsed().as_secs_f32();
        if elapsed >= NOTIFICATION_TTL_SECS {
            return 0.0;
        }
        let fade_start = NOTIFICATION_TTL_SECS - NOTIFICATION_FADE_SECS;
        if elapsed <= fade_start {
            1.0
        } else {
            (NOTIFICATION_TTL_SECS - elapsed) / NOTIFICATION_FADE_SECS
        }
    }
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
            render_find_replace(area, buf, &overlay.find_replace, theme);
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
        Some(OverlayKind::AiClientPicker) => {
            render_ai_client_picker(area, buf, &overlay.ai_client_picker, theme);
        }
        Some(OverlayKind::InputDialog) => {
            if let Some(ref dialog) = overlay.input_dialog {
                render_input_dialog(area, buf, dialog, theme);
            }
        }
        Some(OverlayKind::LanguagePicker) => {
            render_language_picker(area, buf, &overlay.language_picker, theme);
        }
        None => {}
    }
}

/// Render the AI client picker overlay.
#[allow(clippy::cast_possible_truncation)]
fn render_ai_client_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &AiClientPickerState,
    theme: &Theme,
) {
    let popup_w = (area.width * 50 / 100).max(40).min(area.width);
    let popup_h = (state.entries.len() as u16 + 6).min(area.height);
    let popup_x = area.x + (area.width - popup_w) / 2;
    let popup_y = area.y + (area.height - popup_h) / 3;

    let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
    Clear.render(popup_rect, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(" Open AI Session ")
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(popup_rect);
    block.render(popup_rect, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Subtitle
    Line::from(Span::from(" Choose a client to open:").dim())
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Separator
    if inner.height > 1 {
        let sep = "─".repeat(inner.width as usize);
        Line::from(Span::from(sep).dim())
            .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }

    // Entry list — or empty state
    if state.entries.is_empty() {
        let y = inner.y + 2;
        if y < inner.y + inner.height {
            Line::from(Span::from("  No AI clients found in PATH").dim())
                .render(Rect::new(inner.x, y, inner.width, 1), buf);
        }
    } else {
        for (i, entry) in state.entries.iter().enumerate() {
            let y = inner.y + 2 + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let label_text = format!(" {} ({})", entry.label, entry.command);
            let max_label = inner.width.saturating_sub(3) as usize;
            let label_text = if label_text.len() > max_label {
                format!("{}…", &label_text[..max_label.saturating_sub(1)])
            } else {
                label_text
            };

            if i == state.selected {
                // Selected: full row highlighted
                let full = format!(" ● {label_text}");
                Line::from(Span::styled(full, theme.overlay_selected))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            } else {
                // Unselected: colored bullet + plain label
                let bullet = Span::styled(" ● ", Style::new().fg(entry.color));
                let text = Span::from(label_text);
                Line::from(vec![bullet, text]).render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        }
    }

    // Footer hint
    let hint_y = inner.y + inner.height.saturating_sub(1);
    if hint_y > inner.y + 1 + state.entries.len() as u16 {
        Line::from(Span::from(" ↑↓ select · Enter open · Esc cancel").dim())
            .render(Rect::new(inner.x, hint_y, inner.width, 1), buf);
    }
}

/// Render the language picker popup.
#[allow(clippy::cast_possible_truncation)]
fn render_language_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &LanguagePickerState,
    theme: &Theme,
) {
    // Popup dimensions: 40% wide, tall enough for input + up to 12 items + footer.
    let popup_w = (area.width * 40 / 100).max(36).min(area.width);
    let list_rows = (state.filtered.len() as u16).min(12);
    let popup_h = (2 + 1 + list_rows + 1 + 2).min(area.height); // border*2 + input + sep + items + footer
    let popup_x = area.x + (area.width - popup_w) / 2;
    let popup_y = area.y + (area.height - popup_h) / 3;

    let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
    Clear.render(popup_rect, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(" Select Language ")
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(popup_rect);
    block.render(popup_rect, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Input row.
    let cursor = if state.input.is_empty() { "█" } else { "" };
    let input_str = format!(" > {}{}", state.input, cursor);
    Line::from(Span::from(input_str))
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    if inner.height <= 1 {
        return;
    }

    // Separator.
    let sep = "─".repeat(inner.width as usize);
    Line::from(Span::from(sep).dim())
        .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);

    if inner.height <= 2 {
        return;
    }

    let list_y_start = inner.y + 2;
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let list_y_end = footer_y;
    let visible_rows = list_y_end.saturating_sub(list_y_start) as usize;

    if state.filtered.is_empty() {
        Line::from(Span::from("  No matches").dim())
            .render(Rect::new(inner.x, list_y_start, inner.width, 1), buf);
    } else {
        for (row, (idx, lang)) in state
            .filtered
            .iter()
            .enumerate()
            .skip(state.scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let y = list_y_start + row as u16;
            if y >= list_y_end {
                break;
            }
            let label = format!("  {}", lang.name());
            let max_w = inner.width.saturating_sub(2) as usize;
            let label = if label.len() > max_w {
                format!("{}…", &label[..max_w.saturating_sub(1)])
            } else {
                label
            };
            if idx == state.selected {
                Line::from(Span::styled(label, theme.overlay_selected))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            } else {
                Line::from(Span::from(label))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        }
    }

    // Footer hint.
    if footer_y > list_y_start {
        Line::from(Span::from(" ↑↓ select · Enter confirm · Esc cancel").dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}

/// Render the inline input dialog popup.
#[allow(clippy::cast_possible_truncation)]
fn render_input_dialog(area: Rect, buf: &mut Buffer, state: &InputDialogState, theme: &Theme) {
    let popup_w = (area.width * 50 / 100).max(30).min(area.width);
    let popup_h: u16 = 5;
    let popup_x = area.x + (area.width - popup_w) / 2;
    let popup_y = area.y + (area.height - popup_h) / 3;

    let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
    Clear.render(popup_rect, buf);

    let title = format!(" {} ", state.title);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(title)
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(popup_rect);
    block.render(popup_rect, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Input line with block cursor.
    let display_text = if state.input.is_empty() {
        Span::from(state.hint.as_str()).dim()
    } else {
        Span::from(state.input.as_str())
    };

    let input_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1);
    Line::from(display_text).render(input_area, buf);

    // Draw block cursor.
    {
        let cursor_x = inner.x + 1 + state.input[..state.cursor_pos].chars().count() as u16;
        if cursor_x < inner.x + inner.width.saturating_sub(1) {
            let cursor_char = state.input[state.cursor_pos..]
                .chars()
                .next()
                .unwrap_or(' ');
            let cursor_span = Span::styled(
                cursor_char.to_string(),
                Style::new().fg(theme.bg).bg(theme.fg),
            );
            Line::from(cursor_span).render(Rect::new(cursor_x, inner.y, 1, 1), buf);
        }
    }

    // Validation error or hint.
    if inner.height > 1 {
        if let Some(err) = state.validate() {
            if !state.input.is_empty() {
                Line::from(Span::from(err).fg(theme.notif_error))
                    .render(Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1), buf);
            }
        }
    }

    // Footer hint.
    let footer_y = inner.y + inner.height.saturating_sub(1);
    if footer_y > inner.y {
        Line::from(Span::from(" Enter confirm · Esc cancel").dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
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
        .border_type(BorderType::Plain)
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

/// Render the find/replace bar at the top-right of the editor area.
#[allow(clippy::cast_possible_truncation)]
fn render_find_replace(
    area: Rect,
    buf: &mut Buffer,
    state: &FindReplaceState,
    theme: &Theme,
) {
    let bar_w = (area.width * 40 / 100).max(30).min(area.width);
    let rows: u16 = if state.show_replace { 3 } else { 2 };
    let bar_x = area.x + area.width - bar_w;
    let bar_y = area.y;

    let bar_rect = Rect::new(bar_x, bar_y, bar_w, rows);
    Clear.render(bar_rect, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .style(Style::new().fg(theme.overlay_border));
    let inner = block.inner(bar_rect);
    block.render(bar_rect, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Find row.
    let find_label = "Find: ";
    let case_indicator = if state.case_sensitive { "[Aa]" } else { "[aa]" };
    let match_info = state.match_display();
    let extra_len = find_label.len() + case_indicator.len() + match_info.len() + 2;
    let input_w = (inner.width as usize).saturating_sub(extra_len);

    let find_style = if state.active_field == FindReplaceField::Find {
        Style::new().bold()
    } else {
        Style::new().dim()
    };

    let visible_input = if state.find_input.len() > input_w {
        &state.find_input[state.find_input.len() - input_w..]
    } else {
        &state.find_input
    };

    let find_line = vec![
        Span::from(find_label),
        Span::styled(format!("{visible_input:<input_w$}"), find_style),
        Span::from(" "),
        Span::from(match_info).dim(),
        Span::from(" "),
        Span::from(case_indicator).dim(),
    ];
    Line::from(find_line).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Replace row (if visible).
    if state.show_replace && inner.height > 1 {
        let replace_label = "Replace: ";
        let replace_input_w = (inner.width as usize).saturating_sub(replace_label.len() + 1);

        let replace_style = if state.active_field == FindReplaceField::Replace {
            Style::new().bold()
        } else {
            Style::new().dim()
        };

        let visible_replace = if state.replace_input.len() > replace_input_w {
            &state.replace_input[state.replace_input.len() - replace_input_w..]
        } else {
            &state.replace_input
        };

        let replace_line = vec![
            Span::from(replace_label),
            Span::styled(format!("{visible_replace:<replace_input_w$}"), replace_style),
        ];
        Line::from(replace_line)
            .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
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
        .border_type(BorderType::Plain)
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

/// Block characters for progress indicator (8 levels of fill).
const PROGRESS_BLOCKS: &[char] = &['\u{258F}', '\u{258E}', '\u{258D}', '\u{258C}', '\u{258B}', '\u{258A}', '\u{2589}', '\u{2588}'];

/// Render toast notifications in the bottom-right corner with fade-out.
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
        if y < area.y + 3 {
            break;
        }

        let vitality = notif.vitality();
        let msg_width = (notif.message.len() as u16 + 4).min(max_width);
        let x = area.x + area.width - msg_width;
        y = y.saturating_sub(3); // 2 rows per notification + 1 gap

        // Notification text row.
        let text_rect = Rect::new(x, y, msg_width, 1);
        Clear.render(text_rect, buf);

        let base_fg = match notif.level {
            NotificationLevel::Info => theme.notif_info,
            NotificationLevel::Warning => theme.notif_warn,
            NotificationLevel::Error => theme.notif_error,
        };

        // Fade toward black as vitality decreases.
        let fg = if vitality < 1.0 {
            blend_toward(base_fg, 0, 0, 0, 1.0 - vitality)
        } else {
            base_fg
        };

        let text = format!(
            " {} ",
            &notif.message[..notif.message.len().min((msg_width - 2) as usize)]
        );
        Line::from(Span::from(text).style(Style::new().fg(fg))).render(text_rect, buf);

        // Progress bar row.
        let bar_rect = Rect::new(x, y + 1, msg_width, 1);
        Clear.render(bar_rect, buf);

        let filled_width = vitality * f32::from(msg_width);
        #[allow(clippy::cast_sign_loss)]
        let full_blocks = filled_width.max(0.0) as usize;
        #[allow(clippy::cast_precision_loss)]
        let fraction = filled_width - full_blocks as f32;
        #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
        let partial_idx = (fraction * PROGRESS_BLOCKS.len() as f32) as usize;

        let mut bar = String::with_capacity(msg_width as usize);
        for _ in 0..full_blocks.min(msg_width as usize) {
            bar.push('\u{2588}');
        }
        if full_blocks < msg_width as usize && partial_idx > 0 && partial_idx < PROGRESS_BLOCKS.len() {
            bar.push(PROGRESS_BLOCKS[partial_idx]);
        }

        let bar_fg = blend_toward(base_fg, 0, 0, 0, 0.5);
        Line::from(Span::from(bar).style(Style::new().fg(bar_fg)))
            .render(bar_rect, buf);
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
        assert!(
            cp.filtered_commands
                .iter()
                .all(|c| c.label.to_lowercase().contains("sav"))
        );
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

    // ── AI client picker tests ─────────────────────────────────────

    /// Build a picker state from a hand-crafted list of entries (bypasses
    /// PATH scanning so the tests are hermetic).
    fn make_picker_state(entries: Vec<AiClientEntry>) -> AiClientPickerState {
        AiClientPickerState {
            entries,
            selected: 0,
        }
    }

    fn claude_entry() -> AiClientEntry {
        AiClientEntry {
            label: "Claude Code".to_string(),
            command: "claude".to_string(),
            color: Color::Rgb(215, 150, 60),
            kind: AiClientKind::ClaudeCode,
        }
    }

    fn custom_entry(label: &str, command: &str) -> AiClientEntry {
        AiClientEntry {
            label: label.to_string(),
            command: command.to_string(),
            color: Color::Rgb(80, 180, 255),
            kind: AiClientKind::Custom {
                name: label.to_string(),
                command: command.to_string(),
            },
        }
    }

    // ── KNOWN_CLIENTS catalog ──────────────────────────────────────

    #[test]
    fn known_clients_all_six_present() {
        let commands: Vec<&str> = KNOWN_CLIENTS.iter().map(|c| c.command).collect();
        assert_eq!(commands.len(), 6, "expected exactly 6 known clients");
        assert!(commands.contains(&"claude"), "missing claude");
        assert!(commands.contains(&"opencode"), "missing opencode");
        assert!(commands.contains(&"gemini"), "missing gemini");
        assert!(commands.contains(&"kilo"), "missing kilo");
        assert!(commands.contains(&"cline"), "missing cline");
        assert!(commands.contains(&"qwen"), "missing qwen");
    }

    #[test]
    fn known_clients_labels_match_commands() {
        let find = |cmd: &str| KNOWN_CLIENTS.iter().find(|c| c.command == cmd).unwrap();
        assert_eq!(find("claude").label, "Claude Code");
        assert_eq!(find("opencode").label, "OpenCode");
        assert_eq!(find("gemini").label, "Gemini");
        assert_eq!(find("kilo").label, "Kilo Code");
        assert_eq!(find("cline").label, "Cline");
        assert_eq!(find("qwen").label, "Qwen Code");
    }

    #[test]
    fn known_clients_colors_all_distinct() {
        let colors: Vec<Color> = KNOWN_CLIENTS.iter().map(|c| c.color).collect();
        // No two entries share a color.
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "clients at index {i} and {j} share the same color"
                );
            }
        }
    }

    #[test]
    fn known_clients_colors_non_default() {
        // Every color must be an explicit Rgb value, not the default Reset.
        for client in KNOWN_CLIENTS {
            assert!(
                matches!(client.color, Color::Rgb(_, _, _)),
                "client '{}' uses a non-Rgb color: {:?}",
                client.label,
                client.color
            );
        }
    }

    // ── is_command_available ───────────────────────────────────────

    #[test]
    fn is_command_available_returns_false_for_fake_command() {
        assert!(
            !is_command_available("zzzneverexists123"),
            "non-existent command should not be found on PATH"
        );
    }

    #[test]
    fn is_command_available_returns_true_for_sh() {
        assert!(
            is_command_available("sh"),
            "'sh' must be present on PATH in any POSIX environment"
        );
    }

    #[test]
    fn is_command_available_empty_string_returns_false() {
        // An empty command name should never match a real executable.
        assert!(!is_command_available(""));
    }

    // ── AiClientPickerState — empty state ─────────────────────────

    #[test]
    fn picker_empty_select_next_is_noop() {
        let mut state = AiClientPickerState::default();
        state.select_next();
        assert_eq!(
            state.selected, 0,
            "select_next on empty state must not panic or change index"
        );
    }

    #[test]
    fn picker_empty_select_prev_is_noop() {
        let mut state = AiClientPickerState::default();
        state.select_prev();
        assert_eq!(
            state.selected, 0,
            "select_prev on empty state must not panic or change index"
        );
    }

    #[test]
    fn picker_empty_selected_kind_returns_none() {
        let state = AiClientPickerState::default();
        assert!(
            state.selected_kind().is_none(),
            "selected_kind on empty state must return None"
        );
    }

    // ── AiClientPickerState — single entry ────────────────────────

    #[test]
    fn picker_single_entry_next_wraps_to_zero() {
        let mut state = make_picker_state(vec![claude_entry()]);
        state.select_next();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_single_entry_prev_wraps_to_zero() {
        let mut state = make_picker_state(vec![claude_entry()]);
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    // ── AiClientPickerState — multi-entry navigation ───────────────

    #[test]
    fn picker_select_next_advances_index() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn picker_select_next_wraps_at_end() {
        let mut state =
            make_picker_state(vec![claude_entry(), custom_entry("OpenCode", "opencode")]);
        state.select_next(); // → 1
        state.select_next(); // → 0 (wrap)
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_select_prev_decrements_index() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        state.selected = 2;
        state.select_prev();
        assert_eq!(state.selected, 1);
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_select_prev_wraps_at_start() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        assert_eq!(state.selected, 0);
        state.select_prev(); // wraps to len-1 = 2
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn picker_nav_full_round_trip() {
        let n = 4usize;
        let entries = vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
            custom_entry("Kilo Code", "kilo"),
        ];
        let mut state = make_picker_state(entries);
        // Forward round-trip.
        for i in 0..n {
            assert_eq!(state.selected, i);
            state.select_next();
        }
        assert_eq!(
            state.selected, 0,
            "should wrap back to 0 after full forward cycle"
        );
        // Backward: one prev from 0 should go to n-1.
        state.select_prev();
        assert_eq!(state.selected, n - 1);
    }

    // ── AiClientPickerState — selected_kind ───────────────────────

    #[test]
    fn picker_selected_kind_claude_returns_claude_code() {
        let state = make_picker_state(vec![claude_entry()]);
        assert_eq!(state.selected_kind(), Some(AiClientKind::ClaudeCode));
    }

    #[test]
    fn picker_selected_kind_custom_entry() {
        let state = make_picker_state(vec![custom_entry("Gemini", "gemini")]);
        assert_eq!(
            state.selected_kind(),
            Some(AiClientKind::Custom {
                name: "Gemini".to_string(),
                command: "gemini".to_string(),
            })
        );
    }

    #[test]
    fn picker_selected_kind_tracks_selection() {
        let mut state =
            make_picker_state(vec![claude_entry(), custom_entry("OpenCode", "opencode")]);
        assert_eq!(state.selected_kind(), Some(AiClientKind::ClaudeCode));
        state.select_next();
        assert_eq!(
            state.selected_kind(),
            Some(AiClientKind::Custom {
                name: "OpenCode".to_string(),
                command: "opencode".to_string(),
            })
        );
    }

    // ── scan_available integration ────────────────────────────────

    #[test]
    fn scan_available_excludes_fake_commands() {
        // Verify the filtering property: any AI client entry returned by
        // scan_available must correspond to a real executable (Shell is
        // always present regardless).
        let state = AiClientPickerState::scan_available();
        for entry in &state.entries {
            if entry.kind == AiClientKind::Shell {
                continue; // Shell is always appended.
            }
            assert!(
                is_command_available(&entry.command),
                "scan_available returned '{}' but is_command_available says it's absent",
                entry.command
            );
        }
    }

    #[test]
    fn scan_available_selected_starts_at_zero() {
        let state = AiClientPickerState::scan_available();
        assert_eq!(state.selected, 0, "initial selection must be 0");
    }

    #[test]
    fn scan_available_entries_have_known_client_data() {
        // Every returned entry must originate from KNOWN_CLIENTS or be the shell.
        let state = AiClientPickerState::scan_available();
        for entry in &state.entries {
            if entry.kind == AiClientKind::Shell {
                assert_eq!(entry.label, "System Shell");
                continue;
            }
            let known = KNOWN_CLIENTS.iter().find(|k| k.command == entry.command);
            assert!(
                known.is_some(),
                "scan_available returned unknown command '{}'",
                entry.command
            );
            let known = known.unwrap();
            assert_eq!(entry.label, known.label);
            assert_eq!(entry.color, known.color);
        }
    }

    #[test]
    fn scan_available_always_includes_shell() {
        let state = AiClientPickerState::scan_available();
        let shell = state.entries.iter().find(|e| e.kind == AiClientKind::Shell);
        assert!(shell.is_some(), "System Shell must always be present");
        let shell = shell.unwrap();
        assert_eq!(shell.label, "System Shell");
    }

    #[test]
    fn scan_available_claude_entry_maps_to_claude_code_kind() {
        // If claude is on PATH, verify its kind. If not, the test is vacuous.
        let state = AiClientPickerState::scan_available();
        if let Some(entry) = state.entries.iter().find(|e| e.command == "claude") {
            assert_eq!(
                entry.kind,
                AiClientKind::ClaudeCode,
                "claude command must map to AiClientKind::ClaudeCode"
            );
        }
    }

    #[test]
    fn scan_available_non_claude_entries_map_to_custom_or_shell_kind() {
        let state = AiClientPickerState::scan_available();
        for entry in state.entries.iter().filter(|e| e.command != "claude") {
            match &entry.kind {
                AiClientKind::Custom { name, command } => {
                    assert_eq!(name, &entry.label);
                    assert_eq!(command, &entry.command);
                }
                AiClientKind::Shell => {
                    assert_eq!(entry.label, "System Shell");
                }
                other @ AiClientKind::ClaudeCode => panic!(
                    "entry '{}' should be Custom or Shell but got {:?}",
                    entry.command, other
                ),
            }
        }
    }

    // ── overlay open_ai_client_picker ─────────────────────────────

    #[test]
    fn overlay_open_ai_client_picker_sets_active_kind() {
        let mut overlay = OverlayState::default();
        overlay.open_ai_client_picker();
        assert!(overlay.is_active());
        assert!(
            matches!(overlay.active, Some(OverlayKind::AiClientPicker)),
            "active overlay must be AiClientPicker"
        );
    }

    #[test]
    fn overlay_open_ai_client_picker_close_clears() {
        let mut overlay = OverlayState::default();
        overlay.open_ai_client_picker();
        overlay.close();
        assert!(!overlay.is_active());
    }

    // ── Input dialog tests ────────────────────────────────────────────

    #[test]
    fn input_dialog_type_and_backspace() {
        let mut d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        d.type_char('h');
        d.type_char('e');
        d.type_char('l');
        assert_eq!(d.input, "hel");
        assert_eq!(d.cursor_pos, 3);
        d.backspace();
        assert_eq!(d.input, "he");
        assert_eq!(d.cursor_pos, 2);
    }

    #[test]
    fn input_dialog_cursor_movement() {
        let mut d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        d.type_char('a');
        d.type_char('b');
        d.type_char('c');
        d.home();
        assert_eq!(d.cursor_pos, 0);
        d.move_right();
        assert_eq!(d.cursor_pos, 1);
        d.end();
        assert_eq!(d.cursor_pos, 3);
        d.move_left();
        assert_eq!(d.cursor_pos, 2);
    }

    #[test]
    fn input_dialog_delete() {
        let mut d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        d.type_char('a');
        d.type_char('b');
        d.type_char('c');
        d.home();
        d.delete();
        assert_eq!(d.input, "bc");
        assert_eq!(d.cursor_pos, 0);
    }

    #[test]
    fn input_dialog_validate_empty() {
        let d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        assert!(d.validate().is_some());
    }

    #[test]
    fn input_dialog_validate_path_separator() {
        let mut d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        d.type_char('a');
        d.type_char('/');
        d.type_char('b');
        assert!(d.validate().is_some());
    }

    #[test]
    fn input_dialog_validate_ok() {
        let mut d = InputDialogState::new("Test", "hint", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        d.type_char('f');
        d.type_char('o');
        d.type_char('o');
        assert!(d.validate().is_none());
    }

    #[test]
    fn input_dialog_with_input_prefill() {
        let d = InputDialogState::new("Rename", "new name", InputDialogAction::Rename { from: PathBuf::from("/old") })
            .with_input("old_name.txt");
        assert_eq!(d.input, "old_name.txt");
        assert_eq!(d.cursor_pos, 12);
    }

    #[test]
    fn vitality_fresh_is_one() {
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now(),
        };
        assert!((notif.vitality() - 1.0).abs() < 0.01);
    }

    #[test]
    fn vitality_at_expiry_is_zero() {
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now() - std::time::Duration::from_secs(5),
        };
        assert!(notif.vitality() <= 0.0);
    }

    #[test]
    fn vitality_during_fade_is_between() {
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now() - std::time::Duration::from_millis(3500),
        };
        let v = notif.vitality();
        assert!(v > 0.0 && v < 1.0, "vitality during fade should be 0 < {v} < 1");
    }

    #[test]
    fn overlay_open_input_dialog() {
        let mut overlay = OverlayState::default();
        let state = InputDialogState::new("New File", "filename", InputDialogAction::CreateFile { parent: PathBuf::from("/tmp") });
        overlay.open_input_dialog(state);
        assert!(overlay.is_active());
        assert!(matches!(overlay.active, Some(OverlayKind::InputDialog)));
        assert!(overlay.input_dialog.is_some());
    }

    #[test]
    fn find_replace_type_and_backspace() {
        let mut state = FindReplaceState::default();
        state.type_char('h');
        state.type_char('i');
        assert_eq!(state.find_input, "hi");
        state.backspace();
        assert_eq!(state.find_input, "h");
    }

    #[test]
    fn find_replace_toggle_field() {
        let mut state = FindReplaceState::default();
        state.show_replace = true;
        assert_eq!(state.active_field, FindReplaceField::Find);
        state.toggle_field();
        assert_eq!(state.active_field, FindReplaceField::Replace);
        state.toggle_field();
        assert_eq!(state.active_field, FindReplaceField::Find);
    }

    #[test]
    fn find_replace_toggle_field_no_replace() {
        let mut state = FindReplaceState::default();
        state.show_replace = false;
        state.toggle_field();
        // Should stay on Find when replace is hidden.
        assert_eq!(state.active_field, FindReplaceField::Find);
    }

    #[test]
    fn find_replace_toggle_case() {
        let mut state = FindReplaceState::default();
        assert!(!state.case_sensitive);
        state.toggle_case();
        assert!(state.case_sensitive);
    }

    #[test]
    fn find_replace_match_display_empty() {
        let state = FindReplaceState::default();
        assert_eq!(state.match_display(), "");
    }

    #[test]
    fn find_replace_open_methods() {
        let mut overlay = OverlayState::default();
        overlay.open_find();
        assert!(overlay.is_active());
        assert!(!overlay.find_replace.show_replace);
        overlay.close();

        overlay.open_find_replace();
        assert!(overlay.is_active());
        assert!(overlay.find_replace.show_replace);
    }

    #[test]
    fn language_picker_new_sorts_and_dedupes() {
        use lune_core::language::lang;
        let langs = vec![lang::PYTHON, lang::RUST, lang::PYTHON, lang::GO];
        let picker = LanguagePickerState::new(langs);
        // sorted: Go, Python, Rust — Python deduped
        assert_eq!(picker.all_languages.len(), 3);
        assert_eq!(picker.filtered.len(), 3);
        assert_eq!(picker.all_languages[0], lang::GO);
        assert_eq!(picker.all_languages[1], lang::PYTHON);
        assert_eq!(picker.all_languages[2], lang::RUST);
    }

    #[test]
    fn language_picker_filter_by_input() {
        use lune_core::language::lang;
        let mut picker =
            LanguagePickerState::new(vec![lang::RUST, lang::RUBY, lang::PYTHON, lang::GO]);
        picker.type_char('r');
        // "r" matches Rust, Ruby (case-insensitive)
        assert_eq!(picker.filtered.len(), 2);
        picker.type_char('u');
        // "ru" matches Rust, Ruby
        assert_eq!(picker.filtered.len(), 2);
        picker.type_char('s');
        // "rus" matches only Rust
        assert_eq!(picker.filtered.len(), 1);
        assert_eq!(picker.selected_lang(), Some(lang::RUST));
    }

    #[test]
    fn language_picker_backspace_restores_filter() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        picker.type_char('r');
        assert_eq!(picker.filtered.len(), 1);
        picker.backspace();
        assert_eq!(picker.filtered.len(), 2);
    }

    #[test]
    fn language_picker_navigation_wraps() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        assert_eq!(picker.selected, 0);
        picker.select_next();
        assert_eq!(picker.selected, 1);
        picker.select_next(); // wraps to 0
        assert_eq!(picker.selected, 0);
        picker.select_prev(); // wraps to 1
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn language_picker_empty_filter_no_match() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        picker.type_char('z'); // no language contains 'z'
        assert!(picker.filtered.is_empty());
        assert_eq!(picker.selected_lang(), None);
    }

    #[test]
    fn language_picker_overlay_opens_correctly() {
        use lune_core::language::lang;
        let mut overlay = OverlayState::default();
        overlay.open_language_picker(vec![lang::RUST, lang::PYTHON]);
        assert!(overlay.is_active());
        assert!(matches!(overlay.active, Some(OverlayKind::LanguagePicker)));
        assert_eq!(overlay.language_picker.all_languages.len(), 2);
    }
}
