//! # lune-ui
//!
//! TUI layer for Lune Editor.
//!
//! Built on ratatui + rat-salsa + rat-widget + tachyonfx, this crate provides:
//! - **Layout**: VS Code-inspired split layout (sidebar, editor, panel, status bar)
//! - **Widgets**: Tab bar, editor pane, file tree, command palette, diff view
//! - **Event routing**: rat-salsa event loop integration, focus management
//! - **Effects**: tachyonfx-based visual effects and animations

pub mod app;
pub mod event;
pub mod focus;
pub mod highlight;
pub mod keybindings;
pub mod layout;
pub mod vim;
pub mod widgets;
