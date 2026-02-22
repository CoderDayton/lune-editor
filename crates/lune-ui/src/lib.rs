//! # lune-ui
//!
//! TUI layer for Lune Editor.
//!
//! Built on ratatui + rat-salsa + rat-widget + tachyonfx, this crate provides:
//! - **Layout**: VS Code-inspired split layout (sidebar, editor, panel, status bar)
//! - **Widgets**: Tab bar, editor pane, file tree, command palette, diff view
//! - **Event routing**: rat-salsa event loop integration, focus management
//! - **Effects**: tachyonfx-based visual effects and animations

pub mod base;
pub mod highlight;
pub mod runtime;
pub mod style;
pub mod widgets;

pub use base::primitives;
pub use runtime::{app, effects, event, focus, keybindings, layout, vim};
pub use style::{theme, theme_config};
