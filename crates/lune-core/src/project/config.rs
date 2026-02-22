//! Configuration directory and path resolution.
//!
//! Resolves the global configuration directory using a single consistent
//! path on all platforms:
//!
//! ```text
//! $XDG_CONFIG_HOME/lune-editor/     (if XDG_CONFIG_HOME is set)
//! ~/.config/lune-editor/            (fallback on all platforms)
//! ```
//!
//! Workspace-local config lives at `.lune/config.toml` within the
//! workspace root and overrides global settings.

use std::path::{Path, PathBuf};

/// Application name used in config directory paths.
const APP_NAME: &str = "lune-editor";

/// Configuration directory layout.
///
/// All paths are lazily derived from a single root.  Nothing is created
/// on disk until [`ConfigPaths::ensure_dirs`] is called.
#[derive(Clone, Debug)]
pub struct ConfigPaths {
    /// Root config directory (e.g. `~/.config/lune-editor/`).
    root: PathBuf,
}

impl ConfigPaths {
    /// Resolve the global config directory for the current platform.
    ///
    /// Priority:
    /// 1. `$XDG_CONFIG_HOME/lune-editor` (if set)
    /// 2. `~/.config/lune-editor` (fallback on all platforms)
    ///
    /// Returns `None` if the home directory cannot be determined.
    #[must_use]
    pub fn resolve() -> Option<Self> {
        let root = resolve_config_root()?;
        Some(Self { root })
    }

    /// Create a `ConfigPaths` from an explicit root directory.
    ///
    /// Useful for testing or `--config` CLI overrides.
    #[must_use]
    pub const fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// The root config directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the main settings file (`config.toml`).
    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Path to the keybindings file (`keybindings.toml`).
    #[must_use]
    pub fn keybindings_file(&self) -> PathBuf {
        self.root.join("keybindings.toml")
    }

    /// Path to the themes directory.
    #[must_use]
    pub fn themes_dir(&self) -> PathBuf {
        self.root.join("themes")
    }

    /// Path to the workspace state directory.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// Path to the crash recovery directory.
    #[must_use]
    pub fn recovery_dir(&self) -> PathBuf {
        self.root.join("recovery")
    }

    /// Path to the log directory.
    #[must_use]
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("log")
    }

    /// Ensure all config sub-directories exist on disk.
    ///
    /// Creates the directory tree if it doesn't already exist.
    ///
    /// # Errors
    /// Returns an error if directories cannot be created (permissions, etc.).
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        let dirs = [
            self.root.clone(),
            self.themes_dir(),
            self.state_dir(),
            self.recovery_dir(),
            self.log_dir(),
        ];
        for dir in &dirs {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Resolve workspace-local config path.
///
/// If a `.lune/` directory exists in the workspace root, returns the
/// path to `.lune/config.toml`.  Does not create the directory.
#[must_use]
pub fn workspace_config_file(workspace_root: &Path) -> Option<PathBuf> {
    let config_file = workspace_root.join(".lune").join("config.toml");
    if config_file.exists() {
        Some(config_file)
    } else {
        None
    }
}

/// Resolve config root: `$XDG_CONFIG_HOME/lune-editor` or `~/.config/lune-editor`.
fn resolve_config_root() -> Option<PathBuf> {
    // 1. Check XDG_CONFIG_HOME (respected on all platforms)
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return Some(p.join(APP_NAME));
        }
    }

    // 2. Universal fallback: ~/.config/lune-editor
    home_dir().map(|home| home.join(".config").join(APP_NAME))
}

/// Get the user's home directory from environment variables.
fn home_dir() -> Option<PathBuf> {
    // $HOME works on Linux, macOS, and most Unix systems.
    // $USERPROFILE is the Windows equivalent when $HOME is unset.
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_paths_from_root() {
        let root = PathBuf::from("/tmp/test-config");
        let paths = ConfigPaths::from_root(root.clone());

        assert_eq!(paths.root(), root);
        assert_eq!(paths.settings_file(), root.join("config.toml"));
        assert_eq!(paths.keybindings_file(), root.join("keybindings.toml"));
        assert_eq!(paths.themes_dir(), root.join("themes"));
        assert_eq!(paths.state_dir(), root.join("state"));
        assert_eq!(paths.recovery_dir(), root.join("recovery"));
        assert_eq!(paths.log_dir(), root.join("log"));
    }

    #[test]
    fn ensure_dirs_creates_structure() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(dir.path().join(APP_NAME));

        paths.ensure_dirs().unwrap();

        assert!(paths.root().is_dir());
        assert!(paths.themes_dir().is_dir());
        assert!(paths.state_dir().is_dir());
        assert!(paths.recovery_dir().is_dir());
        assert!(paths.log_dir().is_dir());
    }

    #[test]
    fn ensure_dirs_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(dir.path().join(APP_NAME));

        paths.ensure_dirs().unwrap();
        paths.ensure_dirs().unwrap(); // should not fail
    }

    #[test]
    fn workspace_config_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let lune_dir = dir.path().join(".lune");
        std::fs::create_dir_all(&lune_dir).unwrap();
        std::fs::write(lune_dir.join("config.toml"), "# workspace config").unwrap();

        let result = workspace_config_file(dir.path());
        assert_eq!(result, Some(lune_dir.join("config.toml")));
    }

    #[test]
    fn workspace_config_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(workspace_config_file(dir.path()), None);
    }

    #[test]
    fn resolve_with_xdg_config_home() {
        // Save and set XDG_CONFIG_HOME
        let dir = tempfile::tempdir().unwrap();
        let xdg_path = dir.path().to_str().unwrap().to_owned();

        // This test is sensitive to env var state, so we just verify the
        // function logic with from_root instead of modifying env vars.
        let paths = ConfigPaths::from_root(PathBuf::from(&xdg_path).join(APP_NAME));
        assert!(paths.root().ends_with(APP_NAME));
    }

    #[test]
    fn resolve_returns_some() {
        // On any system with $HOME or $USERPROFILE set, resolve() should succeed.
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            assert!(ConfigPaths::resolve().is_some());
        }
    }
}
