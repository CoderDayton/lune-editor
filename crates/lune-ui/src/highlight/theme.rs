//! Syntax theme: maps [`HighlightStyle`] to ratatui [`Style`].
//!
//! Provides a default dark theme and the machinery to resolve highlight
//! categories to terminal-renderable styles.

use crate::primitives::{Modifier, Style};
use crate::style::color::hex;
use lune_core::highlight::HighlightStyle;

// ── Syntax theme ──────────────────────────────────────────────────────

/// Maps `HighlightStyle` categories to ratatui `Style` values.
#[derive(Clone)]
pub struct SyntaxTheme {
    styles: [Style; STYLE_COUNT],
}

const STYLE_COUNT: usize = 16;

impl SyntaxTheme {
    /// Create the default dark theme.
    #[must_use]
    pub fn dark() -> Self {
        let mut styles = [Style::default(); STYLE_COUNT];

        // GitHub × Gruvbox-soft — the same six accent hues the UI chrome
        // uses, so code and chrome read as a single cohesive theme.
        styles[HighlightStyle::Keyword as usize] =
            Style::new().fg(hex("#e67e80")).add_modifier(Modifier::BOLD); // red
        styles[HighlightStyle::Type as usize] = Style::new().fg(hex("#dbbc7f")); // yellow
        styles[HighlightStyle::Function as usize] = Style::new().fg(hex("#83a6d6")); // blue
        styles[HighlightStyle::String as usize] = Style::new().fg(hex("#a7c080")); // green
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(hex("#7d8590")) // muted
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(hex("#d699b6")); // mauve
        styles[HighlightStyle::Operator as usize] = Style::new().fg(hex("#d3c6aa")); // text
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(hex("#7d8590")); // muted
        styles[HighlightStyle::Variable as usize] = Style::new().fg(hex("#d3c6aa")); // text
        styles[HighlightStyle::Constant as usize] =
            Style::new().fg(hex("#d699b6")).add_modifier(Modifier::BOLD); // mauve
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(hex("#dbbc7f")); // yellow
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(hex("#7fbbb3")); // teal
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(hex("#e67e80")) // red
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(hex("#7fbbb3")); // teal
        styles[HighlightStyle::Default as usize] = Style::default();

        Self { styles }
    }

    /// Create a light theme suitable for light-background terminals.
    #[must_use]
    pub fn light() -> Self {
        let mut styles = [Style::default(); STYLE_COUNT];

        // GitHub × Gruvbox-soft (light) — the same six accent hues the
        // light UI chrome uses.
        styles[HighlightStyle::Keyword as usize] =
            Style::new().fg(hex("#c14a3d")).add_modifier(Modifier::BOLD); // red
        styles[HighlightStyle::Type as usize] = Style::new().fg(hex("#b07d2b")); // yellow
        styles[HighlightStyle::Function as usize] = Style::new().fg(hex("#3a7bd5")); // blue
        styles[HighlightStyle::String as usize] = Style::new().fg(hex("#6c802f")); // green
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(hex("#6f6957")) // muted
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(hex("#9a5fb0")); // mauve
        styles[HighlightStyle::Operator as usize] = Style::new().fg(hex("#3c3a32")); // text
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(hex("#6f6957")); // muted
        styles[HighlightStyle::Variable as usize] = Style::new().fg(hex("#3c3a32")); // text
        styles[HighlightStyle::Constant as usize] =
            Style::new().fg(hex("#9a5fb0")).add_modifier(Modifier::BOLD); // mauve
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(hex("#b07d2b")); // yellow
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(hex("#4a8b80")); // teal
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(hex("#c14a3d")) // red
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(hex("#4a8b80")); // teal
        styles[HighlightStyle::Default as usize] = Style::default();

        Self { styles }
    }

    /// Resolve a `HighlightStyle` to a ratatui `Style`.
    #[must_use]
    pub fn resolve(&self, hl: HighlightStyle) -> Style {
        let idx = hl as usize;
        if idx < self.styles.len() {
            self.styles[idx]
        } else {
            Style::default()
        }
    }

    /// Override the style for a specific `HighlightStyle` category.
    pub const fn set(&mut self, hl: HighlightStyle, style: Style) {
        let idx = hl as usize;
        if idx < self.styles.len() {
            self.styles[idx] = style;
        }
    }
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Color;

    #[test]
    fn dark_theme_keyword_is_red_bold() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Keyword);
        assert_eq!(style.fg, Some(Color::Rgb(230, 126, 128)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn dark_theme_string_is_green() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::String);
        assert_eq!(style.fg, Some(Color::Rgb(167, 192, 128)));
    }

    #[test]
    fn dark_theme_comment_is_italic() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Comment);
        assert_eq!(style.fg, Some(Color::Rgb(125, 133, 144)));
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn default_style_is_no_op() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Default);
        assert_eq!(style, Style::default());
    }
}
