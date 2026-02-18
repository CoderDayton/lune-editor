//! Keybinding system.
//!
//! Maps key combinations to [`AppCommand`]s. Supports a default keymap
//! and user-customizable keymaps via TOML config.
//!
//! # Config format
//!
//! ```toml
//! [normal]
//! "ctrl+s" = "save"
//! "ctrl+shift+p" = "command_palette"
//!
//! [vim.normal]
//! "g d" = "go_to_definition"
//! ```

use std::collections::HashMap;
use std::path::Path;

use crate::primitives::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

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
            // Theme switching
            (Char('t'), CTRL, AppCommand::NextTheme),
            (Char('T'), CTRL_SHIFT, AppCommand::PrevTheme),
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

    /// Merge custom overrides into this keymap.
    ///
    /// Overrides replace existing bindings for the same key combo.
    pub fn merge(&mut self, overrides: &HashMap<KeyCombo, AppCommand>) {
        for (combo, cmd) in overrides {
            self.bindings.insert(*combo, cmd.clone());
        }
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::default_global()
    }
}

// ── TOML-based keymap configuration ────────────────────────────────────

/// TOML-serializable keybinding configuration.
///
/// Each section maps key combo strings to command strings.
/// Custom bindings override the defaults (they don't replace the entire
/// keymap — only the specified keys are changed).
///
/// # Example
///
/// ```toml
/// [normal]
/// "ctrl+s" = "save"
/// "ctrl+shift+p" = "command_palette"
/// "f5" = "toggle_git_panel"
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    /// Normal mode keybindings (non-vim, or vim-agnostic).
    pub normal: HashMap<String, String>,
    // Future: pub vim_normal, pub vim_insert, pub vim_visual, etc.
}

impl KeymapConfig {
    /// Load a keymap config from a TOML file.
    ///
    /// Returns default (empty) if the file doesn't exist.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Compile the config into a map of `KeyCombo → AppCommand`.
    ///
    /// Entries with unparseable key combos or unknown commands are
    /// silently skipped (logged at warn level).
    #[must_use]
    pub fn compile_normal(&self) -> HashMap<KeyCombo, AppCommand> {
        let mut result = HashMap::new();
        for (key_str, cmd_str) in &self.normal {
            let Some(combo) = parse_key_combo(key_str) else {
                log::warn!("keybindings: unknown key combo: {key_str:?}");
                continue;
            };
            let Some(cmd) = parse_command(cmd_str) else {
                log::warn!("keybindings: unknown command: {cmd_str:?}");
                continue;
            };
            result.insert(combo, cmd);
        }
        result
    }
}

// ── Key combo string parsing ──────────────────────────────────────────

/// Parse a key combo string like `"ctrl+shift+a"` into a [`KeyCombo`].
///
/// Format: `modifier[+modifier]*+key`
///
/// Supported modifiers: `ctrl`, `alt`, `shift`
/// Supported keys: single chars (`a`-`z`, `0`-`9`, etc.), `space`, `tab`,
/// `backtab`, `enter`, `esc`, `backspace`, `delete`, `up`, `down`,
/// `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1`-`f12`.
///
/// Returns `None` if the string cannot be parsed.
#[must_use]
pub fn parse_key_combo(s: &str) -> Option<KeyCombo> {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split('+').collect();

    if parts.is_empty() {
        return None;
    }

    let mut modifiers = KeyModifiers::NONE;

    // All parts except the last are modifiers.
    for &part in &parts[..parts.len() - 1] {
        match part {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => return None, // Unknown modifier
        }
    }

    // The last part is the key.
    let key_str = parts[parts.len() - 1];
    let code = parse_key_code(key_str)?;

    // If shift is present and the key is a single char, uppercase it.
    let code = if modifiers.contains(KeyModifiers::SHIFT) {
        if let KeyCode::Char(c) = code {
            KeyCode::Char(c.to_uppercase().next().unwrap_or(c))
        } else {
            code
        }
    } else {
        code
    };

    Some(KeyCombo::new(code, modifiers))
}

/// Parse a single key name into a [`KeyCode`].
fn parse_key_code(s: &str) -> Option<KeyCode> {
    // Single character
    if s.len() == 1 {
        return Some(KeyCode::Char(s.chars().next()?));
    }

    // Named keys
    match s {
        "space" => Some(KeyCode::Char(' ')),
        "tab" => Some(KeyCode::Tab),
        "backtab" => Some(KeyCode::BackTab),
        "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "backspace" | "bs" => Some(KeyCode::Backspace),
        "delete" | "del" => Some(KeyCode::Delete),
        "insert" | "ins" => Some(KeyCode::Insert),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "pgdn" => Some(KeyCode::PageDown),
        // Function keys f1-f12
        _ if s.starts_with('f') => {
            let n: u8 = s[1..].parse().ok()?;
            if (1..=12).contains(&n) {
                Some(KeyCode::F(n))
            } else {
                None
            }
        }
        // Backtick / special chars
        "backtick" | "grave" => Some(KeyCode::Char('`')),
        "minus" => Some(KeyCode::Char('-')),
        "plus" => Some(KeyCode::Char('+')),
        "equal" | "equals" => Some(KeyCode::Char('=')),
        _ => None,
    }
}

// ── Command string parsing ────────────────────────────────────────────

/// Parse a command name string into an [`AppCommand`].
///
/// Uses `snake_case` names.  Returns `None` for unknown commands.
#[must_use]
pub fn parse_command(s: &str) -> Option<AppCommand> {
    match s.trim().to_lowercase().as_str() {
        // Lifecycle
        "quit" => Some(AppCommand::Quit),
        "force_quit" => Some(AppCommand::ForceQuit),
        // File
        "save" => Some(AppCommand::Save),
        "save_all" => Some(AppCommand::SaveAll),
        "open_file_picker" => Some(AppCommand::OpenFilePicker),
        // Tabs
        "close_tab" => Some(AppCommand::CloseTab),
        "next_tab" => Some(AppCommand::NextTab),
        "prev_tab" => Some(AppCommand::PrevTab),
        // Panels
        "toggle_file_tree" => Some(AppCommand::ToggleFileTree),
        "toggle_ai_panel" => Some(AppCommand::ToggleAiPanel),
        "toggle_git_panel" => Some(AppCommand::ToggleGitPanel),
        "command_palette" | "open_command_palette" => Some(AppCommand::OpenCommandPalette),
        "toggle_hidden_files" => Some(AppCommand::ToggleHiddenFiles),
        "focus_next_pane" => Some(AppCommand::FocusNextPane),
        // File tree
        "new_file" => Some(AppCommand::NewFile),
        "new_dir" => Some(AppCommand::NewDir),
        "rename_entry" => Some(AppCommand::RenameEntry),
        "delete_entry" => Some(AppCommand::DeleteEntry),
        // Editor
        "undo" => Some(AppCommand::Undo),
        "redo" => Some(AppCommand::Redo),
        "find" => Some(AppCommand::Find),
        "replace" => Some(AppCommand::Replace),
        // Vim modes
        "enter_normal_mode" => Some(AppCommand::EnterNormalMode),
        "enter_insert_mode" => Some(AppCommand::EnterInsertMode),
        "enter_visual_mode" => Some(AppCommand::EnterVisualMode),
        // Git
        "git_stage" => Some(AppCommand::GitStage),
        "git_unstage" => Some(AppCommand::GitUnstage),
        "git_commit" => Some(AppCommand::GitCommit),
        "git_discard" => Some(AppCommand::GitDiscard),
        "git_refresh" => Some(AppCommand::GitRefresh),
        // AI
        "ai_ask_selection" => Some(AppCommand::AiAskSelection),
        "ai_refactor_file" => Some(AppCommand::AiRefactorFile),
        "ai_summarize_changes" => Some(AppCommand::AiSummarizeChanges),
        // Live Mode
        "toggle_live_mode" => Some(AppCommand::ToggleLiveMode),
        // Theme
        "next_theme" => Some(AppCommand::NextTheme),
        "prev_theme" => Some(AppCommand::PrevTheme),
        // Settings
        "open_settings" => Some(AppCommand::OpenSettings),
        "open_keybindings" => Some(AppCommand::OpenKeybindings),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

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

    // ── Key combo parsing ──────────────────────────────────────────────

    #[test]
    fn parse_simple_key() {
        let combo = parse_key_combo("a").unwrap();
        assert_eq!(combo.code, KeyCode::Char('a'));
        assert_eq!(combo.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_ctrl_key() {
        let combo = parse_key_combo("ctrl+s").unwrap();
        assert_eq!(combo.code, KeyCode::Char('s'));
        assert_eq!(combo.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_ctrl_shift_key() {
        let combo = parse_key_combo("ctrl+shift+p").unwrap();
        assert_eq!(combo.code, KeyCode::Char('P'));
        assert_eq!(combo.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_alt_key() {
        let combo = parse_key_combo("alt+f").unwrap();
        assert_eq!(combo.code, KeyCode::Char('f'));
        assert_eq!(combo.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn parse_function_key() {
        let combo = parse_key_combo("f5").unwrap();
        assert_eq!(combo.code, KeyCode::F(5));
        assert_eq!(combo.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_ctrl_function_key() {
        let combo = parse_key_combo("ctrl+f12").unwrap();
        assert_eq!(combo.code, KeyCode::F(12));
        assert_eq!(combo.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_key_combo("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_key_combo("tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key_combo("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key_combo("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(
            parse_key_combo("backspace").unwrap().code,
            KeyCode::Backspace
        );
        assert_eq!(parse_key_combo("delete").unwrap().code, KeyCode::Delete);
        assert_eq!(parse_key_combo("up").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key_combo("down").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key_combo("left").unwrap().code, KeyCode::Left);
        assert_eq!(parse_key_combo("right").unwrap().code, KeyCode::Right);
        assert_eq!(parse_key_combo("home").unwrap().code, KeyCode::Home);
        assert_eq!(parse_key_combo("end").unwrap().code, KeyCode::End);
        assert_eq!(parse_key_combo("pageup").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_key_combo("pagedown").unwrap().code, KeyCode::PageDown);
    }

    #[test]
    fn parse_case_insensitive() {
        let combo = parse_key_combo("Ctrl+Shift+S").unwrap();
        assert_eq!(combo.code, KeyCode::Char('S'));
        assert_eq!(combo.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_unknown_modifier_returns_none() {
        assert!(parse_key_combo("super+a").is_none());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_key_combo("").is_none());
    }

    #[test]
    fn parse_unknown_key_returns_none() {
        assert!(parse_key_combo("ctrl+foobar").is_none());
    }

    // ── Command parsing ────────────────────────────────────────────────

    #[test]
    fn parse_known_commands() {
        assert_eq!(parse_command("quit"), Some(AppCommand::Quit));
        assert_eq!(parse_command("save"), Some(AppCommand::Save));
        assert_eq!(parse_command("save_all"), Some(AppCommand::SaveAll));
        assert_eq!(parse_command("undo"), Some(AppCommand::Undo));
        assert_eq!(parse_command("redo"), Some(AppCommand::Redo));
        assert_eq!(parse_command("next_tab"), Some(AppCommand::NextTab));
        assert_eq!(parse_command("prev_tab"), Some(AppCommand::PrevTab));
        assert_eq!(parse_command("close_tab"), Some(AppCommand::CloseTab));
        assert_eq!(
            parse_command("toggle_file_tree"),
            Some(AppCommand::ToggleFileTree)
        );
        assert_eq!(
            parse_command("toggle_ai_panel"),
            Some(AppCommand::ToggleAiPanel)
        );
        assert_eq!(
            parse_command("toggle_git_panel"),
            Some(AppCommand::ToggleGitPanel)
        );
        assert_eq!(
            parse_command("command_palette"),
            Some(AppCommand::OpenCommandPalette)
        );
        assert_eq!(
            parse_command("toggle_live_mode"),
            Some(AppCommand::ToggleLiveMode)
        );
        assert_eq!(parse_command("next_theme"), Some(AppCommand::NextTheme));
        assert_eq!(parse_command("prev_theme"), Some(AppCommand::PrevTheme));
        assert_eq!(
            parse_command("open_settings"),
            Some(AppCommand::OpenSettings)
        );
        assert_eq!(
            parse_command("open_keybindings"),
            Some(AppCommand::OpenKeybindings)
        );
    }

    #[test]
    fn parse_command_case_insensitive() {
        assert_eq!(parse_command("QUIT"), Some(AppCommand::Quit));
        assert_eq!(parse_command("Save"), Some(AppCommand::Save));
    }

    #[test]
    fn parse_unknown_command_returns_none() {
        assert!(parse_command("nonexistent_command").is_none());
    }

    // ── KeymapConfig ──────────────────────────────────────────────────

    #[test]
    fn keymap_config_compile_normal() {
        let mut config = KeymapConfig::default();
        config.normal.insert("ctrl+s".to_owned(), "save".to_owned());
        config
            .normal
            .insert("f5".to_owned(), "toggle_git_panel".to_owned());

        let compiled = config.compile_normal();
        assert_eq!(compiled.len(), 2);
        assert_eq!(
            compiled.get(&KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(&AppCommand::Save)
        );
        assert_eq!(
            compiled.get(&KeyCombo::new(KeyCode::F(5), KeyModifiers::NONE)),
            Some(&AppCommand::ToggleGitPanel)
        );
    }

    #[test]
    fn keymap_config_skips_invalid_entries() {
        let mut config = KeymapConfig::default();
        config.normal.insert("ctrl+s".to_owned(), "save".to_owned());
        config
            .normal
            .insert("badmod+a".to_owned(), "save".to_owned()); // bad modifier
        config
            .normal
            .insert("ctrl+a".to_owned(), "nosuchcmd".to_owned()); // bad command

        let compiled = config.compile_normal();
        assert_eq!(compiled.len(), 1);
    }

    #[test]
    fn keymap_merge_overrides_existing() {
        let mut km = Keymap::default_global();
        let event = key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(km.lookup(&event), Some(&AppCommand::Save));

        // Override ctrl+s to quit.
        let mut overrides = HashMap::new();
        overrides.insert(
            KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            AppCommand::Quit,
        );
        km.merge(&overrides);

        assert_eq!(km.lookup(&event), Some(&AppCommand::Quit));
    }

    #[test]
    fn keymap_config_load_nonexistent_returns_default() {
        let config = KeymapConfig::load(Path::new("/nonexistent/keybindings.toml")).unwrap();
        assert!(config.normal.is_empty());
    }

    #[test]
    fn keymap_config_roundtrip_toml() {
        let mut config = KeymapConfig::default();
        config.normal.insert("ctrl+s".to_owned(), "save".to_owned());
        config.normal.insert("ctrl+q".to_owned(), "quit".to_owned());

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: KeymapConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.normal.len(), 2);
        assert_eq!(parsed.normal.get("ctrl+s"), Some(&"save".to_owned()));
    }

    #[test]
    fn keymap_config_load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.toml");
        std::fs::write(
            &path,
            r#"
[normal]
"ctrl+s" = "save"
"f5" = "toggle_git_panel"
"#,
        )
        .unwrap();

        let config = KeymapConfig::load(&path).unwrap();
        assert_eq!(config.normal.len(), 2);

        let compiled = config.compile_normal();
        assert_eq!(compiled.len(), 2);
    }

    #[test]
    fn parse_backtick() {
        let combo = parse_key_combo("ctrl+backtick").unwrap();
        assert_eq!(combo.code, KeyCode::Char('`'));
        assert_eq!(combo.modifiers, KeyModifiers::CONTROL);
    }
}
