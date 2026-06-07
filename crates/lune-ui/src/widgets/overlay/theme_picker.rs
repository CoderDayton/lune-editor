//! Theme picker overlay with live preview.

use crate::primitives::{Buffer, Line, Rect, Span, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

use super::util::render_hrule;

/// State for the theme picker overlay with live preview.
#[derive(Clone, Debug, Default)]
pub struct ThemePickerState {
    /// All available themes as `(registry_index, display_name)`.
    pub all_themes: Vec<(usize, String)>,
    /// Filtered subset matching the current input.
    pub filtered: Vec<(usize, String)>,
    /// Selected row index into `filtered`.
    pub selected: usize,
    /// Filter input string.
    pub input: String,
    /// Scroll offset for the visible window.
    pub scroll_offset: usize,
    /// Registry index of the theme that was active when the picker opened.
    /// Used to revert on Escape.
    pub original_idx: usize,
}

impl ThemePickerState {
    /// Build from a list of `(registry_index, name)` pairs.
    ///
    /// Pre-selects the entry matching `current_idx`.
    #[must_use]
    pub fn new(themes: Vec<(usize, String)>, current_idx: usize) -> Self {
        let selected = themes
            .iter()
            .position(|(idx, _)| *idx == current_idx)
            .unwrap_or(0);
        let filtered = themes.clone();
        Self {
            all_themes: themes,
            filtered,
            selected,
            input: String::new(),
            scroll_offset: 0,
            original_idx: current_idx,
        }
    }

    /// Re-filter by `input` (case-insensitive substring).
    pub fn update_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = if query.is_empty() {
            self.all_themes.clone()
        } else {
            self.all_themes
                .iter()
                .filter(|(_, name)| name.to_lowercase().contains(&query))
                .cloned()
                .collect()
        };
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Move selection down (wraps).
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
            self.ensure_visible(10);
        }
    }

    /// Move selection up (wraps).
    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.filtered.len() - 1);
            self.ensure_visible(10);
        }
    }

    const fn ensure_visible(&mut self, list_height: usize) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + list_height {
            self.scroll_offset = self.selected + 1 - list_height;
        }
    }

    /// The registry index of the currently selected theme, if any.
    #[must_use]
    pub fn selected_idx(&self) -> Option<usize> {
        self.filtered.get(self.selected).map(|(idx, _)| *idx)
    }

    /// Append a character to the filter.
    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.update_filter();
    }

    /// Remove the last character from the filter.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_theme_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &ThemePickerState,
    theme: &Theme,
) {
    let popup_w = (area.width * 40 / 100).max(36).min(area.width);
    let list_rows = (state.filtered.len() as u16).min(12);
    let popup_h = (2 + 1 + list_rows + 1 + 2).min(area.height);
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(" Select Theme ")
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
        for (row, (list_idx, (_, name))) in state
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
            let label = format!("  {name}");
            let max_w = inner.width.saturating_sub(2) as usize;
            let label = if label.len() > max_w {
                format!("{}…", &label[..max_w.saturating_sub(1)])
            } else {
                label
            };
            if list_idx == state.selected {
                Line::from(Span::styled(label, theme.overlay_selected))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            } else {
                Line::from(Span::from(label)).render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        }
    }

    // Footer hint — note the live-preview behaviour.
    if footer_y > list_y_start {
        Line::from(Span::from(" ↑↓ preview · Enter apply · Esc revert").dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}
