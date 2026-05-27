//! Application event types.
//!
//! [`AppEvent`] is the unified event type consumed by the rat-salsa event loop.
//! It wraps crossterm terminal events, timer events, and application-level
//! commands.

use std::path::PathBuf;

use crate::primitives::CtEvent;
use lune_ai::session::AiClientKind;
use lune_core::language::LanguageId;
use rat_salsa::event::RenderedEvent;
use rat_salsa::timer::TimeOut;

/// Unified event type for the Lune Editor event loop.
#[derive(Debug)]
pub enum AppEvent {
    /// Raw crossterm terminal event (key, mouse, resize, etc.).
    Terminal(CtEvent),
    /// Timer tick (animations, auto-save, cursor blink, etc.).
    Timer(TimeOut),
    /// File system change notification.
    Fs(FsEvent),
    /// AI session event.
    Ai(AiEvent),
    /// Emitted by `PollRendered` after every frame paint. Used by the
    /// viewport scroll animation to drive its interpolation loop.
    Rendered,
    /// Application-level command (from keybinding, command palette, etc.).
    Command(AppCommand),
    /// One or more image decode workers have results ready. The handler
    /// drains the receiver on [`AppState`] and applies stale-filtered
    /// results to the image preview overlay.
    ImageDecodeReady,
}

// Required by PollCrossterm.
impl From<CtEvent> for AppEvent {
    fn from(event: CtEvent) -> Self {
        Self::Terminal(event)
    }
}

// Required by PollTimers.
impl From<TimeOut> for AppEvent {
    fn from(timeout: TimeOut) -> Self {
        Self::Timer(timeout)
    }
}

// Required by PollRendered.
impl From<RenderedEvent> for AppEvent {
    fn from(_: RenderedEvent) -> Self {
        Self::Rendered
    }
}

/// File system change events (from notify watcher, future).
#[derive(Debug)]
pub enum FsEvent {
    /// A file was modified on disk.
    Changed(PathBuf),
    /// A file was created.
    Created(PathBuf),
    /// A file was deleted.
    Deleted(PathBuf),
}

/// AI session events (from `PollAiSessions`).
#[derive(Debug)]
pub enum AiEvent {
    /// One or more sessions produced output (screen changed).
    OutputChanged,
    /// A session exited.
    SessionExited {
        /// The session that exited.
        session_id: lune_ai::AiSessionId,
        /// Exit code (negative = unknown/error).
        code: i32,
    },
}

/// High-level application commands decoupled from keybindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppCommand {
    // ── Application lifecycle ─────────────────────────────────────
    /// Quit the application.
    Quit,
    /// Force quit without save prompts.
    ForceQuit,

    // ── File operations ───────────────────────────────────────────
    /// Save the active buffer.
    Save,
    /// Save all dirty buffers.
    SaveAll,
    /// Open a file by path.
    OpenFile(PathBuf),
    /// Open the interactive file picker overlay.
    OpenFilePicker,

    // ── Tab management ────────────────────────────────────────────
    /// Close the active tab.
    CloseTab,
    /// Switch to the next tab.
    NextTab,
    /// Switch to the previous tab.
    PrevTab,
    /// Show the root "Editor" tab.
    ShowEditorTab,
    /// Show the root "Agents" tab.
    ShowAgentsTab,
    /// Toggle between the Editor and Agents root tabs.
    ToggleAgentsTab,

    // ── Panel toggles ─────────────────────────────────────────────
    /// Toggle the file tree sidebar.
    ToggleFileTree,
    /// Toggle the git panel.
    ToggleGitPanel,
    /// Open the command palette overlay.
    OpenCommandPalette,
    /// Toggle hidden file visibility in the file tree.
    ToggleHiddenFiles,
    /// Reveal a file in the file tree (expand ancestors, scroll to it).
    RevealInFileTree(PathBuf),

    // ── Focus management ───────────────────────────────────────────
    /// Cycle focus to the next visible pane.
    FocusNextPane,

    // ── File tree operations ──────────────────────────────────────
    /// Create a new file (prompt for name from file tree context).
    NewFile,
    /// Create a new directory (prompt for name from file tree context).
    NewDir,
    /// Rename the selected file tree entry.
    RenameEntry,
    /// Delete the selected file tree entry (with confirmation).
    DeleteEntry,

    /// Confirmed: create a new file at the given path.
    CreateFileConfirmed(PathBuf),
    /// Confirmed: create a new directory at the given path.
    CreateDirConfirmed(PathBuf),
    /// Confirmed: rename from old path to new path.
    RenameConfirmed { from: PathBuf, to: PathBuf },
    /// Confirmed: delete a file or directory at the given path.
    DeleteConfirmed(PathBuf),

    // ── Editor commands ───────────────────────────────────────────
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Open find dialog.
    Find,
    /// Open find-and-replace dialog.
    Replace,
    /// Open the language selector picker overlay.
    OpenLanguagePicker,
    /// Open the theme picker overlay.
    OpenThemePicker,
    /// Toggle a markdown preview overlay for the active buffer.
    ///
    /// The overlay renders the buffer through `tui-markdown`, giving a
    /// formatted view (headings, lists, code blocks, links) over the raw
    /// source. Works for any buffer; most useful for `.md` files.
    ToggleMarkdownPreview,
    /// Toggle the keybinding hints overlay (categorized cheatsheet).
    ToggleKeyHints,

    // ── Vim mode transitions ──────────────────────────────────────
    /// Enter vim normal mode.
    EnterNormalMode,
    /// Enter vim insert mode.
    EnterInsertMode,
    /// Enter vim visual mode.
    EnterVisualMode,
    /// Toggle vim keybinding mode on/off.
    ToggleVimMode,

    // ── Language ────────────────────────────────────────────────────
    /// Override the active buffer's language (changes highlighter).
    ChangeLanguage(LanguageId),

    // ── Git operations ─────────────────────────────────────────────
    /// Stage the currently selected file in the git panel.
    GitStage,
    /// Unstage the currently selected file in the git panel.
    GitUnstage,
    /// Commit staged changes (prompts for message via overlay).
    GitCommit,
    /// Discard changes to the currently selected file (requires confirmation).
    GitDiscard,
    /// Confirmed discard of a specific file's changes.
    GitDiscardConfirmed(PathBuf),
    /// Confirmed commit with the given message.
    GitCommitConfirmed(String),
    /// Refresh git status (manual trigger).
    GitRefresh,
    /// Stage the current hunk in the diff view.
    GitStageHunk,
    /// Unstage the current hunk in the diff view.
    GitUnstageHunk,
    /// Discard the current hunk in the diff view (requires confirmation).
    GitDiscardHunk,

    // ── AI commands ──────────────────────────────────────────────────
    /// Ask the AI about the current selection (sends selection context).
    AiAskSelection,
    /// Ask the AI to refactor the current file (sends file context).
    AiRefactorFile,
    /// Ask the AI to summarize git changes.
    AiSummarizeChanges,
    /// Open the client picker overlay to start a new AI session.
    AiOpenClientPicker,
    /// Start a new AI session with the given client kind.
    AiNewSession(AiClientKind),
    /// Close the currently active AI session.
    AiCloseSession,
    /// Switch to the next AI session tab.
    AiNextSession,
    /// Switch to the previous AI session tab.
    AiPrevSession,

    // ── Agents tab commands ────────────────────────────────────────
    /// Open the layout picker in the Agents tab.
    AgentApplyLayout,
    /// Split a pane using mouse-aware smart direction.
    AgentSplitAuto,
    /// Split the focused agent pane vertically.
    AgentSplitVertical,
    /// Split the focused agent pane horizontally.
    AgentSplitHorizontal,
    /// Close the focused agent pane.
    AgentClosePane,
    /// Focus the next agent pane.
    AgentFocusNext,
    /// Focus the previous agent pane.
    AgentFocusPrev,
    /// Toggle zoom on the focused agent pane.
    AgentToggleZoom,
    /// Save the current agent layout, reusing the active saved layout's
    /// name when one is set. Falls back to [`AgentSaveLayoutAs`] when
    /// there is no active layout to overwrite.
    AgentSaveLayout,
    /// Always prompt for a new name, even when an active layout exists.
    AgentSaveLayoutAs,
    /// Persist the current agent layout under the given name.
    AgentSaveLayoutConfirmed(String),
    /// Persist the current agent layout, replacing any existing layout
    /// with the same normalized name without prompting.
    AgentSaveLayoutOverwriteConfirmed(String),
    /// Delete the saved agent layout at the given index.
    AgentDeleteSavedLayout(usize),
    /// Rename the saved agent layout at the given index.
    AgentRenameSavedLayoutConfirmed { index: usize, name: String },

    // ── Theme commands ──────────────────────────────────────────────
    /// Switch to the next theme in the registry.
    NextTheme,
    /// Switch to the previous theme in the registry.
    PrevTheme,

    // ── Notifications ───────────────────────────────────────────────
    /// Dismiss every currently-visible toast notification.
    DismissNotifications,

    // ── Settings commands ────────────────────────────────────────────
    /// Open the global config file in the editor.
    OpenSettings,
    /// Open the keybindings config file in the editor.
    OpenKeybindings,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_crossterm::crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn app_event_from_crossterm() {
        let ct = CtEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let app_event = AppEvent::from(ct);
        assert!(matches!(app_event, AppEvent::Terminal(_)));
    }

    #[test]
    fn app_command_eq() {
        assert_eq!(AppCommand::Quit, AppCommand::Quit);
        assert_ne!(AppCommand::Quit, AppCommand::Save);
        assert_eq!(AppCommand::NextTab, AppCommand::NextTab);
    }

    #[test]
    fn app_command_clone() {
        let cmd = AppCommand::OpenFile(std::path::PathBuf::from("/test"));
        let clone = cmd.clone();
        assert_eq!(cmd, clone);
    }

    #[test]
    fn fs_event_variants() {
        let _changed = FsEvent::Changed(std::path::PathBuf::from("/a"));
        let _created = FsEvent::Created(std::path::PathBuf::from("/b"));
        let _deleted = FsEvent::Deleted(std::path::PathBuf::from("/c"));
    }

    #[test]
    fn ai_event_variants() {
        let output = AiEvent::OutputChanged;
        assert!(matches!(output, AiEvent::OutputChanged));
        let exited = AiEvent::SessionExited {
            session_id: lune_ai::AiSessionId::nil(),
            code: 0,
        };
        assert!(matches!(exited, AiEvent::SessionExited { code: 0, .. }));
    }
}
