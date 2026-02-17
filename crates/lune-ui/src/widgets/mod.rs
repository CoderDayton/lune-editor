//! UI widgets for Lune Editor.
//!
//! Each widget is a self-contained rendering unit that draws to a
//! `ratatui_core::buffer::Buffer` given a `Rect` and some state.

pub mod diff_view;
pub mod editor_pane;
pub mod file_tree;
pub mod git_panel;
pub mod overlay;
pub mod status_bar;
pub mod tab_bar;
