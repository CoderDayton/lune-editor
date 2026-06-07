//! Centralized theme and design token system for the Lune Editor.
//!
//! All UI colors, border characters, and styling tokens are defined here
//! in a single [`Theme`] struct. Widget code should reference `Theme`
//! fields instead of hard-coding color values, making it possible to swap
//! the entire visual identity by replacing the active theme instance.
//!
//! # Usage
//!
//! ```
//! use lune_ui::theme::Theme;
//!
//! let theme = Theme::dark();
//! // Use theme.accent, theme.border_focused, theme.editor_cursor_normal, etc.
//! ```

use crate::primitives::{Color, Style};

mod builtin;

// ── Theme ─────────────────────────────────────────────────────────────

/// Centralized design token set for the entire Lune Editor UI.
///
/// Every color and style used by widgets is stored here.
/// Construct with [`Theme::dark()`] for the built-in dark theme, or build
/// a custom instance for alternative color schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    // ── Borders ───────────────────────────────────────────────────
    /// Border color when the pane has focus.
    pub border_focused: Color,
    /// Border color when the pane does not have focus.
    pub border_unfocused: Color,

    // ── General UI ────────────────────────────────────────────────
    /// Primary accent color used for highlights and active elements.
    pub accent: Color,
    /// Default background color (`Reset` defers to terminal default).
    pub bg: Color,
    /// Primary foreground color.
    pub fg: Color,
    /// Dimmed foreground for de-emphasized text.
    pub fg_dim: Color,
    /// Muted foreground — between normal and dim.
    pub fg_muted: Color,
    /// Background color for visual selections.
    pub selection_bg: Color,

    // ── Editor ────────────────────────────────────────────────────
    /// Cursor style in normal (block) mode.
    pub editor_cursor_normal: Style,
    /// Cursor style in insert (line) mode.
    pub editor_cursor_insert: Style,
    /// Line-number style for the current (active) line.
    pub editor_gutter_active: Style,
    /// Line-number style for non-active lines.
    pub editor_gutter_inactive: Style,
    /// Color of the thin separator between gutter and editor content.
    pub editor_gutter_separator: Color,

    // ── File tree ─────────────────────────────────────────────────
    /// Foreground color for directory entries.
    pub tree_dir_fg: Color,
    /// Foreground color for regular file entries.
    pub tree_file_fg: Color,
    /// Foreground color for symlink entries.
    pub tree_symlink_fg: Color,
    /// Background color for the selected (highlighted) entry.
    pub tree_selected_bg: Color,

    // ── Git status ────────────────────────────────────────────────
    /// Color for newly added files/hunks.
    pub git_added: Color,
    /// Color for modified files/hunks.
    pub git_modified: Color,
    /// Color for deleted files/hunks.
    pub git_deleted: Color,
    /// Color for merge-conflicted files.
    pub git_conflicted: Color,
    /// Color for renamed files.
    pub git_renamed: Color,
    /// Color for untracked files.
    pub git_untracked: Color,
    /// Color for ignored files.
    pub git_ignored: Color,

    // ── Diff view ─────────────────────────────────────────────────
    /// Foreground color for added diff lines.
    pub diff_add_fg: Color,
    /// Background color for added diff lines.
    pub diff_add_bg: Color,
    /// Foreground color for deleted diff lines.
    pub diff_del_fg: Color,
    /// Background color for deleted diff lines.
    pub diff_del_bg: Color,
    /// Foreground color for diff hunk headers (`@@`).
    pub diff_hunk_fg: Color,

    // ── Tab bar ───────────────────────────────────────────────────
    /// Style for the active tab in a focused pane.
    pub tab_active_focused: Style,
    /// Style for the active tab in an unfocused pane.
    pub tab_active_unfocused: Style,
    /// Style for inactive (background) tabs.
    pub tab_inactive: Style,

    // ── Status bar ────────────────────────────────────────────────
    /// Style for the mode indicator segment (e.g. NORMAL, INSERT).
    pub status_mode: Style,
    /// Style for the brand badge shown on the empty welcome bar. Rendered
    /// like the mode badge but in a distinct color so it is not mistaken
    /// for a vim mode.
    pub status_brand: Style,
    /// Style for informational segments (file path, position).
    pub status_info: Style,
    /// Base background style for the status bar.
    pub status_bg: Style,

    // ── Notifications ─────────────────────────────────────────────
    /// Color for success notifications.
    pub notif_success: Color,
    /// Color for informational notifications.
    pub notif_info: Color,
    /// Color for warning notifications.
    pub notif_warn: Color,
    /// Color for error notifications.
    pub notif_error: Color,
    /// Background fill of the toast panel.
    pub notif_bg: Color,
    /// Primary text color inside a toast.
    pub notif_fg: Color,

    // ── Overlay / popup ───────────────────────────────────────────
    /// Border color for overlay panels (command palette, dialogs).
    pub overlay_border: Color,
    /// Style for the currently selected item in an overlay list.
    pub overlay_selected: Style,
    /// Foreground color for directory entries in overlay lists.
    pub overlay_dir_fg: Color,
    /// Foreground color for file entries in overlay lists.
    pub overlay_file_fg: Color,
    /// Foreground color for hint/shortcut text in overlays.
    pub overlay_hint_fg: Color,

    // ── Search highlights ──────────────────────────────────────────────────────
    /// Background for the currently active search match.
    pub search_current_bg: Color,
    /// Background for other (non-current) search matches.
    pub search_match_bg: Color,

    // ── Welcome screen ────────────────────────────────────────────
    /// Style for the welcome screen title.
    pub welcome_title: Style,
    /// Style for the welcome screen body text.
    pub welcome_text: Style,
}

impl Theme {
    /// Dim `color` toward its own background by `amount` (`[0, 1]`).
    ///
    /// Uses [`crate::style::color::dynamic_shade`] so the direction is
    /// chosen relative to the theme's background — dark themes darken
    /// toward black, light themes lighten toward white. Good for hover
    /// states, disabled controls, and secondary text.
    #[must_use]
    pub fn dim(&self, color: Color, amount: f32) -> Color {
        crate::style::color::dynamic_shade(color, self.bg, amount)
    }

    /// Lift `color` away from its own background by `amount` (`[0, 1]`).
    /// Inverse of [`dim`]: dark themes lighten toward white, light themes
    /// darken toward black. Good for active / emphasized states.
    #[must_use]
    pub fn lift(&self, color: Color, amount: f32) -> Color {
        // The opposite direction of dim: if bg is dark, we lighten; if bg
        // is light, we darken. We achieve that by shading toward the
        // complementary extreme of bg.
        use crate::style::color::{relative_luminance, shade};
        let Some(bg_lum) = relative_luminance(self.bg) else {
            return color;
        };
        // If bg luminance > 0.5 the theme is light; shade the color
        // darker (negative factor). Otherwise shade lighter (positive).
        let factor = if bg_lum > 0.5 { -amount } else { amount };
        shade(color, factor)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Modifier;

    #[test]
    fn dark_theme_accent_matches_border_focused() {
        let t = Theme::dark();
        assert_eq!(t.accent, t.border_focused);
    }

    #[test]
    fn default_theme_is_dark() {
        assert_eq!(Theme::default(), Theme::dark());
    }

    #[test]
    fn dark_and_light_differ() {
        assert_ne!(Theme::dark(), Theme::light());
    }

    #[test]
    fn light_theme_accent_matches_border_focused() {
        let t = Theme::light();
        assert_eq!(t.accent, t.border_focused);
    }

    #[test]
    fn editor_cursor_normal_has_explicit_colors() {
        let t = Theme::dark();
        assert!(t.editor_cursor_normal.fg.is_some());
        assert!(t.editor_cursor_normal.bg.is_some());
    }

    #[test]
    fn editor_cursor_insert_is_underlined() {
        let t = Theme::dark();
        assert!(t.editor_cursor_insert.fg.is_some());
        assert!(
            t.editor_cursor_insert
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn gutter_active_is_bold() {
        let t = Theme::dark();
        assert!(t.editor_gutter_active.fg.is_some());
        assert!(t.editor_gutter_active.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tab_active_focused_uses_accent_color() {
        let t = Theme::dark();
        assert_eq!(t.tab_active_focused.bg, Some(t.accent));
        assert!(t.tab_active_focused.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn status_mode_is_bold_with_colors() {
        let t = Theme::dark();
        assert!(t.status_mode.add_modifier.contains(Modifier::BOLD));
        assert!(t.status_mode.fg.is_some());
        assert!(t.status_mode.bg.is_some());
    }

    #[test]
    fn diff_colors_are_distinct() {
        let t = Theme::dark();
        assert_ne!(t.diff_add_fg, t.diff_del_fg);
        assert_ne!(t.diff_add_bg, t.diff_del_bg);
    }

    #[test]
    fn dark_theme_text_meets_wcag_aa() {
        use crate::style::color::contrast_ratio;
        let t = Theme::dark();
        // WCAG 2.0 AA for normal text: ≥ 4.5:1
        let ratio = contrast_ratio(t.fg, t.bg).expect("concrete colors");
        assert!(
            ratio >= 4.5,
            "dark theme body contrast {ratio:.2} is below WCAG AA (4.5)"
        );
        // Accent-on-bg is used for active tabs and headings, same bar.
        let accent_ratio = contrast_ratio(t.accent, t.bg).expect("concrete colors");
        assert!(
            accent_ratio >= 3.0,
            "dark theme accent contrast {accent_ratio:.2} is below WCAG AA large-text (3.0)"
        );
    }

    #[test]
    fn dim_on_dark_theme_darkens_toward_bg() {
        use crate::style::color::relative_luminance;
        let t = Theme::dark();
        let dimmed = t.dim(t.fg, 0.3);
        assert!(
            relative_luminance(dimmed).unwrap() < relative_luminance(t.fg).unwrap(),
            "dim on dark theme should reduce fg luminance"
        );
    }

    #[test]
    fn dim_on_light_theme_lightens_toward_bg() {
        use crate::style::color::relative_luminance;
        let t = Theme::light();
        let dimmed = t.dim(t.fg, 0.3);
        assert!(
            relative_luminance(dimmed).unwrap() > relative_luminance(t.fg).unwrap(),
            "dim on light theme should raise fg luminance toward white bg"
        );
    }

    #[test]
    fn lift_is_inverse_of_dim_direction() {
        use crate::style::color::relative_luminance;
        let dark = Theme::dark();
        // On dark theme, lifting fg should brighten (away from bg).
        let lifted = dark.lift(dark.fg, 0.2);
        assert!(relative_luminance(lifted).unwrap() > relative_luminance(dark.fg).unwrap());

        let light = Theme::light();
        // On light theme, lifting fg should darken (away from bg).
        let lifted = light.lift(light.fg, 0.2);
        assert!(relative_luminance(lifted).unwrap() < relative_luminance(light.fg).unwrap());
    }

    #[test]
    fn light_theme_text_meets_wcag_aa() {
        use crate::style::color::contrast_ratio;
        let t = Theme::light();
        let ratio = contrast_ratio(t.fg, t.bg).expect("concrete colors");
        assert!(
            ratio >= 4.5,
            "light theme body contrast {ratio:.2} is below WCAG AA (4.5)"
        );
        let accent_ratio = contrast_ratio(t.accent, t.bg).expect("concrete colors");
        assert!(
            accent_ratio >= 3.0,
            "light theme accent contrast {accent_ratio:.2} is below WCAG AA large-text (3.0)"
        );
    }

    #[test]
    fn const_construction() {
        // Verify both themes can be constructed in a const context.
        const DARK: Theme = Theme::dark();
        const LIGHT: Theme = Theme::light();
        assert_eq!(DARK.accent, Color::Rgb(131, 166, 214));
        assert_eq!(LIGHT.accent, Color::Rgb(58, 123, 213));
    }

    #[test]
    fn theme_is_copy() {
        let a = Theme::dark();
        let b = a; // Copy, not move
        assert_eq!(a, b);
    }
}
