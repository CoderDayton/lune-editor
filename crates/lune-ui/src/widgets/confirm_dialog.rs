//! Reusable confirm/cancel modal — the first concrete user of
//! [`crate::widgets::modal::Modal`].
//!
//! Drop-in for unsaved-changes prompts, destructive-action guards
//! (delete, overwrite), and any yes/no decision. Default selection is
//! `Cancel` so an accidental Enter doesn't fire a destructive action.
//!
//! ```no_run
//! use lune_ui::widgets::confirm_dialog::{ConfirmChoice, ConfirmDialogState};
//! let mut dlg = ConfirmDialogState::new("Discard changes?", "Unsaved edits will be lost.")
//!     .destructive(true);
//! dlg.open();
//! // ... drive via handle_key() each frame; render via dlg.render(area, buf, theme).
//! ```

#[cfg(test)]
use crate::primitives::KeyModifiers;
use crate::primitives::{Buffer, KeyCode, KeyEvent, Line, Modifier, Rect, Span, Style, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Modal, ModalState};
use unicode_width::UnicodeWidthStr;

/// Outcome of a confirm dialog interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmChoice {
    Confirm,
    Cancel,
}

/// Confirm-dialog state. Owns the underlying [`ModalState`] so the
/// caller drives the whole component via this struct.
#[derive(Debug, Clone)]
pub struct ConfirmDialogState {
    modal: ModalState,
    title: String,
    message: String,
    confirm_label: String,
    cancel_label: String,
    selected: ConfirmChoice,
    destructive: bool,
}

impl ConfirmDialogState {
    /// New closed dialog with default labels (`Confirm` / `Cancel`)
    /// and `Cancel` selected. Open it with [`Self::open`].
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            modal: ModalState::new(),
            title: title.into(),
            message: message.into(),
            confirm_label: "Confirm".to_string(),
            cancel_label: "Cancel".to_string(),
            selected: ConfirmChoice::Cancel,
            destructive: false,
        }
    }

    /// Override the Confirm button label (e.g. `Delete`, `Overwrite`).
    #[must_use]
    pub fn confirm_label(mut self, label: impl Into<String>) -> Self {
        self.confirm_label = label.into();
        self
    }

    /// Override the Cancel button label (e.g. `Keep`, `Back`).
    #[must_use]
    pub fn cancel_label(mut self, label: impl Into<String>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Mark as destructive: red border, red-highlighted confirm button.
    #[must_use]
    pub const fn destructive(mut self, on: bool) -> Self {
        self.destructive = on;
        self
    }

    /// Make Confirm the initial selection (default is Cancel).
    #[must_use]
    pub const fn default_confirm(mut self) -> Self {
        self.selected = ConfirmChoice::Confirm;
        self
    }

    pub const fn open(&mut self) {
        self.modal.open();
    }

    pub const fn close(&mut self) {
        self.modal.close();
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    #[must_use]
    pub const fn selected(&self) -> ConfirmChoice {
        self.selected
    }

    /// Dispatch a key. Returns `Some(choice)` when the dialog resolves
    /// (and closes itself), `None` when the key only moved the cursor
    /// or wasn't recognized.
    pub const fn handle_key(&mut self, key: &KeyEvent) -> Option<ConfirmChoice> {
        if !self.is_open() {
            return None;
        }
        if !key.modifiers.is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Char('y' | 'Y') => Some(self.resolve(ConfirmChoice::Confirm)),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(self.resolve(ConfirmChoice::Cancel)),
            KeyCode::Enter | KeyCode::Char(' ') => Some(self.resolve(self.selected)),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l') => {
                self.toggle();
                None
            }
            _ => None,
        }
    }

    const fn toggle(&mut self) {
        self.selected = match self.selected {
            ConfirmChoice::Confirm => ConfirmChoice::Cancel,
            ConfirmChoice::Cancel => ConfirmChoice::Confirm,
        };
    }

    const fn resolve(&mut self, choice: ConfirmChoice) -> ConfirmChoice {
        self.selected = choice;
        self.close();
        choice
    }

    /// Render the dialog centered in `area`. No-op when closed.
    #[allow(clippy::cast_possible_truncation)]
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if !self.is_open() {
            return;
        }

        let (border_style, title_style) = if self.destructive {
            (
                Style::new().fg(theme.notif_error),
                Style::new()
                    .fg(theme.notif_error)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                Style::new().fg(theme.accent),
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            )
        };

        // Size the modal to fit the message (clamped). 4 inner rows is
        // the minimum: message + blank + buttons + blank padding row
        // at top, plus we add a row for the title border.
        let msg_w = UnicodeWidthStr::width(self.message.as_str()) as u16;
        let want_w = msg_w.max(self.button_row_width()).saturating_add(6);
        let want_w = want_w.max(40).min(area.width.saturating_sub(2));
        let want_h: u16 = 7;

        let title = format!(" {} ", self.title);
        let selected = self.selected;
        let destructive = self.destructive;
        let message = self.message.clone();
        let confirm_label = self.confirm_label.clone();
        let cancel_label = self.cancel_label.clone();

        Modal::new(theme)
            .title(&title)
            .size_cells(want_w, want_h)
            .border_style(border_style)
            .title_style(title_style)
            .footer(" Enter confirm · Esc cancel ")
            .render(area, buf, &mut self.modal, |inner, buf| {
                render_body(
                    inner,
                    buf,
                    theme,
                    &message,
                    &confirm_label,
                    &cancel_label,
                    selected,
                    destructive,
                );
            });
    }

    #[allow(clippy::cast_possible_truncation)]
    fn button_row_width(&self) -> u16 {
        // Each button is `[ label ]` (4 chrome cells + label chars),
        // separated by 2 spaces.
        let confirm_w = self.confirm_label.chars().count() as u16 + 4;
        let cancel_w = self.cancel_label.chars().count() as u16 + 4;
        confirm_w + 2 + cancel_w
    }
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    inner: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    message: &str,
    confirm_label: &str,
    cancel_label: &str,
    selected: ConfirmChoice,
    destructive: bool,
) {
    // Message: first line of `message`, truncated to inner width.
    if inner.height >= 1 {
        let line = message.lines().next().unwrap_or("");
        let truncated = truncate_chars(line, inner.width as usize);
        Line::from(Span::styled(truncated, Style::new().fg(theme.fg)))
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    }

    // Second message line (optional).
    if inner.height >= 2 {
        let line = message.lines().nth(1).unwrap_or("");
        if !line.is_empty() {
            let truncated = truncate_chars(line, inner.width as usize);
            Line::from(Span::styled(truncated, Style::new().fg(theme.fg_muted)))
                .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
        }
    }

    // Buttons on the last row of the inner area.
    if inner.height >= 1 {
        let button_y = inner.y + inner.height - 1;
        render_buttons(
            Rect::new(inner.x, button_y, inner.width, 1),
            buf,
            theme,
            confirm_label,
            cancel_label,
            selected,
            destructive,
        );
    }
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn render_buttons(
    row: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    confirm_label: &str,
    cancel_label: &str,
    selected: ConfirmChoice,
    destructive: bool,
) {
    let confirm_text = format!("[ {confirm_label} ]");
    let cancel_text = format!("[ {cancel_label} ]");
    let confirm_w = confirm_text.chars().count() as u16;
    let cancel_w = cancel_text.chars().count() as u16;
    let gap: u16 = 2;
    let total = confirm_w + gap + cancel_w;
    if total > row.width {
        return; // not enough room to render either button cleanly
    }
    let start_x = row.x + (row.width - total) / 2;

    let confirm_style = button_style(
        theme,
        selected == ConfirmChoice::Confirm,
        destructive,
        /* is_destructive_action = */ true,
    );
    let cancel_style = button_style(
        theme,
        selected == ConfirmChoice::Cancel,
        destructive,
        /* is_destructive_action = */ false,
    );

    Line::from(Span::styled(confirm_text, confirm_style))
        .render(Rect::new(start_x, row.y, confirm_w, 1), buf);
    Line::from(Span::styled(cancel_text, cancel_style)).render(
        Rect::new(start_x + confirm_w + gap, row.y, cancel_w, 1),
        buf,
    );
}

#[allow(clippy::fn_params_excessive_bools)]
const fn button_style(
    theme: &Theme,
    is_selected: bool,
    destructive: bool,
    is_destructive_action: bool,
) -> Style {
    if is_selected {
        let bg = if destructive && is_destructive_action {
            theme.notif_error
        } else {
            theme.accent
        };
        Style::new()
            .fg(theme.bg)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.fg_muted)
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn defaults_to_cancel_selection() {
        let dlg = ConfirmDialogState::new("title", "msg");
        assert_eq!(dlg.selected(), ConfirmChoice::Cancel);
    }

    #[test]
    fn default_confirm_sets_initial_selection() {
        let dlg = ConfirmDialogState::new("t", "m").default_confirm();
        assert_eq!(dlg.selected(), ConfirmChoice::Confirm);
    }

    #[test]
    fn handle_key_ignored_when_closed() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        // not opened
        let result = dlg.handle_key(&key(KeyCode::Char('y')));
        assert!(result.is_none());
    }

    #[test]
    fn y_resolves_to_confirm_and_closes() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        let result = dlg.handle_key(&key(KeyCode::Char('y')));
        assert_eq!(result, Some(ConfirmChoice::Confirm));
        assert!(!dlg.is_open(), "dialog must close after resolving");
    }

    #[test]
    fn n_resolves_to_cancel_and_closes() {
        let mut dlg = ConfirmDialogState::new("t", "m").default_confirm();
        dlg.open();
        let result = dlg.handle_key(&key(KeyCode::Char('n')));
        assert_eq!(result, Some(ConfirmChoice::Cancel));
        assert!(!dlg.is_open());
    }

    #[test]
    fn esc_resolves_to_cancel() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        assert_eq!(
            dlg.handle_key(&key(KeyCode::Esc)),
            Some(ConfirmChoice::Cancel)
        );
    }

    #[test]
    fn enter_resolves_to_selected_choice() {
        let mut dlg = ConfirmDialogState::new("t", "m").default_confirm();
        dlg.open();
        assert_eq!(
            dlg.handle_key(&key(KeyCode::Enter)),
            Some(ConfirmChoice::Confirm)
        );
    }

    #[test]
    fn tab_toggles_selection_without_resolving() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        assert_eq!(dlg.selected(), ConfirmChoice::Cancel);
        assert!(dlg.handle_key(&key(KeyCode::Tab)).is_none());
        assert_eq!(dlg.selected(), ConfirmChoice::Confirm);
        assert!(dlg.handle_key(&key(KeyCode::Tab)).is_none());
        assert_eq!(dlg.selected(), ConfirmChoice::Cancel);
        assert!(dlg.is_open(), "toggle must not close the dialog");
    }

    #[test]
    fn arrow_keys_toggle_selection() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        dlg.handle_key(&key(KeyCode::Right));
        assert_eq!(dlg.selected(), ConfirmChoice::Confirm);
        dlg.handle_key(&key(KeyCode::Left));
        assert_eq!(dlg.selected(), ConfirmChoice::Cancel);
    }

    #[test]
    fn modified_keys_are_ignored() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        let evt = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert!(dlg.handle_key(&evt).is_none());
        assert!(dlg.is_open());
    }

    #[test]
    fn unrelated_keys_return_none_and_keep_state() {
        let mut dlg = ConfirmDialogState::new("t", "m");
        dlg.open();
        let before = dlg.selected();
        assert!(dlg.handle_key(&key(KeyCode::Char('a'))).is_none());
        assert_eq!(dlg.selected(), before);
        assert!(dlg.is_open());
    }

    #[test]
    fn render_paints_inner_area_and_button_row_when_open() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let mut dlg = ConfirmDialogState::new("Discard changes?", "Unsaved edits will be lost.");
        dlg.open();
        dlg.render(area, &mut buf, &theme);

        // The dialog defaults to Cancel selected; that button gets the
        // accent bg. Probe the bottom-of-inner row for a cell with the
        // accent background.
        let inner_y_bottom_min = area.y + 1;
        let inner_y_bottom_max = area.y + area.height - 2;
        let mut found_accent_button = false;
        'outer: for y in inner_y_bottom_min..=inner_y_bottom_max {
            for x in area.x..area.x + area.width {
                if buf.cell((x, y)).and_then(|c| c.style().bg) == Some(theme.accent) {
                    found_accent_button = true;
                    break 'outer;
                }
            }
        }
        assert!(
            found_accent_button,
            "expected the selected (Cancel) button to use the accent bg",
        );
    }

    #[test]
    fn destructive_render_uses_error_color_on_confirm_selection() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let mut dlg = ConfirmDialogState::new("Delete?", "This cannot be undone.")
            .destructive(true)
            .default_confirm();
        dlg.open();
        dlg.render(area, &mut buf, &theme);

        let mut found_error = false;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if buf.cell((x, y)).and_then(|c| c.style().bg) == Some(theme.notif_error) {
                    found_error = true;
                    break;
                }
            }
        }
        assert!(
            found_error,
            "destructive + Confirm selected must paint a notif_error cell (button bg)",
        );
    }

    #[test]
    fn render_is_a_no_op_when_closed() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        // Pre-fill so any modal paint would be visible.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol("z");
            }
        }
        let mut dlg = ConfirmDialogState::new("t", "m");
        // not opened
        dlg.render(area, &mut buf, &theme);
        assert_eq!(
            buf.cell((0, 0)).map(|c| c.symbol().to_string()),
            Some("z".into())
        );
    }
}
