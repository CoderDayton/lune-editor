//! # lune-ai
//!
//! AI integration layer for Lune Editor.
//!
//! Manages embedded AI clients (Claude Code, shells, custom tools) via PTY:
//! - **[`pty`]**: PTY process handle (spawn, read, write, resize, kill)
//! - **[`session`]**: AI session lifecycle with vt100 terminal emulation
//! - **[`manager`]**: Multi-session management
//! - **[`context`]**: Editor context snapshots for AI context injection

pub mod context;
pub mod port_adapter;
pub mod runtime;

pub use port_adapter::AiManagerAdapter;
pub use runtime::{manager, pty, session};

// Re-exports for convenience.
pub use context::{
    EditorContext, FileContext, GitStatusSummary, SelectionContext, TabContext,
    extract_selection_text,
};
pub use manager::AiManager;
pub use pty::TermSize;
pub use session::{AiClientKind, AiSession, AiSessionId, SessionEvent, SessionState};
