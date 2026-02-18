//! Application state and rat-salsa integration.
//!
//! This module contains the global context (`LuneGlobal`) and application
//! state (`AppState`) used by the rat-salsa event loop, plus the four
//! function pointers required by `run_tui`.

use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Error;
use crossbeam::channel::{self, Receiver, TryRecvError};
use rat_salsa::poll::{PollCrossterm, PollTimers};
use rat_salsa::{run_tui, Control, RunConfig, SalsaAppContext, SalsaContext};

use crate::primitives::{
    Buffer, Constraint, CtEvent, Direction, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, Layout,
    MouseButton, MouseEvent, MouseEventKind, Rect,
};

use lune_core::prelude::*;
use lune_core::ropey::Rope;
use lune_core::settings::Settings;
use lune_core::watcher::{FileWatcher, WatchEvent};
use lune_core::workspace::EntryKind;
use lune_core::workspace_state::make_relative;
use lune_git::{GitService, GutterMarks};

use lune_ai::context::{
    extract_selection_text, EditorContext, FileContext, GitStatusSummary, SelectionContext,
    TabContext,
};
use lune_ai::{AiClientKind, AiManager, LiveModeController, TermSize as AiTermSize};

use crate::highlight;
use crate::highlight::theme::SyntaxTheme;
use crate::theme::Theme;
use crate::theme_config::ThemeRegistry;

use crate::effects::LuneEffects;
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
use crate::widgets::terminal;

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
    /// Whether vim keybindings are enabled. When `false`, only Normal↔Insert
    /// mode switching is active (Escape blocks typing, `i` resumes);
    /// `Visual`, `VisualLine`, and `Command` modes are disabled.
    pub vim_enabled: bool,
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
    highlighters: FxHashMap<BufferId, Box<dyn Highlighter>>,
    /// Language detection registry.
    lang_registry: LanguageRegistry,
    /// Syntax color theme (copied from active theme in registry for fast access).
    syntax_theme: SyntaxTheme,
    /// UI design tokens (copied from active theme in registry for fast access).
    pub theme: Theme,
    /// Theme registry — holds all loaded themes for instant switching.
    pub theme_registry: ThemeRegistry,
    /// Git service (active when workspace is in a git repository).
    git_service: Option<GitService>,
    /// Per-buffer git gutter marks (cached).
    gutter_marks: FxHashMap<BufferId, GutterMarks>,
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
    /// Visual effects manager (tachyonfx).
    pub effects: LuneEffects,
    /// Timestamp of the last render (for effect timing).
    last_render: Instant,
    /// Whether focus has changed and the focus glow needs updating.
    focus_dirty: bool,
    /// AI session manager.
    pub ai_manager: AiManager,
    /// Last known AI terminal size (to avoid redundant resizes).
    last_ai_term_size: Option<AiTermSize>,
    /// Live Mode controller (diff tracking, accept/reject state).
    pub live_mode: LiveModeController,
    /// Whether the AI thinking effect is currently active.
    ai_thinking_active: bool,
    /// Notification count at last render (for detecting new notifications).
    last_notification_count: usize,
    /// Config directory paths (for settings/recovery/workspace state I/O).
    ///
    /// Set after construction via [`AppState::set_config_paths`].
    config_paths: Option<lune_core::config::ConfigPaths>,
    /// Cached settings for hot-reload comparison and re-application.
    cached_settings: Option<Settings>,
    /// Sled-backed reactive state database.
    ///
    /// Set after construction via [`AppState::set_state_db`].  When present,
    /// workspace state is persisted on a debounced timer (~2 s).
    state_db: Option<StateDb>,
    /// Timestamp of the last successful state-db save (for debounce).
    last_state_save: Instant,
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
            vim_enabled: false, // default off; set by apply_settings()
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
            highlighters: FxHashMap::default(),
            lang_registry: LanguageRegistry::new(),
            syntax_theme: SyntaxTheme::dark(),
            theme: Theme::dark(),
            theme_registry: ThemeRegistry::new(),
            git_service: None,
            gutter_marks: FxHashMap::default(),
            git_branch: String::new(),
            git_ahead: 0,
            git_behind: 0,
            git_panel: GitPanelState::new(),
            last_click: None,
            effects: LuneEffects::new(),
            last_render: Instant::now(),
            focus_dirty: true,
            ai_manager: AiManager::new(),
            last_ai_term_size: None,
            live_mode: LiveModeController::new(),
            ai_thinking_active: false,
            last_notification_count: 0,
            config_paths: None,
            cached_settings: None,
            state_db: None,
            last_state_save: Instant::now(),
        }
    }

    /// Switch the active theme from the registry and update cached copies.
    ///
    /// Call this instead of modifying `theme` or `syntax_theme` directly.
    /// The registry performs the lookup; we cache the results into the
    /// flat `theme` and `syntax_theme` fields for zero-cost access during
    /// rendering.
    pub fn apply_active_theme(&mut self) {
        self.theme = *self.theme_registry.current_theme();
        self.syntax_theme = self.theme_registry.current_syntax().clone();
    }

    /// Switch to the next theme in the registry, wrapping around.
    pub fn next_theme(&mut self) {
        self.theme_registry.next();
        self.apply_active_theme();
    }

    /// Switch to the previous theme in the registry, wrapping around.
    pub fn prev_theme(&mut self) {
        self.theme_registry.prev();
        self.apply_active_theme();
    }

    /// Apply loaded [`Settings`] to the application state.
    ///
    /// Should be called once after construction and settings loading,
    /// before the event loop starts.  Maps settings fields onto the
    /// corresponding `AppState` fields (layout, vim mode, effects, theme).
    pub fn apply_settings(&mut self, settings: &Settings) {
        // Layout / UI
        self.layout.show_file_tree = settings.ui.show_file_tree;
        self.layout
            .set_file_tree_width_pct(settings.ui.file_tree_width_pct);
        self.layout
            .set_right_panel_width_pct(settings.ui.right_panel_width_pct);

        if !settings.ui.effects_enabled {
            self.effects.disable_all();
        }

        // Editor / vim
        self.vim_enabled = settings.editor.vim_mode;
        if self.vim_enabled {
            self.vim.enter_normal();
        } else {
            // Non-vim mode: start in Insert so keystrokes type text by default.
            // User can still Escape → Normal to block typing, then `i` to resume.
            self.vim.enter_insert();
        }

        // Theme — try to switch by name from the settings.
        if self.theme_registry.switch_by_name(&settings.theme) {
            self.apply_active_theme();
        }

        // Cache the settings for hot-reload comparison.
        self.cached_settings = Some(settings.clone());
    }

    /// Store resolved config paths on the state for use by settings/recovery
    /// commands and autosave.
    pub fn set_config_paths(&mut self, config_paths: lune_core::config::ConfigPaths) {
        self.config_paths = Some(config_paths);
    }

    /// Borrow the cached config paths, if any.
    #[must_use]
    pub const fn config_paths(&self) -> Option<&lune_core::config::ConfigPaths> {
        self.config_paths.as_ref()
    }

    /// Store the sled-backed state database on the state.
    ///
    /// Once set, workspace state is persisted on a debounced timer
    /// (~2 seconds) during the event loop, plus a final flush on exit.
    pub fn set_state_db(&mut self, db: StateDb) {
        self.state_db = Some(db);
    }

    /// Borrow the state database, if set.
    #[must_use]
    pub const fn state_db(&self) -> Option<&StateDb> {
        self.state_db.as_ref()
    }

    /// Collect the current workspace state for persistence.
    ///
    /// Returns `None` if no workspace is open.  File paths in the
    /// returned state are stored relative to the workspace root.
    #[must_use]
    pub fn collect_workspace_state(&self) -> Option<WorkspaceState> {
        let ws = self.workspace.as_ref()?;
        let root = ws.root().to_path_buf();
        let mut wstate = WorkspaceState::new(root.clone());

        // Open files (relative paths).
        wstate.open_files = self
            .tabs
            .iter()
            .filter_map(|&id| {
                let buf = self.registry.get(id)?;
                let path = buf.file_path.as_ref()?;
                Some(make_relative(path, &root))
            })
            .collect();

        // Active file (relative).
        wstate.active_file = self.active_buf().and_then(|buf| {
            let path = buf.file_path.as_ref()?;
            Some(make_relative(path, &root))
        });

        // Cursor positions keyed by relative path.
        for &id in &self.tabs {
            if let Some(buf) = self.registry.get(id) {
                if let Some(ref path) = buf.file_path {
                    let rel = make_relative(path, &root);
                    let pos = &buf.cursor.primary.head;
                    wstate.cursor_positions.insert(rel, (pos.line, pos.col));
                }
            }
        }

        // Layout.
        wstate.show_file_tree = self.layout.show_file_tree;
        wstate.file_tree_width_pct = self.layout.file_tree_width_pct;
        wstate.show_right_panel = self.layout.show_right_panel();
        wstate.right_panel_width_pct = self.layout.right_panel_width_pct;

        Some(wstate)
    }

    /// Restore workspace state: open files, set cursors, restore layout.
    ///
    /// Skips files that no longer exist.  Must be called after
    /// `open_workspace()` so the workspace root is set.
    pub fn restore_workspace_state(&mut self, wstate: &WorkspaceState) {
        let Some(ref ws) = self.workspace else {
            return;
        };
        let root = ws.root().to_path_buf();

        // Restore layout.
        self.layout.show_file_tree = wstate.show_file_tree;
        self.layout
            .set_file_tree_width_pct(wstate.file_tree_width_pct);
        self.layout
            .set_right_panel_width_pct(wstate.right_panel_width_pct);

        // Open files in order.
        for rel in &wstate.open_files {
            let abs = root.join(rel);
            if abs.exists() {
                if let Err(e) = self.open_file(&abs) {
                    log::warn!("restore: failed to open {}: {e}", abs.display());
                }
            }
        }

        // Restore active file.
        if let Some(ref active_rel) = wstate.active_file {
            let abs = root.join(active_rel);
            // Find the buffer ID for this path and make it active.
            for &id in &self.tabs {
                if self
                    .registry
                    .get(id)
                    .and_then(|b| b.file_path.as_ref())
                    .is_some_and(|p| *p == abs)
                {
                    self.active_buffer = Some(id);
                    break;
                }
            }
        }

        // Restore cursor positions.
        for (rel, &(line, col)) in &wstate.cursor_positions {
            let abs = root.join(rel);
            for &id in &self.tabs {
                if self
                    .registry
                    .get(id)
                    .and_then(|b| b.file_path.as_ref())
                    .is_some_and(|p| *p == abs)
                {
                    if let Some(buf) = self.registry.get_mut(id) {
                        let clamped_line = line.min(buf.line_count().saturating_sub(1));
                        let clamped_col = col.min(buf.line_len(clamped_line).saturating_sub(1));
                        buf.cursor = CursorState::at(Position::new(clamped_line, clamped_col));
                    }
                    break;
                }
            }
        }
    }

    /// Collect dirty buffer contents for crash recovery autosave.
    ///
    /// Returns `(original_path, content)` pairs for all dirty buffers
    /// that have a file path.
    #[must_use]
    pub fn collect_dirty_buffers(&self) -> Vec<(PathBuf, String)> {
        self.tabs
            .iter()
            .filter_map(|&id| {
                let buf = self.registry.get(id)?;
                if !buf.is_dirty() {
                    return None;
                }
                let path = buf.file_path.clone()?;
                Some((path, buf.text()))
            })
            .collect()
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

    /// Switch to an adjacent tab by signed offset (+1 = next, -1 = prev).
    #[allow(clippy::cast_possible_wrap)]
    pub fn cycle_tab(&mut self, delta: isize) {
        let len = self.tabs.len();
        if len == 0 {
            return;
        }
        if let Some(idx) = self
            .active_buffer
            .and_then(|id| self.tabs.iter().position(|&t| t == id))
        {
            let next = (idx as isize + delta).rem_euclid(len as isize) as usize;
            self.active_buffer = Some(self.tabs[next]);
        }
    }

    /// Close the active tab.
    pub fn close_active_tab(&mut self) {
        if let Some(id) = self.active_buffer {
            close_tab_by_id(self, id);
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
            encoding: "UTF-8",
            ai_status: self.build_ai_status(),
            file_type: self.detect_file_type(),
            message: self.status_message.clone(),
            live_mode: self.build_live_mode_status(),
        }
    }

    /// Build the git branch display string: `branch ↑2 ↓1`.
    fn build_git_branch_display(&self) -> String {
        use std::fmt::Write;
        if self.git_branch.is_empty() {
            return String::new();
        }
        let mut s = self.git_branch.clone();
        if self.git_ahead > 0 {
            let _ = write!(s, " ↑{}", self.git_ahead);
        }
        if self.git_behind > 0 {
            let _ = write!(s, " ↓{}", self.git_behind);
        }
        s
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

    /// Build a short AI status string for the status bar.
    fn build_ai_status(&self) -> String {
        if self.ai_manager.is_empty() {
            return String::new();
        }
        let count = self.ai_manager.session_count();
        self.ai_manager.active_session().map_or_else(
            || format!("{count} session(s)"),
            |session| {
                let name = session.kind().display_name();
                let state = match session.state() {
                    lune_ai::SessionState::Starting => "starting",
                    lune_ai::SessionState::Running => "running",
                    lune_ai::SessionState::Exited(0) => "exited",
                    lune_ai::SessionState::Exited(_) => "exited!",
                    lune_ai::SessionState::Error => "error",
                };
                if count > 1 {
                    format!("{name} [{state}] ({count})")
                } else {
                    format!("{name} [{state}]")
                }
            },
        )
    }

    /// Build a Live Mode status string for the status bar.
    fn build_live_mode_status(&self) -> String {
        if !self.live_mode.is_active() {
            return String::new();
        }
        let hunks = self.live_mode.global_stats.total_hunks;
        if hunks > 0 {
            let files = self.live_mode.global_stats.total_files_changed;
            format!("LIVE {hunks}Δ {files}F")
        } else {
            "LIVE".to_string()
        }
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

    /// Collect a snapshot of the current editor context for AI sessions.
    ///
    /// Gathers active file, cursor, selection, open tabs, workspace, and git
    /// info into an [`EditorContext`] that can be encoded as env vars, JSON,
    /// or CLI args.
    fn collect_editor_context(&self) -> EditorContext {
        let workspace_root = self.workspace.as_ref().map(|ws| ws.root().to_path_buf());

        let active_file = self.active_buf().and_then(|buf| {
            let path = buf.file_path.as_ref()?;
            let language = {
                let first_line = buf.line(0);
                self.lang_registry
                    .detect(path, first_line.as_deref())
                    .map(|lid| lid.name().to_string())
            };
            let pos = &buf.cursor.primary.head;
            Some(FileContext {
                path: path.clone(),
                language,
                cursor_line: pos.line + 1,
                cursor_col: pos.col + 1,
                total_lines: buf.line_count(),
            })
        });

        let open_tabs: Vec<TabContext> = self
            .tabs
            .iter()
            .filter_map(|&id| {
                self.registry.get(id).map(|buf| TabContext {
                    path: buf.file_path.clone(),
                    dirty: buf.is_dirty(),
                })
            })
            .collect();

        let selection = self.active_buf().and_then(|buf| {
            let sel = &buf.cursor.primary;
            if sel.is_cursor() {
                return None;
            }
            let path = buf.file_path.as_ref()?;
            let (start, end) = sel.ordered();
            let text = extract_selection_text(buf, start, end);
            Some(SelectionContext {
                text,
                file_path: path.clone(),
                start_line: start.line + 1,
                end_line: end.line + 1,
            })
        });

        let git_status = self.git_service.as_ref().and_then(|git| {
            git.status().ok().map(|status| GitStatusSummary {
                branch: status.branch,
                modified_files: status
                    .files
                    .iter()
                    .filter(|f| !f.staged)
                    .map(|f| f.path.clone())
                    .collect(),
            })
        });

        EditorContext {
            workspace_root,
            active_file,
            open_tabs,
            git_status,
            selection,
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

    // Trigger notification flash when new notifications appear.
    let current_count = state.overlay.notifications.len();
    if current_count > state.last_notification_count {
        state.effects.start_notification_flash();
    }
    state.last_notification_count = current_count;

    // Sync tab manager from registry (include live hunk counts).
    let live_hunks = build_live_hunk_map(&state.live_mode);
    let live_hunks_ref = if live_hunks.is_empty() {
        None
    } else {
        Some(&live_hunks)
    };
    state.tab_mgr.sync_from_registry(
        &state.tabs,
        state.active_buffer,
        &state.registry,
        live_hunks_ref,
    );

    // Compute layout.
    let splits = layout::compute_layout(area, &state.layout);
    state.last_splits = Some(splits.clone());

    // Resize AI sessions to match the right panel area (minus header row).
    if let Some(right_area) = splits.right {
        if state.layout.show_ai_panel && !state.ai_manager.is_empty() {
            let term_rows = right_area.height.saturating_sub(1).max(1);
            let term_cols = right_area.width.max(1);
            let new_size = AiTermSize::new(term_rows, term_cols);
            if state.last_ai_term_size != Some(new_size) {
                state.ai_manager.resize_all(new_size);
                state.last_ai_term_size = Some(new_size);
            }
        }
    }

    // Render left panel (file tree).
    if let Some(left_area) = splits.left {
        let ws_name = state.workspace.as_ref().map_or("EXPLORER", Workspace::name);
        let ft_focused = state.focus.is_focused(PanelId::FileTree);
        file_tree::render_file_tree(
            left_area,
            buf,
            &mut state.file_tree,
            ws_name,
            ft_focused,
            &state.theme,
        );
    }

    // Render center: tab bar + editor.
    let editor_focused = state.focus.is_focused(PanelId::Editor);
    render_center(splits.center, buf, state, editor_focused);

    // Render right panel (git panel, AI terminal, or placeholder).
    if let Some(right_area) = splits.right {
        if state.layout.show_git_panel {
            let gp_focused = state.focus.is_focused(PanelId::GitPanel);
            git_panel::render_git_panel(
                right_area,
                buf,
                &mut state.git_panel,
                gp_focused,
                &state.theme,
            );
        } else if state.layout.show_ai_panel {
            let sessions = state.ai_manager.session_list();
            let session = state.ai_manager.active_session();
            terminal::render_ai_terminal(right_area, buf, &sessions, session, &state.theme);
        }
    }

    // Render status bar.
    let status_state = state.build_status_line();
    status_bar::render_status_bar(splits.status, buf, &status_state, &state.theme);

    // Update focus glow if focus changed.
    if state.focus_dirty {
        state.focus_dirty = false;
        update_focus_glow(state);
    }

    // Apply focus glow effect on the active panel.
    let active_panel = state.focus.active();
    let accent = state.theme.accent;
    let intensity = state.effects.focus_glow_intensity();

    if intensity > 0.0 {
        match active_panel {
            PanelId::FileTree => {
                if let Some(left_area) = splits.left {
                    crate::effects::paint_inner_border(buf, left_area, accent, intensity);
                }
            }
            PanelId::Editor => {
                crate::effects::paint_inner_border(buf, splits.center, accent, intensity);
            }
            PanelId::AiTerminal | PanelId::GitPanel => {
                if let Some(right_area) = splits.right {
                    crate::effects::paint_inner_border(buf, right_area, accent, intensity);
                }
            }
            _ => {}
        }
    }

    // Apply managed visual effects (tachyonfx) — modifies buffer cells in-place.
    let now = Instant::now();
    let elapsed = now.duration_since(state.last_render);
    state.last_render = now;
    state.effects.process(elapsed, buf, area);

    // Render overlays on top.
    overlay::render_overlay(area, buf, &mut state.overlay, &state.theme);

    Ok(())
}

/// Render the center area: tab bar + editor pane.
fn render_center(area: Rect, buf: &mut Buffer, state: &mut AppState, is_focused: bool) {
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
    tab_bar::render_tab_bar(tab_area, buf, &state.tab_mgr, is_focused, &state.theme);

    // Store content area for mouse mapping.
    state.last_editor_content_area = Some(content_area);

    // Compute highlight data for visible lines (plus ±50 line buffer for scroll smoothness).
    let highlighted = state.active_buffer.and_then(|id| {
        let viewport_height = content_area.height as usize;
        let top = state.viewport.top_line.saturating_sub(50);
        let end = state.viewport.top_line + viewport_height + 50;
        state
            .highlighters
            .get_mut(&id)
            .map(|hl| hl.highlight_lines(top..end))
    });

    // Render editor pane.
    let text_buf = state.active_buffer.and_then(|id| state.registry.get(id));
    let active_gutter = state
        .active_buffer
        .and_then(|id| state.gutter_marks.get(&id));

    // Build live diff overlay for the active buffer (if Live Mode is on).
    let live_overlay = state.active_buffer.and_then(|id| {
        state
            .live_mode
            .get_diff_state(id)
            .map(|ds| editor_pane::build_live_overlay(ds, &state.theme))
    });

    editor_pane::render_editor_pane(
        content_area,
        buf,
        text_buf,
        &mut state.viewport,
        state.vim.mode,
        highlighted.as_deref(),
        &state.syntax_theme,
        active_gutter,
        live_overlay.as_ref(),
        &state.theme,
    );
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
            // Debounced reactive state persistence (~2 s).
            maybe_persist_state(state);

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
        AppEvent::Ai(_) => Ok(handle_ai_event(state)),
    }
}

/// Handle file system events (from watcher).
fn handle_fs_event(fs_event: &crate::event::FsEvent, state: &mut AppState) -> Control<AppEvent> {
    let path = match fs_event {
        crate::event::FsEvent::Changed(p)
        | crate::event::FsEvent::Created(p)
        | crate::event::FsEvent::Deleted(p) => p,
    };

    // Feed file changes to Live Mode controller when active.
    if state.live_mode.is_active() {
        if let crate::event::FsEvent::Changed(changed_path) = fs_event {
            feed_live_mode_change(changed_path, state);
        }
    }

    // Hot-reload settings when config.toml is modified.
    if let crate::event::FsEvent::Changed(changed_path) = fs_event {
        check_settings_hot_reload(changed_path, state);
    }

    // Invalidate workspace cache for the parent directory and refresh.
    if let Some(ref mut ws) = state.workspace {
        if let Some(parent) = path.parent() {
            ws.invalidate(parent);
        }
        if let Err(e) = state.file_tree.refresh(ws) {
            log::error!("Failed to refresh file tree after fs event: {e}");
        }
    }

    // Refresh git status on file changes.
    // TODO: throttle to avoid blocking the event loop on large repos.
    state.refresh_git();

    Control::Changed
}

/// Detect changes to `config.toml` and hot-reload settings.
///
/// Compares the changed path's filename against known config files.
/// If it matches, re-loads the settings file and re-applies to state.
fn check_settings_hot_reload(changed_path: &Path, state: &mut AppState) {
    let Some(ref cp) = state.config_paths else {
        return;
    };

    let settings_file = cp.settings_file();
    if changed_path != settings_file {
        return;
    }

    // Attempt to re-load and re-apply settings.
    match Settings::load(&settings_file) {
        Ok(new_settings) => {
            // Only re-apply if the settings actually changed.
            if state.cached_settings.as_ref() != Some(&new_settings) {
                state.apply_settings(&new_settings);
                state
                    .overlay
                    .notify("Settings reloaded", NotificationLevel::Info);
            }
        }
        Err(e) => {
            state.overlay.notify(
                format!("Settings reload failed: {e}"),
                NotificationLevel::Error,
            );
        }
    }
}

/// Debounce interval for reactive state persistence (seconds).
const STATE_SAVE_DEBOUNCE_SECS: u64 = 2;

/// Persist workspace state to the sled database if the debounce interval
/// has elapsed.
///
/// Collects the current layout, open files, and cursor positions, then
/// writes to sled.  Cost is ~10 μs for a typical 20-file workspace, so
/// this runs directly on the main thread without blocking.
fn maybe_persist_state(state: &mut AppState) {
    let Some(ref db) = state.state_db else {
        return;
    };
    if state.last_state_save.elapsed() < Duration::from_secs(STATE_SAVE_DEBOUNCE_SECS) {
        return;
    }

    if let Some(mut wstate) = state.collect_workspace_state() {
        wstate.touch();
        if let Err(e) = db.put_workspace(&wstate) {
            log::error!("Failed to persist workspace state: {e}");
        }
    }
    state.last_state_save = Instant::now();
}

/// Build a map of `BufferId → hunk count` from the live mode controller.
///
/// Used to populate tab badges showing per-file change counts.
fn build_live_hunk_map(ctrl: &LiveModeController) -> HashMap<BufferId, usize> {
    if !ctrl.is_active() {
        return HashMap::new();
    }
    ctrl.tracked_buffers
        .iter()
        .filter_map(|(&id, state)| {
            let count = state.hunks.len();
            if count > 0 {
                Some((id, count))
            } else {
                None
            }
        })
        .collect()
}

/// Read a changed file from disk, update the Live Mode diff, and auto-follow.
///
/// When the controller returns a [`LiveChangeInfo`], we switch to the
/// changed buffer's tab and scroll the viewport to the latest change.
fn feed_live_mode_change(path: &Path, state: &mut AppState) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            log::debug!(
                "Live Mode: failed to read changed file {}: {e}",
                path.display()
            );
            return;
        }
    };
    let rope = Rope::from_str(&contents);
    let Some(info) = state.live_mode.on_file_changed(path, rope) else {
        return;
    };

    // Trigger diff pulse effect (brightness flash on new hunks).
    state.effects.start_diff_pulse(state.theme.diff_add_fg);

    // Auto-follow: switch to the changed buffer and scroll to the change.
    state.active_buffer = Some(info.buffer_id);
    if let Some(buf) = state.registry.get_mut(info.buffer_id) {
        buf.cursor.primary.head = Position::new(info.follow_line, 0);
        buf.cursor.primary.anchor = buf.cursor.primary.head;
    }
}

/// Handle AI session events (poll all sessions for new output).
fn handle_ai_event(state: &mut AppState) -> Control<AppEvent> {
    let changed = state.ai_manager.poll_all();

    // Detect AI thinking state transitions: start/stop the effect
    // when any active session transitions to/from Running.
    let is_running = state
        .ai_manager
        .active_session()
        .is_some_and(|s| s.state() == lune_ai::SessionState::Running);

    if is_running && !state.ai_thinking_active {
        state.ai_thinking_active = true;
        state.effects.start_ai_thinking(state.theme.accent);
    } else if !is_running && state.ai_thinking_active {
        state.ai_thinking_active = false;
        state.effects.cancel_ai_thinking();
    }

    if changed {
        Control::Changed
    } else {
        Control::Continue
    }
}

/// Handle crossterm terminal events.
fn handle_terminal_event(ct_event: &CtEvent, state: &mut AppState) -> Control<AppEvent> {
    match ct_event {
        CtEvent::Key(key_event) if key_event.kind == KeyEventKind::Press => {
            handle_key_event(key_event, state)
        }
        CtEvent::Mouse(mouse_event) => handle_mouse_event(*mouse_event, state),
        CtEvent::Resize(_, _) => {
            // Resize AI sessions to match the new right panel area.
            // The actual area will be computed on the next render;
            // for now, trigger a re-render so the layout recomputes.
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Handle a key press event.
fn handle_key_event(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    // 1. If overlay is active, route to overlay handler.
    if state.overlay.is_active() {
        return handle_overlay_key(key, state);
    }

    // Tab key cycles focus between panes (only outside Insert mode).
    if key.code == KeyCode::Tab && key.modifiers.is_empty() && !state.vim.mode.is_insert() {
        handle_focus_next_pane(state);
        return Control::Changed;
    }

    // 2. Check global keybindings.
    if let Some(cmd) = state.keymap.lookup(key) {
        return Control::Event(AppEvent::Command(cmd.clone()));
    }

    // 3. Escape: return to editor if in file tree, git panel, or AI terminal, else normal mode.
    if key.code == KeyCode::Esc {
        if state.focus.is_focused(PanelId::FileTree)
            || state.focus.is_focused(PanelId::GitPanel)
            || state.focus.is_focused(PanelId::AiTerminal)
        {
            state.focus.focus(PanelId::Editor);
            state.focus_dirty = true;
            return Control::Changed;
        }
        // Escape always enters Normal mode (blocks typing regardless of vim_enabled).
        state.vim.enter_normal();
        state.status_message.clear();
        return Control::Changed;
    }

    // 4. Route to AI terminal if focused (forward all keys to PTY).
    if state.focus.is_focused(PanelId::AiTerminal) {
        return handle_ai_terminal_key(key, state);
    }

    // 4a. Route to file tree if focused.
    if state.focus.is_focused(PanelId::FileTree) {
        return handle_file_tree_key(key, state);
    }

    // 4b. Route to git panel if focused.
    if state.focus.is_focused(PanelId::GitPanel) {
        return handle_git_panel_key(key, state);
    }

    // 5. Route based on vim mode.
    // Normal/Insert switching always works (Escape → Normal blocks typing,
    // `i` → Insert allows typing). Visual, VisualLine, and Command modes
    // are only reachable when vim keybindings are enabled; if somehow
    // entered while vim is disabled, fall back to Insert.
    match state.vim.mode {
        VimMode::Insert => handle_insert_mode(key, state),
        VimMode::Normal => handle_normal_mode(key, state),
        VimMode::Visual | VimMode::VisualLine if state.vim_enabled => {
            handle_visual_mode(key, state)
        }
        VimMode::Command if state.vim_enabled => Control::Continue, // TODO: command-line mode
        // vim disabled but in Visual/Command — snap back to Insert.
        _ => {
            state.vim.enter_insert();
            handle_insert_mode(key, state)
        }
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
                    close_overlay(state);
                    Control::Event(AppEvent::Command(cmd))
                }
                KeyCode::Esc => {
                    close_overlay(state);
                    Control::Changed
                }
                _ => Control::Continue,
            }
        }
        Some(overlay::OverlayKind::FindReplace) => {
            if key.code == KeyCode::Esc {
                close_overlay(state);
                Control::Changed
            } else {
                Control::Continue
            }
        }
        Some(overlay::OverlayKind::FilePicker) => handle_file_picker_key(key, state),
        Some(overlay::OverlayKind::AiClientPicker) => handle_ai_client_picker_key(key, state),
        None => Control::Continue,
    }
}

/// Close the active overlay and return focus.
fn close_overlay(state: &mut AppState) {
    state.overlay.close();
    state.focus.focus_return();
    state.focus_dirty = true;
}

/// Handle keys in the command palette.
fn handle_palette_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => state
            .overlay
            .command_palette
            .selected_command()
            .cloned()
            .map_or(Control::Changed, |cmd| {
                close_overlay(state);
                Control::Event(AppEvent::Command(cmd))
            }),
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
            close_overlay(state);
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
        let path = entry.path;
        close_overlay(state);
        Control::Event(AppEvent::Command(AppCommand::OpenFile(path)))
    }
}

/// Handle key events for the AI client picker overlay.
fn handle_ai_client_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            if let Some(kind) = state.overlay.ai_client_picker.selected_kind() {
                close_overlay(state);
                Control::Event(AppEvent::Command(AppCommand::AiNewSession(kind)))
            } else {
                Control::Continue
            }
        }
        KeyCode::Up => {
            state.overlay.ai_client_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.ai_client_picker.select_next();
            Control::Changed
        }
        _ => Control::Continue,
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
        KeyCode::Char('l') | KeyCode::Right => handle_file_tree_set_expanded(state, true),
        // h/Left: collapse directory (or go to parent).
        KeyCode::Char('h') | KeyCode::Left => handle_file_tree_set_expanded(state, false),
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

/// Handle expand/collapse in the file tree.
fn handle_file_tree_set_expanded(state: &mut AppState, expanded: bool) -> Control<AppEvent> {
    if state.file_tree.selected_is_dir() {
        if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
            if let Some(ref mut ws) = state.workspace {
                ws.set_expanded(&path, expanded);
                state.refresh_file_tree();
            }
        }
    }
    Control::Changed
}

// ── Focus cycling ─────────────────────────────────────────────────

/// Cycle focus to the next visible pane.
///
/// Builds a list of currently visible panels and advances to the next one
/// in order: `FileTree` → Editor → `AiTerminal` → `GitPanel` → (wrap).
/// Panels that are not visible are skipped.
fn handle_focus_next_pane(state: &mut AppState) {
    let mut panes = Vec::with_capacity(4);
    if state.layout.show_file_tree {
        panes.push(PanelId::FileTree);
    }
    panes.push(PanelId::Editor);
    if state.layout.show_ai_panel {
        panes.push(PanelId::AiTerminal);
    }
    if state.layout.show_git_panel {
        panes.push(PanelId::GitPanel);
    }

    let current = state.focus.active();
    let next = panes
        .iter()
        .position(|&p| p == current)
        .map_or(PanelId::Editor, |idx| panes[(idx + 1) % panes.len()]);
    state.focus.set_active(next);
    state.focus_dirty = true;
}

/// Update the focus glow effect based on the currently focused panel.
///
/// Starts a glow on the newly focused panel and cancels glows on all
/// other panels. Only applies to main content panels (`FileTree`, `Editor`,
/// `GitPanel`) — overlays like `CommandPalette` don't get glow effects.
fn update_focus_glow(state: &mut AppState) {
    let active = state.focus.active();
    let accent = state.theme.accent;

    // Cancel all existing panel glows.
    for &panel in &[
        PanelId::FileTree,
        PanelId::Editor,
        PanelId::AiTerminal,
        PanelId::GitPanel,
    ] {
        if panel != active {
            state.effects.cancel_focus_glow(panel);
        }
    }

    // Start glow on the active panel (if it's a content panel).
    match active {
        PanelId::FileTree | PanelId::Editor | PanelId::AiTerminal | PanelId::GitPanel => {
            state.effects.start_focus_glow(active, accent);
        }
        _ => {} // Overlays/status bar don't get glow.
    }
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

// ── AI terminal key handling ──────────────────────────────────────

/// Handle key events when the AI terminal is focused.
///
/// Translates crossterm key events to terminal byte sequences and
/// forwards them to the active PTY session.
fn handle_ai_terminal_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    let bytes = key_event_to_bytes(key);
    if bytes.is_empty() {
        return Control::Continue;
    }
    if let Some(session) = state.ai_manager.active_session_mut() {
        if let Err(e) = session.send_input(&bytes) {
            log::error!("Failed to send input to AI session: {e}");
        }
    }
    Control::Changed
}

/// Translate a crossterm `KeyEvent` into raw terminal byte sequence(s).
///
/// Maps special keys to their VT/xterm escape sequences and control
/// characters. Returns an empty vec for keys we don't handle.
#[allow(clippy::too_many_lines)]
fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(ch) => {
            if ctrl {
                // Ctrl+letter: control characters (0x01–0x1A).
                let code = ch.to_ascii_lowercase();
                if code.is_ascii_lowercase() {
                    let byte = code as u8 - b'a' + 1;
                    return vec![byte];
                }
            }
            if alt {
                // Alt+char: ESC prefix.
                let mut bytes = vec![0x1b];
                let mut char_buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut char_buf).as_bytes());
                return bytes;
            }
            let mut char_buf = [0u8; 4];
            ch.encode_utf8(&mut char_buf);
            let len = ch.len_utf8();
            char_buf[..len].to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => f_key_escape(n),
        _ => Vec::new(),
    }
}

/// Map F-key number to VT escape sequence.
fn f_key_escape(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

/// Resolve the AI client kind from the current settings.
///
/// Falls back to `ClaudeCode` when no settings are cached.
fn ai_client_from_settings(state: &AppState) -> AiClientKind {
    let cmd = state
        .cached_settings
        .as_ref()
        .map(|s| s.ai.default_client.as_str())
        .unwrap_or("claude");
    match cmd {
        "claude" => AiClientKind::ClaudeCode,
        other => AiClientKind::Custom {
            name: other.to_string(),
            command: other.to_string(),
        },
    }
}

/// Start an AI client session using the configured default client.
fn start_default_ai_session(state: &mut AppState) {
    let kind = ai_client_from_settings(state);
    let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
    let size = state
        .last_splits
        .as_ref()
        .and_then(|s| s.right)
        .map_or_else(AiTermSize::default, |r| {
            AiTermSize::new(r.height.saturating_sub(1).max(1), r.width.max(1))
        });
    let client_name = kind.display_name().to_string();
    match state
        .ai_manager
        .new_session(kind, cwd.as_deref(), &HashMap::new(), size)
    {
        Ok(_id) => {
            log::info!("Started AI session: {client_name}");
        }
        Err(e) => {
            log::error!("Failed to start AI session: {e}");
            state.overlay.notify(
                format!("Failed to launch {client_name}: {e}"),
                crate::widgets::overlay::NotificationLevel::Error,
            );
        }
    }
}

/// Start an AI client session with editor context environment variables.
///
/// Collects the current editor context (active file, cursor, selection,
/// git status, open tabs) and passes it as `LUNE_CTX_*` env vars to the
/// spawned process. The AI client uses its own auth — no API key is
/// configured in Lune.
fn start_ai_session_with_context(state: &mut AppState) {
    let kind = ai_client_from_settings(state);
    let ctx = state.collect_editor_context();
    let env = ctx.to_env_vars();
    let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
    let size = state
        .last_splits
        .as_ref()
        .and_then(|s| s.right)
        .map_or_else(AiTermSize::default, |r| {
            AiTermSize::new(r.height.saturating_sub(1).max(1), r.width.max(1))
        });
    let client_name = kind.display_name().to_string();
    match state
        .ai_manager
        .new_session(kind, cwd.as_deref(), &env, size)
    {
        Ok(_id) => {
            log::info!("Started AI session ({client_name}) with editor context");
        }
        Err(e) => {
            log::error!("Failed to start AI session: {e}");
            state.overlay.notify(
                format!("Failed to launch {client_name}: {e}"),
                NotificationLevel::Error,
            );
        }
    }
}

/// Start a new AI session with the given client kind and open the panel.
fn handle_ai_new_session(kind: AiClientKind, state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let env = ctx.to_env_vars();
    let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
    let size = state
        .last_splits
        .as_ref()
        .and_then(|s| s.right)
        .map_or_else(AiTermSize::default, |r| {
            AiTermSize::new(r.height.saturating_sub(1).max(1), r.width.max(1))
        });
    let client_name = kind.display_name().to_string();
    match state.ai_manager.new_session(kind, cwd.as_deref(), &env, size) {
        Ok(_id) => {
            log::info!("Started AI session: {client_name}");
            if !state.layout.show_ai_panel {
                state.layout.toggle_ai_panel();
            }
            state.focus.focus(PanelId::AiTerminal);
            state.focus_dirty = true;
        }
        Err(e) => {
            log::error!("Failed to start AI session: {e}");
            state.overlay.notify(
                format!("Failed to launch {client_name}: {e}"),
                NotificationLevel::Error,
            );
        }
    }
    Control::Changed
}

/// Ensure the AI panel is open and focused, starting a context-aware
/// session if none exists.
fn ensure_ai_panel_open(state: &mut AppState) {
    if !state.layout.show_ai_panel {
        state.layout.toggle_ai_panel();
    }
    if state.ai_manager.is_empty() {
        start_ai_session_with_context(state);
    }
    state.focus.focus(PanelId::AiTerminal);
    state.focus_dirty = true;
}

/// Send a prompt string to the active AI session's PTY.
fn send_prompt_to_ai(state: &mut AppState, prompt: &str) {
    if let Some(session) = state.ai_manager.active_session_mut() {
        // Send the prompt followed by Enter.
        if let Err(e) = session.send_input(prompt.as_bytes()) {
            log::error!("Failed to send prompt to AI: {e}");
        }
        if let Err(e) = session.send_input(b"\n") {
            log::error!("Failed to send newline to AI: {e}");
        }
    }
}

/// Handle "Ask AI about selection" command.
///
/// Opens the AI panel and focuses it. If there is an active text selection,
/// it is automatically included as `LUNE_CTX_SELECTION` in the session
/// environment so the AI client sees it as context. The user then types
/// their prompt directly into the session.
fn handle_ai_ask_selection(state: &mut AppState) -> Control<AppEvent> {
    ensure_ai_panel_open(state);
    Control::Changed
}

/// Handle "Refactor file" command.
///
/// Opens the AI panel and sends a refactoring request with file context.
fn handle_ai_refactor_file(state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let file_path = ctx
        .active_file
        .as_ref()
        .map(|f| f.path.display().to_string())
        .unwrap_or_default();

    if file_path.is_empty() {
        state.overlay.notify(
            "No file open — open a file first",
            NotificationLevel::Warning,
        );
        return Control::Changed;
    }

    ensure_ai_panel_open(state);

    let prompt = format!("Refactor {file_path}");
    send_prompt_to_ai(state, &prompt);
    Control::Changed
}

/// Handle "Summarize git changes" command.
///
/// Opens the AI panel and sends a request to summarize the current
/// git-tracked modifications.
fn handle_ai_summarize_changes(state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let summary = ctx
        .git_status
        .as_ref()
        .map(|g| {
            use std::fmt::Write as _;
            let mut s = format!("Branch: {}\nModified files:\n", g.branch);
            for f in &g.modified_files {
                let _ = writeln!(s, "  - {}", f.display());
            }
            s
        })
        .unwrap_or_default();

    if summary.is_empty() {
        state.overlay.notify(
            "No git repository — open a workspace first",
            NotificationLevel::Warning,
        );
        return Control::Changed;
    }

    ensure_ai_panel_open(state);

    let prompt = format!("Summarize these changes:\n{summary}");
    send_prompt_to_ai(state, &prompt);
    Control::Changed
}

// ── Live Mode command handlers ────────────────────────────────────────

/// Toggle Live Mode: Off ↔ On.
///
/// When entering On from Off, registers all open buffers with file paths
/// as baselines. When entering Off, clears all tracking.
fn handle_toggle_live_mode(state: &mut AppState) -> Control<AppEvent> {
    let was_active = state.live_mode.is_active();
    state.live_mode.toggle();

    if state.live_mode.is_active() && !was_active {
        // Entering live mode: register all open buffers with file paths.
        register_all_buffers_for_live_mode(state);
        state
            .overlay
            .notify("Live Mode: On", NotificationLevel::Info);
    } else {
        state
            .overlay
            .notify("Live Mode: Off", NotificationLevel::Info);
    }

    Control::Changed
}

/// Register all open buffers that have file paths for live tracking.
fn register_all_buffers_for_live_mode(state: &mut AppState) {
    let entries: Vec<(BufferId, PathBuf, Rope)> = state
        .tabs
        .iter()
        .filter_map(|&id| {
            let buf = state.registry.get(id)?;
            let path = buf.file_path.clone()?;
            Some((id, path, buf.rope().clone()))
        })
        .collect();

    for (id, path, content) in entries {
        state.live_mode.register_buffer(id, path, content);
    }
}

/// Handle key events in insert mode — characters are inserted.
fn handle_insert_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let mutates_text = matches!(
        key.code,
        KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab
    );

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
                    buf.delete(Position::new(pos.line, pos.col - 1), pos);
                } else if pos.line > 0 {
                    let prev_len = buf.line_len(pos.line - 1).saturating_sub(1);
                    buf.delete(Position::new(pos.line - 1, prev_len), pos);
                }
            }
            Control::Changed
        }
        KeyCode::Delete => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                buf.delete(pos, Position::new(pos.line, pos.col + 1));
            }
            Control::Changed
        }
        KeyCode::Tab => {
            if let Some(buf) = state.active_buf_mut() {
                let pos = buf.cursor.primary.head;
                buf.insert(pos, "    ");
            }
            Control::Changed
        }
        KeyCode::Home => apply_motion(state, |buf| buf.move_line_start(extend)),
        KeyCode::End => apply_motion(state, |buf| buf.move_line_end(extend)),
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
            apply_arrow_motion(key, state, extend)
        }
        _ => Control::Continue,
    };

    if mutates_text {
        state.update_active_highlighter();
    }
    result
}

/// Handle key events in normal mode — characters are vim commands.
///
/// When vim is disabled, only `i` (return to Insert), `h/j/k/l`
/// navigation, and arrow keys are accepted.  All other vim Normal-mode
/// commands are ignored.
fn handle_normal_mode(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if let KeyCode::Char(ch) = key.code {
        // When vim is disabled, allow `i` and basic h/j/k/l navigation.
        if !state.vim_enabled {
            return match ch {
                'i' => {
                    state.vim.enter_insert();
                    Control::Changed
                }
                'h' => apply_motion(state, |buf| buf.move_left(false)),
                'j' => apply_motion(state, |buf| buf.move_down(false)),
                'k' => apply_motion(state, |buf| buf.move_up(false)),
                'l' => apply_motion(state, |buf| buf.move_right(false)),
                _ => Control::Continue,
            };
        }
        let dummy = TextBuffer::new();
        let buf = state
            .active_buffer
            .and_then(|id| state.registry.get(id))
            .unwrap_or(&dummy);
        let action = state.vim.handle_normal(ch, buf);
        apply_vim_action(&action, state)
    } else {
        apply_arrow_motion(key, state, false)
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

/// Map arrow keys to cursor motion. Returns `Continue` for non-arrow keys.
fn apply_arrow_motion(key: &KeyEvent, state: &mut AppState, extend: bool) -> Control<AppEvent> {
    let method: Option<fn(&mut TextBuffer, bool)> = match key.code {
        KeyCode::Left => Some(TextBuffer::move_left),
        KeyCode::Right => Some(TextBuffer::move_right),
        KeyCode::Up => Some(TextBuffer::move_up),
        KeyCode::Down => Some(TextBuffer::move_down),
        _ => None,
    };
    method.map_or(Control::Continue, |m| {
        apply_motion(state, |buf| m(buf, extend))
    })
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
            if state.focus.is_focused(PanelId::AiTerminal) {
                if let Some(session) = state.ai_manager.active_session_mut() {
                    session.scroll_up(3);
                }
            } else {
                state.viewport.scroll_up(3);
            }
            Control::Changed
        }
        MouseEventKind::ScrollDown => {
            if state.focus.is_focused(PanelId::AiTerminal) {
                if let Some(session) = state.ai_manager.active_session_mut() {
                    session.scroll_down(3);
                }
            } else {
                let total = state
                    .active_buf()
                    .map_or(0, lune_core::buffer::TextBuffer::line_count);
                let height = state
                    .last_editor_content_area
                    .map_or(20, |a| a.height as usize);
                state.viewport.scroll_down(3, total, height);
            }
            Control::Changed
        }
        _ => Control::Continue,
    }
}

/// Check if a point is inside a rect.
const fn point_in_rect(col: u16, row: u16, r: Rect) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

/// Handle left mouse button click.
#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_click(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let (col, row) = (mouse.column, mouse.row);

    // Check panel borders first (start drag).
    if let Some(ref splits) = state.last_splits {
        if layout::is_on_left_border(splits, col) {
            state.dragging_border = Some(DragBorder::Left);
            return Control::Continue;
        }
        if layout::is_on_right_border(splits, col) {
            state.dragging_border = Some(DragBorder::Right);
            return Control::Continue;
        }

        // File tree area.
        if let Some(left_area) = splits.left {
            if point_in_rect(col, row, left_area) {
                state.focus.focus(PanelId::FileTree);
                state.focus_dirty = true;
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

        // Git panel area.
        if state.layout.show_git_panel {
            if let Some(right_area) = splits.right {
                if point_in_rect(col, row, right_area) {
                    state.focus.focus(PanelId::GitPanel);
                    state.focus_dirty = true;
                    return Control::Changed;
                }
            }
        }

        // AI terminal area.
        if state.layout.show_ai_panel {
            if let Some(right_area) = splits.right {
                if point_in_rect(col, row, right_area) {
                    state.focus.focus(PanelId::AiTerminal);
                    state.focus_dirty = true;
                    return Control::Changed;
                }
            }
        }

        // Tab bar (first row of center).
        if row == splits.center.y {
            let tab_area = Rect::new(splits.center.x, splits.center.y, splits.center.width, 1);
            if let Some((idx, is_close)) = state.tab_mgr.hit_test(col, tab_area.x, tab_area.width) {
                if is_close {
                    if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                        close_tab_by_id(state, bid);
                    }
                } else if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                    state.active_buffer = Some(bid);
                }
                return Control::Changed;
            }
        }
    }

    // Editor content area — set cursor.
    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .active_buffer
            .is_some_and(|id| state.gutter_marks.contains_key(&id));
        let has_live = state.live_mode.is_active()
            && state
                .active_buffer
                .and_then(|id| state.live_mode.get_diff_state(id))
                .is_some_and(|ds| !ds.hunks.is_empty());
        if let Some(pos) = editor_pane::click_to_position(
            col,
            row,
            content_area,
            &state.viewport,
            total_lines,
            has_git,
            has_live,
        ) {
            state.focus.set_active(PanelId::Editor);
            state.focus_dirty = true;
            if let Some(buf) = state.active_buf_mut() {
                let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                let clamped_col = pos.col.min(buf.line_len(clamped_line).saturating_sub(1));
                buf.cursor = CursorState::at(Position::new(clamped_line, clamped_col));
            }
            return Control::Changed;
        }
    }

    Control::Continue
}

/// Close a specific tab by buffer ID (used by mouse click and keyboard).
fn close_tab_by_id(state: &mut AppState, bid: BufferId) {
    if let Some(idx) = state.tabs.iter().position(|&id| id == bid) {
        state.tabs.remove(idx);
        state.registry.close(bid);
        state.highlighters.remove(&bid);
        if state.active_buffer == Some(bid) {
            state.active_buffer = if state.tabs.is_empty() {
                None
            } else {
                Some(state.tabs[idx.min(state.tabs.len() - 1)])
            };
        }
    }
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
        VimAction::ModeChanged(mode) => {
            // When vim is disabled, only Normal↔Insert transitions are allowed.
            // Block Visual/VisualLine/Command and snap back to Normal.
            if !state.vim_enabled && !matches!(mode, VimMode::Normal | VimMode::Insert) {
                state.vim.enter_normal();
                return Control::Continue;
            }
            Control::Changed
        }
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
            apply_buf_edit(state, TextBuffer::undo);
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

/// Helper: apply a buffer edit and refresh the highlighter.
fn apply_buf_edit(state: &mut AppState, f: fn(&mut TextBuffer) -> bool) {
    if let Some(buf) = state.active_buf_mut() {
        let _ = f(buf);
    }
    state.update_active_highlighter();
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
            state.cycle_tab(1);
            Control::Changed
        }
        AppCommand::PrevTab => {
            state.cycle_tab(-1);
            Control::Changed
        }
        // Panel toggles and focus.
        AppCommand::ToggleFileTree
        | AppCommand::ToggleAiPanel
        | AppCommand::ToggleGitPanel
        | AppCommand::FocusNextPane
        | AppCommand::OpenCommandPalette
        | AppCommand::OpenFilePicker => handle_panel_command(cmd, state),
        AppCommand::Undo => {
            apply_buf_edit(state, TextBuffer::undo);
            Control::Changed
        }
        AppCommand::Redo => {
            apply_buf_edit(state, TextBuffer::redo);
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
        AppCommand::GitStage => handle_git_file_op(state, GitService::stage, "Staged"),
        AppCommand::GitUnstage => handle_git_file_op(state, GitService::unstage, "Unstaged"),
        AppCommand::GitCommit => handle_git_commit(state),
        AppCommand::GitDiscard => handle_git_discard(state),
        AppCommand::GitRefresh => {
            state.refresh_git();
            Control::Changed
        }
        AppCommand::GitDiscardConfirmed(path) => handle_git_discard_confirmed(path, state),
        // AI commands.
        AppCommand::AiAskSelection => handle_ai_ask_selection(state),
        AppCommand::AiRefactorFile => handle_ai_refactor_file(state),
        AppCommand::AiSummarizeChanges => handle_ai_summarize_changes(state),
        AppCommand::AiOpenClientPicker => {
            state.overlay.open_ai_client_picker();
            Control::Changed
        }
        AppCommand::AiNewSession(kind) => handle_ai_new_session(kind.clone(), state),
        AppCommand::AiCloseSession => {
            if let Some(id) = state.ai_manager.active_id() {
                state.ai_manager.close_session(id);
                if state.ai_manager.is_empty() {
                    state.layout.show_ai_panel = false;
                    state.focus.set_active(PanelId::Editor);
                    state.focus_dirty = true;
                }
            }
            Control::Changed
        }
        AppCommand::AiNextSession => {
            let ids: Vec<_> = state.ai_manager.session_list().into_iter().map(|(id, _, _)| id).collect();
            if let Some(active) = state.ai_manager.active_id() {
                if let Some(pos) = ids.iter().position(|&id| id == active) {
                    let next = ids[(pos + 1) % ids.len()];
                    state.ai_manager.switch_session(next);
                }
            }
            Control::Changed
        }
        AppCommand::AiPrevSession => {
            let ids: Vec<_> = state.ai_manager.session_list().into_iter().map(|(id, _, _)| id).collect();
            if let Some(active) = state.ai_manager.active_id() {
                if let Some(pos) = ids.iter().position(|&id| id == active) {
                    let prev = if pos == 0 { ids.len() - 1 } else { pos - 1 };
                    state.ai_manager.switch_session(ids[prev]);
                }
            }
            Control::Changed
        }
        // Live Mode commands.
        AppCommand::ToggleLiveMode => handle_toggle_live_mode(state),
        // Theme commands.
        AppCommand::NextTheme => {
            state.next_theme();
            let name = state.theme_registry.current_name().to_owned();
            state
                .overlay
                .notify(format!("Theme: {name}"), NotificationLevel::Info);
            Control::Changed
        }
        AppCommand::PrevTheme => {
            state.prev_theme();
            let name = state.theme_registry.current_name().to_owned();
            state
                .overlay
                .notify(format!("Theme: {name}"), NotificationLevel::Info);
            Control::Changed
        }
        AppCommand::OpenSettings | AppCommand::OpenKeybindings => {
            handle_open_config_file(cmd, state)
        }
    }
}

/// Handle panel toggle and focus commands.
fn handle_panel_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    match cmd {
        AppCommand::ToggleFileTree => {
            state.layout.toggle_file_tree();
            if state.layout.show_file_tree {
                state.focus.focus(PanelId::FileTree);
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            state.focus_dirty = true;
            state
                .effects
                .start_panel_transition(PanelId::FileTree, state.theme.accent);
            Control::Changed
        }
        AppCommand::ToggleAiPanel => {
            state.layout.toggle_ai_panel();
            if state.layout.show_ai_panel {
                if state.ai_manager.is_empty() {
                    // No session yet — ask which client to open.
                    state.overlay.open_ai_client_picker();
                } else {
                    state.focus.focus(PanelId::AiTerminal);
                }
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            state.focus_dirty = true;
            state
                .effects
                .start_panel_transition(PanelId::AiTerminal, state.theme.accent);
            Control::Changed
        }
        AppCommand::ToggleGitPanel => {
            state.layout.toggle_git_panel();
            if state.layout.show_git_panel {
                state.focus.focus(PanelId::GitPanel);
                // Refresh git status when opening the panel.
                state.refresh_git();
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            state.focus_dirty = true;
            state
                .effects
                .start_panel_transition(PanelId::GitPanel, state.theme.accent);
            Control::Changed
        }
        AppCommand::FocusNextPane => {
            handle_focus_next_pane(state);
            Control::Changed
        }
        AppCommand::OpenCommandPalette => {
            state.overlay.open_command_palette();
            state.focus.focus(PanelId::CommandPalette);
            state.focus_dirty = true;
            Control::Changed
        }
        AppCommand::OpenFilePicker => {
            let start_dir = state.workspace.as_ref().map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                |ws| ws.root().to_path_buf(),
            );
            state.overlay.open_file_picker(&start_dir);
            state.focus.focus(PanelId::CommandPalette); // Reuse overlay focus
            state.focus_dirty = true;
            Control::Changed
        }
        _ => Control::Continue,
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
            state.focus_dirty = true;
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

/// Handle `OpenSettings` / `OpenKeybindings`: open the config file in the editor.
///
/// If the file doesn't exist yet, creates it with sensible defaults.
fn handle_open_config_file(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    let Some(ref cp) = state.config_paths else {
        state
            .overlay
            .notify("Config directory not available", NotificationLevel::Error);
        return Control::Changed;
    };

    let (path, default_content) = match cmd {
        AppCommand::OpenSettings => (
            cp.settings_file(),
            toml::to_string_pretty(&Settings::default()).unwrap_or_default(),
        ),
        AppCommand::OpenKeybindings => (
            cp.keybindings_file(),
            "# Keybinding overrides\n\
             # Format: \"key_combo\" = \"command\"\n\
             #\n\
             # [normal]\n\
             # \"ctrl+s\" = \"save\"\n\
             # \"ctrl+shift+p\" = \"command_palette\"\n\
             #\n\
             # [vim.normal]\n\
             # \"g d\" = \"go_to_definition\"\n"
                .to_owned(),
        ),
        _ => return Control::Continue,
    };

    // Ensure the config directory exists.
    if let Err(e) = cp.ensure_dirs() {
        state.overlay.notify(
            format!("Failed to create config dirs: {e}"),
            NotificationLevel::Error,
        );
        return Control::Changed;
    }

    // Create the file with defaults if it doesn't exist.
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, &default_content) {
            state.overlay.notify(
                format!("Failed to create {}: {e}", path.display()),
                NotificationLevel::Error,
            );
            return Control::Changed;
        }
    }

    // Open the file in the editor.
    handle_open_file(&path, state)
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

/// Execute a git file operation (stage/unstage) on the selected file.
fn handle_git_file_op(
    state: &mut AppState,
    op: fn(&GitService, &Path) -> anyhow::Result<()>,
    label: &str,
) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };
    if let Some(ref git) = state.git_service {
        match op(git, &file.path) {
            Ok(()) => {
                state.status_message = format!("{label}: {}", file.path.display());
                state.refresh_git();
            }
            Err(e) => {
                state.status_message = format!("{label} failed: {e}");
                state
                    .overlay
                    .notify(format!("{label} failed: {e}"), NotificationLevel::Error);
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
    state.focus_dirty = true;
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

// ── AI session poll integration ───────────────────────────────────────

/// Event source that polls the AI session manager for output and
/// converts changes to `AppEvent::Ai(_)`.
///
/// Unlike `PollFileWatcher`, this doesn't use a separate channel — it
/// directly calls `ai_manager.poll_all()` which drains the per-session
/// crossbeam channels and feeds bytes to each session's vt100 parser.
///
/// Because rat-salsa passes `state` to the event handler separately,
/// we use a shared `AiManager` pointer pattern: the manager lives in
/// `AppState` and this poller just signals "something changed".
pub struct PollAiSessions {
    /// Whether the last poll found changes.
    has_changes: bool,
}

impl PollAiSessions {
    /// Create a new AI session poller.
    #[must_use]
    pub const fn new() -> Self {
        Self { has_changes: false }
    }
}

impl Default for PollAiSessions {
    fn default() -> Self {
        Self::new()
    }
}

impl rat_salsa::poll::PollEvents<AppEvent, Error> for PollAiSessions {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn poll(&mut self) -> Result<bool, Error> {
        // We can't access AppState here, so we always claim we have
        // an event to read. The `read()` method will emit an AI event
        // that triggers `event()`, which calls `poll_all()` on the
        // actual manager. This is a lightweight approach: every poll
        // cycle we signal to drain the AI channels.
        //
        // This works because rat-salsa calls poll() frequently (every
        // cycle), and the event handler will be a no-op if there's
        // actually nothing to drain.
        self.has_changes = true;
        Ok(true)
    }

    fn read(&mut self) -> Result<Control<AppEvent>, Error> {
        if self.has_changes {
            self.has_changes = false;
            Ok(Control::Event(AppEvent::Ai(
                crate::event::AiEvent::OutputChanged,
            )))
        } else {
            Ok(Control::Continue)
        }
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
            .poll(PollFileWatcher::new(watcher_rx))
            .poll(PollAiSessions::new()),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::layout::Rect;

    /// Helper: create a fresh `AppState` and open a temporary file.
    fn state_with_file() -> (AppState, tempfile::NamedTempFile) {
        let mut state = AppState::new();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "line one\nline two\nline three\n").unwrap();
        state.open_file(tmp.path()).unwrap();
        (state, tmp)
    }

    /// Helper: create a state with multiple open tabs.
    fn state_with_tabs(n: usize) -> (AppState, Vec<tempfile::NamedTempFile>) {
        let mut state = AppState::new();
        let mut files = Vec::new();
        for i in 0..n {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(tmp.path(), format!("file {i}\n")).unwrap();
            state.open_file(tmp.path()).unwrap();
            files.push(tmp);
        }
        (state, files)
    }

    // ── AppState construction ─────────────────────────────────────

    #[test]
    fn new_state_has_no_active_buffer() {
        let state = AppState::new();
        assert!(state.active_buffer.is_none());
        assert!(state.tabs.is_empty());
        assert!(state.active_buf().is_none());
    }

    #[test]
    fn default_equals_new() {
        let a = AppState::new();
        let b = AppState::default();
        assert_eq!(a.tabs.len(), b.tabs.len());
        assert_eq!(a.active_buffer, b.active_buffer);
    }

    // ── open_file ─────────────────────────────────────────────────

    #[test]
    fn open_file_sets_active() {
        let (state, _tmp) = state_with_file();
        assert!(state.active_buffer.is_some());
        assert_eq!(state.tabs.len(), 1);
        assert!(state.active_buf().is_some());
    }

    #[test]
    fn open_same_file_twice_reuses_id() {
        let mut state = AppState::new();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "hello").unwrap();
        let id1 = state.open_file(tmp.path()).unwrap();
        let id2 = state.open_file(tmp.path()).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn open_nonexistent_file_returns_error() {
        let mut state = AppState::new();
        let result = state.open_file(std::path::Path::new("/nonexistent/path/file.txt"));
        assert!(result.is_err());
    }

    // ── cycle_tab ─────────────────────────────────────────────────

    #[test]
    fn cycle_tab_forward() {
        let (mut state, _files) = state_with_tabs(3);
        let tabs = state.tabs.clone();
        assert_eq!(state.active_buffer, Some(tabs[2]));
        state.cycle_tab(1);
        assert_eq!(state.active_buffer, Some(tabs[0]));
    }

    #[test]
    fn cycle_tab_backward() {
        let (mut state, _files) = state_with_tabs(3);
        let tabs = state.tabs.clone();
        state.active_buffer = Some(tabs[0]);
        state.cycle_tab(-1);
        assert_eq!(state.active_buffer, Some(tabs[2]));
    }

    #[test]
    fn cycle_tab_empty_noop() {
        let mut state = AppState::new();
        state.cycle_tab(1);
        assert!(state.active_buffer.is_none());
    }

    #[test]
    fn cycle_tab_single_stays() {
        let (mut state, _tmp) = state_with_file();
        let active = state.active_buffer;
        state.cycle_tab(1);
        assert_eq!(state.active_buffer, active);
        state.cycle_tab(-1);
        assert_eq!(state.active_buffer, active);
    }

    // ── close_active_tab / close_tab_by_id ────────────────────────

    #[test]
    fn close_active_tab_removes_tab() {
        let (mut state, _files) = state_with_tabs(3);
        let tabs = state.tabs.clone();
        state.active_buffer = Some(tabs[1]);
        state.close_active_tab();
        assert_eq!(state.tabs.len(), 2);
        assert!(!state.tabs.contains(&tabs[1]));
        assert!(state.active_buffer.is_some());
    }

    #[test]
    fn close_last_tab_sets_none() {
        let (mut state, _tmp) = state_with_file();
        state.close_active_tab();
        assert!(state.active_buffer.is_none());
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn close_tab_by_id_specific() {
        let (mut state, _files) = state_with_tabs(3);
        let tabs = state.tabs.clone();
        close_tab_by_id(&mut state, tabs[0]);
        assert_eq!(state.tabs.len(), 2);
        assert!(!state.tabs.contains(&tabs[0]));
    }

    // ── point_in_rect ─────────────────────────────────────────────

    #[test]
    fn point_in_rect_inside() {
        let r = Rect::new(10, 20, 30, 15);
        assert!(point_in_rect(10, 20, r));
        assert!(point_in_rect(25, 30, r));
        assert!(point_in_rect(39, 34, r));
    }

    #[test]
    fn point_in_rect_outside() {
        let r = Rect::new(10, 20, 30, 15);
        assert!(!point_in_rect(9, 20, r));
        assert!(!point_in_rect(40, 20, r));
        assert!(!point_in_rect(10, 19, r));
        assert!(!point_in_rect(10, 35, r));
    }

    #[test]
    fn point_in_rect_zero_size() {
        let r = Rect::new(5, 5, 0, 0);
        assert!(!point_in_rect(5, 5, r));
    }

    // ── handle_focus_next_pane ────────────────────────────────────

    #[test]
    fn focus_cycles_editor_only() {
        let mut state = AppState::new();
        state.layout.show_file_tree = false;
        state.layout.show_git_panel = false;
        state.focus.set_active(PanelId::Editor);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::Editor);
    }

    #[test]
    fn focus_cycles_with_file_tree() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.layout.show_git_panel = false;
        state.focus.set_active(PanelId::Editor);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::FileTree);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::Editor);
    }

    #[test]
    fn focus_cycles_all_panels() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.layout.show_git_panel = true;
        state.focus.set_active(PanelId::FileTree);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::Editor);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::GitPanel);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::FileTree);
    }

    // ── build_status_line ─────────────────────────────────────────

    #[test]
    fn build_status_line_no_buffer() {
        let state = AppState::new();
        let status = state.build_status_line();
        assert!(status.file_path.is_empty());
        assert!(!status.dirty);
        assert_eq!(status.cursor_line, 0);
    }

    #[test]
    fn build_status_line_with_buffer() {
        let (state, _tmp) = state_with_file();
        let status = state.build_status_line();
        assert!(!status.file_path.is_empty());
        assert!(!status.dirty);
        assert_eq!(status.cursor_line, 1);
        assert_eq!(status.cursor_col, 1);
    }

    // ── build_git_branch_display ──────────────────────────────────

    #[test]
    fn git_branch_empty() {
        let state = AppState::new();
        assert!(state.build_git_branch_display().is_empty());
    }

    #[test]
    fn git_branch_with_ahead_behind() {
        let mut state = AppState::new();
        state.git_branch = "main".to_string();
        state.git_ahead = 2;
        state.git_behind = 1;
        let display = state.build_git_branch_display();
        assert!(display.contains("main"));
        assert!(display.contains("↑2"));
        assert!(display.contains("↓1"));
    }

    #[test]
    fn git_branch_no_ahead_behind() {
        let mut state = AppState::new();
        state.git_branch = "feature".to_string();
        assert_eq!(state.build_git_branch_display(), "feature");
    }

    // ── detect_file_type ──────────────────────────────────────────

    #[test]
    fn detect_file_type_rust() {
        let mut state = AppState::new();
        let tmp = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
        std::fs::write(tmp.path(), "fn main() {}").unwrap();
        state.open_file(tmp.path()).unwrap();
        assert_eq!(state.detect_file_type().to_lowercase(), "rust");
    }

    #[test]
    fn detect_file_type_no_buffer() {
        let state = AppState::new();
        assert!(state.detect_file_type().is_empty());
    }

    // ── handle_command dispatch ────────────────────────────────────

    #[test]
    fn command_quit_returns_quit() {
        let mut state = AppState::new();
        assert!(matches!(
            handle_command(&AppCommand::Quit, &mut state),
            Control::Quit
        ));
    }

    #[test]
    fn command_force_quit_returns_quit() {
        let mut state = AppState::new();
        assert!(matches!(
            handle_command(&AppCommand::ForceQuit, &mut state),
            Control::Quit
        ));
    }

    #[test]
    fn command_close_tab() {
        let (mut state, _files) = state_with_tabs(2);
        let _ = handle_command(&AppCommand::CloseTab, &mut state);
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn command_next_prev_tab() {
        let (mut state, _files) = state_with_tabs(3);
        let tabs = state.tabs.clone();
        state.active_buffer = Some(tabs[0]);
        let _ = handle_command(&AppCommand::NextTab, &mut state);
        assert_eq!(state.active_buffer, Some(tabs[1]));
        let _ = handle_command(&AppCommand::PrevTab, &mut state);
        assert_eq!(state.active_buffer, Some(tabs[0]));
    }

    #[test]
    fn command_enter_modes() {
        let mut state = AppState::new();
        let _ = handle_command(&AppCommand::EnterInsertMode, &mut state);
        assert_eq!(state.vim.mode, VimMode::Insert);
        let _ = handle_command(&AppCommand::EnterNormalMode, &mut state);
        assert_eq!(state.vim.mode, VimMode::Normal);
        let _ = handle_command(&AppCommand::EnterVisualMode, &mut state);
        assert_eq!(state.vim.mode, VimMode::Visual);
    }

    // ── error handler ─────────────────────────────────────────────

    #[test]
    fn error_handler_updates_state() {
        let mut state = AppState::new();
        let mut global = LuneGlobal::default();
        let err = anyhow::anyhow!("test error");
        let result = error(err, &mut state, &mut global).unwrap();
        assert!(matches!(result, Control::Changed));
        assert_eq!(state.error_count, 1);
        assert!(state.last_error.contains("test error"));
    }

    // ── init ──────────────────────────────────────────────────────

    #[test]
    fn init_returns_ok() {
        let mut state = AppState::new();
        let mut global = LuneGlobal::default();
        assert!(init(&mut state, &mut global).is_ok());
    }

    // ── event dispatch ────────────────────────────────────────────

    #[test]
    fn event_ai_without_sessions_is_continue() {
        let mut state = AppState::new();
        let mut global = LuneGlobal::default();
        let ai_event = AppEvent::Ai(crate::event::AiEvent::OutputChanged);
        let result = event(&ai_event, &mut state, &mut global).unwrap();
        // No sessions → poll_all() returns false → Continue.
        assert!(matches!(result, Control::Continue));
    }

    #[test]
    fn event_command_quit() {
        let mut state = AppState::new();
        let mut global = LuneGlobal::default();
        let cmd_event = AppEvent::Command(AppCommand::Quit);
        let result = event(&cmd_event, &mut state, &mut global).unwrap();
        assert!(matches!(result, Control::Quit));
    }

    // ── helpers ───────────────────────────────────────────────────

    #[test]
    fn apply_motion_without_buffer_returns_changed() {
        let mut state = AppState::new();
        let result = apply_motion(&mut state, |_buf| {});
        assert!(matches!(result, Control::Changed));
    }

    #[test]
    fn apply_buf_edit_without_buffer_does_not_panic() {
        let mut state = AppState::new();
        apply_buf_edit(&mut state, TextBuffer::undo);
    }

    #[test]
    fn close_overlay_clears_active() {
        let mut state = AppState::new();
        state.overlay.open_command_palette();
        assert!(state.overlay.is_active());
        close_overlay(&mut state);
        assert!(!state.overlay.is_active());
    }

    // ── handle_save ───────────────────────────────────────────────

    #[test]
    fn handle_save_with_file() {
        let (mut state, _tmp) = state_with_file();
        if let Some(buf) = state.active_buf_mut() {
            buf.insert(Position::new(0, 0), "x");
        }
        let result = handle_save(&mut state);
        assert!(matches!(result, Control::Changed));
        assert!(state.status_message.contains("Saved"));
    }

    #[test]
    fn handle_save_no_buffer() {
        let mut state = AppState::new();
        let result = handle_save(&mut state);
        assert!(matches!(result, Control::Changed));
    }

    // ── collect_editor_context ──────────────────────────────────────

    #[test]
    fn collect_context_empty_state() {
        let state = AppState::new();
        let ctx = state.collect_editor_context();
        assert!(ctx.workspace_root.is_none());
        assert!(ctx.active_file.is_none());
        assert!(ctx.open_tabs.is_empty());
        assert!(ctx.selection.is_none());
        assert!(ctx.git_status.is_none());
    }

    #[test]
    fn collect_context_with_file() {
        let (state, _tmp) = state_with_file();
        let ctx = state.collect_editor_context();
        assert!(ctx.active_file.is_some());
        let file = ctx.active_file.unwrap();
        assert_eq!(file.cursor_line, 1);
        assert_eq!(file.cursor_col, 1);
        assert!(file.total_lines > 0);
        assert!(!ctx.open_tabs.is_empty());
    }

    #[test]
    fn collect_context_env_vars_with_file() {
        let (state, _tmp) = state_with_file();
        let ctx = state.collect_editor_context();
        let env = ctx.to_env_vars();
        assert!(env.contains_key("LUNE_CTX_FILE"));
        assert!(env.contains_key("LUNE_CTX_LINE"));
        assert!(env.contains_key("LUNE_CTX_COL"));
    }

    // ── AI command handlers ─────────────────────────────────────────

    #[test]
    fn ai_ask_selection_opens_panel() {
        let mut state = AppState::new();
        let result = handle_ai_ask_selection(&mut state);
        assert!(matches!(result, Control::Changed));
        // Always opens the AI panel regardless of whether text is selected.
        assert!(state.layout.show_ai_panel);
    }

    #[test]
    fn ai_refactor_no_file_warns() {
        let mut state = AppState::new();
        let result = handle_ai_refactor_file(&mut state);
        assert!(matches!(result, Control::Changed));
        assert!(!state.overlay.notifications.is_empty());
    }

    #[test]
    fn ai_summarize_no_git_warns() {
        let mut state = AppState::new();
        let result = handle_ai_summarize_changes(&mut state);
        assert!(matches!(result, Control::Changed));
        assert!(!state.overlay.notifications.is_empty());
    }

    #[test]
    fn ai_commands_dispatch() {
        let mut state = AppState::new();
        // All three should return Changed (either warn or proceed).
        let r1 = handle_command(&AppCommand::AiAskSelection, &mut state);
        assert!(matches!(r1, Control::Changed));
        let r2 = handle_command(&AppCommand::AiRefactorFile, &mut state);
        assert!(matches!(r2, Control::Changed));
        let r3 = handle_command(&AppCommand::AiSummarizeChanges, &mut state);
        assert!(matches!(r3, Control::Changed));
    }
}
