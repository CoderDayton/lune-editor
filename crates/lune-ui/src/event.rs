//! Application event types.
//!
//! [`AppEvent`] is the unified event type consumed by the rat-salsa event loop.
//! It wraps crossterm terminal events, timer events, and application-level
//! commands.

use std::path::PathBuf;

use lune_core::language::LanguageId;
use rat_salsa::timer::TimeOut;
use ratatui_crossterm::crossterm::event::Event as CtEvent;

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
    /// Application-level command (from keybinding, command palette, etc.).
    Command(AppCommand),
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

/// AI session events (from PTY manager, future).
#[derive(Debug)]
pub enum AiEvent {
    /// Output from the AI session.
    Output { session_id: u64, text: String },
    /// AI session ended.
    SessionEnded { session_id: u64 },
    /// Error in AI session.
    Error { session_id: u64, message: String },
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

    // ── Panel toggles ─────────────────────────────────────────────
    /// Toggle the file tree sidebar.
    ToggleFileTree,
    /// Toggle the AI terminal panel.
    ToggleAiPanel,
    /// Toggle the git panel.
    ToggleGitPanel,
    /// Open the command palette overlay.
    OpenCommandPalette,
    /// Toggle hidden file visibility in the file tree.
    ToggleHiddenFiles,
    /// Reveal a file in the file tree (expand ancestors, scroll to it).
    RevealInFileTree(PathBuf),

    // ── File tree operations ──────────────────────────────────────
    /// Create a new file (prompt for name from file tree context).
    NewFile,
    /// Create a new directory (prompt for name from file tree context).
    NewDir,
    /// Rename the selected file tree entry.
    RenameEntry,
    /// Delete the selected file tree entry (with confirmation).
    DeleteEntry,

    // ── Editor commands ───────────────────────────────────────────
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Open find dialog.
    Find,
    /// Open find-and-replace dialog.
    Replace,

    // ── Vim mode transitions ──────────────────────────────────────
    /// Enter vim normal mode.
    EnterNormalMode,
    /// Enter vim insert mode.
    EnterInsertMode,
    /// Enter vim visual mode.
    EnterVisualMode,

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
    /// Refresh git status (manual trigger).
    GitRefresh,
}
