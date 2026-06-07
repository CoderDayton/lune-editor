//! AI client picker overlay — choose which AI client to launch.

use crate::primitives::{Buffer, Color, Line, Rect, Span, Style, Stylize, Widget};
use crate::style::color as color_util;
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};
use lune_ai::session::AiClientKind;

use super::util::render_hrule;

/// Metadata for a known AI CLI client.
struct KnownClient {
    label: &'static str,
    command: &'static str,
    /// Terminal color used for the colored bullet.
    color: Color,
}

/// The full catalog of known AI CLI clients.
const KNOWN_CLIENTS: &[KnownClient] = &[
    KnownClient {
        label: "Claude Code",
        command: "claude",
        color: color_util::hex("#d7963c"), // amber
    },
    KnownClient {
        label: "OpenCode",
        command: "opencode",
        color: color_util::hex("#50b4ff"), // sky blue
    },
    KnownClient {
        label: "Gemini",
        command: "gemini",
        color: color_util::hex("#42c88c"), // teal-green
    },
    KnownClient {
        label: "Kilo Code",
        command: "kilo",
        color: color_util::hex("#ff6464"), // coral red
    },
    KnownClient {
        label: "Cline",
        command: "cline",
        color: color_util::hex("#a06eff"), // violet
    },
    KnownClient {
        label: "Qwen Code",
        command: "qwen",
        color: color_util::hex("#3cd2c8"), // cyan
    },
];

/// Returns `true` if `cmd` is found as an executable file on `$PATH`.
fn is_command_available(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path_var| std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file()))
}

/// An entry in the AI client picker (only installed clients appear).
#[derive(Clone, Debug)]
pub struct AiClientEntry {
    /// Display name.
    pub label: String,
    /// CLI command.
    pub command: String,
    /// Accent color for the colored bullet.
    pub color: Color,
    /// The client kind to spawn.
    pub kind: AiClientKind,
}

/// State for the AI client picker overlay.
#[derive(Clone, Debug, Default)]
pub struct AiClientPickerState {
    /// Installed clients found on PATH.
    pub entries: Vec<AiClientEntry>,
    /// Currently highlighted index.
    pub selected: usize,
}

impl AiClientPickerState {
    /// Scan PATH for available clients and return a ready state.
    ///
    /// Always appends a "System Shell" entry at the end.
    #[must_use]
    pub fn scan_available() -> Self {
        // Downgrade RGB → nearest 8-bit ANSI for terminals that don't
        // advertise truecolor via $COLORTERM. On truecolor terminals this
        // is a pass-through and the Rgb value is preserved.
        let mut entries: Vec<AiClientEntry> = KNOWN_CLIENTS
            .iter()
            .filter(|c| is_command_available(c.command))
            .map(|c| AiClientEntry {
                label: c.label.to_string(),
                command: c.command.to_string(),
                color: color_util::downgrade_if_needed(c.color),
                kind: if c.command == "claude" {
                    AiClientKind::ClaudeCode
                } else {
                    AiClientKind::Custom {
                        name: c.label.to_string(),
                        command: c.command.to_string(),
                    }
                },
            })
            .collect();

        // Always append a system shell entry.
        let shell_cmd = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        entries.push(AiClientEntry {
            label: "System Shell".to_string(),
            command: shell_cmd,
            color: color_util::downgrade_if_needed(color_util::hex("#78c878")),
            kind: AiClientKind::Shell,
        });

        Self {
            entries,
            selected: 0,
        }
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if !self.entries.is_empty() {
            self.selected = if self.selected == 0 {
                self.entries.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1) % self.entries.len();
        }
    }

    /// Get the client kind for the currently selected entry.
    #[must_use]
    pub fn selected_kind(&self) -> Option<AiClientKind> {
        self.entries.get(self.selected).map(|e| e.kind.clone())
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_ai_client_picker(
    area: Rect,
    buf: &mut Buffer,
    state: &AiClientPickerState,
    theme: &Theme,
) {
    let popup_w = (area.width * 50 / 100).max(40).min(area.width);
    let popup_h = (state.entries.len() as u16 + 6).min(area.height);
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(" Open AI Session ")
        .size_cells(popup_w, popup_h)
        .anchor(Anchor::Top {
            margin: (area.height.saturating_sub(popup_h)) / 3,
        })
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Subtitle
    Line::from(Span::from(" Choose a client to open:").dim())
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    // Separator
    if inner.height > 1 {
        render_hrule(buf, inner.x, inner.y + 1, inner.width);
    }

    // Entry list — or empty state
    if state.entries.is_empty() {
        let y = inner.y + 2;
        if y < inner.y + inner.height {
            Line::from(Span::from("  No AI clients found in PATH").dim())
                .render(Rect::new(inner.x, y, inner.width, 1), buf);
        }
    } else {
        for (i, entry) in state.entries.iter().enumerate() {
            let y = inner.y + 2 + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let label_text = format!(" {} ({})", entry.label, entry.command);
            let max_label = inner.width.saturating_sub(3) as usize;
            let label_text = if label_text.len() > max_label {
                format!("{}…", &label_text[..max_label.saturating_sub(1)])
            } else {
                label_text
            };

            if i == state.selected {
                // Selected: full row highlighted
                let full = format!(" ● {label_text}");
                Line::from(Span::styled(full, theme.overlay_selected))
                    .render(Rect::new(inner.x, y, inner.width, 1), buf);
            } else {
                // Unselected: colored bullet + plain label
                let bullet = Span::styled(" ● ", Style::new().fg(entry.color));
                let text = Span::from(label_text);
                Line::from(vec![bullet, text]).render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        }
    }

    // Footer hint
    let hint_y = inner.y + inner.height.saturating_sub(1);
    if hint_y > inner.y + 1 + state.entries.len() as u16 {
        Line::from(Span::from(" ↑↓ select · Enter open · Esc cancel").dim())
            .render(Rect::new(inner.x, hint_y, inner.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::{OverlayKind, OverlayState};
    fn make_picker_state(entries: Vec<AiClientEntry>) -> AiClientPickerState {
        AiClientPickerState {
            entries,
            selected: 0,
        }
    }

    fn claude_entry() -> AiClientEntry {
        AiClientEntry {
            label: "Claude Code".to_string(),
            command: "claude".to_string(),
            color: Color::Rgb(215, 150, 60),
            kind: AiClientKind::ClaudeCode,
        }
    }

    fn custom_entry(label: &str, command: &str) -> AiClientEntry {
        AiClientEntry {
            label: label.to_string(),
            command: command.to_string(),
            color: Color::Rgb(80, 180, 255),
            kind: AiClientKind::Custom {
                name: label.to_string(),
                command: command.to_string(),
            },
        }
    }

    #[test]
    fn known_clients_all_six_present() {
        let commands: Vec<&str> = KNOWN_CLIENTS.iter().map(|c| c.command).collect();
        assert_eq!(commands.len(), 6, "expected exactly 6 known clients");
        assert!(commands.contains(&"claude"), "missing claude");
        assert!(commands.contains(&"opencode"), "missing opencode");
        assert!(commands.contains(&"gemini"), "missing gemini");
        assert!(commands.contains(&"kilo"), "missing kilo");
        assert!(commands.contains(&"cline"), "missing cline");
        assert!(commands.contains(&"qwen"), "missing qwen");
    }

    #[test]
    fn known_clients_labels_match_commands() {
        let find = |cmd: &str| KNOWN_CLIENTS.iter().find(|c| c.command == cmd).unwrap();
        assert_eq!(find("claude").label, "Claude Code");
        assert_eq!(find("opencode").label, "OpenCode");
        assert_eq!(find("gemini").label, "Gemini");
        assert_eq!(find("kilo").label, "Kilo Code");
        assert_eq!(find("cline").label, "Cline");
        assert_eq!(find("qwen").label, "Qwen Code");
    }

    #[test]
    fn known_clients_colors_all_distinct() {
        let colors: Vec<Color> = KNOWN_CLIENTS.iter().map(|c| c.color).collect();
        // No two entries share a color.
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "clients at index {i} and {j} share the same color"
                );
            }
        }
    }

    #[test]
    fn known_clients_colors_non_default() {
        // Every color must be an explicit Rgb value, not the default Reset.
        for client in KNOWN_CLIENTS {
            assert!(
                matches!(client.color, Color::Rgb(_, _, _)),
                "client '{}' uses a non-Rgb color: {:?}",
                client.label,
                client.color
            );
        }
    }

    #[test]
    fn is_command_available_returns_false_for_fake_command() {
        assert!(
            !is_command_available("zzzneverexists123"),
            "non-existent command should not be found on PATH"
        );
    }

    #[test]
    fn is_command_available_returns_true_for_sh() {
        assert!(
            is_command_available("sh"),
            "'sh' must be present on PATH in any POSIX environment"
        );
    }

    #[test]
    fn is_command_available_empty_string_returns_false() {
        // An empty command name should never match a real executable.
        assert!(!is_command_available(""));
    }

    #[test]
    fn picker_empty_select_next_is_noop() {
        let mut state = AiClientPickerState::default();
        state.select_next();
        assert_eq!(
            state.selected, 0,
            "select_next on empty state must not panic or change index"
        );
    }

    #[test]
    fn picker_empty_select_prev_is_noop() {
        let mut state = AiClientPickerState::default();
        state.select_prev();
        assert_eq!(
            state.selected, 0,
            "select_prev on empty state must not panic or change index"
        );
    }

    #[test]
    fn picker_empty_selected_kind_returns_none() {
        let state = AiClientPickerState::default();
        assert!(
            state.selected_kind().is_none(),
            "selected_kind on empty state must return None"
        );
    }

    #[test]
    fn picker_single_entry_next_wraps_to_zero() {
        let mut state = make_picker_state(vec![claude_entry()]);
        state.select_next();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_single_entry_prev_wraps_to_zero() {
        let mut state = make_picker_state(vec![claude_entry()]);
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_select_next_advances_index() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn picker_select_next_wraps_at_end() {
        let mut state =
            make_picker_state(vec![claude_entry(), custom_entry("OpenCode", "opencode")]);
        state.select_next(); // → 1
        state.select_next(); // → 0 (wrap)
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_select_prev_decrements_index() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        state.selected = 2;
        state.select_prev();
        assert_eq!(state.selected, 1);
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn picker_select_prev_wraps_at_start() {
        let mut state = make_picker_state(vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
        ]);
        assert_eq!(state.selected, 0);
        state.select_prev(); // wraps to len-1 = 2
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn picker_nav_full_round_trip() {
        let n = 4usize;
        let entries = vec![
            claude_entry(),
            custom_entry("OpenCode", "opencode"),
            custom_entry("Gemini", "gemini"),
            custom_entry("Kilo Code", "kilo"),
        ];
        let mut state = make_picker_state(entries);
        // Forward round-trip.
        for i in 0..n {
            assert_eq!(state.selected, i);
            state.select_next();
        }
        assert_eq!(
            state.selected, 0,
            "should wrap back to 0 after full forward cycle"
        );
        // Backward: one prev from 0 should go to n-1.
        state.select_prev();
        assert_eq!(state.selected, n - 1);
    }

    #[test]
    fn picker_selected_kind_claude_returns_claude_code() {
        let state = make_picker_state(vec![claude_entry()]);
        assert_eq!(state.selected_kind(), Some(AiClientKind::ClaudeCode));
    }

    #[test]
    fn picker_selected_kind_custom_entry() {
        let state = make_picker_state(vec![custom_entry("Gemini", "gemini")]);
        assert_eq!(
            state.selected_kind(),
            Some(AiClientKind::Custom {
                name: "Gemini".to_string(),
                command: "gemini".to_string(),
            })
        );
    }

    #[test]
    fn picker_selected_kind_tracks_selection() {
        let mut state =
            make_picker_state(vec![claude_entry(), custom_entry("OpenCode", "opencode")]);
        assert_eq!(state.selected_kind(), Some(AiClientKind::ClaudeCode));
        state.select_next();
        assert_eq!(
            state.selected_kind(),
            Some(AiClientKind::Custom {
                name: "OpenCode".to_string(),
                command: "opencode".to_string(),
            })
        );
    }

    #[test]
    fn scan_available_excludes_fake_commands() {
        // Verify the filtering property: any AI client entry returned by
        // scan_available must correspond to a real executable (Shell is
        // always present regardless).
        let state = AiClientPickerState::scan_available();
        for entry in &state.entries {
            if entry.kind == AiClientKind::Shell {
                continue; // Shell is always appended.
            }
            assert!(
                is_command_available(&entry.command),
                "scan_available returned '{}' but is_command_available says it's absent",
                entry.command
            );
        }
    }

    #[test]
    fn scan_available_selected_starts_at_zero() {
        let state = AiClientPickerState::scan_available();
        assert_eq!(state.selected, 0, "initial selection must be 0");
    }

    #[test]
    fn scan_available_entries_have_known_client_data() {
        // Every returned entry must originate from KNOWN_CLIENTS or be the shell.
        let state = AiClientPickerState::scan_available();
        for entry in &state.entries {
            if entry.kind == AiClientKind::Shell {
                assert_eq!(entry.label, "System Shell");
                continue;
            }
            let known = KNOWN_CLIENTS.iter().find(|k| k.command == entry.command);
            assert!(
                known.is_some(),
                "scan_available returned unknown command '{}'",
                entry.command
            );
            let known = known.unwrap();
            assert_eq!(entry.label, known.label);
            // `scan_available` applies `downgrade_if_needed` to colors so
            // they render correctly on low-color terminals; apply the
            // same transform to the source-of-truth before comparing.
            assert_eq!(entry.color, color_util::downgrade_if_needed(known.color));
        }
    }

    #[test]
    fn scan_available_always_includes_shell() {
        let state = AiClientPickerState::scan_available();
        let shell = state.entries.iter().find(|e| e.kind == AiClientKind::Shell);
        assert!(shell.is_some(), "System Shell must always be present");
        let shell = shell.unwrap();
        assert_eq!(shell.label, "System Shell");
    }

    #[test]
    fn scan_available_claude_entry_maps_to_claude_code_kind() {
        // If claude is on PATH, verify its kind. If not, the test is vacuous.
        let state = AiClientPickerState::scan_available();
        if let Some(entry) = state.entries.iter().find(|e| e.command == "claude") {
            assert_eq!(
                entry.kind,
                AiClientKind::ClaudeCode,
                "claude command must map to AiClientKind::ClaudeCode"
            );
        }
    }

    #[test]
    fn scan_available_non_claude_entries_map_to_custom_or_shell_kind() {
        let state = AiClientPickerState::scan_available();
        for entry in state.entries.iter().filter(|e| e.command != "claude") {
            match &entry.kind {
                AiClientKind::Custom { name, command } => {
                    assert_eq!(name, &entry.label);
                    assert_eq!(command, &entry.command);
                }
                AiClientKind::Shell => {
                    assert_eq!(entry.label, "System Shell");
                }
                other @ AiClientKind::ClaudeCode => panic!(
                    "entry '{}' should be Custom or Shell but got {:?}",
                    entry.command, other
                ),
            }
        }
    }

    #[test]
    fn overlay_open_ai_client_picker_sets_active_kind() {
        let mut overlay = OverlayState::default();
        overlay.open_ai_client_picker();
        assert!(overlay.is_active());
        assert!(
            matches!(overlay.active, Some(OverlayKind::AiClientPicker)),
            "active overlay must be AiClientPicker"
        );
    }

    #[test]
    fn overlay_open_ai_client_picker_close_clears() {
        let mut overlay = OverlayState::default();
        overlay.open_ai_client_picker();
        overlay.close();
        assert!(!overlay.is_active());
    }
}
