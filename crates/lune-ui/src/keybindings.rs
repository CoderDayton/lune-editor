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
        let mut km = Self::new();

        // Application lifecycle.
        km.bind(KeyCode::Char('q'), KeyModifiers::CONTROL, AppCommand::Quit);

        // File operations.
        km.bind(KeyCode::Char('s'), KeyModifiers::CONTROL, AppCommand::Save);
        km.bind(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            AppCommand::SaveAll,
        );
        km.bind(
            KeyCode::Char('o'),
            KeyModifiers::CONTROL,
            AppCommand::OpenFilePicker,
        );

        // Tab management.
        km.bind(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
            AppCommand::CloseTab,
        );
        km.bind(KeyCode::Tab, KeyModifiers::CONTROL, AppCommand::NextTab);
        km.bind(
            KeyCode::BackTab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            AppCommand::PrevTab,
        );

        // Panel toggles.
        km.bind(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
            AppCommand::ToggleFileTree,
        );
        km.bind(
            KeyCode::Char('`'),
            KeyModifiers::CONTROL,
            AppCommand::ToggleAiPanel,
        );

        // Editor commands.
        km.bind(KeyCode::Char('z'), KeyModifiers::CONTROL, AppCommand::Undo);
        km.bind(KeyCode::Char('y'), KeyModifiers::CONTROL, AppCommand::Redo);
        km.bind(KeyCode::Char('f'), KeyModifiers::CONTROL, AppCommand::Find);
        km.bind(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
            AppCommand::Replace,
        );

        // Command palette.
        km.bind(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
            AppCommand::OpenCommandPalette,
        );

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
