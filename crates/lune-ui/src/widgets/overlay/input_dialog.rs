//! Inline text input dialog (new file, rename, etc.).

use std::path::PathBuf;

use crate::primitives::{Buffer, Line, Rect, Span, Style, Stylize, Widget};
use crate::runtime::terminal_layouts;
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

/// Action to perform when an input dialog is confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputDialogAction {
    /// Create a new file in the given parent directory.
    CreateFile { parent: PathBuf },
    /// Create a new directory in the given parent directory.
    CreateDir { parent: PathBuf },
    /// Rename an entry (from is the current path).
    Rename { from: PathBuf },
    /// Commit staged changes with the entered message.
    CommitMessage,
    /// Save the current agent layout under a name.
    SaveAgentLayout,
    /// Rename the selected saved agent layout.
    RenameAgentLayout { index: usize },
}

/// State for the inline input dialog overlay.
#[derive(Clone, Debug)]
pub struct InputDialogState {
    /// Dialog title (e.g. "New File", "Rename").
    pub title: String,
    /// Current input text.
    pub input: String,
    /// Cursor position within the input (byte offset).
    pub cursor_pos: usize,
    /// Hint text shown when input is empty.
    pub hint: String,
    /// The action to perform on confirm.
    pub action: InputDialogAction,
}

impl InputDialogState {
    /// Create a new input dialog state.
    pub fn new(
        title: impl Into<String>,
        hint: impl Into<String>,
        action: InputDialogAction,
    ) -> Self {
        Self {
            title: title.into(),
            input: String::new(),
            cursor_pos: 0,
            hint: hint.into(),
            action,
        }
    }

    /// Create with pre-filled input text (e.g. for rename).
    #[must_use]
    pub fn with_input(mut self, input: impl Into<String>) -> Self {
        self.input = input.into();
        self.cursor_pos = self.input.len();
        self
    }

    /// Type a character at the cursor position.
    pub fn type_char(&mut self, ch: char) {
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            // Find the previous character boundary.
            let prev = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            self.input.drain(prev..self.cursor_pos);
            self.cursor_pos = prev;
        }
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map_or(self.input.len(), |(i, _)| self.cursor_pos + i);
            self.input.drain(self.cursor_pos..next);
        }
    }

    /// Move cursor left by one character.
    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
        }
    }

    /// Move cursor right by one character.
    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map_or(self.input.len(), |(i, _)| self.cursor_pos + i);
        }
    }

    /// Move cursor to the start.
    pub const fn home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to the end.
    pub fn end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Validate the input. Returns an error message if invalid, None if OK.
    pub fn validate(&self) -> Option<&'static str> {
        let trimmed = self.input.trim();
        if matches!(
            self.action,
            InputDialogAction::SaveAgentLayout | InputDialogAction::RenameAgentLayout { .. }
        ) {
            return terminal_layouts::validate_layout_name(&self.input);
        }
        if trimmed.is_empty() {
            return Some("Input cannot be empty");
        }
        // Path separator check only applies to file/dir operations.
        if !matches!(self.action, InputDialogAction::CommitMessage)
            && (trimmed.contains('/') || trimmed.contains('\\'))
        {
            return Some("Name cannot contain path separators");
        }
        None
    }
}

pub(crate) fn render_input_dialog(
    area: Rect,
    buf: &mut Buffer,
    state: &InputDialogState,
    theme: &Theme,
) {
    let popup_w = (area.width * 50 / 100).max(30).min(area.width);
    let popup_h: u16 = 5;
    let title = format!(" {} ", state.title);
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(&title)
        .size_cells(popup_w, popup_h)
        .anchor(Anchor::Top {
            margin: (area.height.saturating_sub(popup_h)) / 3,
        })
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Input line with block cursor.
    let display_text = if state.input.is_empty() {
        Span::from(state.hint.as_str()).dim()
    } else {
        Span::from(state.input.as_str())
    };

    let input_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1);
    Line::from(display_text).render(input_area, buf);

    // Draw block cursor.
    {
        let cursor_x = inner.x
            + 1
            + u16::try_from(state.input[..state.cursor_pos].chars().count()).unwrap_or(u16::MAX);
        if cursor_x < inner.x + inner.width.saturating_sub(1) {
            let cursor_char = state.input[state.cursor_pos..]
                .chars()
                .next()
                .unwrap_or(' ');
            let cursor_span = Span::styled(
                cursor_char.to_string(),
                Style::new().fg(theme.bg).bg(theme.fg),
            );
            Line::from(cursor_span).render(Rect::new(cursor_x, inner.y, 1, 1), buf);
        }
    }

    // Validation error or hint.
    if inner.height > 1 {
        if let Some(err) = state.validate() {
            if !state.input.is_empty() {
                Line::from(Span::from(err).fg(theme.notif_error)).render(
                    Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1),
                    buf,
                );
            }
        }
    }

    // Footer hint.
    let footer_y = inner.y + inner.height.saturating_sub(1);
    if footer_y > inner.y {
        Line::from(Span::from(" Enter confirm · Esc cancel").dim())
            .render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::{OverlayKind, OverlayState};
    #[test]
    fn input_dialog_type_and_backspace() {
        let mut d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        d.type_char('h');
        d.type_char('e');
        d.type_char('l');
        assert_eq!(d.input, "hel");
        assert_eq!(d.cursor_pos, 3);
        d.backspace();
        assert_eq!(d.input, "he");
        assert_eq!(d.cursor_pos, 2);
    }

    #[test]
    fn input_dialog_cursor_movement() {
        let mut d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        d.type_char('a');
        d.type_char('b');
        d.type_char('c');
        d.home();
        assert_eq!(d.cursor_pos, 0);
        d.move_right();
        assert_eq!(d.cursor_pos, 1);
        d.end();
        assert_eq!(d.cursor_pos, 3);
        d.move_left();
        assert_eq!(d.cursor_pos, 2);
    }

    #[test]
    fn input_dialog_delete() {
        let mut d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        d.type_char('a');
        d.type_char('b');
        d.type_char('c');
        d.home();
        d.delete();
        assert_eq!(d.input, "bc");
        assert_eq!(d.cursor_pos, 0);
    }

    #[test]
    fn input_dialog_validate_empty() {
        let d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        assert!(d.validate().is_some());
    }

    #[test]
    fn input_dialog_validate_path_separator() {
        let mut d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        d.type_char('a');
        d.type_char('/');
        d.type_char('b');
        assert!(d.validate().is_some());
    }

    #[test]
    fn input_dialog_validate_ok() {
        let mut d = InputDialogState::new(
            "Test",
            "hint",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        d.type_char('f');
        d.type_char('o');
        d.type_char('o');
        assert!(d.validate().is_none());
    }

    #[test]
    fn input_dialog_with_input_prefill() {
        let d = InputDialogState::new(
            "Rename",
            "new name",
            InputDialogAction::Rename {
                from: PathBuf::from("/old"),
            },
        )
        .with_input("old_name.txt");
        assert_eq!(d.input, "old_name.txt");
        assert_eq!(d.cursor_pos, 12);
    }

    #[test]
    fn overlay_open_input_dialog() {
        let mut overlay = OverlayState::default();
        let state = InputDialogState::new(
            "New File",
            "filename",
            InputDialogAction::CreateFile {
                parent: PathBuf::from("/tmp"),
            },
        );
        overlay.open_input_dialog(state);
        assert!(overlay.is_active());
        assert!(matches!(overlay.active, Some(OverlayKind::InputDialog)));
        assert!(overlay.input_dialog.is_some());
    }
}
