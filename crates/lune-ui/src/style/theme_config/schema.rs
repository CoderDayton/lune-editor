use crate::highlight::theme::SyntaxTheme;
use crate::primitives::{Modifier, Style};
use lune_core::highlight::HighlightStyle;
use serde::{Deserialize, Serialize};

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
    pub(super) fn to_style(&self) -> Style {
        let mut style = Style::new();
        if let Some(ref fg) = self.fg {
            if let Some(c) = crate::style::color::parse_color(fg) {
                style = style.fg(c);
            }
        }
        if let Some(ref bg) = self.bg {
            if let Some(c) = crate::style::color::parse_color(bg) {
                style = style.bg(c);
            }
        }
        if let Some(ref mods) = self.modifiers {
            style = style.add_modifier(parse_modifiers(mods));
        }
        style
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
    pub(super) fn apply_to(&self, base: &SyntaxTheme) -> SyntaxTheme {
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
    pub brand: Option<StyleDef>,
    pub info: Option<StyleDef>,
    pub bg: Option<StyleDef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NotificationColorsConfig {
    pub success: Option<String>,
    pub info: Option<String>,
    pub warn: Option<String>,
    pub error: Option<String>,
    /// Toast panel background fill.
    pub bg: Option<String>,
    /// Toast text color.
    pub fg: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::theme::SyntaxTheme;
    use crate::primitives::{Color, Modifier};
    use lune_core::highlight::HighlightStyle;

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
