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

        // Catppuccin Mocha palette for syntax highlighting.
        styles[HighlightStyle::Keyword as usize] =
            Style::new().fg(hex("#cba6f7")).add_modifier(Modifier::BOLD); // mauve
        styles[HighlightStyle::Type as usize] = Style::new().fg(hex("#f9e2af")); // yellow
        styles[HighlightStyle::Function as usize] = Style::new().fg(hex("#89b4fa")); // blue
        styles[HighlightStyle::String as usize] = Style::new().fg(hex("#a6e3a1")); // green
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(hex("#7f849c")) // overlay1
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(hex("#fab387")); // peach
        styles[HighlightStyle::Operator as usize] = Style::new().fg(hex("#89dceb")); // sky
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(hex("#9399b2")); // overlay2
        styles[HighlightStyle::Variable as usize] = Style::new().fg(hex("#cdd6f4")); // text
        styles[HighlightStyle::Constant as usize] =
            Style::new().fg(hex("#fab387")).add_modifier(Modifier::BOLD); // peach
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(hex("#f9e2af")); // yellow
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(hex("#b4befe")); // lavender
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(hex("#f38ba8")) // red
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(hex("#94e2d5")); // teal
        styles[HighlightStyle::Default as usize] = Style::default();

        Self { styles }
    }

    /// Create a light theme suitable for light-background terminals.
    #[must_use]
    pub fn light() -> Self {
        let mut styles = [Style::default(); STYLE_COUNT];

        styles[HighlightStyle::Keyword as usize] =
            Style::new().fg(hex("#0000b4")).add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Type as usize] = Style::new().fg(hex("#007878"));
        styles[HighlightStyle::Function as usize] = Style::new().fg(hex("#783c00"));
        styles[HighlightStyle::String as usize] = Style::new().fg(hex("#007800"));
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(hex("#828282"))
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(hex("#8c008c"));
        styles[HighlightStyle::Operator as usize] = Style::new().fg(hex("#3c3c3c"));
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(hex("#646464"));
        styles[HighlightStyle::Variable as usize] = Style::new().fg(hex("#b42828"));
        styles[HighlightStyle::Constant as usize] =
            Style::new().fg(hex("#8c008c")).add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(hex("#a06400"));
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(hex("#006464"));
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(hex("#c80000"))
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(hex("#006400"));
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
    fn dark_theme_keyword_is_mauve_bold() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Keyword);
        assert_eq!(style.fg, Some(Color::Rgb(203, 166, 247)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn dark_theme_string_is_green() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::String);
        assert_eq!(style.fg, Some(Color::Rgb(166, 227, 161)));
    }

    #[test]
    fn dark_theme_comment_is_italic() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Comment);
        assert_eq!(style.fg, Some(Color::Rgb(127, 132, 156)));
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn default_style_is_no_op() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Default);
        assert_eq!(style, Style::default());
    }
}
