//! # lune-core
//!
//! Core editor engine for Lune Editor.
//!
//! Provides the fundamental data structures and algorithms:
//! - **`TextBuffer`**: Rope-backed text storage with undo/redo
//! - **`BufferRegistry`**: Manages open buffers by ID
//! - **Cursor / Selection**: Position and selection primitives
//! - **Search / Replace**: Pattern matching within buffers
//! - **Diff**: Change detection via the `similar` crate

pub mod editor;
pub mod ports;
pub mod project;
pub mod session;
pub mod syntax;

pub use editor::{buffer, diff, position, registry, search, undo};
pub use project::{config, recovery, settings, state_db, watcher, workspace, workspace_state};
pub use syntax::{highlight, language};

/// Convenient re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::buffer::{BufferId, TextBuffer};
    pub use crate::config::ConfigPaths;
    pub use crate::diff::{
        InlineHighlight, LiveDiffLine, LiveDiffLineKind, LiveHunk, LiveHunkKind,
    };
    pub use crate::highlight::{HighlightStyle, HighlightedLine, Highlighter, StyledSpan};
    pub use crate::language::{LanguageId, LanguageRegistry};
    pub use crate::position::{CursorState, Position, Selection};
    pub use crate::recovery::RecoveryState;
    pub use crate::registry::BufferRegistry;
    pub use crate::search::SearchState;
    pub use crate::session::SessionModel;
    pub use crate::settings::{CliOverrides, Settings};
    pub use crate::state_db::StateDb;
    pub use crate::workspace::Workspace;
    pub use crate::workspace_state::{RecentWorkspaces, WorkspaceState};
}

/// Re-export key dependencies used by downstream crates.
pub use ropey;
pub use similar;
pub use uuid;
