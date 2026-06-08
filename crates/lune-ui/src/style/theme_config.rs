//! TOML-serializable theme configuration and theme registry.
//!
//! # Architecture
//!
//! The theme system is split into two layers for performance:
//!
//! - **[`ThemeConfig`]** — a serde-friendly TOML representation using hex
//!   color strings and named modifiers.  Parsed once at load time, never
//!   touched in the render path.
//!
//! - **`Theme`** (in `crate::theme`) — a flat `Copy` struct (~564 bytes)
//!   of raw `Color` / `Style` values used by every widget every frame.
//!   Theme switching is a single `usize` index change in the registry.
//!
//! # Performance
//!
//! - `Theme` is `Copy` — switching = memcpy of ~564 B ≈ 0.5 ns.
//! - `ThemeRegistry` stores all loaded themes contiguously in a `Vec`.
//!   1 000 themes ≈ 550 KB — fits in L2 cache.
//! - Render-path accesses `registry.current_theme()` which returns a
//!   `&Theme` reference — zero allocation, zero indirection beyond the
//!   `Vec` bounds check.

mod compile;
mod registry;
mod schema;

pub use schema::{
    ColorsConfig, DiffColorsConfig, EditorConfig, FileTreeColorsConfig, GitColorsConfig,
    NotificationColorsConfig, OverlayColorsConfig, StatusBarConfig, StyleDef, SyntaxColorsConfig,
    TabColorsConfig, ThemeConfig, WelcomeConfig,
};

pub use registry::{ThemeId, ThemeRegistry};
