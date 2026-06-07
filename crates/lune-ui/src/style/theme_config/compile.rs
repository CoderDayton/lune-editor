use std::path::Path;

use crate::highlight::theme::SyntaxTheme;
use crate::primitives::{Color, Style};
use crate::theme::Theme;

use super::schema::{StyleDef, ThemeConfig};

// ── ThemeConfig → Theme conversion ────────────────────────────────────

impl ThemeConfig {
    /// Convert this TOML config into a compiled [`Theme`] and [`SyntaxTheme`].
    ///
    /// Starts from the named `base` theme and applies overrides for any
    /// fields present in the config. This is the only allocation-heavy
    /// path — called once at load, never in the render loop.
    pub fn compile(&self) -> (Theme, SyntaxTheme) {
        let mut t = match self.base.as_str() {
            "light" => Theme::light(),
            _ => Theme::dark(),
        };

        let base_syntax = match self.base.as_str() {
            "light" => SyntaxTheme::light(),
            _ => SyntaxTheme::dark(),
        };

        // ── General colors ───────────────────────────────────────
        apply_color(self.colors.accent.as_ref(), &mut t.accent);
        apply_color(self.colors.bg.as_ref(), &mut t.bg);
        apply_color(self.colors.fg.as_ref(), &mut t.fg);
        apply_color(self.colors.fg_dim.as_ref(), &mut t.fg_dim);
        apply_color(self.colors.fg_muted.as_ref(), &mut t.fg_muted);
        apply_color(self.colors.selection_bg.as_ref(), &mut t.selection_bg);
        apply_color(self.colors.border_focused.as_ref(), &mut t.border_focused);
        apply_color(
            self.colors.border_unfocused.as_ref(),
            &mut t.border_unfocused,
        );

        // ── Editor ───────────────────────────────────────────────
        apply_style(
            self.editor.cursor_normal.as_ref(),
            &mut t.editor_cursor_normal,
        );
        apply_style(
            self.editor.cursor_insert.as_ref(),
            &mut t.editor_cursor_insert,
        );
        apply_style(
            self.editor.gutter_active.as_ref(),
            &mut t.editor_gutter_active,
        );
        apply_style(
            self.editor.gutter_inactive.as_ref(),
            &mut t.editor_gutter_inactive,
        );
        apply_color(
            self.editor.gutter_separator.as_ref(),
            &mut t.editor_gutter_separator,
        );

        // ── File tree ────────────────────────────────────────────
        apply_color(self.file_tree.dir_fg.as_ref(), &mut t.tree_dir_fg);
        apply_color(self.file_tree.file_fg.as_ref(), &mut t.tree_file_fg);
        apply_color(self.file_tree.symlink_fg.as_ref(), &mut t.tree_symlink_fg);
        apply_color(self.file_tree.selected_bg.as_ref(), &mut t.tree_selected_bg);

        // ── Git ──────────────────────────────────────────────────
        apply_color(self.git.added.as_ref(), &mut t.git_added);
        apply_color(self.git.modified.as_ref(), &mut t.git_modified);
        apply_color(self.git.deleted.as_ref(), &mut t.git_deleted);
        apply_color(self.git.conflicted.as_ref(), &mut t.git_conflicted);
        apply_color(self.git.renamed.as_ref(), &mut t.git_renamed);
        apply_color(self.git.untracked.as_ref(), &mut t.git_untracked);
        apply_color(self.git.ignored.as_ref(), &mut t.git_ignored);

        // ── Diff ─────────────────────────────────────────────────
        apply_color(self.diff.add_fg.as_ref(), &mut t.diff_add_fg);
        apply_color(self.diff.add_bg.as_ref(), &mut t.diff_add_bg);
        apply_color(self.diff.del_fg.as_ref(), &mut t.diff_del_fg);
        apply_color(self.diff.del_bg.as_ref(), &mut t.diff_del_bg);
        apply_color(self.diff.hunk_fg.as_ref(), &mut t.diff_hunk_fg);

        // ── Tabs ─────────────────────────────────────────────────
        apply_style(self.tabs.active_focused.as_ref(), &mut t.tab_active_focused);
        apply_style(
            self.tabs.active_unfocused.as_ref(),
            &mut t.tab_active_unfocused,
        );
        apply_style(self.tabs.inactive.as_ref(), &mut t.tab_inactive);

        // ── Status bar ───────────────────────────────────────────
        apply_style(self.status_bar.mode.as_ref(), &mut t.status_mode);
        apply_style(self.status_bar.brand.as_ref(), &mut t.status_brand);
        apply_style(self.status_bar.info.as_ref(), &mut t.status_info);
        apply_style(self.status_bar.bg.as_ref(), &mut t.status_bg);

        // ── Notifications ────────────────────────────────────────
        apply_color(self.notifications.success.as_ref(), &mut t.notif_success);
        apply_color(self.notifications.info.as_ref(), &mut t.notif_info);
        apply_color(self.notifications.warn.as_ref(), &mut t.notif_warn);
        apply_color(self.notifications.error.as_ref(), &mut t.notif_error);
        apply_color(self.notifications.bg.as_ref(), &mut t.notif_bg);
        apply_color(self.notifications.fg.as_ref(), &mut t.notif_fg);

        // ── Overlay ──────────────────────────────────────────────
        apply_color(self.overlay.border.as_ref(), &mut t.overlay_border);
        apply_style(self.overlay.selected.as_ref(), &mut t.overlay_selected);
        apply_color(self.overlay.dir_fg.as_ref(), &mut t.overlay_dir_fg);
        apply_color(self.overlay.file_fg.as_ref(), &mut t.overlay_file_fg);
        apply_color(self.overlay.hint_fg.as_ref(), &mut t.overlay_hint_fg);

        // ── Welcome ──────────────────────────────────────────────
        apply_style(self.welcome.title.as_ref(), &mut t.welcome_title);
        apply_style(self.welcome.text.as_ref(), &mut t.welcome_text);

        // ── Syntax ───────────────────────────────────────────────
        let syntax = self.syntax.apply_to(&base_syntax);

        (t, syntax)
    }

    /// Load a theme config from a TOML file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

/// Apply an optional color string override to a target `Color`.
fn apply_color(src: Option<&String>, target: &mut Color) {
    if let Some(s) = src {
        if let Some(c) = crate::style::color::parse_color(s) {
            *target = c;
        } else {
            log::warn!("theme: unrecognized color {s:?}, keeping default");
        }
    }
}

/// Apply an optional `StyleDef` override to a target `Style`.
fn apply_style(src: Option<&StyleDef>, target: &mut Style) {
    if let Some(sdef) = src {
        // Merge, don't replace: only the fields the override actually sets
        // take effect. An override of just `fg` keeps the base `bg`, and
        // modifiers are added to (not swapped for) the base ones. This is
        // ratatui's `patch` semantics, so partial theme overrides compose
        // instead of silently wiping unspecified attributes.
        *target = target.patch(sdef.to_style());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Color, Modifier};
    use crate::theme::Theme;
    use lune_core::highlight::HighlightStyle;

    #[test]
    fn theme_config_roundtrip_toml() {
        let toml_str = r##"
name = "Test Theme"
base = "dark"

[colors]
accent = "#50C878"
fg = "white"

[syntax]
keyword = { fg = "#569CD6", modifiers = "bold" }
string = { fg = "#CE9178" }
"##;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "Test Theme");
        assert_eq!(config.base, "dark");
        assert_eq!(config.colors.accent.as_deref(), Some("#50C878"));

        let (theme, syntax) = config.compile();
        assert_eq!(theme.accent, Color::Rgb(80, 200, 120));
        assert_eq!(theme.fg, Color::White);
        // Syntax keyword should be overridden
        let kw = syntax.resolve(HighlightStyle::Keyword);
        assert_eq!(kw.fg, Some(Color::Rgb(86, 156, 214)));
        assert!(kw.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn theme_config_partial_override() {
        let toml_str = r##"
name = "Partial"
base = "dark"

[colors]
accent = "#FF0000"
"##;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        let (theme, _) = config.compile();
        assert_eq!(theme.accent, Color::Rgb(255, 0, 0));
        // Non-overridden fields should match the dark base
        assert_eq!(theme.fg, Theme::dark().fg);
        assert_eq!(theme.diff_add_fg, Theme::dark().diff_add_fg);
    }

    #[test]
    fn style_override_merges_onto_base() {
        // Setting only `fg` on a status-bar style must keep the base `bg`
        // (and base modifiers), rather than wiping them — see `apply_style`.
        let toml_str = r##"
name = "Merge"
base = "dark"

[status_bar]
bg = { fg = "#FF0000" }
mode = { modifiers = "italic" }
"##;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        let (theme, _) = config.compile();
        let base = Theme::dark();

        // `bg` override set only fg: fg changes, base bg is preserved.
        assert_eq!(theme.status_bg.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(theme.status_bg.bg, base.status_bg.bg);

        // `mode` override added italic on top of the base bold + colors.
        assert_eq!(theme.status_mode.fg, base.status_mode.fg);
        assert_eq!(theme.status_mode.bg, base.status_mode.bg);
        assert!(theme.status_mode.add_modifier.contains(Modifier::BOLD));
        assert!(theme.status_mode.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn theme_config_light_base() {
        let toml_str = r#"
name = "Custom Light"
base = "light"
"#;
        let config: ThemeConfig = toml::from_str(toml_str).unwrap();
        let (theme, _) = config.compile();
        assert_eq!(theme, Theme::light());
    }
}
