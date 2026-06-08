//! Keybinding hints overlay (which-key style cheatsheet).

use unicode_width::UnicodeWidthStr;

use crate::primitives::{Buffer, Line, Rect, Span, Style, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

use super::util::pop_grapheme_returning;

/// One row in the keybinding cheatsheet.
#[derive(Clone, Debug)]
pub struct KeyHintEntry {
    /// Display key (e.g. `"Ctrl+S"`, `"Alt+\\"`).
    pub key: &'static str,
    /// Human-readable action label.
    pub label: &'static str,
}

/// A group of related bindings (file, edit, tabs, panels, …).
#[derive(Clone, Debug)]
pub struct KeyHintGroup {
    pub title: &'static str,
    pub entries: &'static [KeyHintEntry],
}

/// State for the keybinding hints overlay.
#[derive(Clone, Debug, Default)]
pub struct KeyHintsState {
    /// Live-typed substring filter. Matches against `key` or `label`.
    pub filter: String,
    /// Vertical scroll offset (in display rows).
    pub scroll: u16,
}

impl KeyHintsState {
    pub const fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }
    pub const fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }
    pub fn push_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.scroll = 0;
    }
    pub fn pop_filter(&mut self) -> bool {
        let popped = pop_grapheme_returning(&mut self.filter);
        if popped {
            self.scroll = 0;
        }
        popped
    }
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.scroll = 0;
    }
}

/// Curated cheatsheet, mirroring `Keymap::default_global`.
///
/// Update both lists when adding bindings — the cheatsheet is the
/// user-visible reference, the keymap is the source of truth at
/// runtime, and they intentionally drift only when a binding has no
/// useful natural-language label.
pub const KEY_HINT_GROUPS: &[KeyHintGroup] = &[
    KeyHintGroup {
        title: "File",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+S",
                label: "Save",
            },
            KeyHintEntry {
                key: "Ctrl+K S",
                label: "Save all",
            },
            KeyHintEntry {
                key: "Ctrl+O",
                label: "Open file picker",
            },
            KeyHintEntry {
                key: "Ctrl+N",
                label: "New file",
            },
        ],
    },
    KeyHintGroup {
        title: "Tabs",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+W",
                label: "Close tab",
            },
            KeyHintEntry {
                key: "Ctrl+Tab",
                label: "Next tab",
            },
            KeyHintEntry {
                key: "Ctrl+Shift+Tab",
                label: "Previous tab",
            },
            KeyHintEntry {
                key: "Ctrl+1",
                label: "Show Editor tab",
            },
            KeyHintEntry {
                key: "Ctrl+2",
                label: "Show Agents tab",
            },
            KeyHintEntry {
                key: "Ctrl+`",
                label: "Toggle Agents tab",
            },
        ],
    },
    KeyHintGroup {
        title: "Editor",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+Z",
                label: "Undo",
            },
            KeyHintEntry {
                key: "Ctrl+Y",
                label: "Redo",
            },
            KeyHintEntry {
                key: "Ctrl+F",
                label: "Find",
            },
            KeyHintEntry {
                key: "Ctrl+H",
                label: "Find & replace",
            },
        ],
    },
    KeyHintGroup {
        title: "Panels",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+B",
                label: "Toggle file tree",
            },
            KeyHintEntry {
                key: "Ctrl+G",
                label: "Toggle git panel",
            },
        ],
    },
    KeyHintGroup {
        title: "Pickers / overlays",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+P",
                label: "Command palette",
            },
            KeyHintEntry {
                key: "Ctrl+L",
                label: "Language picker",
            },
            KeyHintEntry {
                key: "Ctrl+T",
                label: "Theme picker",
            },
            KeyHintEntry {
                key: "Ctrl+K M",
                label: "Markdown preview",
            },
            KeyHintEntry {
                key: "F1 / ?",
                label: "This help",
            },
        ],
    },
    KeyHintGroup {
        title: "AI",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+K A",
                label: "Ask about selection",
            },
            KeyHintEntry {
                key: "Ctrl+K R",
                label: "Refactor file",
            },
            KeyHintEntry {
                key: "Ctrl+K C",
                label: "Summarize changes",
            },
            KeyHintEntry {
                key: "Ctrl+K W",
                label: "Close AI session",
            },
            KeyHintEntry {
                key: "Ctrl+]",
                label: "Next AI session",
            },
            KeyHintEntry {
                key: "Ctrl+[",
                label: "Previous AI session",
            },
        ],
    },
    KeyHintGroup {
        title: "Agents pane",
        entries: &[
            KeyHintEntry {
                key: "Alt+\\",
                label: "Split vertical",
            },
            KeyHintEntry {
                key: "Alt+-",
                label: "Split horizontal",
            },
            KeyHintEntry {
                key: "Alt+X",
                label: "Close pane",
            },
            KeyHintEntry {
                key: "Alt+J / Alt+K",
                label: "Focus next / prev",
            },
            KeyHintEntry {
                key: "Alt+Z",
                label: "Toggle zoom",
            },
            KeyHintEntry {
                key: "Alt+,",
                label: "Apply layout",
            },
        ],
    },
    KeyHintGroup {
        title: "Notifications & mode",
        entries: &[
            KeyHintEntry {
                key: "Ctrl+K N",
                label: "Dismiss notifications",
            },
            KeyHintEntry {
                key: "Ctrl+Alt+V",
                label: "Toggle vim mode",
            },
            KeyHintEntry {
                key: "Ctrl+Q",
                label: "Quit",
            },
        ],
    },
];

pub(crate) fn render_key_hints(area: Rect, buf: &mut Buffer, state: &KeyHintsState, theme: &Theme) {
    use crate::primitives::Paragraph;

    let w = area.width.saturating_mul(8) / 10;
    let h = area.height.saturating_mul(8) / 10;
    if w < 30 || h < 8 {
        return;
    }
    let title = if state.filter.is_empty() {
        " Keybindings  (Esc/F1 close · type to filter) ".to_string()
    } else {
        format!(" Keybindings  filter: {} ", state.filter)
    };
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(&title)
        .title_style(Style::new().fg(theme.fg).bold())
        .border_style(Style::new().fg(theme.fg_muted))
        .size_cells(w, h)
        .anchor(Anchor::Center)
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    let needle = state.filter.to_lowercase();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for group in KEY_HINT_GROUPS {
        let matched: Vec<&KeyHintEntry> = group
            .entries
            .iter()
            .filter(|e| {
                needle.is_empty()
                    || e.key.to_lowercase().contains(&needle)
                    || e.label.to_lowercase().contains(&needle)
            })
            .collect();
        if matched.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(
            group.title.to_string(),
            Style::new().fg(theme.accent).bold(),
        )));
        for e in matched {
            // Pad in terminal columns, not Unicode scalars, so a future
            // non-ASCII key glyph wouldn't quietly misalign the column.
            // `format!("{:<16}")` counts scalars; we count display width.
            const KEY_COL_WIDTH: usize = 16;
            let key_cols = UnicodeWidthStr::width(e.key);
            let pad = KEY_COL_WIDTH.saturating_sub(key_cols);
            let mut key_cell = String::with_capacity(2 + e.key.len() + pad);
            key_cell.push_str("  ");
            key_cell.push_str(e.key);
            for _ in 0..pad {
                key_cell.push(' ');
            }
            lines.push(Line::from(vec![
                Span::styled(key_cell, Style::new().fg(theme.fg).bold()),
                Span::raw(" "),
                Span::styled(e.label.to_string(), Style::new().fg(theme.fg_muted)),
            ]));
        }
        lines.push(Line::raw(""));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches".to_string(),
            Style::new().fg(theme.fg_muted),
        )));
    }

    Paragraph::new(lines)
        .scroll((state.scroll, 0))
        .style(Style::new().fg(theme.fg).bg(theme.bg))
        .render(inner, buf);
}
