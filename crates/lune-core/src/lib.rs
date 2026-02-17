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

pub mod buffer;
pub mod highlight;
pub mod language;
pub mod position;
pub mod registry;
pub mod search;
pub mod undo;
pub mod watcher;
pub mod workspace;

/// Convenient re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::buffer::{BufferId, TextBuffer};
    pub use crate::highlight::{HighlightStyle, HighlightedLine, Highlighter, StyledSpan};
    pub use crate::language::{LanguageId, LanguageRegistry};
    pub use crate::position::{CursorState, Position, Selection};
    pub use crate::registry::BufferRegistry;
    pub use crate::search::SearchState;
    pub use crate::workspace::Workspace;
}

/// Re-export key dependencies used by downstream crates.
pub use ropey;
pub use similar;
pub use uuid;
