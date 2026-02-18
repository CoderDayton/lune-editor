//! Application settings with TOML serialization.
//!
//! The [`Settings`] struct represents the full user configuration.
//! It is loaded from `config.toml`, merged with defaults for any
//! missing fields (via serde's `#[serde(default)]`), and can be
//! layered with workspace-local overrides.
//!
//! # Merge order (highest priority wins)
//!
//! 1. CLI flags
//! 2. Workspace-local `.lune/config.toml`
//! 3. Global `~/.config/lune-editor/config.toml`
//! 4. Compiled-in defaults

use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Settings ──────────────────────────────────────────────────────────

/// Top-level application settings.
///
/// Every field has a serde default so that a minimal (or empty) TOML
/// file is valid.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Editor behaviour.
    pub editor: EditorSettings,
    /// UI layout preferences.
    pub ui: UiSettings,
    /// File tree display configuration.
    pub file_tree: FileTreeSettings,
    /// AI integration settings.
    pub ai: AiSettings,
    /// Active theme name (looked up in the theme registry).
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor: EditorSettings::default(),
            ui: UiSettings::default(),
            file_tree: FileTreeSettings::default(),
            ai: AiSettings::default(),
            theme: "Lune Dark".to_owned(),
        }
    }
}

// ── Editor settings ───────────────────────────────────────────────────

/// Settings that affect editing behaviour.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct EditorSettings {
    /// Number of spaces per tab stop.
    pub tab_size: usize,
    /// Use spaces instead of tab characters.
    pub use_spaces: bool,
    /// Wrap long lines.
    pub word_wrap: bool,
    /// Show line numbers in the gutter.
    pub line_numbers: bool,
    /// Show relative line numbers (vim-style).
    pub relative_line_numbers: bool,
    /// Enable cursor blinking.
    pub cursor_blink: bool,
    /// Autosave interval in seconds (`None` to disable).
    pub auto_save_interval_secs: Option<u64>,
    /// Enable vim keybinding mode.
    pub vim_mode: bool,
    /// Enable mouse input.
    pub mouse_enabled: bool,
    /// Lines to keep above/below cursor when scrolling.
    pub scroll_margin: usize,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            tab_size: 4,
            use_spaces: true,
            word_wrap: false,
            line_numbers: true,
            relative_line_numbers: false,
            cursor_blink: true,
            auto_save_interval_secs: Some(60),
            vim_mode: false,
            mouse_enabled: true,
            scroll_margin: 5,
        }
    }
}

// ── UI settings ───────────────────────────────────────────────────────

/// Settings that affect the overall UI layout.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiSettings {
    /// Show the file tree sidebar on startup.
    pub show_file_tree: bool,
    /// File tree sidebar width as percentage of terminal width.
    pub file_tree_width_pct: u16,
    /// Show the AI panel on startup.
    pub show_ai_panel: bool,
    /// Right panel (AI/Git) width as percentage of terminal width.
    pub right_panel_width_pct: u16,
    /// Enable visual effects (tachyonfx animations).
    pub effects_enabled: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_file_tree: true,
            file_tree_width_pct: 20,
            show_ai_panel: false,
            right_panel_width_pct: 30,
            effects_enabled: true,
        }
    }
}

// ── File tree settings ────────────────────────────────────────────────

/// Settings for the file tree display.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FileTreeSettings {
    /// Indentation size (spaces per nesting level).
    pub indent_size: u16,
    /// Show file/folder icons.
    pub icons: bool,
    /// Sort directories before files.
    pub sort_dirs_first: bool,
    /// Show hidden files (dotfiles) by default.
    pub show_hidden: bool,
}

impl Default for FileTreeSettings {
    fn default() -> Self {
        Self {
            indent_size: 2,
            icons: true,
            sort_dirs_first: true,
            show_hidden: false,
        }
    }
}

// ── AI settings ───────────────────────────────────────────────────────

/// Settings for AI client integration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AiSettings {
    /// Default AI client to use (e.g. `"claude"`, `"copilot"`).
    pub default_client: String,
    /// Enable Live Mode on startup.
    pub live_mode_enabled: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_client: "claude".to_owned(),
            live_mode_enabled: false,
        }
    }
}

// ── Load / Save / Merge ───────────────────────────────────────────────

impl Settings {
    /// Load settings from a TOML file.
    ///
    /// Missing fields are filled with defaults.  If the file does not
    /// exist, returns `Settings::default()`.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let settings: Self = toml::from_str(&content)?;
        Ok(settings)
    }

    /// Save settings to a TOML file.
    ///
    /// Uses atomic write: writes to a temporary file then renames, so
    /// a crash mid-write won't corrupt the file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;

        // Atomic write: write to .tmp then rename
        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, content)?;
        std::fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Merge workspace-local overrides onto this settings.
    ///
    /// Only non-default values from `workspace_settings` replace the
    /// corresponding fields.  This allows a minimal workspace config
    /// that only overrides what it needs.
    pub fn merge_workspace(&mut self, workspace: &Self) {
        let defaults = Self::default();

        // Editor overrides
        merge_if_different(
            &mut self.editor.tab_size,
            workspace.editor.tab_size,
            &defaults.editor.tab_size,
        );
        merge_if_different(
            &mut self.editor.use_spaces,
            workspace.editor.use_spaces,
            &defaults.editor.use_spaces,
        );
        merge_if_different(
            &mut self.editor.word_wrap,
            workspace.editor.word_wrap,
            &defaults.editor.word_wrap,
        );
        merge_if_different(
            &mut self.editor.line_numbers,
            workspace.editor.line_numbers,
            &defaults.editor.line_numbers,
        );
        merge_if_different(
            &mut self.editor.relative_line_numbers,
            workspace.editor.relative_line_numbers,
            &defaults.editor.relative_line_numbers,
        );
        merge_if_different(
            &mut self.editor.cursor_blink,
            workspace.editor.cursor_blink,
            &defaults.editor.cursor_blink,
        );
        merge_if_different(
            &mut self.editor.vim_mode,
            workspace.editor.vim_mode,
            &defaults.editor.vim_mode,
        );
        merge_if_different(
            &mut self.editor.mouse_enabled,
            workspace.editor.mouse_enabled,
            &defaults.editor.mouse_enabled,
        );
        merge_if_different(
            &mut self.editor.scroll_margin,
            workspace.editor.scroll_margin,
            &defaults.editor.scroll_margin,
        );

        // UI overrides
        merge_if_different(
            &mut self.ui.show_file_tree,
            workspace.ui.show_file_tree,
            &defaults.ui.show_file_tree,
        );
        merge_if_different(
            &mut self.ui.file_tree_width_pct,
            workspace.ui.file_tree_width_pct,
            &defaults.ui.file_tree_width_pct,
        );
        merge_if_different(
            &mut self.ui.show_ai_panel,
            workspace.ui.show_ai_panel,
            &defaults.ui.show_ai_panel,
        );
        merge_if_different(
            &mut self.ui.right_panel_width_pct,
            workspace.ui.right_panel_width_pct,
            &defaults.ui.right_panel_width_pct,
        );
        merge_if_different(
            &mut self.ui.effects_enabled,
            workspace.ui.effects_enabled,
            &defaults.ui.effects_enabled,
        );

        // File tree overrides
        merge_if_different(
            &mut self.file_tree.indent_size,
            workspace.file_tree.indent_size,
            &defaults.file_tree.indent_size,
        );
        merge_if_different(
            &mut self.file_tree.icons,
            workspace.file_tree.icons,
            &defaults.file_tree.icons,
        );
        merge_if_different(
            &mut self.file_tree.sort_dirs_first,
            workspace.file_tree.sort_dirs_first,
            &defaults.file_tree.sort_dirs_first,
        );
        merge_if_different(
            &mut self.file_tree.show_hidden,
            workspace.file_tree.show_hidden,
            &defaults.file_tree.show_hidden,
        );

        // Theme override
        if workspace.theme != defaults.theme {
            self.theme.clone_from(&workspace.theme);
        }
    }

    /// Apply CLI flag overrides.
    pub fn apply_cli_overrides(&mut self, overrides: &CliOverrides) {
        if let Some(vim) = overrides.vim_mode {
            self.editor.vim_mode = vim;
        }
        if let Some(effects) = overrides.effects_enabled {
            self.ui.effects_enabled = effects;
        }
        if let Some(ref theme) = overrides.theme {
            self.theme.clone_from(theme);
        }
    }
}

/// CLI flag overrides (highest priority).
///
/// `None` means "not specified on command line" — use config value.
#[derive(Clone, Debug, Default)]
pub struct CliOverrides {
    /// `--vim` / `--no-vim`
    pub vim_mode: Option<bool>,
    /// `--no-effects`
    pub effects_enabled: Option<bool>,
    /// `--theme <name>`
    pub theme: Option<String>,
    /// `--config <path>` (handled at a higher level, not merged here)
    pub config_path: Option<std::path::PathBuf>,
}

/// Merge helper: only override if the workspace value differs from default.
fn merge_if_different<T: PartialEq>(target: &mut T, workspace: T, default: &T) {
    if workspace != *default {
        *target = workspace;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let s = Settings::default();
        assert_eq!(s.editor.tab_size, 4);
        assert!(s.editor.use_spaces);
        assert!(!s.editor.vim_mode);
        assert!(s.editor.mouse_enabled);
        assert_eq!(s.editor.scroll_margin, 5);
        assert!(s.ui.show_file_tree);
        assert!(!s.ui.show_ai_panel);
        assert!(s.ui.effects_enabled);
        assert_eq!(s.theme, "Lune Dark");
    }

    #[test]
    fn roundtrip_toml() {
        let settings = Settings::default();
        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let parsed: Settings = toml::from_str(&toml_str).unwrap();
        assert_eq!(settings, parsed);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let toml_str = r"
[editor]
tab_size = 2
vim_mode = true
";
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.editor.tab_size, 2);
        assert!(settings.editor.vim_mode);
        // Everything else should be default
        assert!(settings.editor.use_spaces);
        assert_eq!(settings.editor.scroll_margin, 5);
        assert_eq!(settings.theme, "Lune Dark");
    }

    #[test]
    fn empty_toml_is_defaults() {
        let settings: Settings = toml::from_str("").unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let result = Settings::load(Path::new("/nonexistent/config.toml")).unwrap();
        assert_eq!(result, Settings::default());
    }

    #[test]
    fn load_and_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut settings = Settings::default();
        settings.editor.tab_size = 8;
        settings.editor.vim_mode = true;
        settings.theme = "Monokai".to_owned();

        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn merge_workspace_overrides_only_non_defaults() {
        let mut global = Settings::default();
        global.editor.tab_size = 4; // same as default
        global.editor.vim_mode = true; // non-default

        let mut workspace = Settings::default();
        workspace.editor.tab_size = 2; // override
                                       // vim_mode is default (false), so it should NOT override global

        global.merge_workspace(&workspace);
        assert_eq!(global.editor.tab_size, 2); // overridden
        assert!(global.editor.vim_mode); // kept from global
    }

    #[test]
    fn merge_workspace_theme_override() {
        let mut global = Settings::default();
        let workspace = Settings {
            theme: "Solarized".to_owned(),
            ..Settings::default()
        };

        global.merge_workspace(&workspace);
        assert_eq!(global.theme, "Solarized");
    }

    #[test]
    fn cli_overrides_take_precedence() {
        let mut settings = Settings::default();
        let overrides = CliOverrides {
            vim_mode: Some(true),
            effects_enabled: Some(false),
            theme: Some("Gruvbox".to_owned()),
            config_path: None,
        };

        settings.apply_cli_overrides(&overrides);
        assert!(settings.editor.vim_mode);
        assert!(!settings.ui.effects_enabled);
        assert_eq!(settings.theme, "Gruvbox");
    }

    #[test]
    fn cli_overrides_none_preserves_settings() {
        let mut settings = Settings::default();
        settings.editor.vim_mode = true;

        let overrides = CliOverrides::default();
        settings.apply_cli_overrides(&overrides);
        assert!(settings.editor.vim_mode); // preserved
    }

    #[test]
    fn save_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let settings = Settings::default();
        settings.save(&path).unwrap();

        // The .tmp file should not remain
        assert!(!dir.path().join("config.toml.tmp").exists());
        assert!(path.exists());
    }

    #[test]
    fn auto_save_interval_optional() {
        let toml_str = r"
[editor]
auto_save_interval_secs = 120
";
        let s: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(s.editor.auto_save_interval_secs, Some(120));

        let toml_str_none = r"
[editor]
";
        let s2: Settings = toml::from_str(toml_str_none).unwrap();
        assert_eq!(s2.editor.auto_save_interval_secs, Some(60)); // default
    }
}
