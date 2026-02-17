//! UI widgets for Lune Editor.
//!
//! Each widget is a self-contained rendering unit that draws to a
//! `ratatui_core::buffer::Buffer` given a `Rect` and some state.

use ratatui_core::style::Color;

pub mod diff_view;
pub mod editor_pane;
pub mod file_tree;
pub mod git_panel;
pub mod overlay;
pub mod status_bar;
pub mod tab_bar;

/// Accent color for focused pane borders and headers.
pub const FOCUS_ACCENT: Color = Color::Rgb(80, 130, 220);

/// Border color for unfocused panes.
pub const UNFOCUSED_BORDER: Color = Color::DarkGray;
