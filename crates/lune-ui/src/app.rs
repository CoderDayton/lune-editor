//! Application state and rat-salsa integration.
//!
//! This module contains the global context (`LuneGlobal`) and application
//! state (`AppState`) used by the rat-salsa event loop, plus the four
//! function pointers required by `run_tui`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Error;
use crossbeam::channel::{self, Receiver, TryRecvError};
use rat_salsa::poll::{PollCrossterm, PollTimers};
use rat_salsa::{run_tui, Control, RunConfig, SalsaAppContext, SalsaContext};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Constraint, Direction, Layout, Rect};
use ratatui_core::style::Stylize;
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;
use ratatui_crossterm::crossterm::event::{
    Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use lune_core::prelude::*;
use lune_core::watcher::{FileWatcher, WatchEvent};
use lune_core::workspace::EntryKind;
use lune_git::{GitService, GutterMarks};

use crate::highlight;
use crate::highlight::theme::SyntaxTheme;

use crate::event::{AppCommand, AppEvent};
use crate::focus::{FocusManager, PanelId};
use crate::keybindings::Keymap;
use crate::layout::{self, LayoutSplits, LayoutState};
use crate::vim::{VimAction, VimMode, VimState};
use crate::widgets::editor_pane::{self, ViewportState};
use crate::widgets::file_tree::{self, FileTreeState};
use crate::widgets::git_panel::{self, GitPanelState};
use crate::widgets::overlay::{self, NotificationLevel, OverlayState};
use crate::widgets::status_bar::{self, StatusLineState};
use crate::widgets::tab_bar::{self, TabManager};

/// Global context — embeds the rat-salsa context and shared config.
#[derive(Default)]
pub struct LuneGlobal {
    /// The rat-salsa framework context.
    ctx: SalsaAppContext<AppEvent, Error>,
}

impl SalsaContext<AppEvent, Error> for LuneGlobal {
    fn set_salsa_ctx(&mut self, app_ctx: SalsaAppContext<AppEvent, Error>) {
        self.ctx = app_ctx;
    }

    fn salsa_ctx(&self) -> &SalsaAppContext<AppEvent, Error> {
        &self.ctx
    }
}

/// Application state — holds all mutable application data.
pub struct AppState {
    /// Buffer registry (all open buffers).
    pub registry: BufferRegistry,
    /// Active buffer ID (the one displayed in the editor pane).
    pub active_buffer: Option<BufferId>,
    /// Tab order (list of open buffer IDs).
    pub tabs: Vec<BufferId>,
    /// Focus manager.
    pub focus: FocusManager,
    /// Global keybindings.
    pub keymap: Keymap,
    /// Vim mode state.
    pub vim: VimState,
    /// Status bar message.
    pub status_message: String,
    /// Error count.
    pub error_count: u32,
    /// Last error message.
    pub last_error: String,
    /// Layout configuration (panel visibility and widths).
    pub layout: LayoutState,
    /// Tab manager (display state synced from registry).
    pub tab_mgr: TabManager,
    /// Editor viewport state.
    pub viewport: ViewportState,
    /// Overlay state (command palette, notifications).
    pub overlay: OverlayState,
    /// Last computed layout splits (for mouse hit-testing).
    pub last_splits: Option<LayoutSplits>,
    /// Whether the mouse is currently dragging a panel border.
    pub dragging_border: Option<DragBorder>,
    /// The editor content area from the last render (for mouse mapping).
    pub last_editor_content_area: Option<Rect>,
    /// Workspace (opened project directory).
    pub workspace: Option<Workspace>,
    /// File tree widget state.
    pub file_tree: FileTreeState,
    /// File system watcher (active when a workspace is open).
    watcher: Option<FileWatcher>,
    /// Receiver for watcher events (shared with `PollFileWatcher`).
    watcher_rx: Receiver<WatchEvent>,
    /// Sender for watcher events (passed to `FileWatcher` forwarding thread).
    watcher_tx: channel::Sender<WatchEvent>,
    /// Per-buffer syntax highlighters.
    highlighters: HashMap<BufferId, Box<dyn Highlighter>>,
    /// Language detection registry.
    lang_registry: LanguageRegistry,
    /// Syntax color theme.
    syntax_theme: SyntaxTheme,
    /// Git service (active when workspace is in a git repository).
    git_service: Option<GitService>,
    /// Per-buffer git gutter marks (cached).
    gutter_marks: HashMap<BufferId, GutterMarks>,
    /// Git branch name for the status bar.
    pub git_branch: String,
    /// Git ahead/behind counts.
    pub git_ahead: usize,
    /// Git behind count.
    pub git_behind: usize,
    /// Git panel state.
    pub git_panel: GitPanelState,
    /// Last left-click info for double-click detection: (time, column, row).
    last_click: Option<(Instant, u16, u16)>,
}

/// Which border is being dragged by the mouse.
#[derive(Clone, Copy, Debug)]
pub enum DragBorder {
    /// Dragging the left panel / editor border.
    Left,
    /// Dragging the editor / right panel border.
    Right,
}

impl AppState {
    /// Create a new application state.
    #[must_use]
    pub fn new() -> Self {
        let (watcher_tx, watcher_rx) = channel::unbounded();
        Self {
            registry: BufferRegistry::new(),
            active_buffer: None,
            tabs: Vec::new(),
            focus: FocusManager::new(),
            keymap: Keymap::default_global(),
            vim: VimState::new(),
            status_message: String::new(),
            error_count: 0,
            last_error: String::new(),
            layout: LayoutState::default(),
            tab_mgr: TabManager::new(),
            viewport: ViewportState::default(),
            overlay: OverlayState::default(),
            last_splits: None,
            dragging_border: None,
            last_editor_content_area: None,
            workspace: None,
            file_tree: FileTreeState::new(),
            watcher: None,
            watcher_rx,
            watcher_tx,
            highlighters: HashMap::new(),
            lang_registry: LanguageRegistry::new(),
            syntax_theme: SyntaxTheme::dark(),
            git_service: None,
            gutter_marks: HashMap::new(),
            git_branch: String::new(),
            git_ahead: 0,
            git_behind: 0,
            git_panel: GitPanelState::new(),
            last_click: None,
        }
    }

    /// Open a file and make it the active tab.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub fn open_file(&mut self, path: &std::path::Path) -> anyhow::Result<BufferId> {
        let id = self.registry.open_file(path)?;
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        self.active_buffer = Some(id);

        // Assign a syntax highlighter if we don't already have one for this buffer.
        if !self.highlighters.contains_key(&id) {
            if let Some(buf) = self.registry.get(id) {
                let first_line = buf.line(0);
                let lang_id = self.lang_registry.detect(path, first_line.as_deref());
                if let Some(lid) = lang_id {
                    let mut hl = highlight::create_highlighter(lid);
                    hl.update(buf, None);
                    self.highlighters.insert(id, hl);
                }
            }
        }

        Ok(id)
    }

    /// Open a workspace directory.
    ///
    /// This sets the workspace, enables the file tree panel, starts the
    /// file watcher, and performs an initial file tree refresh.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be opened.
    pub fn open_workspace(&mut self, root: impl Into<std::path::PathBuf>) -> anyhow::Result<()> {
        let root = root.into();
        let mut ws = Workspace::open(&root)?;
        // Pre-populate the root listing.
        let _ = ws.list_dir(ws.root().to_path_buf().as_path());
        self.file_tree.refresh(&mut ws)?;
        self.workspace = Some(ws);
        self.layout.show_file_tree = true;

        // Start file watcher — forward WatchEvents to our shared channel.
        match FileWatcher::new(&root, Duration::from_millis(200)) {
            Ok(fw) => {
                let tx = self.watcher_tx.clone();
                let rx = fw.receiver().clone();
                std::thread::Builder::new()
                    .name("lune-watcher-fwd".into())
                    .spawn(move || {
                        while let Ok(event) = rx.recv() {
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    })
                    .ok();
                self.watcher = Some(fw);
            }
            Err(e) => {
                log::warn!("Failed to start file watcher: {e}");
            }
        }

        // Initialize git service.
        match GitService::open(&root) {
            Ok(Some(git)) => {
                self.refresh_git_status(&git);
                self.git_service = Some(git);
            }
            Ok(None) => {
                log::info!("Workspace is not a git repository");
            }
            Err(e) => {
                log::warn!("Failed to initialize git service: {e}");
            }
        }

        Ok(())
    }

    /// Refresh the file tree from the current workspace.
    pub fn refresh_file_tree(&mut self) {
        if let Some(ref mut ws) = self.workspace {
            if let Err(e) = self.file_tree.refresh(ws) {
                log::error!("Failed to refresh file tree: {e}");
            }
        }
    }

    /// Refresh git status from the stored `GitService`.
    pub fn refresh_git(&mut self) {
        // Take the service temporarily to avoid borrow conflicts.
        if let Some(git) = self.git_service.take() {
            self.refresh_git_status(&git);
            self.git_service = Some(git);
        }
    }

    /// Refresh git-derived state from a `GitService` reference.
    fn refresh_git_status(&mut self, git: &GitService) {
        match git.status() {
            Ok(status) => {
                self.git_branch.clone_from(&status.branch);
                self.git_ahead = status.ahead;
                self.git_behind = status.behind;

                // Update file tree git statuses via workspace cache entries.
                self.apply_git_to_file_tree(&status.files, git);

                // Update gutter marks for open buffers.
                self.refresh_gutter_marks(git);

                // Update git panel.
                self.git_panel.update_status(status);
            }
            Err(e) => {
                log::error!("Failed to query git status: {e}");
            }
        }
    }

    /// Apply git file statuses to the file tree entries.
    fn apply_git_to_file_tree(&mut self, files: &[lune_git::GitFileStatus], git: &GitService) {
        // Set git status on matching file tree entries.
        for (_depth, entry) in &mut self.file_tree.entries {
            entry.git_status = files.iter().find_map(|f| {
                let abs_path = git.root().join(&f.path);
                if entry.path == abs_path {
                    Some(f.status)
                } else {
                    None
                }
            });
        }
    }

    /// Refresh gutter marks for all open buffers.
    fn refresh_gutter_marks(&mut self, git: &GitService) {
        for &id in &self.tabs {
            if let Some(buf) = self.registry.get(id) {
                if let Some(ref path) = buf.file_path {
                    if let Some(rel) = git.repo_relative(path) {
                        let content = buf.text();
                        match git.gutter_marks(&rel, &content) {
                            Ok(marks) => {
                                self.gutter_marks.insert(id, marks);
                            }
                            Err(e) => {
                                log::debug!(
                                    "Failed to compute gutter marks for {}: {e}",
                                    rel.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get a clone of the watcher event receiver for use with `PollFileWatcher`.
    #[must_use]
    pub fn watcher_receiver(&self) -> Receiver<WatchEvent> {
        self.watcher_rx.clone()
    }

    /// Get a reference to the active buffer.
    #[must_use]
    pub fn active_buf(&self) -> Option<&TextBuffer> {
        self.active_buffer.and_then(|id| self.registry.get(id))
    }

    /// Get a mutable reference to the active buffer.
    pub fn active_buf_mut(&mut self) -> Option<&mut TextBuffer> {
        self.active_buffer.and_then(|id| self.registry.get_mut(id))
    }

    /// Switch to the next tab.
    pub fn next_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        if let Some(current) = self.active_buffer {
            if let Some(idx) = self.tabs.iter().position(|&id| id == current) {
                let next = (idx + 1) % self.tabs.len();
                self.active_buffer = Some(self.tabs[next]);
            }
        }
    }

    /// Switch to the previous tab.
    pub fn prev_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        if let Some(current) = self.active_buffer {
            if let Some(idx) = self.tabs.iter().position(|&id| id == current) {
                let prev = if idx == 0 {
                    self.tabs.len() - 1
                } else {
                    idx - 1
                };
                self.active_buffer = Some(self.tabs[prev]);
            }
        }
    }

    /// Close the active tab.
    pub fn close_active_tab(&mut self) {
        if let Some(id) = self.active_buffer {
            if let Some(idx) = self.tabs.iter().position(|&tid| tid == id) {
                self.tabs.remove(idx);
                self.registry.close(id);
                self.highlighters.remove(&id);
                // Activate the nearest remaining tab.
                self.active_buffer = if self.tabs.is_empty() {
                    None
                } else {
                    Some(self.tabs[idx.min(self.tabs.len() - 1)])
                };
            }
        }
    }

    /// Build the status line state from current app state.
    fn build_status_line(&self) -> StatusLineState {
        let (file_path, dirty, cursor_line, cursor_col) = self
            .active_buf()
            .map(|b| {
                let fp = b
                    .file_path
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string());
                let pos = &b.cursor.primary.head;
                (fp, b.is_dirty(), pos.line + 1, pos.col + 1)
            })
            .unwrap_or_default();

        StatusLineState {
            mode: self.vim.mode,
            file_path,
            dirty,
            cursor_line,
            cursor_col,
            git_branch: self.build_git_branch_display(),
            encoding: "UTF-8".to_string(),
            ai_status: String::new(), // TODO: AI integration
            file_type: self.detect_file_type(),
            message: self.status_message.clone(),
        }
    }

    /// Build the git branch display string for the status bar.
    ///
    /// Format: `branch ↑2 ↓1` (with ahead/behind counts if non-zero).
    fn build_git_branch_display(&self) -> String {
        use std::fmt::Write;

        if self.git_branch.is_empty() {
            return String::new();
        }
        let mut display = self.git_branch.clone();
        if self.git_ahead > 0 {
            let _ = write!(display, " ↑{}", self.git_ahead);
        }
        if self.git_behind > 0 {
            let _ = write!(display, " ↓{}", self.git_behind);
        }
        display
    }

    /// Detect file type from the active buffer using the language registry.
    fn detect_file_type(&self) -> String {
        self.active_buf()
            .and_then(|b| {
                let path = b.file_path.as_ref()?;
                let first_line = b.line(0);
                self.lang_registry
                    .detect(path, first_line.as_deref())
                    .map(|lid| lid.name().to_string())
            })
            .unwrap_or_default()
    }

    /// Re-run the highlighter for the active buffer after a text change.
    fn update_active_highlighter(&mut self) {
        if let Some(id) = self.active_buffer {
            // We need to borrow both the buffer (immutable) and the highlighter
            // (mutable) simultaneously, which requires splitting the borrows.
            if let (Some(buf), Some(hl)) = (self.registry.get(id), self.highlighters.get_mut(&id)) {
                hl.update(buf, None);
            }
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── rat-salsa function pointers ───────────────────────────────────────

/// Called once after terminal setup.
#[allow(clippy::missing_const_for_fn)] // rat-salsa callback signature
pub fn init(_state: &mut AppState, _global: &mut LuneGlobal) -> Result<(), Error> {
    Ok(())
}

/// Render the UI.
#[allow(clippy::cast_possible_truncation)] // TUI coords always fit u16
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    state: &mut AppState,
    _global: &mut LuneGlobal,
) -> Result<(), Error> {
    // Prune expired notifications.
    state.overlay.prune_notifications();

    // Sync tab manager from registry.
    state
        .tab_mgr
        .sync_from_registry(&state.tabs, state.active_buffer, &state.registry);

    // Compute layout.
    let splits = layout::compute_layout(area, &state.layout);
    state.last_splits = Some(splits.clone());

    // Render left panel (file tree).
    if let Some(left_area) = splits.left {
        let ws_name = state.workspace.as_ref().map_or("EXPLORER", Workspace::name);
        file_tree::render_file_tree(left_area, buf, &mut state.file_tree, ws_name);
    }

    // Render center: tab bar + editor.
    render_center(splits.center, buf, state);

    // Render right panel (git panel or AI placeholder).
    if let Some(right_area) = splits.right {
        if state.layout.show_git_panel {
            git_panel::render_git_panel(right_area, buf, &mut state.git_panel);
        } else {
            render_right_panel_placeholder(right_area, buf);
        }
    }

    // Render status bar.
    let status_state = state.build_status_line();
    status_bar::render_status_bar(splits.status, buf, &status_state);

    // Render overlays on top.
    overlay::render_overlay(area, buf, &state.overlay);

    Ok(())
}

/// Render the center area: tab bar + editor pane.
fn render_center(area: Rect, buf: &mut Buffer, state: &mut AppState) {
    if area.height < 2 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let tab_area = chunks[0];
    let content_area = chunks[1];

    // Render tab bar.
    tab_bar::render_tab_bar(tab_area, buf, &state.tab_mgr);

    // Store content area for mouse mapping.
    state.last_editor_content_area = Some(content_area);

    // Compute highlight data for visible lines (plus ±50 line buffer for scroll smoothness).
    let highlighted = state.active_buffer.and_then(|id| {
        let viewport_height = content_area.height as usize;
        let top = state.viewport.top_line.saturating_sub(50);
        let end = state.viewport.top_line + viewport_height + 50;
        state
            .highlighters
            .get(&id)
            .map(|hl| hl.highlight_lines(top..end))
    });

    // Render editor pane.
    let text_buf = state.active_buffer.and_then(|id| state.registry.get(id));
    let active_gutter = state
        .active_buffer
        .and_then(|id| state.gutter_marks.get(&id));
    editor_pane::render_editor_pane(
        content_area,
        buf,
        text_buf,
        &mut state.viewport,
        state.vim.mode,
        highlighted.as_deref(),
        &state.syntax_theme,
        active_gutter,
    );
}

/// Render a right panel placeholder (AI/Git).
fn render_right_panel_placeholder(area: Rect, buf: &mut Buffer) {
    if area.height == 0 {
        return;
    }
    let label = " AI TERMINAL";
    Line::from(Span::from(label).bold()).render(Rect::new(area.x, area.y, area.width, 1), buf);

    if area.height > 1 {
        Line::from(Span::from(" (Coming in Plan 08)").dim())
            .render(Rect::new(area.x, area.y + 1, area.width, 1), buf);
    }
}

// ── Event handling ────────────────────────────────────────────────────

/// Handle events from the event loop.
pub fn event(
    event: &AppEvent,
    state: &mut AppState,
    _global: &mut LuneGlobal,
) -> Result<Control<AppEvent>, Error> {
    match event {
        AppEvent::Terminal(ct_event) => Ok(handle_terminal_event(ct_event, state)),
        AppEvent::Timer(_timeout) => {
            // Prune notifications on timer ticks.
            let had = !state.overlay.notifications.is_empty();
            state.overlay.prune_notifications();
            if had && state.overlay.notifications.is_empty() {
                Ok(Control::Changed)
            } else {
                Ok(Control::Continue)
            }
        }
        AppEvent::Command(cmd) => Ok(handle_command(cmd, state)),
        AppEvent::Fs(fs_event) => Ok(handle_fs_event(fs_event, state)),
        AppEvent::Ai(_) => Ok(Control::Continue),
    }
}

/// Handle file system events (from watcher).
fn handle_fs_event(fs_event: &crate::event::FsEvent, state: &mut AppState) -> Control<AppEvent> {
    let path = match fs_event {
        crate::event::FsEvent::Changed(p)
        | crate::event::FsEvent::Created(p)
        | crate::event::FsEvent::Deleted(p) => p,
    };

    // Invalidate workspace cache for the parent directory and refresh.
    if let Some(ref mut ws) = state.workspace {
        if let Some(parent) = path.parent() {
            ws.invalidate(parent);
        }
        if let Err(e) = state.file_tree.refresh(ws) {
            log::error!("Failed to refresh file tree after fs event: {e}");
        }
    }

    // Refresh git status on file changes (but not too frequently —
    // the timer handles periodic refresh).
    state.refresh_git();

    Control::Changed
}

/// Handle crossterm terminal events.
fn handle_terminal_event(ct_event: &CtEvent, state: &mut AppState) -> Control<AppEvent> {
    match ct_event {
        CtEvent::Key(key_event) if key_event.kind == KeyEventKind::Press => {
            handle_key_event(key_event, state)
        }
        CtEvent::Mouse(mouse_event) => handle_mouse_event(*mouse_event, state),
        CtEvent::Resize(_, _) => Control::Changed,
        _ => Control::Continue,
    }
}

/// Handle a key press event.
fn handle_key_event(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    // 1. If overlay is active, route to overlay handler.
    if state.overlay.is_active() {
        return handle_overlay_key(key, state);
    }

    // 2. Check global keybindings.
    if let Some(cmd) = state.keymap.lookup(key) {
        return Control::Event(AppEvent::Command(cmd.clone()));
    }

    // 3. Escape: return to editor if in file tree or git panel, else normal mode.
    if key.code == KeyCode::Esc {
        if state.focus.is_focused(PanelId::FileTree) || state.focus.is_focused(PanelId::GitPanel) {
            state.focus.focus(PanelId::Editor);
            return Control::Changed;
        }
        state.vim.enter_normal();
        state.status_message.clear();
        return Control::Changed;
    }

    // 4. Route to file tree if focused.
    if state.focus.is_focused(PanelId::FileTree) {
        return handle_file_tree_key(key, state);
    }

    // 4b. Route to git panel if focused.
    if state.focus.is_focused(PanelId::GitPanel) {
        return handle_git_panel_key(key, state);
    }

    // 5. Route based on vim mode.
    match state.vim.mode {
        VimMode::Insert => handle_insert_mode(key, state),
        VimMode::Normal => handle_normal_mode(key, state),
        VimMode::Visual | VimMode::VisualLine => handle_visual_mode(key, state),
        VimMode::Command => Control::Continue, // TODO: command-line mode
    }
}

/// Handle keys when an overlay is active.
fn handle_overlay_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match &state.overlay.active {
        Some(overlay::OverlayKind::CommandPalette) => handle_palette_key(key, state),
        Some(overlay::OverlayKind::ConfirmDialog { on_confirm, .. }) => {
            let cmd = on_confirm.clone();
            match key.code {
                KeyCode::Enter => {
                    state.overlay.close();
                    state.focus.focus_return();
                    Control::Event(AppEvent::Command(cmd))
                }
                KeyCode::Esc => {
                    state.overlay.close();
                    state.focus.focus_return();
                    Control::Changed
                }
                _ => Control::Continue,
            }
        }
        Some(overlay::OverlayKind::FindReplace) => {
            if key.code == KeyCode::Esc {
                state.overlay.close();
                state.focus.focus_return();
                Control::Changed
            } else {
                Control::Continue
            }
        }
        Some(overlay::OverlayKind::FilePicker) => handle_file_picker_key(key, state),
        None => Control::Continue,
    }
}

/// Handle keys in the command palette.
fn handle_palette_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            state.overlay.close();
            state.focus.focus_return();
            Control::Changed
        }
        KeyCode::Enter => {
            if let Some(cmd) = state.overlay.command_palette.selected_command().cloned() {
                state.overlay.close();
                state.focus.focus_return();
                Control::Event(AppEvent::Command(cmd))
            } else {
                Control::Changed
            }
        }
        KeyCode::Up => {
            state.overlay.command_palette.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.command_palette.select_next();
            Control::Changed
        }
        KeyCode::Backspace => {
            state.overlay.command_palette.backspace();
            Control::Changed
        }
        KeyCode::Char(ch) => {
            state.overlay.command_palette.type_char(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Handle keys in the file picker overlay.
fn handle_file_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            state.overlay.close();
            state.focus.focus_return();
            Control::Changed
        }
        KeyCode::Enter => handle_file_picker_enter(state),
        KeyCode::Up => {
            state.overlay.file_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.file_picker.select_next();
            Control::Changed
        }
        KeyCode::Backspace => {
            // If input is empty, go up one directory.
            if !state.overlay.file_picker.backspace() {
                state.overlay.file_picker.go_up();
            }
            Control::Changed
        }
        KeyCode::Char(ch) => {
            state.overlay.file_picker.type_char(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Handle Enter in the file picker: open file or enter directory.
fn handle_file_picker_enter(state: &mut AppState) -> Control<AppEvent> {
    let Some(entry) = state.overlay.file_picker.selected_entry().cloned() else {
        return Control::Continue;
    };

    if entry.is_dir {
        // Navigate into the directory.
        state.overlay.file_picker.enter_directory(&entry.path);
        Control::Changed
    } else {
        // Open the file and close the picker.
        let path = entry.path;
        state.overlay.close();
        state.focus.focus_return();
        Control::Event(AppEvent::Command(AppCommand::OpenFile(path)))
    }
}

/// Handle key events when the file tree is focused.
fn handle_file_tree_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        // Navigation: j/down, k/up.
        KeyCode::Char('j') | KeyCode::Down => {
            state.file_tree.select_next(1);
            Control::Changed
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.file_tree.select_prev(1);
            Control::Changed
        }
        // Enter: open file or toggle directory.
        KeyCode::Enter => handle_file_tree_enter(state),
        // l/Right: expand directory.
        KeyCode::Char('l') | KeyCode::Right => handle_file_tree_expand(state),
        // h/Left: collapse directory (or go to parent).
        KeyCode::Char('h') | KeyCode::Left => handle_file_tree_collapse(state),
        // Toggle hidden files.
        KeyCode::Char('H') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Control::Event(AppEvent::Command(AppCommand::ToggleHiddenFiles))
        }
        // File operations.
        KeyCode::Char('n') => Control::Event(AppEvent::Command(AppCommand::NewFile)),
        KeyCode::Char('N') => Control::Event(AppEvent::Command(AppCommand::NewDir)),
        KeyCode::Char('r') => Control::Event(AppEvent::Command(AppCommand::RenameEntry)),
        KeyCode::Char('d') => Control::Event(AppEvent::Command(AppCommand::DeleteEntry)),
        _ => Control::Continue,
    }
}

/// Handle Enter in the file tree.
fn handle_file_tree_enter(state: &mut AppState) -> Control<AppEvent> {
    let Some((_, entry)) = state.file_tree.selected_entry().cloned() else {
        return Control::Continue;
    };

    match entry.kind {
        EntryKind::File | EntryKind::Symlink => {
            // Open file in editor.
            Control::Event(AppEvent::Command(AppCommand::OpenFile(entry.path)))
        }
        EntryKind::Directory { .. } => {
            // Toggle expand/collapse.
            toggle_selected_dir(state)
        }
    }
}

/// Handle expand (l/Right) in the file tree.
fn handle_file_tree_expand(state: &mut AppState) -> Control<AppEvent> {
    if state.file_tree.selected_is_dir() {
        if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
            if let Some(ref mut ws) = state.workspace {
                ws.set_expanded(&path, true);
                state.refresh_file_tree();
            }
        }
    }
    Control::Changed
}

/// Handle collapse (h/Left) in the file tree.
fn handle_file_tree_collapse(state: &mut AppState) -> Control<AppEvent> {
    if state.file_tree.selected_is_dir() {
        if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
            if let Some(ref mut ws) = state.workspace {
                ws.set_expanded(&path, false);
                state.refresh_file_tree();
            }
        }
    }
    Control::Changed
}

// ── Git panel key handling ────────────────────────────────────────

/// Handle key events when the git panel is focused.
fn handle_git_panel_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        // Navigation: j/down, k/up.
        KeyCode::Char('j') | KeyCode::Down => {
            state.git_panel.select_next();
            Control::Changed
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.git_panel.select_prev();
            Control::Changed
        }
        // Stage selected file.
        KeyCode::Char('s') => Control::Event(AppEvent::Command(AppCommand::GitStage)),
        // Unstage selected file.
        KeyCode::Char('u') => Control::Event(AppEvent::Command(AppCommand::GitUnstage)),
        // Discard changes.
        KeyCode::Char('d') => Control::Event(AppEvent::Command(AppCommand::GitDiscard)),
        // Commit.
        KeyCode::Char('c') => Control::Event(AppEvent::Command(AppCommand::GitCommit)),
        // Refresh.
        KeyCode::Char('r') => Control::Event(AppEvent::Command(AppCommand::GitRefresh)),
        // Enter: open diff view (TODO: wire to diff view widget).
        KeyCode::Enter => {
            // For now, just open the selected file in the editor.
            if let Some(file) = state.git_panel.selected_file().cloned() {
                if let Some(git) = &state.git_service {
                    let abs_path = git.root().join(&file.path);
                    return Control::Event(AppEvent::Command(AppCommand::OpenFile(abs_path)));
                }
            }
            Control::Continue
        }
        _ => Control::Continue,
    }
}

/// Toggle expand/collapse on the selected directory.
fn toggle_selected_dir(state: &mut AppState) -> Control<AppEvent> {
    if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
        if let Some(ref mut ws) = state.workspace {
            ws.toggle_expanded(&path);
            state.refresh_file_tree();
        }
    }
    Control::Changed
}

/// Handle key events in insert mode — characters are inserted.
fn handle_insert_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    let result = match key.code {
        KeyCode::Char(ch) => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                buf.insert(pos, &ch.to_string());
            }
            Control::Changed
        }
        KeyCode::Enter => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "\n");
            }
            Control::Changed
        }
        KeyCode::Backspace => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                if pos.col > 0 {
                    let start = Position::new(pos.line, pos.col - 1);
                    buf.delete(start, pos);
                } else if pos.line > 0 {
                    let prev_line_len = buf.line_len(pos.line - 1).saturating_sub(1);
                    let start = Position::new(pos.line - 1, prev_line_len);
                    buf.delete(start, pos);
                }
            }
            Control::Changed
        }
        KeyCode::Delete => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                let end = Position::new(pos.line, pos.col + 1);
                buf.delete(pos, end);
            }
            Control::Changed
        }
        KeyCode::Left => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_left(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::Right => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_right(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::Up => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_up(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::Down => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_down(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::Home => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_line_start(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::End => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_line_end(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            Control::Changed
        }
        KeyCode::Tab => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "    "); // 4-space tabs
            }
            Control::Changed
        }
        _ => Control::Continue,
    };

    // Update highlighter after text-mutating keys.
    if matches!(
        key.code,
        KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab
    ) {
        state.update_active_highlighter();
    }

    result
}

/// Handle key events in normal mode — characters are vim commands.
fn handle_normal_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if let KeyCode::Char(ch) = key.code {
        let dummy = TextBuffer::new();
        let buf = state
            .active_buffer
            .and_then(|id| state.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action(&action, state)
    } else {
        handle_arrow_keys(key, state, false)
    }
}

/// Handle key events in visual mode.
fn handle_visual_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if let KeyCode::Char(ch) = key.code {
        let dummy = TextBuffer::new();
        let buf = state
            .active_buffer
            .and_then(|id| state.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action_visual(&action, state)
    } else {
        Control::Continue
    }
}

/// Handle arrow keys (shared between normal/insert modes).
fn handle_arrow_keys(key: &KeyEvent, state: &mut AppState, extend: bool) -> Control<AppEvent> {
    match key.code {
        KeyCode::Left => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_left(extend);
            }
            Control::Changed
        }
        KeyCode::Right => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_right(extend);
            }
            Control::Changed
        }
        KeyCode::Up => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_up(extend);
            }
            Control::Changed
        }
        KeyCode::Down => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_down(extend);
            }
            Control::Changed
        }
        _ => Control::Continue,
    }
}

// ── Mouse handling ────────────────────────────────────────────────────

/// Handle mouse events.
#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_event(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_mouse_click(mouse, state),
        MouseEventKind::Drag(MouseButton::Left) => handle_mouse_drag(mouse, state),
        MouseEventKind::Up(MouseButton::Left) => {
            state.dragging_border = None;
            Control::Continue
        }
        MouseEventKind::ScrollUp => {
            state.viewport.scroll_up(3);
            Control::Changed
        }
        MouseEventKind::ScrollDown => {
            let total = state
                .active_buf()
                .map_or(0, lune_core::buffer::TextBuffer::line_count);
            let height = state
                .last_editor_content_area
                .map_or(20, |a| a.height as usize);
            state.viewport.scroll_down(3, total, height);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Handle left mouse button click.
#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_click(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let col = mouse.column;
    let row = mouse.row;

    // Check if clicking on panel borders (start drag).
    if let Some(ref splits) = state.last_splits {
        if layout::is_on_left_border(splits, col) {
            state.dragging_border = Some(DragBorder::Left);
            return Control::Continue;
        }
        if layout::is_on_right_border(splits, col) {
            state.dragging_border = Some(DragBorder::Right);
            return Control::Continue;
        }
    }

    // Check if clicking in file tree area.
    if let Some(ref splits) = state.last_splits {
        if let Some(left_area) = splits.left {
            if col >= left_area.x
                && col < left_area.x + left_area.width
                && row >= left_area.y
                && row < left_area.y + left_area.height
            {
                state.focus.focus(PanelId::FileTree);
                // Detect double-click: same position within 500ms.
                let now = Instant::now();
                let is_double = state.last_click.is_some_and(|(t, c, r)| {
                    c == col && r == row && now.duration_since(t).as_millis() < 500
                });
                if let Some(idx) = state.file_tree.hit_test(row, left_area) {
                    state.file_tree.selected = idx;
                    if is_double {
                        state.last_click = None;
                        return handle_file_tree_enter(state);
                    }
                }
                state.last_click = Some((now, col, row));
                return Control::Changed;
            }
        }

        // Check if clicking in right panel area (git panel).
        if let Some(right_area) = splits.right {
            if col >= right_area.x
                && col < right_area.x + right_area.width
                && row >= right_area.y
                && row < right_area.y + right_area.height
                && state.layout.show_git_panel
            {
                state.focus.focus(PanelId::GitPanel);
                return Control::Changed;
            }
        }
    }

    // Check if clicking in editor content area (set cursor position).
    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .active_buffer
            .is_some_and(|id| state.gutter_marks.contains_key(&id));
        if let Some(pos) = editor_pane::click_to_position(
            col,
            row,
            content_area,
            &state.viewport,
            total_lines,
            has_git,
        ) {
            if let Some(buf) = state.active_buf_mut() {
                let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                let clamped_col = pos.col.min(buf.line_len(clamped_line).saturating_sub(1));
                buf.cursor = CursorState::at(Position::new(clamped_line, clamped_col));
            }
            return Control::Changed;
        }
    }

    // Check if clicking on a tab.
    if let Some(ref splits) = state.last_splits {
        // Tab area is the first row of the center area.
        let tab_area = Rect::new(splits.center.x, splits.center.y, splits.center.width, 1);
        if row == tab_area.y {
            if let Some((idx, is_close)) = state.tab_mgr.hit_test(col, tab_area.x, tab_area.width) {
                if is_close {
                    // Close the clicked tab.
                    if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                        if let Some(tab_idx) = state.tabs.iter().position(|&id| id == bid) {
                            state.tabs.remove(tab_idx);
                            state.registry.close(bid);
                            state.highlighters.remove(&bid);
                            if state.active_buffer == Some(bid) {
                                state.active_buffer = if state.tabs.is_empty() {
                                    None
                                } else {
                                    Some(state.tabs[tab_idx.min(state.tabs.len() - 1)])
                                };
                            }
                        }
                    }
                } else if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                    // Switch to clicked tab.
                    state.active_buffer = Some(bid);
                }
                return Control::Changed;
            }
        }
    }

    Control::Continue
}

/// Handle mouse drag for panel resizing.
#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_drag(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let Some(border) = state.dragging_border else {
        return Control::Continue;
    };

    let Some(ref splits) = state.last_splits else {
        return Control::Continue;
    };

    let total_width = splits.status.width; // Full terminal width.
    if total_width == 0 {
        return Control::Continue;
    }

    let pct = ((u32::from(mouse.column)) * 100 / u32::from(total_width)) as u16;

    match border {
        DragBorder::Left => state.layout.set_file_tree_width_pct(pct),
        DragBorder::Right => {
            let right_pct = 100u16.saturating_sub(pct);
            state.layout.set_right_panel_width_pct(right_pct);
        }
    }

    Control::Changed
}

// ── Vim action dispatch ───────────────────────────────────────────────

/// Apply a vim action to the editor state.
fn apply_vim_action(action: &VimAction, state: &mut AppState) -> Control<AppEvent> {
    match action {
        VimAction::ModeChanged(_) => Control::Changed,
        VimAction::MoveLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_left);
        }),
        VimAction::MoveRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_right);
        }),
        VimAction::MoveUp(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_up);
        }),
        VimAction::MoveDown(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_down);
        }),
        VimAction::MoveWordRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_word_right);
        }),
        VimAction::MoveWordLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, false, TextBuffer::move_word_left);
        }),
        VimAction::MoveLineStart => apply_motion(state, |buf| buf.move_line_start(false)),
        VimAction::MoveLineEnd => apply_motion(state, |buf| buf.move_line_end(false)),
        VimAction::MoveBufferEnd => apply_motion(state, |buf| buf.move_buffer_end(false)),
        VimAction::MoveToLine(line) => {
            if let Some(buf) = state.active_buf_mut() {
                let clamped = (*line).min(buf.line_count().saturating_sub(1));
                buf.cursor = CursorState::at(Position::new(clamped, 0));
            }
            Control::Changed
        }
        VimAction::OpenLineBelow => {
            if let Some(buf) = state.active_buf_mut() {
                buf.move_line_end(false);
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "\n");
            }
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::OpenLineAbove => {
            if let Some(buf) = state.active_buf_mut() {
                let line = buf.cursor.primary.head.line;
                let pos = Position::new(line, 0);
                buf.insert(pos, "\n");
                buf.cursor = CursorState::at(pos);
            }
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::DeleteCharForward(n) => {
            if let Some(buf) = state.active_buf_mut() {
                for _ in 0..*n {
                    let pos = buf.cursor.primary.head;
                    let end = Position::new(pos.line, pos.col + 1);
                    buf.delete(pos, end);
                }
            }
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::DeleteLine(n) => {
            if let Some(buf) = state.active_buf_mut() {
                for _ in 0..*n {
                    let line = buf.cursor.primary.head.line;
                    let start = Position::new(line, 0);
                    let end = if line + 1 < buf.line_count() {
                        Position::new(line + 1, 0)
                    } else {
                        let len = buf.line_len(line);
                        Position::new(line, len)
                    };
                    buf.delete(start, end);
                }
            }
            state.update_active_highlighter();
            Control::Changed
        }
        VimAction::Undo => {
            if let Some(buf) = state.active_buf_mut() {
                buf.undo();
            }
            state.update_active_highlighter();
            Control::Changed
        }
        // TODO: implement remaining actions (YankLine, ChangeLine, *Motion).
        _ => Control::Continue,
    }
}

/// Apply a vim action in visual mode (motions extend selection).
fn apply_vim_action_visual(action: &VimAction, state: &mut AppState) -> Control<AppEvent> {
    match action {
        VimAction::MoveLeft(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_left);
        }),
        VimAction::MoveRight(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_right);
        }),
        VimAction::MoveUp(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_up);
        }),
        VimAction::MoveDown(n) => apply_motion(state, |buf| {
            move_n(buf, *n, true, TextBuffer::move_down);
        }),
        VimAction::ModeChanged(VimMode::Normal | VimMode::Insert) => Control::Changed,
        _ => Control::Continue,
    }
}

/// Helper: apply a closure to the active buffer and return `Changed`.
fn apply_motion(state: &mut AppState, f: impl FnOnce(&mut TextBuffer)) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        f(buf);
    }
    Control::Changed
}

/// Helper: repeat a motion method N times.
fn move_n(buf: &mut TextBuffer, n: usize, extend: bool, method: fn(&mut TextBuffer, bool)) {
    for _ in 0..n {
        method(buf, extend);
    }
}

// ── Command handling ──────────────────────────────────────────────────

/// Handle application commands.
fn handle_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    match cmd {
        AppCommand::Quit | AppCommand::ForceQuit => Control::Quit,
        AppCommand::Save => handle_save(state),
        AppCommand::SaveAll => handle_save_all(state),
        AppCommand::CloseTab => {
            state.close_active_tab();
            Control::Changed
        }
        AppCommand::NextTab => {
            state.next_tab();
            Control::Changed
        }
        AppCommand::PrevTab => {
            state.prev_tab();
            Control::Changed
        }
        AppCommand::ToggleFileTree => {
            state.layout.toggle_file_tree();
            Control::Changed
        }
        AppCommand::ToggleAiPanel => {
            state.layout.toggle_ai_panel();
            Control::Changed
        }
        AppCommand::ToggleGitPanel => {
            state.layout.toggle_git_panel();
            if state.layout.show_git_panel {
                state.focus.focus(PanelId::GitPanel);
                // Refresh git status when opening the panel.
                state.refresh_git();
            } else {
                state.focus.focus(PanelId::Editor);
            }
            Control::Changed
        }
        AppCommand::OpenCommandPalette => {
            state.overlay.open_command_palette();
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::OpenFilePicker => {
            let start_dir = state.workspace.as_ref().map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                |ws| ws.root().to_path_buf(),
            );
            state.overlay.open_file_picker(&start_dir);
            state.focus.focus(PanelId::CommandPalette); // Reuse overlay focus
            Control::Changed
        }
        AppCommand::Undo => {
            if let Some(buf) = state.active_buf_mut() {
                buf.undo();
            }
            state.update_active_highlighter();
            Control::Changed
        }
        AppCommand::Redo => {
            if let Some(buf) = state.active_buf_mut() {
                buf.redo();
            }
            state.update_active_highlighter();
            Control::Changed
        }
        AppCommand::EnterNormalMode => {
            state.vim.enter_normal();
            Control::Changed
        }
        AppCommand::EnterInsertMode => {
            state.vim.enter_insert();
            Control::Changed
        }
        AppCommand::EnterVisualMode => {
            state.vim.enter_visual();
            Control::Changed
        }
        // File tree & workspace commands.
        AppCommand::OpenFile(path) => handle_open_file(path, state),
        AppCommand::ToggleHiddenFiles
        | AppCommand::RevealInFileTree(_)
        | AppCommand::NewFile
        | AppCommand::NewDir
        | AppCommand::RenameEntry
        | AppCommand::DeleteEntry => handle_file_tree_command(cmd, state),
        AppCommand::Find | AppCommand::Replace => {
            // TODO: implement find/replace
            Control::Continue
        }
        AppCommand::ChangeLanguage(lang_id) => handle_change_language(*lang_id, state),
        // Git operations.
        AppCommand::GitStage => handle_git_stage(state),
        AppCommand::GitUnstage => handle_git_unstage(state),
        AppCommand::GitCommit => handle_git_commit(state),
        AppCommand::GitDiscard => handle_git_discard(state),
        AppCommand::GitRefresh => {
            state.refresh_git();
            Control::Changed
        }
        AppCommand::GitDiscardConfirmed(path) => handle_git_discard_confirmed(path, state),
    }
}

/// Handle save command.
fn handle_save(state: &mut AppState) -> Control<AppEvent> {
    if let Some(buf) = state.active_buf_mut() {
        match buf.save() {
            Ok(()) => {
                state.status_message = "Saved.".to_string();
                state.overlay.notify("File saved", NotificationLevel::Info);
            }
            Err(e) => {
                state.status_message = format!("Save failed: {e}");
                state
                    .overlay
                    .notify(format!("Save failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    // Refresh git status after save to update gutter marks and file tree.
    state.refresh_git();
    Control::Changed
}

/// Handle save-all command.
fn handle_save_all(state: &mut AppState) -> Control<AppEvent> {
    let ids: Vec<_> = state.tabs.clone();
    let mut saved = 0;
    let mut errors = 0;
    for id in ids {
        if let Some(buf) = state.registry.get_mut(id) {
            if buf.is_dirty() {
                match buf.save() {
                    Ok(()) => saved += 1,
                    Err(_) => errors += 1,
                }
            }
        }
    }
    state.status_message = format!("Saved {saved} file(s), {errors} error(s).");
    state
        .overlay
        .notify(format!("Saved {saved} file(s)"), NotificationLevel::Info);
    Control::Changed
}

/// Handle file-tree–related commands.
fn handle_file_tree_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    match cmd {
        AppCommand::ToggleHiddenFiles => {
            if let Some(ref mut ws) = state.workspace {
                ws.toggle_hidden();
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
            }
            Control::Changed
        }
        AppCommand::RevealInFileTree(path) => {
            let path = path.clone();
            if let Some(ref mut ws) = state.workspace {
                if let Err(e) = state.file_tree.reveal_path(&path, ws) {
                    log::error!("Failed to reveal path: {e}");
                }
                if let Err(e) = state.file_tree.refresh(ws) {
                    log::error!("Failed to refresh file tree: {e}");
                }
                // Select the revealed path (estimate visible height as 20).
                state.file_tree.select_by_path(&path, 20);
            }
            Control::Changed
        }
        AppCommand::NewFile | AppCommand::NewDir => {
            // TODO: implement input dialog for file/dir name
            state
                .overlay
                .notify("Input dialogs not yet implemented", NotificationLevel::Info);
            Control::Changed
        }
        AppCommand::RenameEntry => {
            // TODO: implement rename input dialog
            state
                .overlay
                .notify("Rename not yet implemented", NotificationLevel::Info);
            Control::Changed
        }
        AppCommand::DeleteEntry => {
            // TODO: implement delete confirmation dialog
            state
                .overlay
                .notify("Delete not yet implemented", NotificationLevel::Info);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Handle the `OpenFile` command: open a file and switch to editor.
fn handle_open_file(path: &std::path::Path, state: &mut AppState) -> Control<AppEvent> {
    match state.open_file(path) {
        Ok(_) => {
            state.focus.focus(PanelId::Editor);
            state.status_message = format!("Opened: {}", path.display());
        }
        Err(e) => {
            state
                .overlay
                .notify(format!("Open failed: {e}"), NotificationLevel::Error);
            state.status_message = format!("Open failed: {e}");
        }
    }
    Control::Changed
}

/// Handle the `ChangeLanguage` command: re-assign the highlighter for the active buffer.
fn handle_change_language(lang_id: LanguageId, state: &mut AppState) -> Control<AppEvent> {
    let Some(id) = state.active_buffer else {
        return Control::Continue;
    };

    // Create a new highlighter for the requested language.
    let mut hl = highlight::create_highlighter(lang_id);
    if let Some(buf) = state.registry.get(id) {
        hl.update(buf, None);
    }
    state.highlighters.insert(id, hl);
    state.status_message = format!("Language: {}", lang_id.name());
    Control::Changed
}

/// Handle git stage: stage the selected file in the git panel.
fn handle_git_stage(state: &mut AppState) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };

    if let Some(ref git) = state.git_service {
        match git.stage(&file.path) {
            Ok(()) => {
                state.status_message = format!("Staged: {}", file.path.display());
                state.refresh_git();
            }
            Err(e) => {
                state.status_message = format!("Stage failed: {e}");
                state
                    .overlay
                    .notify(format!("Stage failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

/// Handle git unstage: unstage the selected file in the git panel.
fn handle_git_unstage(state: &mut AppState) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };

    if let Some(ref git) = state.git_service {
        match git.unstage(&file.path) {
            Ok(()) => {
                state.status_message = format!("Unstaged: {}", file.path.display());
                state.refresh_git();
            }
            Err(e) => {
                state.status_message = format!("Unstage failed: {e}");
                state
                    .overlay
                    .notify(format!("Unstage failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

/// Handle git commit: commit staged changes.
///
/// For now, uses a hardcoded message. In the future, this should open
/// an input overlay for the commit message.
fn handle_git_commit(state: &mut AppState) -> Control<AppEvent> {
    // Check if there are staged files.
    let has_staged = state
        .git_panel
        .status
        .as_ref()
        .is_some_and(|s| s.files.iter().any(|f| f.staged));

    if !has_staged {
        state
            .overlay
            .notify("Nothing staged to commit", NotificationLevel::Info);
        return Control::Changed;
    }

    // TODO: open an input overlay for the commit message.
    // For now, show a notification that the feature needs an input dialog.
    state.overlay.notify(
        "Commit message input not yet implemented. Use terminal for now.",
        NotificationLevel::Info,
    );
    Control::Changed
}

/// Handle git discard: discard changes to the selected file (with confirmation).
fn handle_git_discard(state: &mut AppState) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };

    // Require confirmation via overlay dialog.
    state.overlay.open_confirm(
        format!("Discard changes to {}?", file.path.display()),
        AppCommand::GitDiscardConfirmed(file.path),
    );
    state.focus.focus(PanelId::CommandPalette); // Use overlay focus
    Control::Changed
}

/// Handle confirmed git discard.
fn handle_git_discard_confirmed(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    if let Some(ref git) = state.git_service {
        match git.discard_file(path) {
            Ok(()) => {
                state.status_message = format!("Discarded: {}", path.display());
                state.refresh_git();
            }
            Err(e) => {
                state.status_message = format!("Discard failed: {e}");
                state
                    .overlay
                    .notify(format!("Discard failed: {e}"), NotificationLevel::Error);
            }
        }
    }
    Control::Changed
}

/// Handle errors from the event loop.
#[allow(clippy::needless_pass_by_value)] // rat-salsa callback requires owned Error
pub fn error(
    err: Error,
    state: &mut AppState,
    _global: &mut LuneGlobal,
) -> Result<Control<AppEvent>, Error> {
    state.error_count += 1;
    state.last_error = format!("{err}");
    state.status_message = format!("Error: {err}");
    state
        .overlay
        .notify(format!("Error: {err}"), NotificationLevel::Error);
    log::error!("Application error: {err}");
    Ok(Control::Changed)
}

// ── File watcher poll integration ─────────────────────────────────────

/// Event source that polls the file watcher channel for `WatchEvent`s
/// and converts them to `AppEvent::Fs(_)`.
pub struct PollFileWatcher {
    rx: Receiver<WatchEvent>,
    pending: Option<WatchEvent>,
}

impl PollFileWatcher {
    /// Create a new poller from a watcher event receiver.
    #[must_use]
    pub const fn new(rx: Receiver<WatchEvent>) -> Self {
        Self { rx, pending: None }
    }
}

impl rat_salsa::poll::PollEvents<AppEvent, Error> for PollFileWatcher {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn poll(&mut self) -> Result<bool, Error> {
        match self.rx.try_recv() {
            Ok(event) => {
                self.pending = Some(event);
                Ok(true)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(false),
        }
    }

    fn read(&mut self) -> Result<Control<AppEvent>, Error> {
        self.pending.take().map_or(Ok(Control::Continue), |event| {
            let fs_event = match event {
                WatchEvent::Created(path) => crate::event::FsEvent::Created(path),
                WatchEvent::Modified(path) => crate::event::FsEvent::Changed(path),
                WatchEvent::Deleted(path) => crate::event::FsEvent::Deleted(path),
                WatchEvent::Renamed { from: _, to } => {
                    // Treat rename as a create of the new path.
                    // The old path will be handled as a delete by the watcher
                    // if it emits separate events.
                    crate::event::FsEvent::Created(to)
                }
            };
            Ok(Control::Event(AppEvent::Fs(fs_event)))
        })
    }
}

/// Run the Lune Editor TUI event loop.
///
/// # Errors
/// Returns an error if the terminal cannot be initialized or the event
/// loop encounters an unrecoverable error.
pub fn run(state: &mut AppState) -> Result<(), Error> {
    let mut global = LuneGlobal::default();
    let watcher_rx = state.watcher_receiver();

    run_tui(
        init,
        render,
        event,
        error,
        &mut global,
        state,
        RunConfig::default()?
            .poll(PollCrossterm)
            .poll(PollTimers::default())
            .poll(PollFileWatcher::new(watcher_rx)),
    )?;

    Ok(())
}
