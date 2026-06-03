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

pub use ratatui_core::layout::{Alignment, Constraint, Direction, Layout, Rect};

// ── Style ─────────────────────────────────────────────────────────────

pub use ratatui_core::style::{Color, Modifier, Style, Stylize};

// ── Text ──────────────────────────────────────────────────────────────

pub use ratatui_core::text::{Line, Span};

// ── Widget traits ─────────────────────────────────────────────────────

pub use ratatui_core::widgets::{StatefulWidget, Widget};

// ── Ratatui high-level widgets (overlay borders, etc.) ────────────────

pub use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Tabs, Wrap,
};

// ── Glyph constants (line/block/scrollbar etc.) ───────────────────────
//
// Re-exported so widgets can reference ratatui's symbol set
// (`symbols::line::VERTICAL`, `symbols::block::FULL`, …) instead of
// hard-coding the same characters as raw string literals.

pub use ratatui_core::symbols;

// ── Crossterm events ──────────────────────────────────────────────────

pub use ratatui_crossterm::crossterm::event::{
    Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
