//! Overlay system (command palette, find/replace, file picker, notifications).
//!
//! Overlays render on top of the main layout and capture keyboard input
//! when active. The command palette provides fuzzy-filtered command search.
//! The file picker provides interactive directory browsing.
//!
//! # Module layout
//!
//! Each overlay has its own sub-module under `overlay/`; this root file
//! holds the orchestrator types ([`OverlayState`], [`OverlayKind`]) and
//! re-exports every public item so all `crate::widgets::overlay::X` paths
//! remain unchanged.

pub mod ai_client_picker;
pub mod command_palette;
pub mod file_picker;
pub mod find_replace;
pub mod image_preview;
pub mod input_dialog;
pub mod key_hints;
pub mod language_picker;
pub mod layout_picker;
pub mod markdown_preview;
pub mod notifications;
pub mod project_search;
pub mod theme_picker;
pub(crate) mod util;

pub use ai_client_picker::*;
pub use command_palette::*;
pub use file_picker::*;
pub use find_replace::*;
pub use image_preview::*;
pub use input_dialog::*;
pub use key_hints::*;
pub use language_picker::*;
pub use layout_picker::*;
pub use markdown_preview::*;
pub use notifications::*;
pub use project_search::*;
pub use theme_picker::*;

use std::path::Path;
use std::time::Instant;

use crate::event::AppCommand;
use crate::primitives::{Buffer, Rect};
use crate::theme::Theme;
use crate::widgets::confirm_dialog::ConfirmDialogState;
use lune_core::buffer::BufferId;
use lune_core::language::LanguageId;
use lune_core::undo::RevisionId;

use ai_client_picker::render_ai_client_picker;
use command_palette::render_command_palette;
use file_picker::render_file_picker;
use find_replace::render_find_replace;
use image_preview::render_image_preview;
use input_dialog::render_input_dialog;
use key_hints::render_key_hints;
use language_picker::render_language_picker;
use layout_picker::render_layout_picker;
use markdown_preview::{MIN_LIVE_REFRESH_INTERVAL, parse_markdown, render_markdown_preview};
use notifications::render_notifications;
use project_search::render_project_search;
use theme_picker::render_theme_picker;

// ── Overlay kinds ─────────────────────────────────────────────────────

/// The type of overlay currently displayed.
#[derive(Clone, Debug)]
pub enum OverlayKind {
    /// Command palette with fuzzy search.
    CommandPalette,
    /// Project-wide text search ("search in files").
    ProjectSearch,
    /// Find/replace dialog.
    FindReplace,
    /// Confirm dialog. The dialog widget and the command to dispatch on
    /// confirmation live in [`OverlayState::confirm`].
    ConfirmDialog,
    /// Interactive file/directory picker.
    FilePicker,
    /// AI client picker (choose which client to launch).
    AiClientPicker,
    /// Inline text input dialog (new file, rename, etc.).
    InputDialog,
    /// Language selector (fuzzy-filtered list of all known languages).
    LanguagePicker,
    /// Theme picker with live preview.
    ThemePicker,
    /// Layout picker for agent pane tiling presets.
    LayoutPicker,
    /// Markdown preview of the active buffer (rendered via `tui-markdown`).
    MarkdownPreview,
    /// Image preview for a file from disk (rendered via `ratatui-image`).
    ImagePreview,
    /// Keybinding hints overlay (which-key style) — categorized cheatsheet
    /// of all active key bindings. Bound to `F1` and `?`.
    KeyHints,
}

/// State backing an open [`OverlayKind::ConfirmDialog`]: the Modal-based
/// [`ConfirmDialogState`] that renders and handles input, plus the command
/// emitted when the user confirms.
#[derive(Clone, Debug)]
pub struct ConfirmOverlayState {
    /// Reusable confirm/cancel dialog widget state.
    pub dialog: ConfirmDialogState,
    /// Command dispatched when the dialog resolves to `Confirm`.
    pub on_confirm: AppCommand,
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
    /// Project-wide search ("search in files") state.
    pub project_search: ProjectSearchState,
    /// Active notifications (toast messages).
    pub notifications: Vec<Notification>,
    /// Tuning (timeouts, queue caps, width) for the notification system.
    pub notification_config: NotificationConfig,
    /// AI client picker state.
    pub ai_client_picker: AiClientPickerState,
    /// Input dialog state (for file operations).
    pub input_dialog: Option<InputDialogState>,
    /// Find/replace bar state.
    pub find_replace: FindReplaceState,
    /// Language picker state.
    pub language_picker: LanguagePickerState,
    /// Theme picker state.
    pub theme_picker: ThemePickerState,
    /// Layout picker state.
    pub layout_picker: LayoutPickerState,
    /// Markdown preview state — owns the rendered text + scroll offset.
    pub markdown_preview: MarkdownPreviewState,
    /// Image preview state — owns the decoded image protocol.
    pub image_preview: ImagePreviewState,
    /// Keybinding hints overlay state — scroll offset + typed filter.
    pub key_hints: KeyHintsState,
    /// Active confirm dialog — the Modal-based dialog plus the command to
    /// dispatch on confirmation. `None` when no confirm overlay is open.
    pub confirm: Option<ConfirmOverlayState>,
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

    /// Open the project-wide search overlay rooted at `root`.
    pub fn open_project_search(&mut self, root: &Path) {
        self.project_search.open(root);
        self.active = Some(OverlayKind::ProjectSearch);
    }

    /// Close whatever overlay is open.
    pub fn close(&mut self) {
        // Per-kind cleanup for overlays that hold heavy data (rendered
        // markdown text, decoded image protocol). Otherwise those would
        // stay resident until the next open.
        match self.active {
            Some(OverlayKind::MarkdownPreview) => {
                self.markdown_preview = MarkdownPreviewState::default();
            }
            Some(OverlayKind::ImagePreview) => {
                self.image_preview = ImagePreviewState::default();
            }
            Some(OverlayKind::ConfirmDialog) => {
                self.confirm = None;
            }
            Some(OverlayKind::ProjectSearch) => {
                self.project_search = ProjectSearchState::default();
            }
            _ => {}
        }
        self.active = None;
    }

    /// Open an input dialog.
    pub fn open_input_dialog(&mut self, state: InputDialogState) {
        self.input_dialog = Some(state);
        self.active = Some(OverlayKind::InputDialog);
    }

    /// Open find bar (no replace row).
    pub const fn open_find(&mut self) {
        self.find_replace.show_replace = false;
        self.find_replace.active_field = FindReplaceField::Find;
        self.active = Some(OverlayKind::FindReplace);
    }

    /// Open find and replace bar.
    pub const fn open_find_replace(&mut self) {
        self.find_replace.show_replace = true;
        self.find_replace.active_field = FindReplaceField::Find;
        self.active = Some(OverlayKind::FindReplace);
    }

    /// Open the language picker loaded with the given language list.
    pub fn open_language_picker(&mut self, languages: Vec<LanguageId>) {
        self.language_picker = LanguagePickerState::new(languages);
        self.active = Some(OverlayKind::LanguagePicker);
    }

    /// Open the theme picker with the current theme pre-selected.
    ///
    /// `themes` is a list of `(index, name)` pairs from the registry.
    /// `current_idx` is the active theme's position (stored for Escape revert).
    pub fn open_theme_picker(&mut self, themes: Vec<(usize, String)>, current_idx: usize) {
        self.theme_picker = ThemePickerState::new(themes, current_idx);
        self.active = Some(OverlayKind::ThemePicker);
    }

    /// Open the layout picker for agent pane tiling presets.
    pub fn open_layout_picker(&mut self, entries: Vec<LayoutPickerEntry>) {
        self.layout_picker = LayoutPickerState::new(entries);
        self.active = Some(OverlayKind::LayoutPicker);
    }

    /// Open the markdown preview for `source` with a frame title `title`.
    ///
    /// Parses once into an owned `Text<'static>` cached on the state so
    /// the render path doesn't re-parse on every frame. Passing `source_key`
    /// = `Some((buf_id, revision))` seeds the cache key so a follow-up
    /// [`Self::refresh_markdown_preview`] call at the same revision is a
    /// no-op — pass `None` for a one-shot preview not backed by a buffer.
    pub fn open_markdown_preview(
        &mut self,
        source: String,
        title: String,
        source_key: Option<(BufferId, RevisionId)>,
    ) {
        let (source, rendered) = parse_markdown(source);
        let (source_buffer, source_revision) = match source_key {
            Some((id, rev)) => (Some(id), Some(rev)),
            None => (None, None),
        };
        self.markdown_preview = MarkdownPreviewState {
            source,
            title,
            scroll: 0,
            rendered: Some(rendered),
            source_buffer,
            source_revision,
            last_parsed_at: Some(Instant::now()),
        };
        self.active = Some(OverlayKind::MarkdownPreview);
    }

    /// Re-parse the markdown preview if `(buf_id, revision)` differs from
    /// the cached key. `source_fn` and `title_fn` are only invoked when
    /// a re-parse is actually needed — we don't materialize the rope into a
    /// `String` on frames where the buffer hasn't changed, and `title_fn`
    /// is skipped on revision-only updates since the title only changes
    /// when the source buffer changes identity.
    ///
    /// Revision-only updates against the same buffer are debounced by
    /// [`MIN_LIVE_REFRESH_INTERVAL`]: `tui_markdown` runs synchronously on
    /// the UI thread and re-parsing on every keystroke can produce
    /// noticeable input lag for large markdown sources. Buffer swaps and
    /// the initial parse always bypass the debounce so newly-opened
    /// previews are never stale. Scroll is preserved across refreshes.
    ///
    /// `now` is taken as a parameter so tests can drive time deterministically;
    /// production callers pass `Instant::now()`.
    ///
    /// No-op when the markdown preview overlay is not currently active.
    pub fn refresh_markdown_preview<F, G>(
        &mut self,
        buf_id: BufferId,
        revision: RevisionId,
        now: Instant,
        source_fn: F,
        title_fn: G,
    ) where
        F: FnOnce() -> String,
        G: FnOnce() -> String,
    {
        if !matches!(self.active, Some(OverlayKind::MarkdownPreview)) {
            return;
        }
        let cached_buffer = self.markdown_preview.source_buffer;
        let cached_revision = self.markdown_preview.source_revision;
        if cached_buffer == Some(buf_id) && cached_revision == Some(revision) {
            return;
        }
        let same_buffer = cached_buffer == Some(buf_id);
        // Debounce revision-only bumps on the same buffer: this is the hot
        // path (every keystroke). Buffer swaps and the initial parse must
        // run immediately to avoid showing stale content from a stale buffer.
        if same_buffer
            && let Some(last) = self.markdown_preview.last_parsed_at
            && now.duration_since(last) < MIN_LIVE_REFRESH_INTERVAL
        {
            return;
        }
        let scroll = self.markdown_preview.scroll;
        let title = if same_buffer {
            std::mem::take(&mut self.markdown_preview.title)
        } else {
            title_fn()
        };
        let (source, rendered) = parse_markdown(source_fn());
        self.markdown_preview = MarkdownPreviewState {
            source,
            title,
            scroll,
            rendered: Some(rendered),
            source_buffer: Some(buf_id),
            source_revision: Some(revision),
            last_parsed_at: Some(now),
        };
    }

    /// Open the image preview overlay for `path`.
    ///
    /// The decode runs on a background worker thread; the overlay opens
    /// immediately in the `Loading` state and is updated when the worker
    /// posts an `AppEvent::ImageDecodeReady`. Bumping the per-state
    /// generation on every open means that a fast sequence of opens
    /// drops stale results instead of overwriting the active preview.
    pub fn open_image_preview(&mut self, path: &Path, decoder: &ImageDecoder) {
        let generation = self.image_preview.begin_load(path);
        decoder.spawn(generation, path.to_path_buf());
        self.active = Some(OverlayKind::ImagePreview);
    }

    /// Open the keybinding hints overlay (which-key style cheatsheet).
    pub fn open_key_hints(&mut self) {
        self.key_hints = KeyHintsState::default();
        self.active = Some(OverlayKind::KeyHints);
    }

    /// Open a confirmation dialog.
    pub fn open_confirm(&mut self, message: impl Into<String>, on_confirm: AppCommand) {
        let mut dialog = ConfirmDialogState::new("Confirm", message);
        dialog.open();
        self.confirm = Some(ConfirmOverlayState { dialog, on_confirm });
        self.active = Some(OverlayKind::ConfirmDialog);
    }

    /// Push a notification toast.
    ///
    /// Deduplicates: if the most recent notification has the same level
    /// and message, its `count` is incremented and its TTL reset instead
    /// of creating a new entry. This turns "Saved" × 5 into a single toast
    /// rendered as `Saved ×5`.
    ///
    /// Enforces the queue cap from [`NotificationConfig::max_queue`]:
    /// when exceeded, the oldest entry is dropped.
    pub fn notify(&mut self, message: impl Into<String>, level: NotificationLevel) {
        self.notify_at(message, level, Instant::now());
    }

    /// [`notify`](Self::notify) with an injectable timestamp so the
    /// entrance/TTL clocks can be driven deterministically in tests.
    ///
    /// On dedup only `created` (the TTL clock) is reset; `spawned` (the
    /// entrance clock) is preserved so the toast doesn't replay its
    /// slide-in animation on every repeat.
    pub fn notify_at(
        &mut self,
        message: impl Into<String>,
        level: NotificationLevel,
        now: Instant,
    ) {
        let message = message.into();
        if let Some(last) = self.notifications.last_mut() {
            if last.level == level && last.message == message {
                last.count = last.count.saturating_add(1);
                last.created = now;
                return;
            }
        }
        self.notifications.push(Notification {
            message,
            level,
            created: now,
            spawned: now,
            count: 1,
        });
        while self.notifications.len() > self.notification_config.max_queue {
            self.notifications.remove(0);
        }
    }

    /// Convenience: success toast.
    pub fn notify_success(&mut self, message: impl Into<String>) {
        self.notify(message, NotificationLevel::Success);
    }

    /// Convenience: info toast.
    pub fn notify_info(&mut self, message: impl Into<String>) {
        self.notify(message, NotificationLevel::Info);
    }

    /// Convenience: warning toast.
    pub fn notify_warn(&mut self, message: impl Into<String>) {
        self.notify(message, NotificationLevel::Warning);
    }

    /// Convenience: error toast.
    pub fn notify_error(&mut self, message: impl Into<String>) {
        self.notify(message, NotificationLevel::Error);
    }

    /// Drop expired notifications (those past their severity-specific TTL).
    pub fn prune_notifications(&mut self) {
        let cfg = self.notification_config;
        self.notifications.retain(|n| !n.is_expired(&cfg));
    }

    /// Clear every pending and visible notification at once. Used by the
    /// dismiss-all keybinding.
    pub fn dismiss_all_notifications(&mut self) {
        self.notifications.clear();
    }

    /// Whether any toast is currently queued. Drives registration of the
    /// notification animation timer so toasts prune and fade on a quiet
    /// terminal without waiting for unrelated input.
    #[must_use]
    pub fn has_active_notifications(&self) -> bool {
        !self.notifications.is_empty()
    }
}

pub fn render_overlay(area: Rect, buf: &mut Buffer, overlay: &mut OverlayState, theme: &Theme) {
    // Render notifications (bottom-right toasts).
    render_notifications(
        area,
        buf,
        &overlay.notifications,
        &overlay.notification_config,
        theme,
    );

    // Render the active overlay.
    match &overlay.active {
        Some(OverlayKind::CommandPalette) => {
            render_command_palette(area, buf, &mut overlay.command_palette, theme);
        }
        Some(OverlayKind::FindReplace) => {
            render_find_replace(area, buf, &overlay.find_replace, theme);
        }
        Some(OverlayKind::ConfirmDialog) => {
            if let Some(confirm) = overlay.confirm.as_mut() {
                confirm.dialog.render(area, buf, theme);
            }
        }
        Some(OverlayKind::ProjectSearch) => {
            render_project_search(area, buf, &mut overlay.project_search, theme);
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
        Some(OverlayKind::ThemePicker) => {
            render_theme_picker(area, buf, &overlay.theme_picker, theme);
        }
        Some(OverlayKind::LayoutPicker) => {
            render_layout_picker(area, buf, &overlay.layout_picker, theme);
        }
        Some(OverlayKind::MarkdownPreview) => {
            render_markdown_preview(area, buf, &overlay.markdown_preview, theme);
        }
        Some(OverlayKind::ImagePreview) => {
            render_image_preview(area, buf, &mut overlay.image_preview, theme);
        }
        Some(OverlayKind::KeyHints) => {
            render_key_hints(area, buf, &overlay.key_hints, theme);
        }
        None => {}
    }
}
