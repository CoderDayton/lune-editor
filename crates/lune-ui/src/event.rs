//! Application event types.
//!
//! [`AppEvent`] is the unified event type consumed by the rat-salsa event loop.
//! It wraps crossterm terminal events, timer events, and application-level
//! commands.

use std::path::PathBuf;

use crate::primitives::CtEvent;
use lune_ai::session::AiClientKind;
use lune_core::language::LanguageId;
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

    // ── Live Mode commands ────────────────────────────────────────────
    /// Toggle Live Mode: Off ↔ On.
    ToggleLiveMode,

    // ── Theme commands ──────────────────────────────────────────────
    /// Switch to the next theme in the registry.
    NextTheme,
    /// Switch to the previous theme in the registry.
    PrevTheme,

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
