//! # lune-ai
//!
//! AI integration layer for Lune Editor.
//!
//! Manages embedded AI clients (Claude Code, shells, custom tools) via PTY:
//! - **[`pty`]**: PTY process handle (spawn, read, write, resize, kill)
//! - **[`session`]**: AI session lifecycle with vt100 terminal emulation
//! - **[`manager`]**: Multi-session management
//! - **[`context`]**: Editor context snapshots for AI context injection
//! - **[`live_mode`]**: Live Mode controller for detecting AI-driven file changes

pub mod context;
pub mod live_mode;
pub mod manager;
pub mod pty;
pub mod session;

// Re-exports for convenience.
pub use context::{
    extract_selection_text, EditorContext, FileContext, GitStatusSummary, SelectionContext,
    TabContext,
};
pub use live_mode::{
    LiveChangeInfo, LiveDiffState, LiveModeController, LiveModeState, LiveModeStats,
};
pub use manager::AiManager;
pub use pty::TermSize;
pub use session::{AiClientKind, AiSession, AiSessionId, SessionEvent, SessionState};
