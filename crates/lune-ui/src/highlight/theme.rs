//! Syntax theme: maps [`HighlightStyle`] to ratatui [`Style`].
//!
//! Provides a default dark theme and the machinery to resolve highlight
//! categories to terminal-renderable styles.

use crate::primitives::{Color, Modifier, Style};
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
        styles[HighlightStyle::Keyword as usize] = Style::new()
            .fg(Color::Rgb(203, 166, 247))
            .add_modifier(Modifier::BOLD); // mauve
        styles[HighlightStyle::Type as usize] = Style::new().fg(Color::Rgb(249, 226, 175)); // yellow
        styles[HighlightStyle::Function as usize] = Style::new().fg(Color::Rgb(137, 180, 250)); // blue
        styles[HighlightStyle::String as usize] = Style::new().fg(Color::Rgb(166, 227, 161)); // green
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(Color::Rgb(127, 132, 156)) // overlay1
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(Color::Rgb(250, 179, 135)); // peach
        styles[HighlightStyle::Operator as usize] = Style::new().fg(Color::Rgb(137, 220, 235)); // sky
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(Color::Rgb(147, 153, 178)); // overlay2
        styles[HighlightStyle::Variable as usize] = Style::new().fg(Color::Rgb(205, 214, 244)); // text
        styles[HighlightStyle::Constant as usize] = Style::new()
            .fg(Color::Rgb(250, 179, 135))
            .add_modifier(Modifier::BOLD); // peach
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(Color::Rgb(249, 226, 175)); // yellow
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(Color::Rgb(180, 190, 254)); // lavender
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(Color::Rgb(243, 139, 168)) // red
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(Color::Rgb(148, 226, 213)); // teal
        styles[HighlightStyle::Default as usize] = Style::default();

        Self { styles }
    }

    /// Create a light theme suitable for light-background terminals.
    #[must_use]
    pub fn light() -> Self {
        let mut styles = [Style::default(); STYLE_COUNT];

        styles[HighlightStyle::Keyword as usize] = Style::new()
            .fg(Color::Rgb(0, 0, 180))
            .add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Type as usize] = Style::new().fg(Color::Rgb(0, 120, 120));
        styles[HighlightStyle::Function as usize] = Style::new().fg(Color::Rgb(120, 60, 0));
        styles[HighlightStyle::String as usize] = Style::new().fg(Color::Rgb(0, 120, 0));
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(Color::Rgb(130, 130, 130))
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(Color::Rgb(140, 0, 140));
        styles[HighlightStyle::Operator as usize] = Style::new().fg(Color::Rgb(60, 60, 60));
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(Color::Rgb(100, 100, 100));
        styles[HighlightStyle::Variable as usize] = Style::new().fg(Color::Rgb(180, 40, 40));
        styles[HighlightStyle::Constant as usize] = Style::new()
            .fg(Color::Rgb(140, 0, 140))
            .add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(Color::Rgb(160, 100, 0));
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(Color::Rgb(0, 100, 100));
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(Color::Rgb(200, 0, 0))
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(Color::Rgb(0, 100, 0));
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
