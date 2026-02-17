//! Tab bar widget.
//!
//! Renders a horizontal row of tabs showing open buffers, with active tab
//! highlighting, dirty indicators, and close buttons. Supports overflow
//! with scroll indicators and mouse click handling.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Style, Stylize};
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;

use lune_core::prelude::*;

use super::FOCUS_ACCENT;

// ── Tab entry ─────────────────────────────────────────────────────────

/// A single tab in the tab bar.
#[derive(Clone, Debug)]
pub struct TabEntry {
    /// The buffer ID this tab represents.
    pub buffer_id: BufferId,
    /// Display title (filename or "Untitled").
    pub title: String,
    /// Whether the buffer has unsaved changes.
    pub dirty: bool,
    /// Whether the tab is pinned (pinned tabs can't be closed easily).
    pub pinned: bool,
}

// ── Tab manager ───────────────────────────────────────────────────────

/// Manages the list of open tabs and scroll offset.
#[derive(Clone, Debug, Default)]
pub struct TabManager {
    /// Ordered list of tabs.
    pub tabs: Vec<TabEntry>,
    /// The index of the currently active tab.
    pub active_index: usize,
    /// Horizontal scroll offset (first visible tab index) for overflow.
    pub scroll_offset: usize,
}

impl TabManager {
    /// Create a new empty tab manager.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            scroll_offset: 0,
        }
    }

    /// Sync tabs from the buffer registry and tab order.
    pub fn sync_from_registry(
        &mut self,
        tab_ids: &[BufferId],
        active: Option<BufferId>,
        registry: &BufferRegistry,
    ) {
        self.tabs.clear();
        for &id in tab_ids {
            if let Some(buf) = registry.get(id) {
                let title = buf
                    .file_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or_else(
                        || "Untitled".to_string(),
                        |n| n.to_string_lossy().to_string(),
                    );
                self.tabs.push(TabEntry {
                    buffer_id: id,
                    title,
                    dirty: buf.is_dirty(),
                    pinned: false,
                });
            }
        }
        self.active_index = active
            .and_then(|aid| self.tabs.iter().position(|t| t.buffer_id == aid))
            .unwrap_or(0);

        // Ensure active tab is visible.
        self.ensure_active_visible();
    }

    /// Ensure the active tab is within the visible scroll range.
    const fn ensure_active_visible(&mut self) {
        if self.active_index < self.scroll_offset {
            self.scroll_offset = self.active_index;
        }
        // We can't pre-compute how many tabs fit without knowing the area
        // width, but we at least ensure scroll_offset <= active_index.
    }

    /// Get the buffer ID at the given tab index, if any.
    #[must_use]
    pub fn buffer_at(&self, index: usize) -> Option<BufferId> {
        self.tabs.get(index).map(|t| t.buffer_id)
    }

    /// Find the tab index at a given screen x-coordinate within the tab bar.
    /// Returns `Some((index, is_close_button))`.
    #[must_use]
    pub fn hit_test(&self, x: u16, area_x: u16, area_width: u16) -> Option<(usize, bool)> {
        let mut cx = area_x;
        let max_x = area_x + area_width;

        // Account for left scroll indicator.
        if self.scroll_offset > 0 {
            if x < cx + 2 {
                // Clicked on the left scroll indicator.
                return None;
            }
            cx += 2;
        }

        for (i, tab) in self.tabs.iter().enumerate().skip(self.scroll_offset) {
            if cx >= max_x {
                break;
            }

            let label_len = Self::tab_label_width(tab);
            let tab_end = cx + label_len;

            if x >= cx && x < tab_end {
                // Check if click is on the close "x" area (last 2 chars before separator).
                let is_close = x >= tab_end.saturating_sub(3) && x < tab_end.saturating_sub(1);
                return Some((i, is_close));
            }

            cx = tab_end + 1; // +1 for separator
        }

        None
    }

    /// Compute the display width of a tab label.
    #[allow(clippy::cast_possible_truncation)]
    fn tab_label_width(tab: &TabEntry) -> u16 {
        // " filename [+] x " or " filename x "
        let base = tab.title.len() + 2; // " filename "
        let dirty = if tab.dirty { 4 } else { 0 }; // "[+] "
        let close = 2; // "x "
        (base + dirty + close) as u16
    }
}

// ── Rendering ─────────────────────────────────────────────────────────

/// Render the tab bar into the given area.
///
/// When `is_focused` is true, the active tab uses the accent color to
/// indicate the editor pane has focus.
#[allow(clippy::cast_possible_truncation)]
pub fn render_tab_bar(area: Rect, buf: &mut Buffer, tab_mgr: &TabManager, is_focused: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    if tab_mgr.tabs.is_empty() {
        Line::from(Span::from(" No open files ").dim()).render(area, buf);
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    let mut used_width: u16 = 0;
    let max_width = area.width;

    // Left scroll indicator.
    let has_left_overflow = tab_mgr.scroll_offset > 0;
    if has_left_overflow {
        spans.push(Span::from("◄ ").dim());
        used_width += 2;
    }

    let mut last_visible_idx = tab_mgr.tabs.len();

    for (i, tab) in tab_mgr.tabs.iter().enumerate().skip(tab_mgr.scroll_offset) {
        let is_active = i == tab_mgr.active_index;

        let dirty_mark = if tab.dirty { " [+]" } else { "" };
        let label = format!(" {}{dirty_mark} x ", tab.title);
        let label_width = label.len() as u16;

        // Check if this tab fits. Reserve 2 chars for right overflow indicator.
        let reserve = if i + 1 < tab_mgr.tabs.len() { 2 } else { 0 };
        if used_width + label_width + 1 + reserve > max_width {
            last_visible_idx = i;
            break;
        }

        if is_active {
            if is_focused {
                spans.push(
                    Span::from(label)
                        .bold()
                        .style(Style::new().fg(FOCUS_ACCENT)),
                );
            } else {
                spans.push(Span::from(label).bold().reversed());
            }
        } else {
            spans.push(Span::from(label).dim());
        }

        used_width += label_width;

        // Separator.
        if i + 1 < tab_mgr.tabs.len() {
            spans.push(Span::from("│").dim());
            used_width += 1;
        }
    }

    // Right scroll indicator.
    if last_visible_idx < tab_mgr.tabs.len() {
        spans.push(Span::from(" ►").dim());
    }

    Line::from(spans).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tab_manager() {
        let mgr = TabManager::new();
        assert!(mgr.tabs.is_empty());
        assert_eq!(mgr.active_index, 0);
    }

    #[test]
    fn hit_test_basic() {
        let mut mgr = TabManager::new();
        mgr.tabs.push(TabEntry {
            buffer_id: BufferId::new(),
            title: "main.rs".to_string(),
            dirty: false,
            pinned: false,
        });
        mgr.tabs.push(TabEntry {
            buffer_id: BufferId::new(),
            title: "lib.rs".to_string(),
            dirty: false,
            pinned: false,
        });
        mgr.active_index = 0;

        // First tab starts at x=0, label " main.rs x " = 12 chars.
        let result = mgr.hit_test(0, 0, 80);
        assert!(result.is_some());
        let (idx, _is_close) = result.unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn tab_label_width_dirty() {
        let tab = TabEntry {
            buffer_id: BufferId::new(),
            title: "test.rs".to_string(),
            dirty: true,
            pinned: false,
        };
        // " test.rs " = 9, "[+] " = 4, "x " = 2 => 15
        assert_eq!(TabManager::tab_label_width(&tab), 15);
    }

    #[test]
    fn tab_label_width_clean() {
        let tab = TabEntry {
            buffer_id: BufferId::new(),
            title: "test.rs".to_string(),
            dirty: false,
            pinned: false,
        };
        // " test.rs " = 9, "x " = 2 => 11
        assert_eq!(TabManager::tab_label_width(&tab), 11);
    }
}
