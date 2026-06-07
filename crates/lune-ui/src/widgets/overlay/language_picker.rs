//! Language selector overlay — fuzzy-filtered list of all known languages.

use crate::primitives::{Buffer, Line, Rect, Span, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};
use lune_core::language::LanguageId;

use super::util::render_hrule;

/// State for the language selector overlay.
#[derive(Clone, Debug, Default)]
pub struct LanguagePickerState {
    /// All available language IDs (sorted alphabetically, deduplicated).
    pub all_languages: Vec<LanguageId>,
    /// Currently displayed (filtered) subset.
    pub filtered: Vec<LanguageId>,
    /// Highlighted index into `filtered`.
    pub selected: usize,
    /// Filter input string.
    pub input: String,
    /// Scroll offset for the visible list window.
    pub scroll_offset: usize,
}

impl LanguagePickerState {
    /// Build from a list of language IDs (sorts and deduplicates).
    #[must_use]
    pub fn new(mut languages: Vec<LanguageId>) -> Self {
        languages.sort_by_key(|l| l.0);
        languages.dedup();
        let filtered = languages.clone();
        Self {
            all_languages: languages,
            filtered,
            selected: 0,
            input: String::new(),
            scroll_offset: 0,
        }
    }

    /// Re-filter `all_languages` by `input` (case-insensitive substring).
    pub fn update_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = if query.is_empty() {
            self.all_languages.clone()
        } else {
            self.all_languages
                .iter()
                .filter(|l| l.0.to_lowercase().contains(&query))
                .copied()
                .collect()
        };
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Move selection down (wraps).
    pub fn select_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.filtered.len();
        self.ensure_visible(10);
    }

    /// Move selection up (wraps).
    pub fn select_prev(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.filtered.len() - 1);
        self.ensure_visible(10);
    }

    /// Scroll so that `selected` is within a window of `list_height` rows.
    const fn ensure_visible(&mut self, list_height: usize) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + list_height {
            self.scroll_offset = self.selected + 1 - list_height;
        }
    }

    /// The currently selected language, if any.
    #[must_use]
    pub fn selected_lang(&self) -> Option<LanguageId> {
        self.filtered.get(self.selected).copied()
    }

    /// Append a character to the filter input.
    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.update_filter();
    }

    /// Remove the last character from the filter input.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_language_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &LanguagePickerState,
    theme: &Theme,
) {
    // Popup dimensions: 40% wide, tall enough for input + up to 12 items + footer.
    let popup_w = (area.width * 40 / 100).max(36).min(area.width);
    let list_rows = (state.filtered.len() as u16).min(12);
    let popup_h = (2 + 1 + list_rows + 1 + 2).min(area.height); // border*2 + input + sep + items + footer
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(" Select Language ")
        .size_cells(popup_w, popup_h)
        .anchor(Anchor::Top {
            margin: (area.height.saturating_sub(popup_h)) / 3,
        })
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Input row.
    let cursor = if state.input.is_empty() { "█" } else { "" };
    let input_str = format!(" > {}{}", state.input, cursor);
    Line::from(Span::from(input_str)).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    if inner.height <= 1 {
        return;
    }

    // Separator.
    render_hrule(buf, inner.x, inner.y + 1, inner.width);

    if inner.height <= 2 {
        return;
    }

    let list_y_start = inner.y + 2;
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let list_y_end = footer_y;
    let visible_rows = list_y_end.saturating_sub(list_y_start) as usize;

    if state.filtered.is_empty() {
        Line::from(Span::from("  No matches").dim())
            .render(Rect::new(inner.x, list_y_start, inner.width, 1), buf);
    } else {
        for (row, (idx, lang)) in state
            .filtered
            .iter()
            .enumerate()
            .skip(state.scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let y = list_y_start + row as u16;
            if y >= list_y_end {
                break;
            }
            let label = format!("  {}", lang.name());
            let max_w = inner.width.saturating_sub(2) as usize;
            let label = if label.len() > max_w {
                format!("{}…", &label[..max_w.saturating_sub(1)])
            } else {
                label
            };
            if idx == state.selected {
                Line::from(Span::styled(label, theme.overlay_selected))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            } else {
                Line::from(Span::from(label)).render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        }
    }

    // Footer hint.
    if footer_y > list_y_start {
        Line::from(Span::from(" ↑↓ select · Enter confirm · Esc cancel").dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::{OverlayKind, OverlayState};
    #[test]
    fn language_picker_new_sorts_and_dedupes() {
        use lune_core::language::lang;
        let langs = vec![lang::PYTHON, lang::RUST, lang::PYTHON, lang::GO];
        let picker = LanguagePickerState::new(langs);
        // sorted: Go, Python, Rust — Python deduped
        assert_eq!(picker.all_languages.len(), 3);
        assert_eq!(picker.filtered.len(), 3);
        assert_eq!(picker.all_languages[0], lang::GO);
        assert_eq!(picker.all_languages[1], lang::PYTHON);
        assert_eq!(picker.all_languages[2], lang::RUST);
    }

    #[test]
    fn language_picker_filter_by_input() {
        use lune_core::language::lang;
        let mut picker =
            LanguagePickerState::new(vec![lang::RUST, lang::RUBY, lang::PYTHON, lang::GO]);
        picker.type_char('r');
        // "r" matches Rust, Ruby (case-insensitive)
        assert_eq!(picker.filtered.len(), 2);
        picker.type_char('u');
        // "ru" matches Rust, Ruby
        assert_eq!(picker.filtered.len(), 2);
        picker.type_char('s');
        // "rus" matches only Rust
        assert_eq!(picker.filtered.len(), 1);
        assert_eq!(picker.selected_lang(), Some(lang::RUST));
    }

    #[test]
    fn language_picker_backspace_restores_filter() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        picker.type_char('r');
        assert_eq!(picker.filtered.len(), 1);
        picker.backspace();
        assert_eq!(picker.filtered.len(), 2);
    }

    #[test]
    fn language_picker_navigation_wraps() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        assert_eq!(picker.selected, 0);
        picker.select_next();
        assert_eq!(picker.selected, 1);
        picker.select_next(); // wraps to 0
        assert_eq!(picker.selected, 0);
        picker.select_prev(); // wraps to 1
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn language_picker_empty_filter_no_match() {
        use lune_core::language::lang;
        let mut picker = LanguagePickerState::new(vec![lang::RUST, lang::PYTHON]);
        picker.type_char('z'); // no language contains 'z'
        assert!(picker.filtered.is_empty());
        assert_eq!(picker.selected_lang(), None);
    }

    #[test]
    fn language_picker_overlay_opens_correctly() {
        use lune_core::language::lang;
        let mut overlay = OverlayState::default();
        overlay.open_language_picker(vec![lang::RUST, lang::PYTHON]);
        assert!(overlay.is_active());
        assert!(matches!(overlay.active, Some(OverlayKind::LanguagePicker)));
        assert_eq!(overlay.language_picker.all_languages.len(), 2);
    }
}
