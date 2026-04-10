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
//! - **[`Theme`]** (in `crate::theme`) — a flat `Copy` struct (~564 bytes)
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

use std::path::Path;

use crate::primitives::{Color, Modifier, Style};
use lune_core::highlight::HighlightStyle;
use serde::{Deserialize, Serialize};

use crate::highlight::theme::SyntaxTheme;
use crate::theme::{BorderChars, Theme};

// ── Color parsing ─────────────────────────────────────────────────────

/// Parse a hex color string (`"#RRGGBB"`) into a ratatui `Color`.
///
/// Also accepts named colors like `"red"`, `"blue"`, `"reset"`, etc.
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    match s.to_ascii_lowercase().as_str() {
        "reset" | "default" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Some(Color::DarkGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

/// Parse modifier names from a comma-separated string.
///
/// Recognized: `bold`, `dim`, `italic`, `underlined`, `reversed`.
fn parse_modifiers(s: &str) -> Modifier {
    let mut m = Modifier::empty();
    for part in s.split(',') {
        match part.trim().to_ascii_lowercase().as_str() {
            "bold" => m |= Modifier::BOLD,
            "dim" => m |= Modifier::DIM,
            "italic" => m |= Modifier::ITALIC,
            "underlined" | "underline" => m |= Modifier::UNDERLINED,
            "reversed" | "reverse" => m |= Modifier::REVERSED,
            "hidden" => m |= Modifier::HIDDEN,
            "crossed_out" | "crossedout" | "strikethrough" => m |= Modifier::CROSSED_OUT,
            _ => {}
        }
    }
    m
}

// ── Style definition (TOML-friendly) ──────────────────────────────────

/// A single style definition as it appears in a TOML theme file.
///
/// All fields are optional — missing fields inherit from the base theme.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StyleDef {
    /// Foreground color (hex `"#RRGGBB"` or named).
    pub fg: Option<String>,
    /// Background color (hex `"#RRGGBB"` or named).
    pub bg: Option<String>,
    /// Comma-separated modifiers: `"bold"`, `"italic"`, `"underlined"`,
    /// `"reversed"`, `"dim"`.
    pub modifiers: Option<String>,
}

impl StyleDef {
    /// Convert to a ratatui `Style`, returning `None` if all fields are
    /// empty or contain unparseable values.
    fn to_style(&self) -> Style {
        let mut style = Style::new();
        if let Some(ref fg) = self.fg {
            if let Some(c) = parse_color(fg) {
                style = style.fg(c);
            }
        }
        if let Some(ref bg) = self.bg {
            if let Some(c) = parse_color(bg) {
                style = style.bg(c);
            }
        }
        if let Some(ref mods) = self.modifiers {
            style = style.add_modifier(parse_modifiers(mods));
        }
        style
    }
}

// ── Border config ─────────────────────────────────────────────────────

/// TOML-serializable border character set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderCharsConfig {
    pub top_left: Option<char>,
    pub top_right: Option<char>,
    pub bottom_left: Option<char>,
    pub bottom_right: Option<char>,
    pub vertical: Option<char>,
    pub horizontal: Option<char>,
}

impl BorderCharsConfig {
    fn apply_to(&self, base: BorderChars) -> BorderChars {
        BorderChars {
            top_left: self.top_left.unwrap_or(base.top_left),
            top_right: self.top_right.unwrap_or(base.top_right),
            bottom_left: self.bottom_left.unwrap_or(base.bottom_left),
            bottom_right: self.bottom_right.unwrap_or(base.bottom_right),
            vertical: self.vertical.unwrap_or(base.vertical),
            horizontal: self.horizontal.unwrap_or(base.horizontal),
        }
    }
}

// ── Syntax colors config ──────────────────────────────────────────────

/// Syntax highlighting colors in TOML.
///
/// Keys match [`HighlightStyle`] variant names (`snake_case`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SyntaxColorsConfig {
    pub keyword: Option<StyleDef>,
    #[serde(rename = "type")]
    pub type_: Option<StyleDef>,
    pub function: Option<StyleDef>,
    pub string: Option<StyleDef>,
    pub comment: Option<StyleDef>,
    pub number: Option<StyleDef>,
    pub operator: Option<StyleDef>,
    pub punctuation: Option<StyleDef>,
    pub variable: Option<StyleDef>,
    pub constant: Option<StyleDef>,
    pub attribute: Option<StyleDef>,
    pub namespace: Option<StyleDef>,
    pub error: Option<StyleDef>,
    pub embedded: Option<StyleDef>,
}

impl SyntaxColorsConfig {
    /// Apply overrides onto a base `SyntaxTheme`, returning a new one.
    fn apply_to(&self, base: &SyntaxTheme) -> SyntaxTheme {
        let mut theme = base.clone();
        let overrides: [(HighlightStyle, &Option<StyleDef>); 14] = [
            (HighlightStyle::Keyword, &self.keyword),
            (HighlightStyle::Type, &self.type_),
            (HighlightStyle::Function, &self.function),
            (HighlightStyle::String, &self.string),
            (HighlightStyle::Comment, &self.comment),
            (HighlightStyle::Number, &self.number),
            (HighlightStyle::Operator, &self.operator),
            (HighlightStyle::Punctuation, &self.punctuation),
            (HighlightStyle::Variable, &self.variable),
            (HighlightStyle::Constant, &self.constant),
            (HighlightStyle::Attribute, &self.attribute),
            (HighlightStyle::Namespace, &self.namespace),
            (HighlightStyle::Error, &self.error),
            (HighlightStyle::Embedded, &self.embedded),
        ];
        for (hl, def) in overrides {
            if let Some(sdef) = def {
                theme.set(hl, sdef.to_style());
            }
        }
        theme
    }
}

// ── Theme config (top-level TOML) ─────────────────────────────────────

/// Top-level TOML theme configuration file.
///
/// # Example
///
/// ```toml
/// name = "My Dark Theme"
/// base = "dark"
///
/// [colors]
/// accent = "#50C878"
/// bg = "reset"
/// fg = "#E0E0E0"
///
/// [editor]
/// cursor_normal = { modifiers = "reversed" }
/// gutter_active = { fg = "white", modifiers = "bold" }
///
/// [syntax]
/// keyword = { fg = "#569CD6", modifiers = "bold" }
/// string = { fg = "#CE9178" }
/// comment = { fg = "#6A9955", modifiers = "italic" }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Display name for the theme.
    pub name: String,

    /// Base theme to start from: `"dark"` or `"light"`.
    /// Missing fields in the config inherit from the base.
    #[serde(default = "default_base")]
    pub base: String,

    /// General UI colors (accent, bg, fg, etc.).
    #[serde(default)]
    pub colors: ColorsConfig,

    /// Border character overrides.
    pub borders: Option<BorderCharsConfig>,

    /// Editor pane style overrides.
    #[serde(default)]
    pub editor: EditorConfig,

    /// File tree color overrides.
    #[serde(default)]
    pub file_tree: FileTreeColorsConfig,

    /// Git status color overrides.
    #[serde(default)]
    pub git: GitColorsConfig,

    /// Diff view color overrides.
    #[serde(default)]
    pub diff: DiffColorsConfig,

    /// Tab bar style overrides.
    #[serde(default)]
    pub tabs: TabColorsConfig,

    /// Status bar style overrides.
    #[serde(default)]
    pub status_bar: StatusBarConfig,

    /// Notification color overrides.
    #[serde(default)]
    pub notifications: NotificationColorsConfig,

    /// Overlay / popup style overrides.
    #[serde(default)]
    pub overlay: OverlayColorsConfig,

    /// Welcome screen style overrides.
    #[serde(default)]
    pub welcome: WelcomeConfig,

    /// Syntax highlighting color overrides.
    #[serde(default)]
    pub syntax: SyntaxColorsConfig,
}

fn default_base() -> String {
    "dark".to_owned()
}

// ── Sub-config sections ───────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ColorsConfig {
    pub accent: Option<String>,
    pub bg: Option<String>,
    pub fg: Option<String>,
    pub fg_dim: Option<String>,
    pub fg_muted: Option<String>,
    pub selection_bg: Option<String>,
    pub border_focused: Option<String>,
    pub border_unfocused: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditorConfig {
    pub cursor_normal: Option<StyleDef>,
    pub cursor_insert: Option<StyleDef>,
    pub gutter_active: Option<StyleDef>,
    pub gutter_inactive: Option<StyleDef>,
    pub gutter_separator: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileTreeColorsConfig {
    pub dir_fg: Option<String>,
    pub file_fg: Option<String>,
    pub symlink_fg: Option<String>,
    pub selected_bg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GitColorsConfig {
    pub added: Option<String>,
    pub modified: Option<String>,
    pub deleted: Option<String>,
    pub conflicted: Option<String>,
    pub renamed: Option<String>,
    pub untracked: Option<String>,
    pub ignored: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiffColorsConfig {
    pub add_fg: Option<String>,
    pub add_bg: Option<String>,
    pub del_fg: Option<String>,
    pub del_bg: Option<String>,
    pub hunk_fg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TabColorsConfig {
    pub active_focused: Option<StyleDef>,
    pub active_unfocused: Option<StyleDef>,
    pub inactive: Option<StyleDef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatusBarConfig {
    pub mode: Option<StyleDef>,
    pub info: Option<StyleDef>,
    pub bg: Option<StyleDef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NotificationColorsConfig {
    pub success: Option<String>,
    pub info: Option<String>,
    pub warn: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OverlayColorsConfig {
    pub border: Option<String>,
    pub selected: Option<StyleDef>,
    pub dir_fg: Option<String>,
    pub file_fg: Option<String>,
    pub hint_fg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WelcomeConfig {
    pub title: Option<StyleDef>,
    pub text: Option<StyleDef>,
}

// ── ThemeConfig → Theme conversion ────────────────────────────────────

impl ThemeConfig {
    /// Convert this TOML config into a compiled [`Theme`] and [`SyntaxTheme`].
    ///
    /// Starts from the named `base` theme and applies overrides for any
    /// fields present in the config. This is the only allocation-heavy
    /// path — called once at load, never in the render loop.
    pub fn compile(&self) -> (Theme, SyntaxTheme) {
        let mut t = match self.base.as_str() {
            "light" => Theme::light(),
            _ => Theme::dark(),
        };

        let base_syntax = match self.base.as_str() {
            "light" => SyntaxTheme::light(),
            _ => SyntaxTheme::dark(),
        };

        // ── General colors ───────────────────────────────────────
        apply_color(self.colors.accent.as_ref(), &mut t.accent);
        apply_color(self.colors.bg.as_ref(), &mut t.bg);
        apply_color(self.colors.fg.as_ref(), &mut t.fg);
        apply_color(self.colors.fg_dim.as_ref(), &mut t.fg_dim);
        apply_color(self.colors.fg_muted.as_ref(), &mut t.fg_muted);
        apply_color(self.colors.selection_bg.as_ref(), &mut t.selection_bg);
        apply_color(self.colors.border_focused.as_ref(), &mut t.border_focused);
        apply_color(
            self.colors.border_unfocused.as_ref(),
            &mut t.border_unfocused,
        );

        // ── Borders ──────────────────────────────────────────────
        if let Some(ref bc) = self.borders {
            t.border_chars = bc.apply_to(t.border_chars);
        }

        // ── Editor ───────────────────────────────────────────────
        apply_style(
            self.editor.cursor_normal.as_ref(),
            &mut t.editor_cursor_normal,
        );
        apply_style(
            self.editor.cursor_insert.as_ref(),
            &mut t.editor_cursor_insert,
        );
        apply_style(
            self.editor.gutter_active.as_ref(),
            &mut t.editor_gutter_active,
        );
        apply_style(
            self.editor.gutter_inactive.as_ref(),
            &mut t.editor_gutter_inactive,
        );
        apply_color(
            self.editor.gutter_separator.as_ref(),
            &mut t.editor_gutter_separator,
        );

        // ── File tree ────────────────────────────────────────────
        apply_color(self.file_tree.dir_fg.as_ref(), &mut t.tree_dir_fg);
        apply_color(self.file_tree.file_fg.as_ref(), &mut t.tree_file_fg);
        apply_color(self.file_tree.symlink_fg.as_ref(), &mut t.tree_symlink_fg);
        apply_color(self.file_tree.selected_bg.as_ref(), &mut t.tree_selected_bg);

        // ── Git ──────────────────────────────────────────────────
        apply_color(self.git.added.as_ref(), &mut t.git_added);
        apply_color(self.git.modified.as_ref(), &mut t.git_modified);
        apply_color(self.git.deleted.as_ref(), &mut t.git_deleted);
        apply_color(self.git.conflicted.as_ref(), &mut t.git_conflicted);
        apply_color(self.git.renamed.as_ref(), &mut t.git_renamed);
        apply_color(self.git.untracked.as_ref(), &mut t.git_untracked);
        apply_color(self.git.ignored.as_ref(), &mut t.git_ignored);

        // ── Diff ─────────────────────────────────────────────────
        apply_color(self.diff.add_fg.as_ref(), &mut t.diff_add_fg);
        apply_color(self.diff.add_bg.as_ref(), &mut t.diff_add_bg);
        apply_color(self.diff.del_fg.as_ref(), &mut t.diff_del_fg);
        apply_color(self.diff.del_bg.as_ref(), &mut t.diff_del_bg);
        apply_color(self.diff.hunk_fg.as_ref(), &mut t.diff_hunk_fg);

        // ── Tabs ─────────────────────────────────────────────────
        apply_style(self.tabs.active_focused.as_ref(), &mut t.tab_active_focused);
        apply_style(
            self.tabs.active_unfocused.as_ref(),
            &mut t.tab_active_unfocused,
        );
        apply_style(self.tabs.inactive.as_ref(), &mut t.tab_inactive);

        // ── Status bar ───────────────────────────────────────────
        apply_style(self.status_bar.mode.as_ref(), &mut t.status_mode);
        apply_style(self.status_bar.info.as_ref(), &mut t.status_info);
        apply_style(self.status_bar.bg.as_ref(), &mut t.status_bg);

        // ── Notifications ────────────────────────────────────────
        apply_color(self.notifications.success.as_ref(), &mut t.notif_success);
        apply_color(self.notifications.info.as_ref(), &mut t.notif_info);
        apply_color(self.notifications.warn.as_ref(), &mut t.notif_warn);
        apply_color(self.notifications.error.as_ref(), &mut t.notif_error);

        // ── Overlay ──────────────────────────────────────────────
        apply_color(self.overlay.border.as_ref(), &mut t.overlay_border);
        apply_style(self.overlay.selected.as_ref(), &mut t.overlay_selected);
        apply_color(self.overlay.dir_fg.as_ref(), &mut t.overlay_dir_fg);
        apply_color(self.overlay.file_fg.as_ref(), &mut t.overlay_file_fg);
        apply_color(self.overlay.hint_fg.as_ref(), &mut t.overlay_hint_fg);

        // ── Welcome ──────────────────────────────────────────────
        apply_style(self.welcome.title.as_ref(), &mut t.welcome_title);
        apply_style(self.welcome.text.as_ref(), &mut t.welcome_text);

        // ── Syntax ───────────────────────────────────────────────
        let syntax = self.syntax.apply_to(&base_syntax);

        (t, syntax)
    }

    /// Load a theme config from a TOML file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

/// Apply an optional color string override to a target `Color`.
fn apply_color(src: Option<&String>, target: &mut Color) {
    if let Some(s) = src {
        if let Some(c) = parse_color(s) {
            *target = c;
        }
    }
}

/// Apply an optional `StyleDef` override to a target `Style`.
fn apply_style(src: Option<&StyleDef>, target: &mut Style) {
    if let Some(sdef) = src {
        *target = sdef.to_style();
    }
}

// ── Theme Registry ────────────────────────────────────────────────────

/// Identifies a theme in the [`ThemeRegistry`].
///
/// This is a lightweight `Copy` index — switching themes means changing
/// this value and the next render picks up the new `&Theme` reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeId(pub usize);

/// A named, pre-compiled theme entry.
struct ThemeEntry {
    /// Human-readable name (e.g. `"Lune Dark"`, `"Solarized Light"`).
    name: String,
    /// Compiled UI theme (the `Copy` struct).
    theme: Theme,
    /// Compiled syntax highlighting theme.
    syntax: SyntaxTheme,
}

/// Pre-loaded theme collection for instant switching.
///
/// All themes are compiled (TOML parsed, colors resolved) at load time
/// and stored contiguously.  Switching themes is O(1) — change the
/// active index.
///
/// # Performance
///
/// - Each `Theme` is ~564 bytes (`Copy`, no heap).
/// - 1 000 themes ≈ 550 KB — fits in L2 cache.
/// - `current_theme()` / `current_syntax()` return references — zero
///   allocation per frame.
pub struct ThemeRegistry {
    themes: Vec<ThemeEntry>,
    active: usize,
}

impl ThemeRegistry {
    /// Create a registry pre-loaded with the built-in dark and light themes.
    #[must_use]
    pub fn new() -> Self {
        let themes = vec![
            ThemeEntry {
                name: "Lune Dark".to_owned(),
                theme: Theme::dark(),
                syntax: SyntaxTheme::dark(),
            },
            ThemeEntry {
                name: "Lune Light".to_owned(),
                theme: Theme::light(),
                syntax: SyntaxTheme::light(),
            },
        ];
        Self { themes, active: 0 }
    }

    /// Number of loaded themes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.themes.len()
    }

    /// Whether the registry is empty (should never be in practice).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Add a theme from a compiled [`ThemeConfig`].
    ///
    /// Returns the assigned [`ThemeId`].
    pub fn add(&mut self, config: &ThemeConfig) -> ThemeId {
        let (theme, syntax) = config.compile();
        let id = ThemeId(self.themes.len());
        self.themes.push(ThemeEntry {
            name: config.name.clone(),
            theme,
            syntax,
        });
        id
    }

    /// Load all `.toml` theme files from a directory.
    ///
    /// Files that fail to parse are logged and skipped.  Returns the
    /// number of themes successfully loaded.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut count = 0;
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            match ThemeConfig::load(&path) {
                Ok(config) => {
                    // Deduplicate by name — later file wins.
                    let name = config.name.clone();
                    if let Some(existing) = self.themes.iter_mut().find(|e| e.name == name) {
                        let (theme, syntax) = config.compile();
                        existing.theme = theme;
                        existing.syntax = syntax;
                    } else {
                        self.add(&config);
                    }
                    count += 1;
                }
                Err(e) => {
                    log::warn!("Failed to load theme {}: {e}", path.display());
                }
            }
        }
        count
    }

    /// Switch to a theme by ID.
    ///
    /// Returns `true` if the switch succeeded (valid ID).
    pub fn switch(&mut self, id: ThemeId) -> bool {
        if id.0 < self.themes.len() {
            self.active = id.0;
            true
        } else {
            false
        }
    }

    /// Switch to the next theme in the list, wrapping around.
    pub fn next(&mut self) {
        if !self.themes.is_empty() {
            self.active = (self.active + 1) % self.themes.len();
        }
    }

    /// Switch to the previous theme in the list, wrapping around.
    pub fn prev(&mut self) {
        if !self.themes.is_empty() {
            self.active = self.active.checked_sub(1).unwrap_or(self.themes.len() - 1);
        }
    }

    /// Switch to a theme by name (case-insensitive).
    ///
    /// Returns `true` if a theme with that name was found.
    pub fn switch_by_name(&mut self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if let Some(idx) = self
            .themes
            .iter()
            .position(|e| e.name.to_ascii_lowercase() == lower)
        {
            self.active = idx;
            true
        } else {
            false
        }
    }

    /// The currently active [`ThemeId`].
    #[must_use]
    pub const fn active_id(&self) -> ThemeId {
        ThemeId(self.active)
    }

    /// The currently active UI theme (zero-cost reference).
    #[must_use]
    pub fn current_theme(&self) -> &Theme {
        &self.themes[self.active].theme
    }

    /// The currently active syntax theme (zero-cost reference).
    #[must_use]
    pub fn current_syntax(&self) -> &SyntaxTheme {
        &self.themes[self.active].syntax
    }

    /// The display name of the currently active theme.
    #[must_use]
    pub fn current_name(&self) -> &str {
        &self.themes[self.active].name
    }

    /// List all loaded themes as `(ThemeId, name)` pairs.
    #[must_use]
    pub fn list(&self) -> Vec<(ThemeId, &str)> {
        self.themes
            .iter()
            .enumerate()
            .map(|(i, e)| (ThemeId(i), e.name.as_str()))
            .collect()
    }

    /// Find a theme ID by name (case-insensitive).
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<ThemeId> {
        let lower = name.to_ascii_lowercase();
        self.themes
            .iter()
            .position(|e| e.name.to_ascii_lowercase() == lower)
            .map(ThemeId)
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color() {
        assert_eq!(parse_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("#0000FF"), Some(Color::Rgb(0, 0, 255)));
        assert_eq!(parse_color("#5082DC"), Some(Color::Rgb(80, 130, 220)));
    }

    #[test]
    fn parse_named_colors() {
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("Blue"), Some(Color::Blue));
        assert_eq!(parse_color("RESET"), Some(Color::Reset));
        assert_eq!(parse_color("dark_gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("light_red"), Some(Color::LightRed));
    }

    #[test]
    fn parse_invalid_color() {
        assert_eq!(parse_color("#GG0000"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("foobar"), None);
    }

    #[test]
    fn parse_modifier_string() {
        let m = parse_modifiers("bold,italic");
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::ITALIC));
        assert!(!m.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn parse_modifiers_with_spaces() {
        let m = parse_modifiers(" bold , underlined , reversed ");
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::UNDERLINED));
        assert!(m.contains(Modifier::REVERSED));
    }

    #[test]
    fn style_def_to_style() {
        let def = StyleDef {
            fg: Some("#FF0000".to_owned()),
            bg: Some("blue".to_owned()),
            modifiers: Some("bold".to_owned()),
        };
        let style = def.to_style();
        assert_eq!(style.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(style.bg, Some(Color::Blue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn theme_config_roundtrip_toml() {
        let toml_str = r##"
name = "Test Theme"
base = "dark"

[colors]
accent = "#50C878"
fg = "white"

[syntax]
keyword = { fg = "#569CD6", modifiers = "bold" }
string = { fg = "#CE9178" }
"##;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "Test Theme");
        assert_eq!(config.base, "dark");
        assert_eq!(config.colors.accent.as_deref(), Some("#50C878"));

        let (theme, syntax) = config.compile();
        assert_eq!(theme.accent, Color::Rgb(80, 200, 120));
        assert_eq!(theme.fg, Color::White);
        // Syntax keyword should be overridden
        let kw = syntax.resolve(HighlightStyle::Keyword);
        assert_eq!(kw.fg, Some(Color::Rgb(86, 156, 214)));
        assert!(kw.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn theme_config_partial_override() {
        let toml_str = r##"
name = "Partial"
base = "dark"

[colors]
accent = "#FF0000"
"##;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        let (theme, _) = config.compile();
        assert_eq!(theme.accent, Color::Rgb(255, 0, 0));
        // Non-overridden fields should match the dark base
        assert_eq!(theme.fg, Theme::dark().fg);
        assert_eq!(theme.diff_add_fg, Theme::dark().diff_add_fg);
    }

    #[test]
    fn theme_config_light_base() {
        let toml_str = r#"
name = "Custom Light"
base = "light"
"#;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        let (theme, _) = config.compile();
        assert_eq!(theme, Theme::light());
    }

    #[test]
    fn registry_new_has_builtins() {
        let reg = ThemeRegistry::new();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.current_name(), "Lune Dark");
        assert_eq!(*reg.current_theme(), Theme::dark());
    }

    #[test]
    fn registry_switch_by_id() {
        let mut reg = ThemeRegistry::new();
        assert!(reg.switch(ThemeId(1)));
        assert_eq!(reg.current_name(), "Lune Light");
        assert_eq!(*reg.current_theme(), Theme::light());
    }

    #[test]
    fn registry_switch_invalid_id() {
        let mut reg = ThemeRegistry::new();
        assert!(!reg.switch(ThemeId(999)));
        assert_eq!(reg.active_id(), ThemeId(0));
    }

    #[test]
    fn registry_switch_by_name() {
        let mut reg = ThemeRegistry::new();
        assert!(reg.switch_by_name("lune light"));
        assert_eq!(reg.active_id(), ThemeId(1));
    }

    #[test]
    fn registry_next_prev_wrap() {
        let mut reg = ThemeRegistry::new();
        reg.next();
        assert_eq!(reg.active_id(), ThemeId(1));
        reg.next();
        assert_eq!(reg.active_id(), ThemeId(0)); // wrapped
        reg.prev();
        assert_eq!(reg.active_id(), ThemeId(1)); // wrapped backwards
    }

    #[test]
    fn registry_add_custom_theme() {
        let mut reg = ThemeRegistry::new();
        let config: ThemeConfig = toml::from_str(
            r##"
name = "Custom"
base = "dark"
[colors]
accent = "#FF0000"
"##,
        )
        .unwrap();
        let id = reg.add(&config);
        assert_eq!(reg.len(), 3);
        assert!(reg.switch(id));
        assert_eq!(reg.current_theme().accent, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn registry_list() {
        let reg = ThemeRegistry::new();
        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], (ThemeId(0), "Lune Dark"));
        assert_eq!(list[1], (ThemeId(1), "Lune Light"));
    }

    #[test]
    fn registry_find_by_name() {
        let reg = ThemeRegistry::new();
        assert_eq!(reg.find_by_name("Lune Dark"), Some(ThemeId(0)));
        assert_eq!(reg.find_by_name("lune dark"), Some(ThemeId(0)));
        assert_eq!(reg.find_by_name("nonexistent"), None);
    }

    #[test]
    fn registry_load_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = ThemeRegistry::new();
        assert_eq!(reg.load_dir(dir.path()), 0);
        assert_eq!(reg.len(), 2); // still just builtins
    }

    #[test]
    fn registry_load_dir_with_theme_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("monokai.toml"),
            r##"
name = "Monokai"
base = "dark"
[colors]
accent = "#F92672"
"##,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("solarized.toml"),
            r##"
name = "Solarized Light"
base = "light"
[colors]
accent = "#268BD2"
"##,
        )
        .unwrap();
        // Non-toml files should be ignored
        std::fs::write(dir.path().join("readme.md"), "not a theme").unwrap();

        let mut reg = ThemeRegistry::new();
        let loaded = reg.load_dir(dir.path());
        assert_eq!(loaded, 2);
        assert_eq!(reg.len(), 4); // 2 builtin + 2 loaded

        assert!(reg.switch_by_name("Monokai"));
        assert_eq!(reg.current_theme().accent, Color::Rgb(249, 38, 114));
    }

    #[test]
    fn registry_load_dir_deduplicates_by_name() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file that overrides the built-in "Lune Dark"
        std::fs::write(
            dir.path().join("override.toml"),
            r##"
name = "Lune Dark"
base = "dark"
[colors]
accent = "#ABCDEF"
"##,
        )
        .unwrap();

        let mut reg = ThemeRegistry::new();
        let loaded = reg.load_dir(dir.path());
        assert_eq!(loaded, 1);
        assert_eq!(reg.len(), 2); // deduped, not 3

        assert!(reg.switch_by_name("Lune Dark"));
        assert_eq!(reg.current_theme().accent, Color::Rgb(0xAB, 0xCD, 0xEF));
    }

    #[test]
    fn border_chars_config_partial_override() {
        let bc = BorderCharsConfig {
            top_left: Some('┌'),
            top_right: None,
            bottom_left: None,
            bottom_right: Some('┘'),
            vertical: None,
            horizontal: None,
        };
        let result = bc.apply_to(BorderChars::plain());
        assert_eq!(result.top_left, '┌');
        assert_eq!(result.top_right, '┐'); // kept from base
        assert_eq!(result.bottom_right, '┘');
        assert_eq!(result.vertical, '│'); // kept from base
    }

    #[test]
    fn syntax_colors_partial_override() {
        let cfg = SyntaxColorsConfig {
            keyword: Some(StyleDef {
                fg: Some("#FF0000".to_owned()),
                bg: None,
                modifiers: Some("bold".to_owned()),
            }),
            ..SyntaxColorsConfig::default()
        };
        let base = SyntaxTheme::dark();
        let result = cfg.apply_to(&base);

        // Overridden
        let kw = result.resolve(HighlightStyle::Keyword);
        assert_eq!(kw.fg, Some(Color::Rgb(255, 0, 0)));
        assert!(kw.add_modifier.contains(Modifier::BOLD));

        // Non-overridden should match base
        let string_style = result.resolve(HighlightStyle::String);
        assert_eq!(string_style, base.resolve(HighlightStyle::String));
    }
}
