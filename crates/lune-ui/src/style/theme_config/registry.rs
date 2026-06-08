use std::path::Path;

use crate::highlight::theme::SyntaxTheme;
use crate::theme::Theme;

use super::schema::ThemeConfig;

// ── Theme Registry ────────────────────────────────────────────────────

/// Identifies a theme in the [`ThemeRegistry`].
///
/// This is a lightweight `Copy` index — switching themes means changing
/// this value and the next render picks up the new `&Theme` reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeId(pub usize);

/// A named, pre-compiled theme entry.
struct ThemeEntry {
    /// Human-readable name (e.g. `"Lune Dark"`, `"Solarized Light"`).
    name: String,
    /// Compiled UI theme (the `Copy` struct).
    theme: Theme,
    /// Compiled syntax highlighting theme.
    syntax: SyntaxTheme,
}

/// Pre-loaded theme collection for instant switching.
///
/// All themes are compiled (TOML parsed, colors resolved) at load time
/// and stored contiguously.  Switching themes is O(1) — change the
/// active index.
///
/// # Performance
///
/// - Each `Theme` is ~564 bytes (`Copy`, no heap).
/// - 1 000 themes ≈ 550 KB — fits in L2 cache.
/// - `current_theme()` / `current_syntax()` return references — zero
///   allocation per frame.
pub struct ThemeRegistry {
    themes: Vec<ThemeEntry>,
    active: usize,
}

impl ThemeRegistry {
    /// Create a registry pre-loaded with the built-in dark and light themes.
    #[must_use]
    pub fn new() -> Self {
        let themes = vec![
            ThemeEntry {
                name: "Lune Dark".to_owned(),
                theme: Theme::dark(),
                syntax: SyntaxTheme::dark(),
            },
            ThemeEntry {
                name: "Lune Light".to_owned(),
                theme: Theme::light(),
                syntax: SyntaxTheme::light(),
            },
        ];
        Self { themes, active: 0 }
    }

    /// Number of loaded themes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.themes.len()
    }

    /// Whether the registry is empty (should never be in practice).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Add a theme from a compiled [`ThemeConfig`].
    ///
    /// Returns the assigned [`ThemeId`].
    pub fn add(&mut self, config: &ThemeConfig) -> ThemeId {
        let (theme, syntax) = config.compile();
        let id = ThemeId(self.themes.len());
        self.themes.push(ThemeEntry {
            name: config.name.clone(),
            theme,
            syntax,
        });
        id
    }

    /// Load all `.toml` theme files from a directory.
    ///
    /// Files that fail to parse are logged and skipped.  Returns the
    /// number of themes successfully loaded.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut count = 0;
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            match ThemeConfig::load(&path) {
                Ok(config) => {
                    // Deduplicate by name — later file wins.
                    let name = config.name.clone();
                    if let Some(existing) = self.themes.iter_mut().find(|e| e.name == name) {
                        let (theme, syntax) = config.compile();
                        existing.theme = theme;
                        existing.syntax = syntax;
                    } else {
                        self.add(&config);
                    }
                    count += 1;
                }
                Err(e) => {
                    log::warn!("Failed to load theme {}: {e}", path.display());
                }
            }
        }
        count
    }

    /// Switch to a theme by ID.
    ///
    /// Returns `true` if the switch succeeded (valid ID).
    pub const fn switch(&mut self, id: ThemeId) -> bool {
        if id.0 < self.themes.len() {
            self.active = id.0;
            true
        } else {
            false
        }
    }

    /// Switch to the next theme in the list, wrapping around.
    pub const fn next(&mut self) {
        if !self.themes.is_empty() {
            self.active = (self.active + 1) % self.themes.len();
        }
    }

    /// Switch to the previous theme in the list, wrapping around.
    pub fn prev(&mut self) {
        if !self.themes.is_empty() {
            self.active = self.active.checked_sub(1).unwrap_or(self.themes.len() - 1);
        }
    }

    /// Switch to a theme by name (case-insensitive).
    ///
    /// Returns `true` if a theme with that name was found.
    pub fn switch_by_name(&mut self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if let Some(idx) = self
            .themes
            .iter()
            .position(|e| e.name.to_ascii_lowercase() == lower)
        {
            self.active = idx;
            true
        } else {
            false
        }
    }

    /// The currently active [`ThemeId`].
    #[must_use]
    pub const fn active_id(&self) -> ThemeId {
        ThemeId(self.active)
    }

    /// The currently active UI theme (zero-cost reference).
    #[must_use]
    pub fn current_theme(&self) -> &Theme {
        &self.themes[self.active].theme
    }

    /// The currently active syntax theme (zero-cost reference).
    #[must_use]
    pub fn current_syntax(&self) -> &SyntaxTheme {
        &self.themes[self.active].syntax
    }

    /// The display name of the currently active theme.
    #[must_use]
    pub fn current_name(&self) -> &str {
        &self.themes[self.active].name
    }

    /// List all loaded themes as `(ThemeId, name)` pairs.
    #[must_use]
    pub fn list(&self) -> Vec<(ThemeId, &str)> {
        self.themes
            .iter()
            .enumerate()
            .map(|(i, e)| (ThemeId(i), e.name.as_str()))
            .collect()
    }

    /// Find a theme ID by name (case-insensitive).
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<ThemeId> {
        let lower = name.to_ascii_lowercase();
        self.themes
            .iter()
            .position(|e| e.name.to_ascii_lowercase() == lower)
            .map(ThemeId)
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Color;

    #[test]
    fn registry_new_has_builtins() {
        let reg = ThemeRegistry::new();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.current_name(), "Lune Dark");
        assert_eq!(*reg.current_theme(), Theme::dark());
    }

    #[test]
    fn registry_switch_by_id() {
        let mut reg = ThemeRegistry::new();
        assert!(reg.switch(ThemeId(1)));
        assert_eq!(reg.current_name(), "Lune Light");
        assert_eq!(*reg.current_theme(), Theme::light());
    }

    #[test]
    fn registry_switch_invalid_id() {
        let mut reg = ThemeRegistry::new();
        assert!(!reg.switch(ThemeId(999)));
        assert_eq!(reg.active_id(), ThemeId(0));
    }

    #[test]
    fn registry_switch_by_name() {
        let mut reg = ThemeRegistry::new();
        assert!(reg.switch_by_name("lune light"));
        assert_eq!(reg.active_id(), ThemeId(1));
    }

    #[test]
    fn registry_next_prev_wrap() {
        let mut reg = ThemeRegistry::new();
        reg.next();
        assert_eq!(reg.active_id(), ThemeId(1));
        reg.next();
        assert_eq!(reg.active_id(), ThemeId(0)); // wrapped
        reg.prev();
        assert_eq!(reg.active_id(), ThemeId(1)); // wrapped backwards
    }

    #[test]
    fn registry_add_custom_theme() {
        let mut reg = ThemeRegistry::new();
        let config: ThemeConfig = toml::from_str(
            r##"
name = "Custom"
base = "dark"
[colors]
accent = "#FF0000"
"##,
        )
        .unwrap();
        let id = reg.add(&config);
        assert_eq!(reg.len(), 3);
        assert!(reg.switch(id));
        assert_eq!(reg.current_theme().accent, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn registry_list() {
        let reg = ThemeRegistry::new();
        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], (ThemeId(0), "Lune Dark"));
        assert_eq!(list[1], (ThemeId(1), "Lune Light"));
    }

    #[test]
    fn registry_find_by_name() {
        let reg = ThemeRegistry::new();
        assert_eq!(reg.find_by_name("Lune Dark"), Some(ThemeId(0)));
        assert_eq!(reg.find_by_name("lune dark"), Some(ThemeId(0)));
        assert_eq!(reg.find_by_name("nonexistent"), None);
    }

    #[test]
    fn registry_load_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = ThemeRegistry::new();
        assert_eq!(reg.load_dir(dir.path()), 0);
        assert_eq!(reg.len(), 2); // still just builtins
    }

    #[test]
    fn registry_load_dir_with_theme_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("monokai.toml"),
            r##"
name = "Monokai"
base = "dark"
[colors]
accent = "#F92672"
"##,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("solarized.toml"),
            r##"
name = "Solarized Light"
base = "light"
[colors]
accent = "#268BD2"
"##,
        )
        .unwrap();
        // Non-toml files should be ignored
        std::fs::write(dir.path().join("readme.md"), "not a theme").unwrap();

        let mut reg = ThemeRegistry::new();
        let loaded = reg.load_dir(dir.path());
        assert_eq!(loaded, 2);
        assert_eq!(reg.len(), 4); // 2 builtin + 2 loaded

        assert!(reg.switch_by_name("Monokai"));
        assert_eq!(reg.current_theme().accent, Color::Rgb(249, 38, 114));
    }

    #[test]
    fn registry_load_dir_deduplicates_by_name() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file that overrides the built-in "Lune Dark"
        std::fs::write(
            dir.path().join("override.toml"),
            r##"
name = "Lune Dark"
base = "dark"
[colors]
accent = "#ABCDEF"
"##,
        )
        .unwrap();

        let mut reg = ThemeRegistry::new();
        let loaded = reg.load_dir(dir.path());
        assert_eq!(loaded, 1);
        assert_eq!(reg.len(), 2); // deduped, not 3

        assert!(reg.switch_by_name("Lune Dark"));
        assert_eq!(reg.current_theme().accent, Color::Rgb(0xAB, 0xCD, 0xEF));
    }
}
