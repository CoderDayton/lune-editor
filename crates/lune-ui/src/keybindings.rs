//! Keybinding system.
//!
//! Maps key combinations to [`AppCommand`]s. Supports a default keymap
//! and will later support user-customizable keymaps via config.

use std::collections::HashMap;

use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::event::AppCommand;

/// A key combination: a key code + modifier flags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyCombo {
    /// The key code (letter, function key, etc.).
    pub code: KeyCode,
    /// Modifier flags (Ctrl, Shift, Alt, etc.).
    pub modifiers: KeyModifiers,
}

impl KeyCombo {
    /// Create a new key combo.
    #[must_use]
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Create a key combo from a `KeyEvent`, normalizing modifiers.
    #[must_use]
    pub fn from_key_event(event: &KeyEvent) -> Self {
        Self {
            code: event.code,
            // Mask out NONE and keep only meaningful modifiers.
            modifiers: event.modifiers
                & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT),
        }
    }
}

/// Maps key combos to application commands.
#[derive(Debug)]
pub struct Keymap {
    bindings: HashMap<KeyCombo, AppCommand>,
}

impl Keymap {
    /// Create an empty keymap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Create the default global keymap.
    #[must_use]
    pub fn default_global() -> Self {
        use KeyCode::{BackTab, Char, Tab};
        const CTRL: KeyModifiers = KeyModifiers::CONTROL;
        const CTRL_SHIFT: KeyModifiers = KeyModifiers::from_bits_truncate(
            KeyModifiers::CONTROL.bits() | KeyModifiers::SHIFT.bits(),
        );

        let bindings: &[(KeyCode, KeyModifiers, AppCommand)] = &[
            // Application lifecycle
            (Char('q'), CTRL, AppCommand::Quit),
            // File operations
            (Char('s'), CTRL, AppCommand::Save),
            (Char('S'), CTRL_SHIFT, AppCommand::SaveAll),
            (Char('o'), CTRL, AppCommand::OpenFilePicker),
            // Tab management
            (Char('w'), CTRL, AppCommand::CloseTab),
            (Tab, CTRL, AppCommand::NextTab),
            (BackTab, CTRL_SHIFT, AppCommand::PrevTab),
            // Panel toggles
            (Char('b'), CTRL, AppCommand::ToggleFileTree),
            (Char('`'), CTRL, AppCommand::ToggleAiPanel),
            (Char('G'), CTRL_SHIFT, AppCommand::ToggleGitPanel),
            // AI commands
            (Char('A'), CTRL_SHIFT, AppCommand::AiAskSelection),
            (Char('R'), CTRL_SHIFT, AppCommand::AiRefactorFile),
            (Char('I'), CTRL_SHIFT, AppCommand::AiSummarizeChanges),
            // Editor commands
            (Char('z'), CTRL, AppCommand::Undo),
            (Char('y'), CTRL, AppCommand::Redo),
            (Char('f'), CTRL, AppCommand::Find),
            (Char('h'), CTRL, AppCommand::Replace),
            // Command palette
            (Char('p'), CTRL, AppCommand::OpenCommandPalette),
            // Live Mode
            (Char('l'), CTRL, AppCommand::ToggleLiveMode),
        ];

        let mut km = Self::new();
        for (code, mods, cmd) in bindings {
            km.bind(*code, *mods, cmd.clone());
        }
        km
    }

    /// Add a binding.
    pub fn bind(&mut self, code: KeyCode, modifiers: KeyModifiers, command: AppCommand) {
        self.bindings
            .insert(KeyCombo::new(code, modifiers), command);
    }

    /// Look up a key event in the keymap.
    #[must_use]
    pub fn lookup(&self, event: &KeyEvent) -> Option<&AppCommand> {
        let combo = KeyCombo::from_key_event(event);
        self.bindings.get(&combo)
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::default_global()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_crossterm::crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    };

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn default_keymap_has_quit() {
        let km = Keymap::default_global();
        let event = key_event(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(km.lookup(&event), Some(&AppCommand::Quit));
    }

    #[test]
    fn default_keymap_has_save() {
        let km = Keymap::default_global();
        let event = key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(km.lookup(&event), Some(&AppCommand::Save));
    }

    #[test]
    fn unbound_key_returns_none() {
        let km = Keymap::default_global();
        let event = key_event(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(km.lookup(&event), None);
    }

    #[test]
    fn custom_binding() {
        let mut km = Keymap::new();
        km.bind(
            KeyCode::F(5),
            KeyModifiers::NONE,
            AppCommand::ToggleGitPanel,
        );
        let event = key_event(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(km.lookup(&event), Some(&AppCommand::ToggleGitPanel));
    }
}
