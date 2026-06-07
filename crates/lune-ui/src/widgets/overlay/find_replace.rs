//! Find/replace bar overlay.

use crate::primitives::{Buffer, Line, Rect, Span, Style, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, HAlign, Modal, ModalState};

use super::util::pop_grapheme;

/// Which field is active in the find/replace bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindReplaceField {
    #[default]
    Find,
    Replace,
}

/// State for the find/replace overlay bar.
#[derive(Clone, Debug, Default)]
pub struct FindReplaceState {
    /// Current find input.
    pub find_input: String,
    /// Current replace input.
    pub replace_input: String,
    /// Which field is active.
    pub active_field: FindReplaceField,
    /// Whether search is case-sensitive.
    pub case_sensitive: bool,
    /// Whether to show the replace row.
    pub show_replace: bool,
    /// Cached search results from the active buffer.
    pub search_state: lune_core::search::SearchState,
}

impl FindReplaceState {
    /// Type a character into the active field.
    pub fn type_char(&mut self, ch: char) {
        match self.active_field {
            FindReplaceField::Find => self.find_input.push(ch),
            FindReplaceField::Replace => self.replace_input.push(ch),
        }
    }

    /// Delete the last *grapheme cluster* from the active field.
    ///
    /// `String::pop` removes one codepoint, which peels composite emoji
    /// (👨‍👩‍👧‍👦, 🧑‍🚀) apart one ZWJ/modifier at a time and leaves
    /// orphan code-points behind.  Popping a whole grapheme keeps the
    /// visible cluster intact.
    pub fn backspace(&mut self) {
        let buf = match self.active_field {
            FindReplaceField::Find => &mut self.find_input,
            FindReplaceField::Replace => &mut self.replace_input,
        };
        pop_grapheme(buf);
    }

    /// Toggle between find and replace fields.
    pub const fn toggle_field(&mut self) {
        self.active_field = match self.active_field {
            FindReplaceField::Find => {
                if self.show_replace {
                    FindReplaceField::Replace
                } else {
                    FindReplaceField::Find
                }
            }
            FindReplaceField::Replace => FindReplaceField::Find,
        };
    }

    /// Toggle case sensitivity.
    pub const fn toggle_case(&mut self) {
        self.case_sensitive = !self.case_sensitive;
    }

    /// Format the match count display.
    #[must_use]
    pub fn match_display(&self) -> String {
        let count = self.search_state.match_count();
        if self.find_input.is_empty() {
            String::new()
        } else if count == 0 {
            "No results".to_string()
        } else if let Some(idx) = self.search_state.current_match {
            format!("{} of {count}", idx + 1)
        } else {
            format!("{count} results")
        }
    }
}

pub(crate) fn render_find_replace(
    area: Rect,
    buf: &mut Buffer,
    state: &FindReplaceState,
    theme: &Theme,
) {
    let bar_w = (area.width * 40 / 100).max(30).min(area.width);
    // Content rows (find + optional replace) + 2 for the top/bottom border.
    let bar_h: u16 = if state.show_replace { 4 } else { 3 };

    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .size_cells(bar_w, bar_h)
        .anchor(Anchor::Top { margin: 0 })
        .h_align(HAlign::Right { margin: 1 })
        // No backdrop: find/replace is an inline bar, not a blocking modal —
        // dimming would obscure the matches the user is scanning.
        .no_backdrop()
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Find row.
    let find_label = "Find: ";
    let case_indicator = if state.case_sensitive { "[Aa]" } else { "[aa]" };
    let match_info = state.match_display();
    let extra_len = find_label.len() + case_indicator.len() + match_info.len() + 2;
    let input_w = (inner.width as usize).saturating_sub(extra_len);

    let find_style = if state.active_field == FindReplaceField::Find {
        Style::new().bold()
    } else {
        Style::new().dim()
    };

    let visible_input = if state.find_input.len() > input_w {
        &state.find_input[state.find_input.len() - input_w..]
    } else {
        &state.find_input
    };

    let find_line = vec![
        Span::from(find_label),
        Span::styled(format!("{visible_input:<input_w$}"), find_style),
        Span::from(" "),
        Span::from(match_info).dim(),
        Span::from(" "),
        Span::from(case_indicator).dim(),
    ];
    Line::from(find_line).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Replace row (if visible).
    if state.show_replace && inner.height > 1 {
        let replace_label = "Replace: ";
        let replace_input_w = (inner.width as usize).saturating_sub(replace_label.len() + 1);

        let replace_style = if state.active_field == FindReplaceField::Replace {
            Style::new().bold()
        } else {
            Style::new().dim()
        };

        let visible_replace = if state.replace_input.len() > replace_input_w {
            &state.replace_input[state.replace_input.len() - replace_input_w..]
        } else {
            &state.replace_input
        };

        let replace_line = vec![
            Span::from(replace_label),
            Span::styled(
                format!("{visible_replace:<replace_input_w$}"),
                replace_style,
            ),
        ];
        Line::from(replace_line).render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::OverlayState;

    #[test]
    fn render_find_replace_shows_find_row() {
        use crate::primitives::{Buffer, Rect};
        use crate::theme::Theme;

        let theme = Theme::dark();
        let mut state = FindReplaceState {
            find_input: "needle".to_string(),
            ..Default::default()
        };
        // Find-only is the case the old sizing collapsed to zero inner rows,
        // so the find row vanished. Both cases must show the input.
        for show_replace in [false, true] {
            state.show_replace = show_replace;
            let area = Rect::new(0, 0, 120, 40);
            let mut buf = Buffer::empty(area);
            render_find_replace(area, &mut buf, &state, &theme);
            let text: String = (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .map(|(x, y)| buf[(x, y)].symbol().to_string())
                .collect();
            assert!(
                text.contains("Find:"),
                "find row must be visible (show_replace={show_replace})"
            );
            assert!(text.contains("needle"), "find input must render");
        }
    }

    #[test]
    fn find_replace_type_and_backspace() {
        let mut state = FindReplaceState::default();
        state.type_char('h');
        state.type_char('i');
        assert_eq!(state.find_input, "hi");
        state.backspace();
        assert_eq!(state.find_input, "h");
    }

    #[test]
    fn find_replace_toggle_field() {
        let mut state = FindReplaceState {
            show_replace: true,
            ..Default::default()
        };
        assert_eq!(state.active_field, FindReplaceField::Find);
        state.toggle_field();
        assert_eq!(state.active_field, FindReplaceField::Replace);
        state.toggle_field();
        assert_eq!(state.active_field, FindReplaceField::Find);
    }

    #[test]
    fn find_replace_toggle_field_no_replace() {
        let mut state = FindReplaceState {
            show_replace: false,
            ..Default::default()
        };
        state.toggle_field();
        // Should stay on Find when replace is hidden.
        assert_eq!(state.active_field, FindReplaceField::Find);
    }

    #[test]
    fn find_replace_toggle_case() {
        let mut state = FindReplaceState::default();
        assert!(!state.case_sensitive);
        state.toggle_case();
        assert!(state.case_sensitive);
    }

    #[test]
    fn find_replace_match_display_empty() {
        let state = FindReplaceState::default();
        assert_eq!(state.match_display(), "");
    }

    #[test]
    fn find_replace_open_methods() {
        let mut overlay = OverlayState::default();
        overlay.open_find();
        assert!(overlay.is_active());
        assert!(!overlay.find_replace.show_replace);
        overlay.close();

        overlay.open_find_replace();
        assert!(overlay.is_active());
        assert!(overlay.find_replace.show_replace);
    }
}
