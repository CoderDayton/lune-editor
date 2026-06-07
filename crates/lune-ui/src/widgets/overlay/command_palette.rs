//! Command palette overlay — fuzzy-filtered command search.

use crate::event::AppCommand;
use crate::primitives::{Buffer, Line, Rect, Span, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};

use super::util::render_hrule;

/// A command that can appear in the command palette.
#[derive(Clone, Debug)]
pub struct PaletteCommand {
    /// Display name.
    pub label: String,
    /// Pre-computed lowercase label for filtering.
    label_lower: String,
    /// The command to execute.
    pub command: AppCommand,
}

/// State for the command palette overlay.
#[derive(Clone, Debug, Default)]
pub struct CommandPaletteState {
    /// User's search input.
    pub input: String,
    /// Index of the currently selected command.
    pub selected: usize,
    /// Scroll offset for the visible list window.
    pub scroll_offset: usize,
    /// Filtered list of commands matching the input.
    pub filtered_commands: Vec<PaletteCommand>,
    /// Cached full command list (built once, reused across filter calls).
    all_commands: Vec<PaletteCommand>,
}

impl CommandPaletteState {
    /// Ensure the cached command list is populated.
    pub(crate) fn ensure_commands_cached(&mut self) {
        if self.all_commands.is_empty() {
            self.all_commands = all_palette_commands();
        }
    }

    /// Update the filtered command list based on current input.
    pub fn update_filter(&mut self) {
        self.ensure_commands_cached();
        let query = self.input.to_lowercase();

        // Reuse the existing Vec allocation where possible.
        self.filtered_commands.clear();
        if query.is_empty() {
            self.filtered_commands
                .extend(self.all_commands.iter().cloned());
        } else {
            self.filtered_commands.extend(
                self.all_commands
                    .iter()
                    .filter(|cmd| cmd.label_lower.contains(&query))
                    .cloned(),
            );
        }

        // Clamp selection.
        if self.filtered_commands.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered_commands.len() - 1);
        }
        self.scroll_offset = self.scroll_offset.min(self.selected);
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if !self.filtered_commands.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered_commands.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.filtered_commands.is_empty() {
            self.selected = (self.selected + 1) % self.filtered_commands.len();
        }
    }

    /// Adjust `scroll_offset` so `selected` is visible within `visible_rows`.
    pub const fn ensure_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected + 1 - visible_rows;
        }
    }

    /// Get the currently selected command.
    #[must_use]
    pub fn selected_command(&self) -> Option<&AppCommand> {
        self.filtered_commands
            .get(self.selected)
            .map(|c| &c.command)
    }

    /// Feed a character into the input.
    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
        self.update_filter();
    }

    /// Delete the last character from the input.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

/// Helper to build a `PaletteCommand` with pre-computed lowercase label.
fn palette_cmd(label: &str, command: AppCommand) -> PaletteCommand {
    PaletteCommand {
        label_lower: label.to_lowercase(),
        label: label.to_string(),
        command,
    }
}

/// Build the full list of palette commands.
fn all_palette_commands() -> Vec<PaletteCommand> {
    let mut cmds = vec![
        palette_cmd("Save", AppCommand::Save),
        palette_cmd("Save All", AppCommand::SaveAll),
        palette_cmd("Open File", AppCommand::OpenFilePicker),
        palette_cmd("Search in Files", AppCommand::OpenProjectSearch),
        palette_cmd("Close Tab", AppCommand::CloseTab),
        palette_cmd("Next Tab", AppCommand::NextTab),
        palette_cmd("Previous Tab", AppCommand::PrevTab),
        palette_cmd("Show Editor", AppCommand::ShowEditorTab),
        palette_cmd("Show Agents", AppCommand::ShowAgentsTab),
        palette_cmd("Toggle File Tree", AppCommand::ToggleFileTree),
        palette_cmd("Close AI Session", AppCommand::AiCloseSession),
        palette_cmd("Next AI Session", AppCommand::AiNextSession),
        palette_cmd("Previous AI Session", AppCommand::AiPrevSession),
        palette_cmd("Toggle Git Panel", AppCommand::ToggleGitPanel),
        palette_cmd("Stage Hunk", AppCommand::GitStageHunk),
        palette_cmd("Unstage Hunk", AppCommand::GitUnstageHunk),
        palette_cmd("Discard Hunk", AppCommand::GitDiscardHunk),
        palette_cmd("Undo", AppCommand::Undo),
        palette_cmd("Redo", AppCommand::Redo),
        palette_cmd("Find", AppCommand::Find),
        palette_cmd("Find and Replace", AppCommand::Replace),
        palette_cmd("Quit", AppCommand::Quit),
        palette_cmd("Select Language", AppCommand::OpenLanguagePicker),
        palette_cmd("Toggle Vim Mode", AppCommand::ToggleVimMode),
        palette_cmd("Select Theme", AppCommand::OpenThemePicker),
        palette_cmd("Dismiss Notifications", AppCommand::DismissNotifications),
        // Agent pane commands
        palette_cmd("Agent: Split Vertical", AppCommand::AgentSplitVertical),
        palette_cmd("Agent: Split Horizontal", AppCommand::AgentSplitHorizontal),
        palette_cmd("Agent: Split Smart", AppCommand::AgentSplitAuto),
        palette_cmd("Agent: Close Pane", AppCommand::AgentClosePane),
        palette_cmd("Agent: Focus Next", AppCommand::AgentFocusNext),
        palette_cmd("Agent: Focus Previous", AppCommand::AgentFocusPrev),
        palette_cmd("Agent: Toggle Zoom", AppCommand::AgentToggleZoom),
        palette_cmd("Agent: Select Layout", AppCommand::AgentApplyLayout),
        palette_cmd("Agent: Save Current Layout", AppCommand::AgentSaveLayout),
        palette_cmd("Agent: Save Layout As…", AppCommand::AgentSaveLayoutAs),
    ];

    cmds.sort_by(|a, b| a.label_lower.cmp(&b.label_lower));
    cmds
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_command_palette(
    area: Rect,
    buf: &mut Buffer,
    state: &mut CommandPaletteState,
    theme: &Theme,
) {
    // The overlay enum already gates visibility, so the modal is opened
    // transiently for this frame; no persistent lifecycle state needed.
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(" Command Palette ")
        .size_percent(60, 40)
        .min_size(34, 8)
        .anchor(Anchor::Top { margin: 3 })
        .footer(" ↑↓ select · Enter run · Esc close ")
        .render(area, buf, &mut modal, |inner, buf| {
            // Input line.
            let input_line = format!("> {}", state.input);
            Line::from(Span::from(input_line).bold())
                .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

            // Separator under the input.
            if inner.height > 1 {
                render_hrule(buf, inner.x, inner.y + 1, inner.width);
            }

            let list_start_y = inner.y + 2;
            let list_height = inner.height.saturating_sub(2) as usize;
            state.ensure_visible(list_height);

            for (vi, i) in (state.scroll_offset..).take(list_height).enumerate() {
                if i >= state.filtered_commands.len() {
                    break;
                }
                let y = list_start_y + vi as u16;
                if y >= inner.y + inner.height {
                    break;
                }

                let cmd = &state.filtered_commands[i];
                let label = format!("  {}", cmd.label);
                let span = if i == state.selected {
                    Span::styled(label, theme.overlay_selected)
                } else {
                    Span::from(label)
                };
                Line::from(span).render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::OverlayState;
    fn make_palette() -> CommandPaletteState {
        let all = all_palette_commands();
        CommandPaletteState {
            filtered_commands: all.clone(),
            all_commands: all,
            ..Default::default()
        }
    }

    #[test]
    fn palette_filter_empty() {
        let mut cp = make_palette();
        cp.update_filter();
        assert!(!cp.filtered_commands.is_empty());
    }

    #[test]
    fn palette_filter_narrows() {
        let mut cp = make_palette();
        cp.type_char('s');
        cp.type_char('a');
        cp.type_char('v');
        // Should match "Save" and "Save All".
        assert!(cp.filtered_commands.len() >= 2);
        assert!(
            cp.filtered_commands
                .iter()
                .all(|c| c.label.to_lowercase().contains("sav"))
        );
    }

    #[test]
    fn palette_select_wrap() {
        let mut cp = make_palette();
        let count = cp.filtered_commands.len();
        cp.selected = 0;
        cp.select_prev();
        assert_eq!(cp.selected, count - 1);
        cp.select_next();
        assert_eq!(cp.selected, 0);
    }

    #[test]
    fn palette_backspace() {
        let mut cp = make_palette();
        cp.type_char('x');
        cp.type_char('y');
        cp.type_char('z');
        assert!(cp.filtered_commands.is_empty());
        cp.backspace();
        cp.backspace();
        cp.backspace();
        assert!(!cp.filtered_commands.is_empty());
    }

    #[test]
    fn overlay_open_close() {
        let mut overlay = OverlayState::default();
        assert!(!overlay.is_active());
        overlay.open_command_palette();
        assert!(overlay.is_active());
        overlay.close();
        assert!(!overlay.is_active());
    }

    #[test]
    fn palette_includes_open_file_command() {
        let cmds = all_palette_commands();
        assert!(cmds.iter().any(|c| c.label == "Open File"));
    }
}
