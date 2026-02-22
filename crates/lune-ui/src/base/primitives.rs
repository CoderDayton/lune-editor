//! Re-exports of external TUI primitives.
//!
//! Every ratatui, ratatui-core, and crossterm type used by `lune-ui` is
//! funneled through this single module.  Internal code imports from
//! `crate::primitives` instead of reaching into the underlying crates
//! directly.
//!
//! **Why this exists:** if we ever swap the rendering backend (ratatui →
//! something else), only this file needs to change — all other modules
//! reference these re-exports.

// ── Buffer ────────────────────────────────────────────────────────────

pub use ratatui_core::buffer::Buffer;

// ── Layout ────────────────────────────────────────────────────────────

pub use ratatui_core::layout::{Constraint, Direction, Layout, Rect};

// ── Style ─────────────────────────────────────────────────────────────

pub use ratatui_core::style::{Color, Modifier, Style, Stylize};

// ── Text ──────────────────────────────────────────────────────────────

pub use ratatui_core::text::{Line, Span};

// ── Widget trait ──────────────────────────────────────────────────────

pub use ratatui_core::widgets::Widget;

// ── Ratatui high-level widgets (overlay borders, etc.) ────────────────

pub use ratatui::widgets::{Block, BorderType, Borders, Clear, Tabs};

// ── Crossterm events ──────────────────────────────────────────────────

pub use ratatui_crossterm::crossterm::event::{
    Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
