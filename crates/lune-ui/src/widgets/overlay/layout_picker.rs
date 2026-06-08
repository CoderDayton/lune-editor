//! Layout picker overlay for agent pane tiling presets.

use crate::primitives::{
    Block, BorderType, Borders, Buffer, Line, Rect, Span, Style, Stylize, Widget,
};
use crate::runtime::tiling::{SavedTileNode, SplitDirection};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

use super::util::truncate_inline_text;

/// State for the agent pane layout picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutPickerEntryKind {
    Preset(usize),
    Saved(usize),
}

/// One selectable layout picker row.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPickerEntry {
    pub label: String,
    pub pane_count: usize,
    pub kind: LayoutPickerEntryKind,
    /// Serializable tree shape used for the picker preview.
    pub preview: SavedTileNode,
    /// Whether this entry is the currently active layout for the tab.
    pub is_active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LayoutPickerState {
    /// Full list of layout entries available.
    pub entries: Vec<LayoutPickerEntry>,
    /// Current selection, expressed as an index into
    /// [`Self::filtered_indices`] (not into `entries`).
    pub selected: usize,
    /// First visible row when the filtered list needs to scroll.
    pub scroll_offset: usize,
    /// Live-typed fuzzy filter text.
    pub filter: String,
    /// Indices into [`Self::entries`] matching the current filter.
    pub filtered_indices: Vec<usize>,
}

impl LayoutPickerState {
    /// Create a new layout picker with the first preset selected.
    #[must_use]
    pub fn new(entries: Vec<LayoutPickerEntry>) -> Self {
        let filtered_indices = (0..entries.len()).collect();
        Self {
            entries,
            selected: 0,
            scroll_offset: 0,
            filter: String::new(),
            filtered_indices,
        }
    }

    /// Number of entries that currently pass the filter.
    #[must_use]
    pub const fn filtered_len(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Append a char to the filter and refresh the match set.
    pub fn push_filter_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.recompute_filter();
    }

    /// Remove the last char from the filter. Returns `true` if a char was
    /// actually removed.
    pub fn pop_filter_char(&mut self) -> bool {
        if self.filter.pop().is_some() {
            self.recompute_filter();
            true
        } else {
            false
        }
    }

    /// Clear the filter entirely.
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.recompute_filter();
    }

    /// Replace the current filter text and refresh the match set.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.recompute_filter();
    }

    fn recompute_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.entries.len()).collect();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered_indices = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.label.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len() - 1;
        }
        self.scroll_offset = 0;
    }

    /// Select a specific row, clamping against the filtered list.
    pub fn select(&mut self, index: usize) {
        let len = self.filtered_indices.len();
        if len == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }
        self.selected = index.min(len - 1);
    }

    /// Select a concrete entry index from [`Self::entries`].
    pub fn select_entry_index(&mut self, entry_index: usize) {
        let Some(filtered_index) = self
            .filtered_indices
            .iter()
            .position(|&idx| idx == entry_index)
        else {
            self.select(0);
            return;
        };
        self.select(filtered_index);
    }

    /// Move selection down (wraps).
    pub const fn select_next(&mut self) {
        let len = self.filtered_indices.len();
        if len != 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    /// Move selection up (wraps).
    pub fn select_prev(&mut self) {
        let len = self.filtered_indices.len();
        if len != 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(len - 1);
        }
    }

    /// Jump to the first entry in the filtered list.
    pub fn select_first(&mut self) {
        self.select(0);
    }

    /// Jump to the last entry in the filtered list.
    pub fn select_last(&mut self) {
        let len = self.filtered_indices.len();
        if len != 0 {
            self.select(len - 1);
        }
    }

    /// Move selection by a page of rows.
    pub fn move_page(&mut self, delta: isize, page_size: usize) {
        let len = self.filtered_indices.len();
        if len == 0 {
            return;
        }

        let last = len.saturating_sub(1);
        let magnitude = page_size.max(1).saturating_mul(delta.unsigned_abs());
        let next = if delta.is_negative() {
            self.selected.saturating_sub(magnitude)
        } else {
            self.selected.saturating_add(magnitude).min(last)
        };
        self.select(next);
    }

    /// Borrow the selected entry, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&LayoutPickerEntry> {
        let entry_idx = *self.filtered_indices.get(self.selected)?;
        self.entries.get(entry_idx)
    }

    /// Compute the visible entry window for a given number of rows.
    #[must_use]
    pub fn visible_range(&self, visible_rows: usize) -> (usize, usize) {
        let len = self.filtered_indices.len();
        if len == 0 || visible_rows == 0 {
            return (0, 0);
        }

        let max_start = len.saturating_sub(visible_rows);
        let mut start = self.scroll_offset.min(max_start);
        if self.selected < start {
            start = self.selected;
        } else if self.selected >= start + visible_rows {
            start = self.selected + 1 - visible_rows;
        }
        let end = (start + visible_rows).min(len);
        (start, end)
    }
}

const LAYOUT_PICKER_PREVIEW_MIN_INNER_WIDTH: u16 = 46;
/// Width reserved for the list column when the preview is shown.
const LAYOUT_PICKER_LIST_COL_WIDTH: u16 = 26;

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_layout_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &LayoutPickerState,
    theme: &Theme,
) {
    // Widen the popup so the preview has room to breathe.
    let popup_w = (area.width * 60 / 100).clamp(30, area.width.min(90));
    let preferred_h: u16 = 16;
    let popup_h = preferred_h.min(area.height.saturating_sub(2)).max(6);
    let title = if state.filter.is_empty() {
        " Select Layout ".to_string()
    } else {
        format!(" Select Layout · filter: {} ", state.filter)
    };
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(&title)
        .size_cells(popup_w, popup_h)
        .anchor(Anchor::Top {
            margin: area.height.saturating_sub(popup_h) / 3,
        })
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Decide whether there is room for a side-by-side preview column.
    let (list_area, preview_area) = if inner.width >= LAYOUT_PICKER_PREVIEW_MIN_INNER_WIDTH {
        let list_w = LAYOUT_PICKER_LIST_COL_WIDTH.min(inner.width - 10);
        let list = Rect::new(inner.x, inner.y, list_w, inner.height);
        // Gap of 1 cell for a subtle column divider.
        let preview = Rect::new(
            inner.x + list_w + 1,
            inner.y,
            inner.width - list_w - 1,
            inner.height,
        );
        (list, Some(preview))
    } else {
        (inner, None)
    };

    let list_y = list_area.y;
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let list_height = footer_y.saturating_sub(list_y);
    let visible_rows = usize::from(list_height);
    let (start, end) = state.visible_range(visible_rows);

    for (row, entry_idx) in state.filtered_indices[start..end].iter().enumerate() {
        let Some(entry) = state.entries.get(*entry_idx) else {
            continue;
        };
        let i = start + row;
        let y = list_y + row as u16;
        let marker = if entry.is_active { "● " } else { "  " };
        let label = match entry.kind {
            LayoutPickerEntryKind::Preset(_) => {
                format!("{marker}{} ({} panes)", entry.label, entry.pane_count)
            }
            LayoutPickerEntryKind::Saved(_) => {
                format!(
                    "{marker}{} [Saved] ({} panes)",
                    entry.label, entry.pane_count
                )
            }
        };
        let label = truncate_inline_text(&label, list_area.width as usize);
        if i == state.selected {
            Line::from(Span::styled(label, theme.overlay_selected))
                .render(Rect::new(list_area.x, y, list_area.width, 1), buf);
        } else {
            Line::from(Span::from(label))
                .render(Rect::new(list_area.x, y, list_area.width, 1), buf);
        }
    }

    if let Some(preview_area) = preview_area {
        // Subtle divider between list and preview columns.
        let divider_x = preview_area.x.saturating_sub(1);
        for y in inner.y..footer_y {
            buf[(divider_x, y)]
                .set_char('│')
                .set_fg(theme.overlay_border);
        }

        if let Some(entry) = state.selected_entry() {
            let preview_rect = Rect::new(
                preview_area.x,
                preview_area.y,
                preview_area.width,
                list_height,
            );
            render_saved_layout_preview(preview_rect, buf, &entry.preview, theme);
        }
    }

    if footer_y > list_y {
        let footer = layout_picker_footer(
            state,
            start > 0 || state.filtered_indices.len() > end,
            inner.width as usize,
        );
        Line::from(Span::from(footer).dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}

pub(crate) fn render_saved_layout_preview(
    area: Rect,
    buf: &mut Buffer,
    tree: &SavedTileNode,
    theme: &Theme,
) {
    if area.width < 8 || area.height < 4 {
        return;
    }

    // Outer frame
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .style(Style::new().fg(theme.fg_muted));
    block.render(area, buf);

    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }
    let inner = Rect::new(inner_x, inner_y, inner_w, inner_h);

    let mut line_masks = vec![0_u8; usize::from(inner.width) * usize::from(inner.height)];
    let mut leaves: Vec<Rect> = Vec::new();
    render_preview_tree(tree, inner, inner, &mut line_masks, &mut leaves);

    for y in 0..inner.height {
        for x in 0..inner.width {
            let mask = line_masks[preview_mask_index(inner.width, x, y)];
            let Some(ch) = preview_mask_char(mask) else {
                continue;
            };
            buf[(inner.x + x, inner.y + y)]
                .set_char(ch)
                .set_fg(theme.fg_muted);
        }
    }

    // Number each leaf in its center.
    for (i, rect) in leaves.iter().enumerate() {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let label = format!("{}", i + 1);
        let x = rect.x + rect.width / 2;
        let y = rect.y + rect.height / 2;
        buf[(x, y)]
            .set_symbol(&label)
            .set_fg(theme.fg)
            .set_bg(theme.bg);
    }
}

/// Recursive half of the preview renderer. Draws split dividers and
/// collects the leaf rectangles that remain.
const PREVIEW_NORTH: u8 = 1 << 0;
const PREVIEW_EAST: u8 = 1 << 1;
const PREVIEW_SOUTH: u8 = 1 << 2;
const PREVIEW_WEST: u8 = 1 << 3;
const PREVIEW_VERTICAL: u8 = PREVIEW_NORTH | PREVIEW_SOUTH;
const PREVIEW_HORIZONTAL: u8 = PREVIEW_EAST | PREVIEW_WEST;
const PREVIEW_T_DOWN: u8 = PREVIEW_HORIZONTAL | PREVIEW_SOUTH;
const PREVIEW_T_UP: u8 = PREVIEW_HORIZONTAL | PREVIEW_NORTH;
const PREVIEW_T_RIGHT: u8 = PREVIEW_VERTICAL | PREVIEW_EAST;
const PREVIEW_T_LEFT: u8 = PREVIEW_VERTICAL | PREVIEW_WEST;

fn preview_mask_index(width: u16, x: u16, y: u16) -> usize {
    usize::from(y) * usize::from(width) + usize::from(x)
}

fn add_preview_mask(masks: &mut [u8], bounds: Rect, x: u16, y: u16, mask: u8) {
    let rel_x = x.saturating_sub(bounds.x);
    let rel_y = y.saturating_sub(bounds.y);
    let idx = preview_mask_index(bounds.width, rel_x, rel_y);
    masks[idx] |= mask;
}

const fn preview_mask_char(mask: u8) -> Option<char> {
    match mask {
        0 => None,
        m if m == PREVIEW_NORTH || m == PREVIEW_SOUTH || m == PREVIEW_VERTICAL => Some('│'),
        m if m == PREVIEW_EAST || m == PREVIEW_WEST || m == PREVIEW_HORIZONTAL => Some('─'),
        PREVIEW_T_DOWN => Some('┬'),
        PREVIEW_T_UP => Some('┴'),
        PREVIEW_T_RIGHT => Some('├'),
        PREVIEW_T_LEFT => Some('┤'),
        _ => Some('┼'),
    }
}

fn render_preview_tree(
    tree: &SavedTileNode,
    bounds: Rect,
    area: Rect,
    line_masks: &mut [u8],
    leaves: &mut Vec<Rect>,
) {
    match tree {
        SavedTileNode::Leaf => leaves.push(area),
        SavedTileNode::Split {
            direction,
            ratio,
            first,
            second,
        } => match direction {
            SplitDirection::Vertical => {
                if area.width < 3 {
                    leaves.push(area);
                    return;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let first_w = ((f64::from(area.width) * ratio).round() as u16)
                    .clamp(1, area.width.saturating_sub(2));
                let split_col = area.x + first_w;
                for y in area.y..area.y + area.height {
                    let mut mask = 0;
                    if y > area.y {
                        mask |= PREVIEW_NORTH;
                    }
                    if y + 1 < area.y + area.height {
                        mask |= PREVIEW_SOUTH;
                    }
                    add_preview_mask(line_masks, bounds, split_col, y, mask);
                }
                if area.y > bounds.y {
                    add_preview_mask(line_masks, bounds, split_col, area.y - 1, PREVIEW_SOUTH);
                }
                if area.y + area.height < bounds.y + bounds.height {
                    add_preview_mask(
                        line_masks,
                        bounds,
                        split_col,
                        area.y + area.height,
                        PREVIEW_NORTH,
                    );
                }
                let first_rect = Rect::new(area.x, area.y, first_w, area.height);
                let second_rect =
                    Rect::new(split_col + 1, area.y, area.width - first_w - 1, area.height);
                render_preview_tree(first, bounds, first_rect, line_masks, leaves);
                render_preview_tree(second, bounds, second_rect, line_masks, leaves);
            }
            SplitDirection::Horizontal => {
                if area.height < 3 {
                    leaves.push(area);
                    return;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let first_h = ((f64::from(area.height) * ratio).round() as u16)
                    .clamp(1, area.height.saturating_sub(2));
                let split_row = area.y + first_h;
                for x in area.x..area.x + area.width {
                    let mut mask = 0;
                    if x > area.x {
                        mask |= PREVIEW_WEST;
                    }
                    if x + 1 < area.x + area.width {
                        mask |= PREVIEW_EAST;
                    }
                    add_preview_mask(line_masks, bounds, x, split_row, mask);
                }
                if area.x > bounds.x {
                    add_preview_mask(line_masks, bounds, area.x - 1, split_row, PREVIEW_EAST);
                }
                if area.x + area.width < bounds.x + bounds.width {
                    add_preview_mask(
                        line_masks,
                        bounds,
                        area.x + area.width,
                        split_row,
                        PREVIEW_WEST,
                    );
                }
                let first_rect = Rect::new(area.x, area.y, area.width, first_h);
                let second_rect =
                    Rect::new(area.x, split_row + 1, area.width, area.height - first_h - 1);
                render_preview_tree(first, bounds, first_rect, line_masks, leaves);
                render_preview_tree(second, bounds, second_rect, line_masks, leaves);
            }
        },
    }
}

fn layout_picker_footer(
    state: &LayoutPickerState,
    show_position: bool,
    max_width: usize,
) -> String {
    let candidates: &[&str] = match state.selected_entry().map(|entry| &entry.kind) {
        Some(LayoutPickerEntryKind::Saved(_)) => &[
            " type to filter · Enter apply · Alt+↑↓ reorder · ^R rename · ^D delete · ^S save · Esc",
            " type to filter · Enter apply · Alt+↑↓ · ^R/^D/^S · Esc",
            " Enter apply · ^R ^D ^S · Esc",
            " Enter · Esc",
        ],
        _ => &[
            " type to filter · Enter apply · ^S save · Esc cancel",
            " type to filter · Enter apply · ^S · Esc",
            " Enter apply · ^S · Esc",
            " Enter · Esc",
        ],
    };

    let mut footer = candidates
        .iter()
        .find(|candidate| candidate.chars().count() <= max_width)
        .map_or_else(
            || truncate_inline_text(candidates.last().copied().unwrap_or(""), max_width),
            |candidate| (*candidate).to_string(),
        );

    if show_position && !state.filtered_indices.is_empty() {
        let position = format!("  {}/{} ", state.selected + 1, state.filtered_indices.len());
        if footer.chars().count() + position.chars().count() <= max_width {
            footer.push_str(&position);
        }
    }

    footer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_entry(i: usize) -> LayoutPickerEntry {
        LayoutPickerEntry {
            label: format!("Layout {i}"),
            pane_count: i + 1,
            kind: LayoutPickerEntryKind::Preset(i),
            preview: SavedTileNode::Leaf,
            is_active: false,
        }
    }

    fn saved_entry(label: &str) -> LayoutPickerEntry {
        LayoutPickerEntry {
            label: label.to_string(),
            pane_count: 2,
            kind: LayoutPickerEntryKind::Saved(0),
            preview: SavedTileNode::Leaf,
            is_active: false,
        }
    }

    #[test]
    fn layout_picker_visible_range_keeps_selected_row_in_view() {
        let mut state = LayoutPickerState::new((0..8).map(preset_entry).collect());
        state.selected = 6;

        assert_eq!(state.visible_range(3), (4, 7));
    }

    #[test]
    fn layout_picker_footer_omits_rename_delete_for_presets() {
        let state = LayoutPickerState::new(vec![LayoutPickerEntry {
            label: "Single".to_string(),
            pane_count: 1,
            kind: LayoutPickerEntryKind::Preset(0),
            preview: SavedTileNode::Leaf,
            is_active: false,
        }]);

        let footer = layout_picker_footer(&state, false, 120);
        assert!(footer.contains("Enter apply"));
        assert!(!footer.contains("R rename"));
        assert!(!footer.contains("D delete"));
    }

    #[test]
    fn layout_picker_footer_includes_saved_actions_for_saved_layouts() {
        let state = LayoutPickerState::new(vec![saved_entry("Saved")]);

        let footer = layout_picker_footer(&state, false, 120);
        assert!(footer.contains("R rename"));
        assert!(footer.contains("D delete"));
    }

    #[test]
    fn layout_picker_footer_compacts_for_narrow_popups() {
        let state = LayoutPickerState::new(vec![saved_entry("Saved")]);

        let footer = layout_picker_footer(&state, false, 32);
        assert!(footer.contains("R/D") || footer.contains("Enter"));
        assert!(!footer.contains("D delete"));
        assert!(footer.chars().count() <= 32);
    }

    #[test]
    fn layout_picker_footer_never_exceeds_available_width() {
        let state = LayoutPickerState::new(vec![saved_entry("Saved")]);

        let footer = layout_picker_footer(&state, true, 12);
        assert!(footer.chars().count() <= 12, "footer was {footer:?}");
    }

    #[test]
    fn layout_picker_preview_numbers_every_leaf() {
        use crate::theme::Theme;

        // A 2×2 grid: [first = vertical split, second = vertical split] joined horizontally.
        let leaf = || Box::new(SavedTileNode::Leaf);
        let tree = SavedTileNode::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            first: Box::new(SavedTileNode::Split {
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                first: leaf(),
                second: leaf(),
            }),
            second: Box::new(SavedTileNode::Split {
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                first: leaf(),
                second: leaf(),
            }),
        };

        let area = Rect::new(0, 0, 30, 14);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        render_saved_layout_preview(area, &mut buf, &tree, &theme);

        let rendered: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect::<String>();
        assert!(rendered.contains('1'), "missing leaf label 1");
        assert!(rendered.contains('2'), "missing leaf label 2");
        assert!(rendered.contains('3'), "missing leaf label 3");
        assert!(rendered.contains('4'), "missing leaf label 4");
    }

    #[test]
    fn layout_picker_preview_noop_on_tiny_area() {
        use crate::theme::Theme;

        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        // Must not panic, must not draw into buf beyond its bounds.
        render_saved_layout_preview(area, &mut buf, &SavedTileNode::Leaf, &theme);
    }

    #[test]
    fn layout_picker_preview_single_leaf_leaves_interior_empty() {
        use crate::theme::Theme;

        let area = Rect::new(0, 0, 20, 8);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        render_saved_layout_preview(area, &mut buf, &SavedTileNode::Leaf, &theme);

        // Outer frame should be drawn.
        assert_eq!(buf[(0, 0)].symbol(), "┌");
        assert_eq!(buf[(area.width - 1, 0)].symbol(), "┐");
        assert_eq!(buf[(0, area.height - 1)].symbol(), "└");
        assert_eq!(buf[(area.width - 1, area.height - 1)].symbol(), "┘");
        // A "1" label should appear somewhere.
        let rendered: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect::<String>();
        assert!(rendered.contains('1'));
    }

    #[test]
    fn layout_picker_preview_draws_junctions_for_nested_splits() {
        use crate::theme::Theme;

        let tree = SavedTileNode::Split {
            direction: SplitDirection::Vertical,
            ratio: 0.5,
            first: Box::new(SavedTileNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: 0.5,
                first: Box::new(SavedTileNode::Leaf),
                second: Box::new(SavedTileNode::Leaf),
            }),
            second: Box::new(SavedTileNode::Leaf),
        };

        let area = Rect::new(0, 0, 24, 10);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        render_saved_layout_preview(area, &mut buf, &tree, &theme);

        let rendered: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect::<String>();
        assert!(
            rendered.contains('┤')
                || rendered.contains('├')
                || rendered.contains('┬')
                || rendered.contains('┴')
                || rendered.contains('┼')
        );
    }

    #[test]
    fn layout_picker_filter_narrows_entries_case_insensitive() {
        let mut state = LayoutPickerState::new(vec![
            LayoutPickerEntry {
                label: "Main Stack".to_string(),
                pane_count: 3,
                kind: LayoutPickerEntryKind::Saved(0),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
            LayoutPickerEntry {
                label: "Editor + Shell".to_string(),
                pane_count: 2,
                kind: LayoutPickerEntryKind::Saved(1),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
            LayoutPickerEntry {
                label: "Grid (2×2)".to_string(),
                pane_count: 4,
                kind: LayoutPickerEntryKind::Preset(3),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
        ]);
        assert_eq!(state.filtered_len(), 3);

        state.push_filter_char('M');
        state.push_filter_char('a');
        state.push_filter_char('i');
        state.push_filter_char('n');
        assert_eq!(state.filtered_len(), 1);
        assert_eq!(state.selected_entry().unwrap().label, "Main Stack");

        state.pop_filter_char();
        state.pop_filter_char();
        assert_eq!(state.filter, "Ma");
        assert_eq!(state.filtered_len(), 1);

        state.clear_filter();
        assert_eq!(state.filtered_len(), 3);
    }

    #[test]
    fn layout_picker_filter_keeps_selection_in_bounds() {
        let mut state = LayoutPickerState::new(vec![
            LayoutPickerEntry {
                label: "Alpha".to_string(),
                pane_count: 1,
                kind: LayoutPickerEntryKind::Preset(0),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
            LayoutPickerEntry {
                label: "Beta".to_string(),
                pane_count: 1,
                kind: LayoutPickerEntryKind::Preset(1),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
            LayoutPickerEntry {
                label: "Gamma".to_string(),
                pane_count: 1,
                kind: LayoutPickerEntryKind::Preset(2),
                preview: SavedTileNode::Leaf,
                is_active: false,
            },
        ]);
        state.select(2); // pick Gamma
        assert_eq!(state.selected_entry().unwrap().label, "Gamma");

        state.push_filter_char('a'); // matches Alpha, Beta, Gamma (all contain 'a')
        assert_eq!(state.filtered_len(), 3);

        state.push_filter_char('l'); // only "Alpha"
        assert_eq!(state.filtered_len(), 1);
        assert_eq!(state.selected_entry().unwrap().label, "Alpha");
    }

    #[test]
    fn layout_picker_select_entry_index_targets_filtered_row() {
        let mut state = LayoutPickerState::new(vec![
            saved_entry("Alpha"),
            saved_entry("Beta"),
            saved_entry("Gamma"),
        ]);

        state.set_filter("mm".to_string());
        state.select_entry_index(2);

        assert_eq!(state.filtered_len(), 1);
        assert_eq!(state.selected, 0);
        assert_eq!(state.selected_entry().unwrap().label, "Gamma");
    }

    #[test]
    fn layout_picker_page_move_clamps_within_bounds() {
        let mut state = LayoutPickerState::new((0..10).map(preset_entry).collect());
        state.select(8);
        state.move_page(1, 5);
        assert_eq!(state.selected, 9);
        state.move_page(-1, 20);
        assert_eq!(state.selected, 0);
    }
}
