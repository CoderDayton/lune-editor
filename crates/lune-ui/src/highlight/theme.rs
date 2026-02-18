//! Syntax theme: maps [`HighlightStyle`] to ratatui [`Style`].
//!
//! Provides a default dark theme and the machinery to resolve highlight
//! categories to terminal-renderable styles.

use lune_core::highlight::HighlightStyle;
use ratatui_core::style::{Color, Modifier, Style};

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

        styles[HighlightStyle::Keyword as usize] =
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Type as usize] = Style::new().fg(Color::Yellow);
        styles[HighlightStyle::Function as usize] = Style::new().fg(Color::Blue);
        styles[HighlightStyle::String as usize] = Style::new().fg(Color::Green);
        styles[HighlightStyle::Comment as usize] = Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        styles[HighlightStyle::Number as usize] = Style::new().fg(Color::Magenta);
        styles[HighlightStyle::Operator as usize] = Style::new().fg(Color::White);
        styles[HighlightStyle::Punctuation as usize] = Style::new().fg(Color::DarkGray);
        styles[HighlightStyle::Variable as usize] = Style::new().fg(Color::LightRed);
        styles[HighlightStyle::Constant as usize] =
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
        styles[HighlightStyle::Attribute as usize] = Style::new().fg(Color::LightYellow);
        styles[HighlightStyle::Namespace as usize] = Style::new().fg(Color::LightCyan);
        styles[HighlightStyle::Error as usize] = Style::new()
            .fg(Color::Red)
            .add_modifier(Modifier::UNDERLINED);
        styles[HighlightStyle::Embedded as usize] = Style::new().fg(Color::LightGreen);
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
    fn dark_theme_keyword_is_cyan_bold() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Keyword);
        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn dark_theme_string_is_green() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::String);
        assert_eq!(style.fg, Some(Color::Green));
    }

    #[test]
    fn dark_theme_comment_is_italic() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Comment);
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn default_style_is_no_op() {
        let theme = SyntaxTheme::dark();
        let style = theme.resolve(HighlightStyle::Default);
        assert_eq!(style, Style::default());
    }
}
