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
//! "ctrl+k a" = "ai_ask_selection"   # a leader chord (Ctrl+K, then a)
//! ```

use rustc_hash::{FxHashMap, FxHashSet};
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

/// Result of resolving a key sequence against the keymap.
pub enum KeyMatch<'a> {
    /// The sequence is a complete binding.
    Exact(&'a AppCommand),
    /// The sequence is a prefix of one or more longer bindings (a live leader).
    Prefix,
    /// The sequence matches no binding.
    None,
}

/// Maps key *sequences* to application commands.
///
/// A plain binding is a one-combo sequence; a chord/leader binding (e.g.
/// `Ctrl+K` then `a`) is a multi-combo sequence. [`Self::resolve`] drives the
/// runtime's pending-key state machine, telling a completed binding apart from
/// an in-progress leader prefix.
#[derive(Debug)]
pub struct Keymap {
    /// Full key sequences → command.
    ///
    /// `FxHash` is fine here: keys come from the built-in defaults and the
    /// user's own local config file, never from untrusted/network input.
    bindings: FxHashMap<Vec<KeyCombo>, AppCommand>,
    /// Every proper prefix of a bound sequence, so the runtime waits for more
    /// keys instead of treating a leader as unbound.
    prefixes: FxHashSet<Vec<KeyCombo>>,
}

impl Keymap {
    /// Create an empty keymap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: FxHashMap::default(),
            prefixes: FxHashSet::default(),
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
        const CTRL_ALT: KeyModifiers = KeyModifiers::from_bits_truncate(
            KeyModifiers::CONTROL.bits() | KeyModifiers::ALT.bits(),
        );
        const ALT: KeyModifiers = KeyModifiers::ALT;

        let singles: &[(KeyCode, KeyModifiers, AppCommand)] = &[
            // Application lifecycle
            (Char('q'), CTRL, AppCommand::Quit),
            // File operations
            (Char('s'), CTRL, AppCommand::Save),
            (Char('o'), CTRL, AppCommand::OpenFilePicker),
            // Tab management
            (Char('w'), CTRL, AppCommand::CloseTab),
            (Tab, CTRL, AppCommand::NextTab),
            (BackTab, CTRL_SHIFT, AppCommand::PrevTab),
            (Char('1'), CTRL, AppCommand::ShowEditorTab),
            (Char('2'), CTRL, AppCommand::ShowAgentsTab),
            (Char('`'), CTRL, AppCommand::ToggleAgentsTab),
            // Agents tab — pane multiplexer
            (Char('\\'), ALT, AppCommand::AgentSplitVertical),
            (Char('-'), ALT, AppCommand::AgentSplitHorizontal),
            (Char('x'), ALT, AppCommand::AgentClosePane),
            (Char('j'), ALT, AppCommand::AgentFocusNext),
            (Char('k'), ALT, AppCommand::AgentFocusPrev),
            (Char('z'), ALT, AppCommand::AgentToggleZoom),
            (Char(','), ALT, AppCommand::AgentApplyLayout),
            // Panel toggles
            (Char('b'), CTRL, AppCommand::ToggleFileTree),
            (Char('g'), CTRL, AppCommand::ToggleGitPanel),
            // AI sessions
            (Char(']'), CTRL, AppCommand::AiNextSession),
            (Char('['), CTRL, AppCommand::AiPrevSession),
            // Editor commands
            (Char('z'), CTRL, AppCommand::Undo),
            (Char('y'), CTRL, AppCommand::Redo),
            (Char('f'), CTRL, AppCommand::Find),
            (Char('h'), CTRL, AppCommand::Replace),
            // Command palette
            (Char('p'), CTRL, AppCommand::OpenCommandPalette),
            // File / language picker
            (Char('n'), CTRL, AppCommand::NewFile),
            (Char('l'), CTRL, AppCommand::OpenLanguagePicker),
            // Theme
            (Char('t'), CTRL, AppCommand::OpenThemePicker),
            // Keybinding hints (which-key style cheatsheet).
            //
            // `?` is intentionally NOT bound globally — it would block
            // typing `?` into a buffer. Bind `?` in vim normal mode in a
            // future change once the vim keymap layer grows the hook.
            (
                KeyCode::F(1),
                KeyModifiers::empty(),
                AppCommand::ToggleKeyHints,
            ),
            // Vim mode
            (Char('v'), CTRL_ALT, AppCommand::ToggleVimMode),
        ];

        let mut km = Self::new();
        for (code, mods, cmd) in singles {
            km.bind(vec![KeyCombo::new(*code, *mods)], cmd.clone());
        }

        // `Ctrl+K` leader chords. These secondary actions sit behind a
        // terminal-safe leader rather than `Ctrl+Shift+<letter>`, which legacy
        // terminals can't transmit and emulators reserve for copy/paste/tabs.
        let leader: &[(&str, AppCommand)] = &[
            ("ctrl+k a", AppCommand::AiAskSelection),
            ("ctrl+k r", AppCommand::AiRefactorFile),
            ("ctrl+k c", AppCommand::AiSummarizeChanges),
            ("ctrl+k s", AppCommand::SaveAll),
            ("ctrl+k f", AppCommand::OpenProjectSearch),
            ("ctrl+k m", AppCommand::ToggleMarkdownPreview),
            ("ctrl+k n", AppCommand::DismissNotifications),
            ("ctrl+k w", AppCommand::AiCloseSession),
        ];
        for (seq, cmd) in leader {
            if let Some(sequence) = parse_key_sequence(seq) {
                km.bind(sequence, cmd.clone());
            }
        }

        km
    }

    /// Bind a key sequence to a command.
    ///
    /// A one-combo sequence is a plain binding; a multi-combo sequence is a
    /// chord/leader. Every proper prefix is recorded so [`Self::resolve`] can
    /// report an in-progress leader.
    pub fn bind(&mut self, sequence: Vec<KeyCombo>, command: AppCommand) {
        for i in 1..sequence.len() {
            self.prefixes.insert(sequence[..i].to_vec());
        }
        self.bindings.insert(sequence, command);
    }

    /// Resolve a key sequence: a completed binding, a live leader prefix, or
    /// nothing.
    #[must_use]
    pub fn resolve(&self, sequence: &[KeyCombo]) -> KeyMatch<'_> {
        match self.bindings.get(sequence) {
            Some(cmd) => KeyMatch::Exact(cmd),
            None if self.prefixes.contains(sequence) => KeyMatch::Prefix,
            None => KeyMatch::None,
        }
    }

    /// Look up a single key event as a completed one-combo binding.
    ///
    /// Returns `None` for a key that merely *begins* a leader chord — use
    /// [`Self::resolve`] for chord-aware dispatch. A convenience for callers and
    /// tests that only deal in single keys.
    #[must_use]
    pub fn lookup(&self, event: &KeyEvent) -> Option<&AppCommand> {
        match self.resolve(&[KeyCombo::from_key_event(event)]) {
            KeyMatch::Exact(cmd) => Some(cmd),
            KeyMatch::Prefix | KeyMatch::None => None,
        }
    }

    /// Next-key options that continue `prefix`, for the which-key hint.
    /// Sorted by key for stable display.
    #[must_use]
    pub fn continuations(&self, prefix: &[KeyCombo]) -> Vec<(KeyCombo, &AppCommand)> {
        let mut out: Vec<(KeyCombo, &AppCommand)> = self
            .bindings
            .iter()
            .filter(|(seq, _)| seq.len() == prefix.len() + 1 && seq.starts_with(prefix))
            .map(|(seq, cmd)| (seq[prefix.len()], cmd))
            .collect();
        out.sort_by(|(a, _), (b, _)| combo_key_str(a).cmp(&combo_key_str(b)));
        out
    }

    /// Build a one-line which-key hint for an in-progress leader `prefix`,
    /// e.g. `ctrl+k  a:ask AI  r:refactor  …  ·  esc cancel`.
    #[must_use]
    pub fn which_key_hint(&self, prefix: &[KeyCombo]) -> String {
        let prefix_str = prefix
            .iter()
            .map(combo_key_str)
            .collect::<Vec<_>>()
            .join(" ");
        let opts = self
            .continuations(prefix)
            .into_iter()
            .map(|(combo, cmd)| format!("{}:{}", combo_key_str(&combo), command_hint_label(cmd)))
            .collect::<Vec<_>>()
            .join("  ");
        format!("{prefix_str}  {opts}  ·  esc cancel")
    }

    /// Merge custom sequence overrides into this keymap.
    ///
    /// Overrides replace existing bindings for the same sequence.
    pub fn merge(&mut self, overrides: &FxHashMap<Vec<KeyCombo>, AppCommand>) {
        for (sequence, cmd) in overrides {
            self.bind(sequence.clone(), cmd.clone());
        }
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::default_global()
    }
}

/// Short human label for a command, used in the which-key leader hint.
const fn command_hint_label(cmd: &AppCommand) -> &'static str {
    match cmd {
        AppCommand::AiAskSelection => "ask AI",
        AppCommand::AiRefactorFile => "refactor",
        AppCommand::AiSummarizeChanges => "summarize",
        AppCommand::SaveAll => "save all",
        AppCommand::OpenProjectSearch => "find in files",
        AppCommand::ToggleMarkdownPreview => "markdown",
        AppCommand::DismissNotifications => "dismiss",
        AppCommand::AiCloseSession => "close AI",
        // Fallback for user-defined chords whose command isn't listed above.
        _ => "command",
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
/// "ctrl+k o" = "open_settings"   # a leader chord (Ctrl+K, then o)
/// "f5" = "toggle_git_panel"
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    /// Normal mode keybindings (non-vim, or vim-agnostic).
    pub normal: FxHashMap<String, String>,
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
    pub fn compile_normal(&self) -> FxHashMap<Vec<KeyCombo>, AppCommand> {
        let mut result = FxHashMap::default();
        for (key_str, cmd_str) in &self.normal {
            let Some(sequence) = parse_key_sequence(key_str) else {
                log::warn!("keybindings: unknown key sequence: {key_str:?}");
                continue;
            };
            let Some(cmd) = parse_command(cmd_str) else {
                log::warn!("keybindings: unknown command: {cmd_str:?}");
                continue;
            };
            result.insert(sequence, cmd);
        }
        result
    }
}

// ── Key combo string parsing ──────────────────────────────────────────

/// Maximum combos in a single key sequence. Caps how deep a leader chord can
/// nest, bounding the pending-key buffer and the prefix set even for a hostile
/// or fat-fingered config.
const MAX_SEQUENCE_LEN: usize = 5;

/// Parse a key *sequence* — one or more whitespace-separated combos.
///
/// E.g. `"ctrl+s"` or `"ctrl+k a"` (a leader chord). Returns `None` if the
/// string is empty, longer than `MAX_SEQUENCE_LEN`, or any combo is unparseable.
#[must_use]
pub fn parse_key_sequence(s: &str) -> Option<Vec<KeyCombo>> {
    let combos = s
        .split_whitespace()
        .map(parse_key_combo)
        .collect::<Option<Vec<_>>>()?;
    if combos.is_empty() || combos.len() > MAX_SEQUENCE_LEN {
        return None;
    }
    Some(combos)
}

/// Render a single combo for the which-key hint, e.g. `ctrl+k` or `a`.
fn combo_key_str(combo: &KeyCombo) -> String {
    let mut s = String::new();
    if combo.modifiers.contains(KeyModifiers::CONTROL) {
        s.push_str("ctrl+");
    }
    if combo.modifiers.contains(KeyModifiers::ALT) {
        s.push_str("alt+");
    }
    if combo.modifiers.contains(KeyModifiers::SHIFT) {
        s.push_str("shift+");
    }
    match combo.code {
        KeyCode::Char(c) => s.push(c),
        KeyCode::Tab => s.push_str("tab"),
        KeyCode::BackTab => s.push_str("backtab"),
        KeyCode::Enter => s.push_str("enter"),
        KeyCode::Esc => s.push_str("esc"),
        KeyCode::F(n) => {
            s.push('f');
            s.push_str(&n.to_string());
        }
        other => s.push_str(&format!("{other:?}").to_lowercase()),
    }
    s
}

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
        "show_editor_tab" => Some(AppCommand::ShowEditorTab),
        "show_agents_tab" => Some(AppCommand::ShowAgentsTab),
        "toggle_agents_tab" => Some(AppCommand::ToggleAgentsTab),
        // Panels
        "toggle_file_tree" => Some(AppCommand::ToggleFileTree),
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
        "toggle_vim_mode" => Some(AppCommand::ToggleVimMode),
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
        "ai_open_client_picker" => Some(AppCommand::AiOpenClientPicker),
        "ai_close_session" => Some(AppCommand::AiCloseSession),
        "ai_next_session" => Some(AppCommand::AiNextSession),
        "ai_prev_session" => Some(AppCommand::AiPrevSession),
        // Theme
        "next_theme" => Some(AppCommand::NextTheme),
        "prev_theme" => Some(AppCommand::PrevTheme),
        "dismiss_notifications" => Some(AppCommand::DismissNotifications),
        "open_theme_picker" => Some(AppCommand::OpenThemePicker),
        "toggle_markdown_preview" => Some(AppCommand::ToggleMarkdownPreview),
        "toggle_key_hints" => Some(AppCommand::ToggleKeyHints),
        // Agent pane
        "agent_split_auto" => Some(AppCommand::AgentSplitAuto),
        "agent_split_vertical" => Some(AppCommand::AgentSplitVertical),
        "agent_split_horizontal" => Some(AppCommand::AgentSplitHorizontal),
        "agent_close_pane" => Some(AppCommand::AgentClosePane),
        "agent_focus_next" => Some(AppCommand::AgentFocusNext),
        "agent_focus_prev" => Some(AppCommand::AgentFocusPrev),
        "agent_toggle_zoom" => Some(AppCommand::AgentToggleZoom),
        "agent_apply_layout" => Some(AppCommand::AgentApplyLayout),
        "agent_save_layout" => Some(AppCommand::AgentSaveLayout),
        "agent_save_layout_as" => Some(AppCommand::AgentSaveLayoutAs),
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
    fn default_keymap_has_root_tab_switches() {
        let km = Keymap::default_global();
        let editor = key_event(KeyCode::Char('1'), KeyModifiers::CONTROL);
        let agents = key_event(KeyCode::Char('2'), KeyModifiers::CONTROL);
        let toggle = key_event(KeyCode::Char('`'), KeyModifiers::CONTROL);
        assert_eq!(km.lookup(&editor), Some(&AppCommand::ShowEditorTab));
        assert_eq!(km.lookup(&agents), Some(&AppCommand::ShowAgentsTab));
        assert_eq!(km.lookup(&toggle), Some(&AppCommand::ToggleAgentsTab));
    }

    #[test]
    fn default_keymap_keeps_ctrl_n_for_new_file_and_unbinds_ctrl_shift_n_agent_split() {
        let km = Keymap::default_global();
        let ctrl_n = key_event(KeyCode::Char('n'), KeyModifiers::CONTROL);
        let ctrl_shift_n = key_event(
            KeyCode::Char('N'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert_eq!(km.lookup(&ctrl_n), Some(&AppCommand::NewFile));
        assert_eq!(km.lookup(&ctrl_shift_n), None);
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
            vec![KeyCombo::new(KeyCode::F(5), KeyModifiers::NONE)],
            AppCommand::ToggleGitPanel,
        );
        let event = key_event(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(km.lookup(&event), Some(&AppCommand::ToggleGitPanel));
    }

    // ── Chords & leaders ───────────────────────────────────────────────

    #[test]
    fn parse_sequence_single_and_chord() {
        assert_eq!(parse_key_sequence("ctrl+s").unwrap().len(), 1);
        let chord = parse_key_sequence("ctrl+k a").unwrap();
        assert_eq!(
            chord,
            vec![
                KeyCombo::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                KeyCombo::new(KeyCode::Char('a'), KeyModifiers::NONE),
            ]
        );
        assert!(parse_key_sequence("   ").is_none());
    }

    #[test]
    fn resolve_exact_prefix_and_none() {
        let km = Keymap::default_global();
        let ctrl_k = KeyCombo::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        let a = KeyCombo::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let x = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::NONE);
        // Ctrl+K alone is a live leader prefix, not a command.
        assert!(matches!(km.resolve(&[ctrl_k]), KeyMatch::Prefix));
        // Ctrl+K a completes to the AI ask command.
        assert!(matches!(
            km.resolve(&[ctrl_k, a]),
            KeyMatch::Exact(AppCommand::AiAskSelection)
        ));
        // An unrelated key resolves to nothing.
        assert!(matches!(km.resolve(&[x]), KeyMatch::None));
    }

    #[test]
    fn leader_continuations_and_hint() {
        let km = Keymap::default_global();
        let ctrl_k = KeyCombo::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        let keys: Vec<char> = km
            .continuations(&[ctrl_k])
            .into_iter()
            .filter_map(|(combo, _)| match combo.code {
                KeyCode::Char(c) => Some(c),
                _ => None,
            })
            .collect();
        for expected in ['a', 'r', 'c', 's', 'f', 'm', 'n', 'w'] {
            assert!(
                keys.contains(&expected),
                "leader missing `{expected}`: {keys:?}"
            );
        }
        let hint = km.which_key_hint(&[ctrl_k]);
        assert!(hint.contains("a:ask AI"), "hint: {hint}");
        assert!(hint.contains("esc cancel"), "hint: {hint}");
    }

    #[test]
    fn custom_chord_config_round_trips() {
        // A multi-combo binding from a user config compiles, merges, and fires.
        let mut config = KeymapConfig::default();
        config
            .normal
            .insert("ctrl+x ctrl+s".to_owned(), "save".to_owned());
        let mut km = Keymap::new();
        km.merge(&config.compile_normal());

        let ctrl_x = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let ctrl_s = KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(matches!(km.resolve(&[ctrl_x]), KeyMatch::Prefix));
        assert!(matches!(
            km.resolve(&[ctrl_x, ctrl_s]),
            KeyMatch::Exact(AppCommand::Save)
        ));
    }

    #[test]
    fn single_binding_shadows_chord_with_same_prefix() {
        // A key bound both alone and as a chord prefix resolves to the immediate
        // single-key command; the chord is unreachable (documented constraint).
        let mut km = Keymap::new();
        let s = KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        let x = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::NONE);
        km.bind(vec![s], AppCommand::Save);
        km.bind(vec![s, x], AppCommand::SaveAll);
        assert!(matches!(
            km.resolve(&[s]),
            KeyMatch::Exact(AppCommand::Save)
        ));
    }

    #[test]
    fn parse_sequence_rejects_overlong_and_invalid() {
        // Bounded by MAX_SEQUENCE_LEN (5): a 6-combo sequence is rejected.
        assert!(parse_key_sequence("a b c d e f").is_none());
        // A valid combo followed by an unparseable one is rejected whole.
        assert!(parse_key_sequence("ctrl+s nope!").is_none());
    }

    #[test]
    fn ctrl_shift_letters_are_not_bound() {
        // Ctrl+Shift+<letter> is untransmittable in legacy terminals, so the
        // default keymap must not depend on it.
        let km = Keymap::default_global();
        for c in ['A', 'R', 'I', 'F', 'V', 'S', 'W', 'K'] {
            let combo = KeyCombo::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            );
            assert!(
                matches!(km.resolve(&[combo]), KeyMatch::None),
                "Ctrl+Shift+{c} should be unbound"
            );
        }
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
            parse_command("show_editor_tab"),
            Some(AppCommand::ShowEditorTab)
        );
        assert_eq!(
            parse_command("show_agents_tab"),
            Some(AppCommand::ShowAgentsTab)
        );
        assert_eq!(
            parse_command("toggle_file_tree"),
            Some(AppCommand::ToggleFileTree)
        );
        assert_eq!(
            parse_command("toggle_git_panel"),
            Some(AppCommand::ToggleGitPanel)
        );
        assert_eq!(
            parse_command("command_palette"),
            Some(AppCommand::OpenCommandPalette)
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
            compiled.get([KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL)].as_slice()),
            Some(&AppCommand::Save)
        );
        assert_eq!(
            compiled.get([KeyCombo::new(KeyCode::F(5), KeyModifiers::NONE)].as_slice()),
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
        let mut overrides = FxHashMap::default();
        overrides.insert(
            vec![KeyCombo::new(KeyCode::Char('s'), KeyModifiers::CONTROL)],
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
