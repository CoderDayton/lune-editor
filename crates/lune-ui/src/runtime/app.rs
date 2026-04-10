//! Application state and rat-salsa integration.
//!
//! This module contains the global context (`LuneGlobal`) and application
//! state (`AppState`) used by the rat-salsa event loop, plus the four
//! function pointers required by `run_tui`.

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::Error;
use crossbeam::channel::{self, Receiver, TryRecvError};
use rat_salsa::poll::{PollCrossterm, PollTimers};
use rat_salsa::{Control, RunConfig, SalsaAppContext, SalsaContext, run_tui};

use crate::primitives::{
    Block, Borders, Buffer, Constraint, CtEvent, Direction, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, Layout, Line, MouseButton, MouseEvent, MouseEventKind, Rect, Style, Tabs, Widget,
};

use lune_core::prelude::*;
use lune_core::settings::Settings;
use lune_core::watcher::{FileWatcher, WatchEvent};
use lune_core::workspace::EntryKind;
use lune_core::workspace_state::make_relative;
use lune_git::{GitService, GutterMarks};

use lune_ai::context::{
    EditorContext, FileContext, GitStatusSummary, SelectionContext, TabContext,
    extract_selection_text,
};
use lune_ai::{AiClientKind, AiManager, TermSize as AiTermSize};

use arboard::Clipboard;

use crate::highlight;
use crate::highlight::theme::SyntaxTheme;
use crate::theme::Theme;
use crate::theme_config::{ThemeId, ThemeRegistry};

use crate::event::{AppCommand, AppEvent};
use crate::focus::{FocusManager, PanelId};
use crate::keybindings::Keymap;
use crate::layout::{self, LayoutSplits, LayoutState};
use crate::runtime::terminal_layouts;
use crate::vim::{VimAction, VimMode, VimState};
use crate::widgets::editor_pane::{self, ViewportState};
use crate::widgets::file_tree::{self, FileTreeState};
use crate::widgets::git_panel::{self, GitPanelState};
use crate::widgets::overlay::{self, NotificationLevel, OverlayState};
use crate::widgets::status_bar::{self, StatusLineState};
use crate::widgets::tab_bar::{self, TabManager};
use crate::widgets::terminal;

mod agent_layouts;
mod agent_tab;
mod ai_commands;
mod editor_actions;
mod editor_modes;
mod git_commands;
mod overlay_handlers;
mod ui_interaction;
mod ui_render;
mod workspace_commands;

#[cfg(test)]
use self::agent_layouts::apply_agent_layout_entry;
use self::agent_layouts::{
    agent_pane_term_size, handle_layout_picker_key, open_agent_layout_picker,
    open_agent_layout_picker_with_selection, open_save_agent_layout_dialog,
};
#[cfg(test)]
use self::agent_tab::split_from_point;
use self::agent_tab::{
    begin_agent_split_session, handle_agents_mouse_down, handle_agents_mouse_drag,
    handle_agents_tab_key, handle_ai_client_picker_key, render_agents_tab, sync_agent_session_size,
};
use self::ai_commands::handle_ai_command;
#[cfg(test)]
use self::ai_commands::{
    handle_ai_new_session, handle_ai_refactor_file, handle_ai_summarize_changes,
};
use self::editor_actions::{apply_buf_edit, apply_motion};
use self::editor_modes::{
    handle_insert_mode, handle_normal_mode, handle_vim_command_key, handle_visual_mode,
};
use self::git_commands::handle_git_command;
use self::overlay_handlers::{handle_overlay_key, update_find_search};
#[cfg(test)]
use self::ui_interaction::handle_focus_next_pane;
use self::ui_interaction::{handle_panel_command, handle_terminal_event};
use self::ui_render::{render_editor_tab, render_root_tabs};
use self::workspace_commands::{handle_save, handle_save_all, handle_workspace_command};

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

/// Top-level application tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RootTab {
    /// Main editor workspace.
    #[default]
    Editor,
    /// AI/agent session overview.
    Agents,
}

impl RootTab {
    /// Convert to a `Tabs` selected index.
    const fn as_index(self) -> usize {
        match self {
            Self::Editor => 0,
            Self::Agents => 1,
        }
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
    /// Active top-level UI tab.
    pub root_tab: RootTab,
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
    /// Whether editor rendering should keep the viewport snapped to the cursor.
    ///
    /// Mouse-wheel scrolling disables this so users can freely browse away
    /// from the cursor; cursor-movement/edit actions re-enable it.
    viewport_follow_cursor: bool,
    /// Overlay state (command palette, notifications).
    pub overlay: OverlayState,
    /// Last computed layout splits (for mouse hit-testing).
    pub last_splits: Option<LayoutSplits>,
    /// Area of the top-level root tabs row.
    last_root_tabs_area: Option<Rect>,
    /// Whether the mouse is currently dragging a panel border.
    pub dragging_border: Option<DragBorder>,
    /// The editor content area from the last render (for mouse mapping).
    pub last_editor_content_area: Option<Rect>,
    /// The Agents tab content area from the last render.
    last_agents_content_area: Option<Rect>,
    /// Pane rectangles from the last Agents tab render.
    last_agent_pane_rects: Vec<(super::tiling::PaneId, Rect)>,
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
    /// Last left-click info for editor multi-click gestures.
    last_click: Option<MouseClickState>,
    /// Anchor for an in-progress block selection drag.
    block_select_anchor: Option<Position>,
    /// Last known mouse position within the terminal.
    last_mouse_pos: Option<(u16, u16)>,
    /// AI session manager.
    pub ai_manager: AiManager,
    /// Agents tab tiling layout state.
    pub agents_tab: super::agents::AgentsTabState,
    /// Pane ID waiting for an AI client selection (from the picker).
    pub agents_tab_pending_pane: Option<super::tiling::PaneId>,
    /// User-saved agent layout templates persisted across the app.
    saved_agent_layouts: Vec<super::tiling::SavedAgentLayout>,
    /// Last known AI terminal size (to avoid redundant resizes).
    #[allow(dead_code)]
    last_ai_term_size: Option<AiTermSize>,
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
    /// Dragging the editor scrollbar thumb/track.
    Scrollbar,
}

/// Recent mouse click state for editor multi-click gestures.
#[derive(Clone, Copy, Debug)]
struct MouseClickState {
    at: Instant,
    col: u16,
    row: u16,
    count: u8,
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
            root_tab: RootTab::Editor,
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
            viewport_follow_cursor: true,
            overlay: OverlayState::default(),
            last_splits: None,
            last_root_tabs_area: None,
            dragging_border: None,
            last_editor_content_area: None,
            last_agents_content_area: None,
            last_agent_pane_rects: Vec::new(),
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
            block_select_anchor: None,
            last_mouse_pos: None,
            ai_manager: AiManager::new(),
            agents_tab: super::agents::AgentsTabState::new(),
            agents_tab_pending_pane: None,
            saved_agent_layouts: Vec::new(),
            last_ai_term_size: None,
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

    /// Switch the active top-level UI tab.
    pub const fn set_root_tab(&mut self, tab: RootTab) {
        self.root_tab = tab;
        // Keep focus on the editor panel by default when changing root tabs.
        self.focus.set_active(PanelId::Editor);
    }

    /// Apply loaded [`Settings`] to the application state.
    ///
    /// Should be called once after construction and settings loading,
    /// before the event loop starts.  Maps settings fields onto the
    /// corresponding `AppState` fields (layout, vim mode, theme).
    pub fn apply_settings(&mut self, settings: &Settings) {
        // Layout / UI
        self.layout.show_file_tree = settings.ui.show_file_tree;
        self.layout
            .set_file_tree_width_pct(settings.ui.file_tree_width_pct);
        self.layout
            .set_right_panel_width_pct(settings.ui.right_panel_width_pct);

        // Editor / vim
        self.vim_enabled = settings.editor.vim_mode;
        if self.vim_enabled {
            self.vim.enter_normal();
        } else {
            // Non-vim mode: start in Insert so keystrokes type text by default.
            // User can still Escape → Normal to block typing, then `i` to resume.
            self.vim.enter_insert();
        }

        // File tree — apply show_hidden from settings.
        if let Some(ref mut ws) = self.workspace {
            ws.set_show_hidden(settings.file_tree.show_hidden);
            if let Err(e) = self.file_tree.refresh(ws) {
                log::error!("Failed to refresh file tree after settings: {e}");
            }
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
        self.load_saved_agent_layouts();
    }

    /// Borrow the state database, if set.
    #[must_use]
    pub const fn state_db(&self) -> Option<&StateDb> {
        self.state_db.as_ref()
    }

    fn load_saved_agent_layouts(&mut self) {
        const AGENT_LAYOUTS_KEY: &[u8] = b"ui:agent_layouts";

        let Some(db) = self.state_db() else {
            return;
        };

        match db.get_raw(AGENT_LAYOUTS_KEY) {
            Ok(Some(layouts)) => {
                self.saved_agent_layouts = layouts;
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("failed to load saved agent layouts: {e}");
            }
        }
    }

    fn persist_saved_agent_layouts(&self) {
        const AGENT_LAYOUTS_KEY: &[u8] = b"ui:agent_layouts";

        let Some(db) = self.state_db() else {
            return;
        };

        if let Err(e) = db.put_raw(AGENT_LAYOUTS_KEY, &self.saved_agent_layouts) {
            log::warn!("failed to persist saved agent layouts: {e}");
        }
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

        // Restore undo/redo history per buffer.
        if let Some(db) = &self.state_db {
            for &id in &self.tabs {
                let file_path = self.registry.get(id).and_then(|b| b.file_path.clone());
                let Some(file_path) = file_path else {
                    continue;
                };
                match db.get_undo(&root, &file_path) {
                    Ok(Some(undo_state)) => {
                        if let Some(buf) = self.registry.get_mut(id) {
                            if !buf.restore_undo_state(undo_state) {
                                log::debug!(
                                    "undo state hash mismatch for {}, discarding",
                                    file_path.display()
                                );
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("failed to load undo state for {}: {e}", file_path.display());
                    }
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

    /// Persist undo/redo history for all open buffers to sled.
    ///
    /// Called on clean exit. Iterates open buffers, extracts undo state
    /// (capped at 1000 transactions), and writes each to the database.
    pub fn persist_undo_history(&self) {
        let (Some(db), Some(ws)) = (self.state_db(), self.workspace.as_ref()) else {
            return;
        };
        let root = ws.root().to_path_buf();

        for &id in &self.tabs {
            let Some(buf) = self.registry.get(id) else {
                continue;
            };
            let Some(ref file_path) = buf.file_path else {
                continue;
            };
            let state = buf.extract_undo_state(1000);
            if state.undo_entries.is_empty() && state.redo_entries.is_empty() {
                continue;
            }
            if let Err(e) = db.put_undo(&root, file_path, &state) {
                log::warn!("persist undo for {}: {e}", file_path.display());
            }
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
                            Ok(new_marks) => {
                                self.gutter_marks.insert(id, new_marks);
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

    /// Move focus to the file tree when there is no open buffer.
    ///
    /// Intended to be called once after startup file/workspace loading so
    /// the user lands on the file tree (rather than an empty editor pane)
    /// when they launched Lune without any specific file.
    pub fn focus_file_tree_if_no_buffer(&mut self) {
        if self.active_buffer.is_none()
            && self.layout.show_file_tree
            && self.focus.is_focused(PanelId::Editor)
        {
            self.focus.focus(PanelId::FileTree);
        }
    }

    /// Build the status line state from current app state.
    fn build_status_line(&self) -> StatusLineState {
        let (file_path, dirty, cursor_line, cursor_col, selection_chars, line_ending) = self
            .active_buf()
            .map(|b| {
                let fp = b
                    .file_path
                    .as_ref()
                    .map_or_else(String::new, |p| self.status_path_display(p));
                let pos = &b.cursor.primary.head;
                let sel_chars = if b.cursor.primary.is_cursor() {
                    0
                } else {
                    let (s, e) = b.cursor.primary.ordered();
                    let start_idx = b.pos_to_char(s);
                    let end_idx = b.pos_to_char(e);
                    end_idx.saturating_sub(start_idx)
                };
                // Check first line for CRLF to avoid full-buffer allocation.
                let le = if b.line(0).is_some_and(|l| l.contains("\r\n")) {
                    "CRLF"
                } else {
                    "LF"
                };
                (fp, b.is_dirty(), pos.line + 1, pos.col + 1, sel_chars, le)
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
            selection_chars,
            line_ending,
            vim_cmdline: (self.vim.mode == VimMode::Command).then(|| self.vim.cmdline.clone()),
        }
    }

    /// Build a status-bar file path display rooted at the workspace folder.
    ///
    /// Examples:
    /// - `lune-editor/Cargo.toml`
    /// - `lune-editor/crates/lune-ui/src/runtime/app.rs`
    fn status_path_display(&self, path: &Path) -> String {
        let Some(ws) = self.workspace.as_ref() else {
            return path.display().to_string();
        };

        let root = ws.root();
        let Ok(rel) = path.strip_prefix(root) else {
            return path.display().to_string();
        };

        let root_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace");
        if rel.as_os_str().is_empty() {
            root_name.to_string()
        } else {
            format!("{root_name}/{}", rel.display())
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
#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)] // TUI coords always fit u16
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

    if area.width == 0 || area.height == 0 {
        return Ok(());
    }

    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    let root_tabs_area = root_chunks[0];
    let content_area = root_chunks[1];

    state.last_root_tabs_area = Some(root_tabs_area);
    render_root_tabs(root_tabs_area, buf, state);

    match state.root_tab {
        RootTab::Editor => render_editor_tab(content_area, buf, state),
        RootTab::Agents => render_agents_tab(content_area, buf, state),
    }

    // Render overlays on top.
    overlay::render_overlay(area, buf, &mut state.overlay, &state.theme);

    Ok(())
}

/// Labels for top-level root tabs.
const ROOT_TAB_TITLES: [&str; 2] = ["Editor", "Agents"];
/// Divider text between root tabs.
const ROOT_TAB_DIVIDER: &str = " ";

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

/// Handle AI session events (poll all sessions for new output).
fn handle_ai_event(state: &mut AppState) -> Control<AppEvent> {
    let polled = state.ai_manager.poll_all();
    let finished_cleaned = cleanup_finished_agent_sessions(state);
    let orphaned_cleaned = prune_orphaned_agent_panes(state);

    if polled || finished_cleaned || orphaned_cleaned {
        Control::Changed
    } else {
        Control::Continue
    }
}

fn cleanup_finished_agent_sessions(state: &mut AppState) -> bool {
    let finished: Vec<_> = state
        .agents_tab
        .panes
        .iter()
        .filter_map(|(pane_id, pane)| {
            let session = state.ai_manager.session(pane.session_id)?;
            match session.state() {
                lune_ai::SessionState::Starting | lune_ai::SessionState::Running => None,
                lune_ai::SessionState::Exited(code) => Some((
                    *pane_id,
                    pane.session_id,
                    pane.title.clone(),
                    NotificationLevel::Info,
                    format!("{} exited ({code})", pane.title),
                )),
                lune_ai::SessionState::Error => Some((
                    *pane_id,
                    pane.session_id,
                    pane.title.clone(),
                    NotificationLevel::Error,
                    format!("{} session errored", pane.title),
                )),
            }
        })
        .collect();

    if finished.is_empty() {
        return false;
    }

    for (pane_id, session_id, _title, level, message) in finished {
        state.agents_tab.discard_pane(pane_id);
        state.ai_manager.close_session(session_id);
        state.overlay.notify(message, level);
    }

    true
}

fn prune_orphaned_agent_panes(state: &mut AppState) -> bool {
    let stale_panes: Vec<_> = state
        .agents_tab
        .panes
        .iter()
        .filter_map(|(pane_id, pane)| {
            state
                .ai_manager
                .session(pane.session_id)
                .is_none()
                .then_some(*pane_id)
        })
        .collect();

    if stale_panes.is_empty() {
        return false;
    }

    for pane_id in stale_panes {
        state.agents_tab.discard_pane(pane_id);
    }

    true
}

/// Close the active overlay and return focus.
fn close_overlay(state: &mut AppState) {
    state.overlay.close();
    state.focus.focus_return();
}

/// Check if a point is inside a rect.
const fn point_in_rect(col: u16, row: u16, r: Rect) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
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
        // If closing left nothing to edit, fall back to the file tree so the
        // user isn't stuck focused on an empty editor pane.
        if state.active_buffer.is_none()
            && state.layout.show_file_tree
            && state.focus.is_focused(PanelId::Editor)
        {
            state.focus.focus(PanelId::FileTree);
        }
    }
}

// ── Command handling ──────────────────────────────────────────────────

/// Handle application commands.
#[allow(clippy::too_many_lines)]
fn handle_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    if let Some(control) = handle_workspace_command(cmd, state) {
        return control;
    }
    if let Some(control) = handle_ai_command(cmd, state) {
        return control;
    }
    if let Some(control) = handle_git_command(cmd, state) {
        return control;
    }

    match cmd {
        AppCommand::Quit | AppCommand::ForceQuit => Control::Quit,
        AppCommand::CloseTab => {
            state.close_active_tab();
            state.viewport_follow_cursor = true;
            Control::Changed
        }
        AppCommand::NextTab => {
            state.cycle_tab(1);
            state.viewport_follow_cursor = true;
            Control::Changed
        }
        AppCommand::PrevTab => {
            state.cycle_tab(-1);
            state.viewport_follow_cursor = true;
            Control::Changed
        }
        AppCommand::ShowEditorTab => {
            state.set_root_tab(RootTab::Editor);
            Control::Changed
        }
        AppCommand::ShowAgentsTab => {
            state.set_root_tab(RootTab::Agents);
            Control::Changed
        }
        AppCommand::ToggleAgentsTab => {
            let next = match state.root_tab {
                RootTab::Editor => RootTab::Agents,
                RootTab::Agents => RootTab::Editor,
            };
            state.set_root_tab(next);
            Control::Changed
        }
        // Panel toggles and focus.
        AppCommand::ToggleFileTree
        | AppCommand::ToggleGitPanel
        | AppCommand::FocusNextPane
        | AppCommand::OpenCommandPalette
        | AppCommand::OpenFilePicker
        | AppCommand::OpenLanguagePicker
        | AppCommand::OpenThemePicker => handle_panel_command(cmd, state),
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
        AppCommand::ToggleVimMode => {
            state.vim_enabled = !state.vim_enabled;
            // Keep cached settings in sync so a hot-reload doesn't immediately revert.
            if let Some(s) = state.cached_settings.as_mut() {
                s.editor.vim_mode = state.vim_enabled;
            }
            if state.vim_enabled {
                state.vim.enter_normal();
                state
                    .overlay
                    .notify("Vim mode enabled", NotificationLevel::Info);
            } else {
                state.vim.enter_insert();
                state
                    .overlay
                    .notify("Vim mode disabled", NotificationLevel::Info);
            }
            Control::Changed
        }
        AppCommand::Find => {
            state.overlay.open_find();
            state.focus.focus(PanelId::CommandPalette);
            // Trigger initial search if there's already text in the find input.
            if !state.overlay.find_replace.find_input.is_empty() {
                update_find_search(state);
            }
            Control::Changed
        }
        AppCommand::Replace => {
            state.overlay.open_find_replace();
            state.focus.focus(PanelId::CommandPalette);
            if !state.overlay.find_replace.find_input.is_empty() {
                update_find_search(state);
            }
            Control::Changed
        }
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
        // Agent pane commands.
        AppCommand::AgentSplitAuto => begin_agent_split_session(state, None),
        AppCommand::AgentSplitVertical => begin_agent_split_session(
            state,
            Some((
                super::tiling::SplitDirection::Vertical,
                super::tiling::SplitSide::Second,
            )),
        ),
        AppCommand::AgentSplitHorizontal => begin_agent_split_session(
            state,
            Some((
                super::tiling::SplitDirection::Horizontal,
                super::tiling::SplitSide::Second,
            )),
        ),
        AppCommand::AgentClosePane => {
            if let Some(session_id) = state.agents_tab.close_focused() {
                state.ai_manager.close_session(session_id);
            }
            Control::Changed
        }
        AppCommand::AgentFocusNext => {
            state.agents_tab.focus_next();
            Control::Changed
        }
        AppCommand::AgentFocusPrev => {
            state.agents_tab.focus_prev();
            Control::Changed
        }
        AppCommand::AgentToggleZoom => {
            state.agents_tab.toggle_zoom();
            Control::Changed
        }
        AppCommand::AgentApplyLayout => {
            open_agent_layout_picker(state);
            Control::Changed
        }
        AppCommand::AgentSaveLayout => {
            if state.agents_tab.layout.is_some() {
                open_save_agent_layout_dialog(state);
            } else {
                state
                    .overlay
                    .notify("No agent layout to save yet", NotificationLevel::Warning);
            }
            Control::Changed
        }
        AppCommand::AgentSaveLayoutConfirmed(name) => {
            if let Some(saved) = state.agents_tab.save_layout(name.clone()) {
                match terminal_layouts::upsert_saved_layout(&mut state.saved_agent_layouts, saved) {
                    Some(terminal_layouts::SaveLayoutOutcome::Inserted { name, .. }) => {
                        state
                            .overlay
                            .notify(format!("Saved layout: {name}"), NotificationLevel::Info);
                        state.persist_saved_agent_layouts();
                    }
                    Some(terminal_layouts::SaveLayoutOutcome::Updated { name, .. }) => {
                        state.overlay.notify(
                            format!("Updated saved layout: {name}"),
                            NotificationLevel::Info,
                        );
                        state.persist_saved_agent_layouts();
                    }
                    None => {
                        state
                            .overlay
                            .notify("Layout name cannot be empty", NotificationLevel::Warning);
                    }
                }
            } else {
                state
                    .overlay
                    .notify("No agent layout to save yet", NotificationLevel::Warning);
            }
            Control::Changed
        }
        AppCommand::AgentDeleteSavedLayout(index) => {
            if let Some(deleted) =
                terminal_layouts::delete_saved_layout(&mut state.saved_agent_layouts, *index)
            {
                state.persist_saved_agent_layouts();
                state.overlay.notify(
                    format!("Deleted saved layout: {}", deleted.name),
                    NotificationLevel::Info,
                );
                let next_selection = if state.saved_agent_layouts.is_empty() {
                    None
                } else {
                    Some((*index).min(state.saved_agent_layouts.len() - 1))
                };
                open_agent_layout_picker_with_selection(next_selection, state);
            } else {
                state
                    .overlay
                    .notify("Saved layout not found", NotificationLevel::Warning);
            }
            Control::Changed
        }
        AppCommand::AgentRenameSavedLayoutConfirmed { index, name } => {
            match terminal_layouts::rename_saved_layout(
                &mut state.saved_agent_layouts,
                *index,
                name,
            ) {
                Some(terminal_layouts::RenameLayoutOutcome::Renamed { to, .. }) => {
                    state.persist_saved_agent_layouts();
                    state.overlay.notify(
                        format!("Renamed saved layout to: {to}"),
                        NotificationLevel::Info,
                    );
                    open_agent_layout_picker_with_selection(Some(*index), state);
                }
                Some(terminal_layouts::RenameLayoutOutcome::ReplacedExisting {
                    index: selected,
                    to,
                    ..
                }) => {
                    state.persist_saved_agent_layouts();
                    state.overlay.notify(
                        format!("Renamed layout and replaced existing: {to}"),
                        NotificationLevel::Info,
                    );
                    open_agent_layout_picker_with_selection(Some(selected), state);
                }
                None => {
                    state
                        .overlay
                        .notify("Saved layout not found", NotificationLevel::Warning);
                }
            }
            Control::Changed
        }
        AppCommand::OpenSettings
        | AppCommand::OpenKeybindings
        | AppCommand::Save
        | AppCommand::SaveAll
        | AppCommand::OpenFile(_)
        | AppCommand::ToggleHiddenFiles
        | AppCommand::RevealInFileTree(_)
        | AppCommand::NewFile
        | AppCommand::NewDir
        | AppCommand::RenameEntry
        | AppCommand::DeleteEntry
        | AppCommand::CreateFileConfirmed(_)
        | AppCommand::CreateDirConfirmed(_)
        | AppCommand::RenameConfirmed { .. }
        | AppCommand::DeleteConfirmed(_)
        | AppCommand::ChangeLanguage(_) => unreachable!("workspace commands handled above"),
        AppCommand::AiAskSelection
        | AppCommand::AiRefactorFile
        | AppCommand::AiSummarizeChanges
        | AppCommand::AiOpenClientPicker
        | AppCommand::AiNewSession(_)
        | AppCommand::AiCloseSession
        | AppCommand::AiNextSession
        | AppCommand::AiPrevSession => unreachable!("AI commands handled above"),
        AppCommand::GitStage
        | AppCommand::GitUnstage
        | AppCommand::GitCommit
        | AppCommand::GitDiscard
        | AppCommand::GitRefresh
        | AppCommand::GitDiscardConfirmed(_)
        | AppCommand::GitCommitConfirmed(_)
        | AppCommand::GitStageHunk
        | AppCommand::GitUnstageHunk
        | AppCommand::GitDiscardHunk => unreachable!("git commands handled above"),
    }
}

/// Handle save command.
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
/// Session reader threads emit lightweight wake notifications into a shared
/// channel. This poller drains that channel and only emits `OutputChanged`
/// when there is real PTY activity to process.
pub struct PollAiSessions {
    /// Shared pending-activity flag flipped by session reader threads.
    pending_wake: Arc<AtomicBool>,
    /// Whether the last poll found changes.
    has_changes: bool,
}

impl PollAiSessions {
    /// Create a new AI session poller.
    #[must_use]
    pub const fn new(pending_wake: Arc<AtomicBool>) -> Self {
        Self {
            pending_wake,
            has_changes: false,
        }
    }
}

impl rat_salsa::poll::PollEvents<AppEvent, Error> for PollAiSessions {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn poll(&mut self) -> Result<bool, Error> {
        self.has_changes = self.pending_wake.swap(false, Ordering::AcqRel);
        Ok(self.has_changes)
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
    let ai_wake_flag = state.ai_manager.wake_flag();

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
            .poll(PollAiSessions::new(ai_wake_flag)),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tiling;
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

    fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
        (0..area.width)
            .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
            .collect()
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

    fn state_with_scratch_buffer() -> AppState {
        let mut state = AppState::new();
        let id = state.registry.new_scratch();
        state.active_buffer = Some(id);
        state.tabs.push(id);
        state
    }

    #[test]
    fn focus_cycles_editor_only() {
        let mut state = state_with_scratch_buffer();
        state.layout.show_file_tree = false;
        state.layout.show_git_panel = false;
        state.focus.set_active(PanelId::Editor);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::Editor);
    }

    #[test]
    fn focus_cycles_with_file_tree() {
        let mut state = state_with_scratch_buffer();
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
        let mut state = state_with_scratch_buffer();
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

    #[test]
    fn focus_cycle_skips_editor_when_no_buffer_is_open() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.layout.show_git_panel = true;
        state.focus.set_active(PanelId::FileTree);

        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::GitPanel);
        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::FileTree);
    }

    #[test]
    fn focus_next_pane_from_empty_editor_lands_on_file_tree() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.layout.show_git_panel = false;
        state.focus.set_active(PanelId::Editor);

        handle_focus_next_pane(&mut state);
        assert_eq!(state.focus.active(), PanelId::FileTree);
    }

    #[test]
    fn focus_file_tree_if_no_buffer_moves_focus() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.focus.set_active(PanelId::Editor);

        state.focus_file_tree_if_no_buffer();
        assert_eq!(state.focus.active(), PanelId::FileTree);
    }

    #[test]
    fn focus_file_tree_if_no_buffer_is_noop_with_hidden_tree() {
        let mut state = AppState::new();
        state.layout.show_file_tree = false;
        state.focus.set_active(PanelId::Editor);

        state.focus_file_tree_if_no_buffer();
        assert_eq!(state.focus.active(), PanelId::Editor);
    }

    #[test]
    fn focus_file_tree_if_no_buffer_preserves_git_panel_focus() {
        let mut state = AppState::new();
        state.layout.show_file_tree = true;
        state.layout.show_git_panel = true;
        state.focus.set_active(PanelId::GitPanel);

        state.focus_file_tree_if_no_buffer();
        assert_eq!(state.focus.active(), PanelId::GitPanel);
    }

    #[test]
    fn closing_last_tab_returns_focus_to_file_tree() {
        let (mut state, _tmp) = state_with_file();
        state.layout.show_file_tree = true;
        state.focus.set_active(PanelId::Editor);

        state.close_active_tab();

        assert!(state.active_buffer.is_none());
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
    fn command_switch_root_tabs() {
        let mut state = AppState::new();
        assert_eq!(state.root_tab, RootTab::Editor);

        let _ = handle_command(&AppCommand::ShowAgentsTab, &mut state);
        assert_eq!(state.root_tab, RootTab::Agents);

        let _ = handle_command(&AppCommand::ShowEditorTab, &mut state);
        assert_eq!(state.root_tab, RootTab::Editor);
    }

    #[test]
    fn toggle_agents_tab_flips_between_editor_and_agents() {
        let mut state = AppState::new();
        assert_eq!(state.root_tab, RootTab::Editor);

        let _ = handle_command(&AppCommand::ToggleAgentsTab, &mut state);
        assert_eq!(state.root_tab, RootTab::Agents);

        let _ = handle_command(&AppCommand::ToggleAgentsTab, &mut state);
        assert_eq!(state.root_tab, RootTab::Editor);
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
    fn command_toggle_git_panel() {
        let mut state = AppState::new();
        assert!(!state.layout.show_git_panel);

        let r = handle_command(&AppCommand::ToggleGitPanel, &mut state);
        assert!(matches!(r, Control::Changed));
        assert!(state.layout.show_git_panel);
        assert!(state.focus.is_focused(PanelId::GitPanel));

        // Toggle off.
        let r = handle_command(&AppCommand::ToggleGitPanel, &mut state);
        assert!(matches!(r, Control::Changed));
        assert!(!state.layout.show_git_panel);
        assert!(state.focus.is_focused(PanelId::Editor));
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

    #[test]
    fn ai_picker_escape_discards_pending_agent_pane() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        state.agents_tab_pending_pane = Some(pane_id);
        state.overlay.open_ai_client_picker();

        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let result = handle_ai_client_picker_key(&key, &mut state);

        assert!(matches!(result, Control::Changed));
        assert!(state.agents_tab.is_empty());
        assert!(state.agents_tab_pending_pane.is_none());
        assert!(!state.overlay.is_active());
    }

    #[test]
    fn ai_spawn_failure_discards_pending_agent_pane() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        state.agents_tab_pending_pane = Some(pane_id);

        let result = handle_ai_new_session(
            AiClientKind::Custom {
                name: "Missing".to_string(),
                command: "/definitely/not/a/real/command".to_string(),
            },
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert!(state.agents_tab.is_empty());
        assert!(state.agents_tab_pending_pane.is_none());
    }

    #[test]
    fn render_agents_tab_tracks_exact_rects_and_resizes_session() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        let session_id = state
            .ai_manager
            .new_session(
                AiClientKind::Shell,
                None,
                &std::collections::HashMap::new(),
                AiTermSize::new(24, 80),
            )
            .unwrap();
        state
            .agents_tab
            .register_pane(pane_id, session_id, "Shell".to_string());

        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        render_agents_tab(area, &mut buf, &mut state);

        assert_eq!(
            state.last_agents_content_area,
            Some(Rect::new(0, 0, 80, 11))
        );
        assert_eq!(
            state.last_agent_pane_rects,
            vec![(pane_id, Rect::new(0, 0, 80, 11))]
        );
        let size = state
            .ai_manager
            .session(session_id)
            .unwrap()
            .screen()
            .size();
        assert_eq!(size, (11, 80));

        state.ai_manager.close_all();
    }

    #[test]
    fn render_agents_tab_refreshes_cached_rects_on_area_change() {
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Agents);
        let pane_id = state.agents_tab.add_first_pane();
        let session_id = state
            .ai_manager
            .new_session(
                AiClientKind::Shell,
                None,
                &std::collections::HashMap::new(),
                AiTermSize::new(24, 80),
            )
            .unwrap();
        state
            .agents_tab
            .register_pane(pane_id, session_id, "Shell".to_string());

        let area1 = Rect::new(0, 0, 80, 12);
        let mut buf1 = Buffer::empty(area1);
        render_agents_tab(area1, &mut buf1, &mut state);
        assert_eq!(
            state.last_agent_pane_rects,
            vec![(pane_id, Rect::new(0, 0, 80, 11))]
        );

        let area2 = Rect::new(0, 0, 60, 18);
        let mut buf2 = Buffer::empty(area2);
        render_agents_tab(area2, &mut buf2, &mut state);

        assert_eq!(
            state.last_agent_pane_rects,
            vec![(pane_id, Rect::new(0, 0, 60, 17))]
        );
        let size = state
            .ai_manager
            .session(session_id)
            .unwrap()
            .screen()
            .size();
        assert_eq!(size, (17, 60));

        state.ai_manager.close_all();
    }

    #[test]
    fn render_switching_away_from_agents_clears_cached_rects() {
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Agents);
        let pane_id = state.agents_tab.add_first_pane();
        let session_id = state
            .ai_manager
            .new_session(
                AiClientKind::Shell,
                None,
                &std::collections::HashMap::new(),
                AiTermSize::new(24, 80),
            )
            .unwrap();
        state
            .agents_tab
            .register_pane(pane_id, session_id, "Shell".to_string());

        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        let mut global = LuneGlobal::default();
        render(area, &mut buf, &mut state, &mut global).unwrap();
        assert!(!state.last_agent_pane_rects.is_empty());

        state.set_root_tab(RootTab::Editor);
        render(area, &mut buf, &mut state, &mut global).unwrap();

        assert!(state.last_agent_pane_rects.is_empty());
        assert!(state.last_agents_content_area.is_none());

        state.ai_manager.close_all();
    }

    #[test]
    fn render_editor_tab_clears_agent_render_cache() {
        let mut state = AppState::new();
        state.last_agents_content_area = Some(Rect::new(0, 0, 80, 11));
        state.last_agent_pane_rects = vec![(tiling::PaneId(7), Rect::new(0, 0, 80, 11))];

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_editor_tab(area, &mut buf, &mut state);

        assert_eq!(state.last_agents_content_area, None);
        assert!(state.last_agent_pane_rects.is_empty());
    }

    #[test]
    fn render_agents_tab_empty_state_clears_editor_cache_and_stale_panes() {
        let mut state = AppState::new();
        state.last_editor_content_area = Some(Rect::new(2, 1, 70, 18));
        state.last_agent_pane_rects = vec![(tiling::PaneId(9), Rect::new(1, 1, 20, 8))];

        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        render_agents_tab(area, &mut buf, &mut state);

        assert_eq!(state.last_editor_content_area, None);
        assert_eq!(
            state.last_agents_content_area,
            Some(Rect::new(0, 0, 80, 11))
        );
        assert!(state.last_agent_pane_rects.is_empty());
    }

    #[test]
    fn render_agents_tab_empty_state_keeps_status_bar_visible() {
        let mut state = AppState::new();
        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);

        render_agents_tab(area, &mut buf, &mut state);

        let bottom = row_text(&buf, area, area.height - 1);
        assert!(bottom.contains("NORMAL"), "bottom row was {bottom:?}");
    }

    #[test]
    fn render_agents_tab_degraded_state_keeps_status_bar_visible() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        state.agents_tab.register_pane(
            pane_id,
            lune_ai::AiSessionId::new_v4(),
            "Shell".to_string(),
        );
        state.agents_tab.layout = None;

        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        render_agents_tab(area, &mut buf, &mut state);

        let bottom = row_text(&buf, area, area.height - 1);
        assert!(bottom.contains("NORMAL"), "bottom row was {bottom:?}");
    }

    #[test]
    fn prune_orphaned_agent_panes_discards_missing_sessions() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        state.agents_tab.register_pane(
            pane_id,
            lune_ai::AiSessionId::new_v4(),
            "Orphan".to_string(),
        );

        assert!(prune_orphaned_agent_panes(&mut state));
        assert!(state.agents_tab.is_empty());
    }

    #[test]
    fn handle_ai_event_cleans_finished_agent_sessions() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        let session_id = state
            .ai_manager
            .new_session(
                AiClientKind::Custom {
                    name: "true".to_string(),
                    command: "/bin/true".to_string(),
                },
                None,
                &std::collections::HashMap::new(),
                AiTermSize::new(10, 20),
            )
            .unwrap();
        state
            .agents_tab
            .register_pane(pane_id, session_id, "true".to_string());

        std::thread::sleep(std::time::Duration::from_millis(150));
        let result = handle_ai_event(&mut state);

        assert!(matches!(result, Control::Changed));
        assert!(state.agents_tab.is_empty());
        assert!(state.ai_manager.is_empty());
        assert!(!state.overlay.notifications.is_empty());
    }

    #[test]
    fn split_from_point_prefers_left_and_top_sides() {
        let rect = Rect::new(10, 10, 40, 20);
        assert_eq!(
            split_from_point(rect, 12, 20),
            (tiling::SplitDirection::Vertical, tiling::SplitSide::First)
        );
        assert_eq!(
            split_from_point(rect, 30, 11),
            (tiling::SplitDirection::Horizontal, tiling::SplitSide::First)
        );
    }

    #[test]
    fn agent_split_auto_uses_mouse_side() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(10, 5, 40, 20))];
        state.last_mouse_pos = Some((12, 15));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], first);
        assert_eq!(state.agents_tab_pending_pane, Some(ids[0]));
        assert!(matches!(
            state.overlay.active,
            Some(overlay::OverlayKind::AiClientPicker)
        ));
    }

    #[test]
    fn agent_split_auto_targets_pane_under_mouse() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Left".to_string());
        let second = state
            .agents_tab
            .split_focused(tiling::SplitDirection::Vertical)
            .unwrap();
        state
            .agents_tab
            .register_pane(second, lune_ai::AiSessionId::new_v4(), "Right".to_string());

        state.agents_tab.focused = Some(second);
        state.last_agent_pane_rects = vec![
            (first, Rect::new(0, 0, 40, 20)),
            (second, Rect::new(41, 0, 39, 20)),
        ];
        state.last_mouse_pos = Some((5, 10));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[2], second);
        assert_eq!(state.agents_tab_pending_pane, Some(ids[0]));
        assert_eq!(state.agents_tab.focused, Some(ids[0]));
    }

    #[test]
    fn agent_split_auto_defaults_to_right_without_mouse() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 80, 20))];

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], first);
        assert_eq!(state.agents_tab_pending_pane, Some(ids[1]));
    }

    #[test]
    fn agent_split_auto_falls_back_to_horizontal_when_pane_is_too_narrow() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 20, 12))];
        state.last_mouse_pos = Some((1, 6));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let Some(tiling::TileNode::Split { direction, .. }) = state.agents_tab.layout.as_ref()
        else {
            panic!("expected split layout");
        };
        assert_eq!(*direction, tiling::SplitDirection::Horizontal);
    }

    #[test]
    fn agent_split_auto_warns_when_pane_is_too_small_in_both_axes() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 10, 7))];
        state.last_mouse_pos = Some((5, 3));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.agents_tab.pane_count(), 1);
        assert!(state.agents_tab_pending_pane.is_none());
        let warning = state
            .overlay
            .notifications
            .last()
            .map_or("", |n| n.message.as_str());
        assert!(warning.contains("too small"));
    }

    #[test]
    fn agent_split_vertical_warns_when_requested_direction_does_not_fit() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 20, 12))];

        let result = handle_command(&AppCommand::AgentSplitVertical, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.agents_tab.pane_count(), 1);
        assert!(state.agents_tab_pending_pane.is_none());
        let warning = state
            .overlay
            .notifications
            .last()
            .map_or("", |n| n.message.as_str());
        assert!(warning.contains("too small"));
    }

    #[test]
    fn agent_split_vertical_uses_computed_rect_when_render_cache_is_empty() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agents_content_area = Some(Rect::new(0, 0, 20, 12));
        state.last_agent_pane_rects.clear();

        let result = handle_command(&AppCommand::AgentSplitVertical, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.agents_tab.pane_count(), 1);
        let warning = state
            .overlay
            .notifications
            .last()
            .map_or("", |n| n.message.as_str());
        assert!(warning.contains("too small"));
    }

    #[test]
    fn agent_split_auto_uses_computed_rect_when_render_cache_is_empty() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Left".to_string());
        let second = state
            .agents_tab
            .split_focused(tiling::SplitDirection::Vertical)
            .unwrap();
        state
            .agents_tab
            .register_pane(second, lune_ai::AiSessionId::new_v4(), "Right".to_string());

        state.agents_tab.focused = Some(first);
        state.last_agents_content_area = Some(Rect::new(0, 0, 40, 12));
        state.last_agent_pane_rects.clear();
        state.last_mouse_pos = Some((1, 6));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let Some(tiling::TileNode::Split { first, .. }) = state.agents_tab.layout.as_ref() else {
            panic!("expected split layout");
        };
        let tiling::TileNode::Split { direction, .. } = first.as_ref() else {
            panic!("expected nested split on focused pane");
        };
        assert_eq!(*direction, tiling::SplitDirection::Horizontal);
    }

    #[test]
    fn agent_split_ignores_stale_render_cache_after_area_change() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agents_content_area = Some(Rect::new(0, 0, 20, 12));
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 80, 20))];

        let result = handle_command(&AppCommand::AgentSplitVertical, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.agents_tab.pane_count(), 1);
        let warning = state
            .overlay
            .notifications
            .last()
            .map_or("", |n| n.message.as_str());
        assert!(warning.contains("too small"));
    }

    #[test]
    fn agent_split_auto_targets_pane_under_mouse_when_cache_is_recomputed() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Left".to_string());
        let second = state
            .agents_tab
            .split_focused(tiling::SplitDirection::Vertical)
            .unwrap();
        state
            .agents_tab
            .register_pane(second, lune_ai::AiSessionId::new_v4(), "Right".to_string());

        state.agents_tab.focused = Some(first);
        state.last_agents_content_area = Some(Rect::new(0, 0, 80, 20));
        state.last_agent_pane_rects.clear();
        state.last_mouse_pos = Some((60, 10));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_ne!(state.agents_tab.focused, Some(first));
        let ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        assert_eq!(ids.len(), 3);
        assert_eq!(state.agents_tab_pending_pane, state.agents_tab.focused);
    }

    #[test]
    fn agent_split_vertical_warns_in_zoomed_mode_using_content_area() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.agents_tab.zoomed = true;
        state.last_agents_content_area = Some(Rect::new(0, 0, 20, 12));
        state.last_agent_pane_rects.clear();

        let result = handle_command(&AppCommand::AgentSplitVertical, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.agents_tab.pane_count(), 1);
        let warning = state
            .overlay
            .notifications
            .last()
            .map_or("", |n| n.message.as_str());
        assert!(warning.contains("too small"));
    }

    #[test]
    fn agent_split_auto_does_not_duplicate_when_pending_exists() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 80, 20))];
        state.last_mouse_pos = Some((70, 10));

        let first_split = handle_command(&AppCommand::AgentSplitAuto, &mut state);
        assert!(matches!(first_split, Control::Changed));
        let first_ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        let pending = state.agents_tab_pending_pane.unwrap();
        assert_eq!(first_ids.len(), 2);

        let second_split = handle_command(&AppCommand::AgentSplitAuto, &mut state);
        assert!(matches!(second_split, Control::Changed));
        let second_ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();

        assert_eq!(second_ids, first_ids);
        assert_eq!(state.agents_tab_pending_pane, Some(pending));
        assert!(matches!(
            state.overlay.active,
            Some(overlay::OverlayKind::AiClientPicker)
        ));
    }

    #[test]
    fn save_agent_layout_persists_to_state_db() {
        let mut state = AppState::new();
        let dir = tempfile::tempdir().unwrap();
        state.set_state_db(lune_core::state_db::StateDb::open(dir.path()).unwrap());
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());

        let result = handle_command(
            &AppCommand::AgentSaveLayoutConfirmed("Saved".to_string()),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.saved_agent_layouts.len(), 1);
        let saved: Vec<tiling::SavedAgentLayout> = state
            .state_db()
            .unwrap()
            .get_raw(b"ui:agent_layouts")
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "Saved");
    }

    #[test]
    fn save_agent_layout_overwrites_existing_name_after_normalization() {
        let mut state = AppState::new();
        let dir = tempfile::tempdir().unwrap();
        state.set_state_db(lune_core::state_db::StateDb::open(dir.path()).unwrap());
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());

        let first_save = handle_command(
            &AppCommand::AgentSaveLayoutConfirmed("  Main   Stack ".to_string()),
            &mut state,
        );
        assert!(matches!(first_save, Control::Changed));

        state
            .agents_tab
            .split_focused(tiling::SplitDirection::Vertical);
        let second_save = handle_command(
            &AppCommand::AgentSaveLayoutConfirmed("main stack".to_string()),
            &mut state,
        );

        assert!(matches!(second_save, Control::Changed));
        assert_eq!(state.saved_agent_layouts.len(), 1);
        assert_eq!(state.saved_agent_layouts[0].name, "main stack");
        assert_eq!(state.saved_agent_layouts[0].pane_count(), 2);
    }

    #[test]
    fn rename_and_delete_saved_agent_layouts_persist() {
        let mut state = AppState::new();
        let dir = tempfile::tempdir().unwrap();
        state.set_state_db(lune_core::state_db::StateDb::open(dir.path()).unwrap());
        state.saved_agent_layouts = vec![
            tiling::SavedAgentLayout {
                name: "One".to_string(),
                root: tiling::SavedTileNode::Leaf,
            },
            tiling::SavedAgentLayout {
                name: "Two".to_string(),
                root: tiling::SavedTileNode::Split {
                    direction: tiling::SplitDirection::Vertical,
                    ratio: 0.5,
                    first: Box::new(tiling::SavedTileNode::Leaf),
                    second: Box::new(tiling::SavedTileNode::Leaf),
                },
            },
        ];
        state.persist_saved_agent_layouts();

        let rename = handle_command(
            &AppCommand::AgentRenameSavedLayoutConfirmed {
                index: 0,
                name: "  Two ".to_string(),
            },
            &mut state,
        );
        assert!(matches!(rename, Control::Changed));
        assert_eq!(state.saved_agent_layouts.len(), 1);
        assert_eq!(state.saved_agent_layouts[0].name, "Two");
        assert_eq!(state.saved_agent_layouts[0].pane_count(), 1);

        let delete = handle_command(&AppCommand::AgentDeleteSavedLayout(0), &mut state);
        assert!(matches!(delete, Control::Changed));
        assert!(state.saved_agent_layouts.is_empty());
        let saved: Vec<tiling::SavedAgentLayout> = state
            .state_db()
            .unwrap()
            .get_raw(b"ui:agent_layouts")
            .unwrap()
            .unwrap();
        assert!(saved.is_empty());
    }

    #[test]
    fn rename_saved_layout_reopens_picker_with_selected_layout() {
        let mut state = AppState::new();
        state.saved_agent_layouts = vec![tiling::SavedAgentLayout {
            name: "One".to_string(),
            root: tiling::SavedTileNode::Leaf,
        }];

        let result = handle_command(
            &AppCommand::AgentRenameSavedLayoutConfirmed {
                index: 0,
                name: "Renamed".to_string(),
            },
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert!(matches!(
            state.overlay.active,
            Some(overlay::OverlayKind::LayoutPicker)
        ));
        assert_eq!(state.overlay.layout_picker.selected, 0);
        assert_eq!(state.saved_agent_layouts[0].name, "Renamed");
    }

    #[test]
    fn delete_saved_layout_reopens_picker_on_remaining_entries() {
        let mut state = AppState::new();
        state.saved_agent_layouts = vec![
            tiling::SavedAgentLayout {
                name: "One".to_string(),
                root: tiling::SavedTileNode::Leaf,
            },
            tiling::SavedAgentLayout {
                name: "Two".to_string(),
                root: tiling::SavedTileNode::Leaf,
            },
        ];

        let result = handle_command(&AppCommand::AgentDeleteSavedLayout(0), &mut state);

        assert!(matches!(result, Control::Changed));
        assert!(matches!(
            state.overlay.active,
            Some(overlay::OverlayKind::LayoutPicker)
        ));
        assert_eq!(state.saved_agent_layouts.len(), 1);
        assert_eq!(state.overlay.layout_picker.selected, 0);
        assert_eq!(state.saved_agent_layouts[0].name, "Two");
    }

    #[test]
    fn pending_agent_session_uses_pane_rect_for_initial_size() {
        let mut state = AppState::new();
        let pane_id = state.agents_tab.add_first_pane();
        state.agents_tab_pending_pane = Some(pane_id);
        state.last_agent_pane_rects = vec![(pane_id, Rect::new(4, 2, 33, 9))];

        let result = handle_ai_new_session(AiClientKind::Shell, &mut state);

        assert!(matches!(result, Control::Changed));
        let session_id = state.agents_tab.panes.get(&pane_id).unwrap().session_id;
        let size = state
            .ai_manager
            .session(session_id)
            .unwrap()
            .screen()
            .size();
        assert_eq!(size, (9, 33));

        state.ai_manager.close_all();
    }

    #[test]
    fn applying_layout_spawns_shells_with_current_pane_sizes() {
        let mut state = AppState::new();
        state.last_agents_content_area = Some(Rect::new(0, 0, 80, 20));

        let saved = tiling::SavedAgentLayout {
            name: "Two".to_string(),
            root: tiling::SavedTileNode::Split {
                direction: tiling::SplitDirection::Vertical,
                ratio: 0.5,
                first: Box::new(tiling::SavedTileNode::Leaf),
                second: Box::new(tiling::SavedTileNode::Leaf),
            },
        };
        let entry = overlay::LayoutPickerEntry {
            label: saved.name.clone(),
            pane_count: saved.pane_count(),
            kind: overlay::LayoutPickerEntryKind::Saved(0),
        };
        state.saved_agent_layouts.push(saved);

        apply_agent_layout_entry(&entry, &mut state);

        let mut expected: Vec<_> = state
            .agents_tab
            .layout
            .as_ref()
            .unwrap()
            .compute_rects(state.last_agents_content_area.unwrap())
            .into_iter()
            .map(|(_, rect)| (rect.height, rect.width))
            .collect();
        expected.sort_unstable();
        let mut sizes: Vec<_> = state
            .agents_tab
            .panes
            .values()
            .filter_map(|pane| state.ai_manager.session(pane.session_id))
            .map(|session| session.screen().size())
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, expected);

        state.ai_manager.close_all();
    }

    #[test]
    fn set_state_db_loads_saved_agent_layouts() {
        let dir = tempfile::tempdir().unwrap();
        let db = lune_core::state_db::StateDb::open(dir.path()).unwrap();
        let saved = vec![tiling::SavedAgentLayout {
            name: "Persisted".to_string(),
            root: tiling::SavedTileNode::Leaf,
        }];
        db.put_raw(b"ui:agent_layouts", &saved).unwrap();

        let mut state = AppState::new();
        state.set_state_db(db);

        assert_eq!(state.saved_agent_layouts, saved);
    }

    #[test]
    fn agent_split_auto_defaults_to_right_when_mouse_outside_any_pane() {
        let mut state = AppState::new();
        let first = state.agents_tab.add_first_pane();
        state
            .agents_tab
            .register_pane(first, lune_ai::AiSessionId::new_v4(), "Shell".to_string());
        state.last_agent_pane_rects = vec![(first, Rect::new(0, 0, 80, 20))];
        state.last_mouse_pos = Some((120, 80));

        let result = handle_command(&AppCommand::AgentSplitAuto, &mut state);

        assert!(matches!(result, Control::Changed));
        let ids = state.agents_tab.layout.as_ref().unwrap().pane_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], first);
        assert_eq!(state.agents_tab.focused, Some(ids[1]));
        assert_eq!(state.agents_tab_pending_pane, Some(ids[1]));
    }

    #[test]
    fn agent_apply_layout_opens_entries_with_saved_layouts() {
        let mut state = AppState::new();
        state.saved_agent_layouts.push(tiling::SavedAgentLayout {
            name: "Saved".to_string(),
            root: tiling::SavedTileNode::Leaf,
        });

        let result = handle_command(&AppCommand::AgentApplyLayout, &mut state);

        assert!(matches!(result, Control::Changed));
        assert!(state.overlay.layout_picker.entries.len() > tiling::PRESET_LIST.len());
        assert!(
            state
                .overlay
                .layout_picker
                .entries
                .iter()
                .any(|entry| matches!(entry.kind, overlay::LayoutPickerEntryKind::Saved(_)))
        );
    }
}
