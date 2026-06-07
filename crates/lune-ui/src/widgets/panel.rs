//! Shared building blocks for the bordered panel widgets
//! (`file_tree`, `git_panel`, `terminal`, editor pane).
//!
//! Four panels independently rebuilt the same Block construction
//! (focus-aware border color + standard title styling). This module
//! collapses that into one helper pair, so adding a new panel — or
//! changing the focus color scheme — only touches one place.

use crate::primitives::{Block, BorderType, Borders, Line, Modifier, Span, Style};
use crate::theme::Theme;

/// Build a focus-aware bordered Block.
///
/// Border color tracks `is_focused` against the theme's
/// `border_focused` / `border_unfocused` slots; the caller picks the
/// sides to draw via `borders`.
#[must_use]
pub fn panel_block<'a>(theme: &Theme, is_focused: bool, borders: Borders) -> Block<'a> {
    let color = if is_focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };
    Block::default()
        .borders(borders)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
}

/// Standard panel title: ` label ` styled BOLD, plus the theme accent
/// color when the panel is focused.
#[must_use]
pub fn panel_title<'a>(label: impl Into<String>, theme: &Theme, is_focused: bool) -> Line<'a> {
    let style = if is_focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(Span::styled(format!(" {} ", label.into()), style))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Borders, Buffer, Modifier, Rect, Widget};
    use crate::theme::Theme;

    #[test]
    fn panel_block_uses_focused_color_when_focused() {
        let theme = Theme::dark();
        let block = panel_block(&theme, true, Borders::ALL);
        let area = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.border_focused));
    }

    #[test]
    fn panel_block_uses_unfocused_color_when_not_focused() {
        let theme = Theme::dark();
        let block = panel_block(&theme, false, Borders::ALL);
        let area = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.border_unfocused));
    }

    #[test]
    fn panel_block_honors_borders_argument() {
        // TOP|BOTTOM only: the top row is `─`, the leftmost column of
        // row 1 must NOT be a border character.
        let theme = Theme::dark();
        let block = panel_block(&theme, true, Borders::TOP | Borders::BOTTOM);
        let area = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "─", "top-left should be a top rule");
        // Row 1 column 0 is inside the block (no left border drawn).
        // The cell is whatever the Buffer default is — definitely not
        // a border glyph.
        assert_ne!(buf[(0, 1)].symbol(), "│");
        assert_ne!(buf[(0, 1)].symbol(), "┌");
    }

    #[test]
    fn panel_title_focused_uses_accent_and_bold() {
        let theme = Theme::dark();
        let title = panel_title("EXPLORER", &theme, true);
        let spans: Vec<_> = title.spans.iter().collect();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " EXPLORER ");
        assert_eq!(spans[0].style.fg, Some(theme.accent));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn panel_title_unfocused_is_bold_without_accent() {
        let theme = Theme::dark();
        let title = panel_title("SOURCE CONTROL", &theme, false);
        let spans: Vec<_> = title.spans.iter().collect();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " SOURCE CONTROL ");
        assert_eq!(spans[0].style.fg, None);
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }
}
