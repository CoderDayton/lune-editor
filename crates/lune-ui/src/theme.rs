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

use crate::primitives::{Color, Modifier, Style};

// ── Border characters ─────────────────────────────────────────────────

/// A set of Unicode border-drawing characters.
///
/// The default set uses rounded corners which produce a softer look than
/// sharp box-drawing characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderChars {
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
    pub vertical: char,
    pub horizontal: char,
}

impl BorderChars {
    /// Plain border character set: `┌┐└┘│─`.
    #[inline]
    pub const fn plain() -> Self {
        Self {
            top_left: '┌',
            top_right: '┐',
            bottom_left: '└',
            bottom_right: '┘',
            vertical: '│',
            horizontal: '─',
        }
    }
}

impl Default for BorderChars {
    fn default() -> Self {
        Self::plain()
    }
}

// ── Theme ─────────────────────────────────────────────────────────────

/// Centralized design token set for the entire Lune Editor UI.
///
/// Every color, style, and border character used by widgets is stored here.
/// Construct with [`Theme::dark()`] for the built-in dark theme, or build
/// a custom instance for alternative color schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    // ── Borders ───────────────────────────────────────────────────
    /// Unicode border character set.
    pub border_chars: BorderChars,
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

    // ── Live Mode overlay ─────────────────────────────────────────
    /// Background tint for lines in the currently visible change region.
    pub live_change_bg: Color,

    // ── Tab bar ───────────────────────────────────────────────────
    /// Style for the active tab in a focused pane.
    pub tab_active_focused: Style,
    /// Style for the active tab in an unfocused pane.
    pub tab_active_unfocused: Style,
    /// Style for inactive (background) tabs.
    pub tab_inactive: Style,
    /// Style for the live mode hunk-count badge (`●3`) on tabs with changes.
    pub tab_live_badge: Style,

    // ── Status bar ────────────────────────────────────────────────
    /// Style for the mode indicator segment (e.g. NORMAL, INSERT).
    pub status_mode: Style,
    /// Style for informational segments (file path, position).
    pub status_info: Style,
    /// Base background style for the status bar.
    pub status_bg: Style,

    // ── Notifications ─────────────────────────────────────────────
    /// Color for informational notifications.
    pub notif_info: Color,
    /// Color for warning notifications.
    pub notif_warn: Color,
    /// Color for error notifications.
    pub notif_error: Color,

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

    // ── Welcome screen ────────────────────────────────────────────
    /// Style for the welcome screen title.
    pub welcome_title: Style,
    /// Style for the welcome screen body text.
    pub welcome_text: Style,
}

impl Theme {
    /// The built-in dark theme.
    ///
    /// Uses a consistent RGB palette for predictable rendering across
    /// terminals. Inspired by Catppuccin Mocha with a deep blue-gray base.
    ///
    /// All style methods on `ratatui_core::style::Style` are `const fn`
    /// in ratatui-core 0.1, so this constructor is fully const-evaluable.
    pub const fn dark() -> Self {
        // ── Palette ──────────────────────────────────────────────
        let base = Color::Rgb(30, 30, 46); // #1e1e2e  background
        let surface0 = Color::Rgb(49, 50, 68); // #313244  raised surfaces
        let surface1 = Color::Rgb(69, 71, 90); // #45475a  borders, gutters
        let surface2 = Color::Rgb(88, 91, 112); // #585b70  dimmed text
        let subtext0 = Color::Rgb(127, 132, 156); // #7f849c  muted text
        let text = Color::Rgb(205, 214, 244); // #cdd6f4  primary text
        let accent = Color::Rgb(137, 180, 250); // #89b4fa  blue accent
        let green = Color::Rgb(166, 227, 161); // #a6e3a1
        let yellow = Color::Rgb(249, 226, 175); // #f9e2af
        let red = Color::Rgb(243, 139, 168); // #f38ba8
        let mauve = Color::Rgb(203, 166, 247); // #cba6f7
        let teal = Color::Rgb(148, 226, 213); // #94e2d5
        let mantle = Color::Rgb(24, 24, 37); // #181825  status bar bg

        Self {
            // Borders
            border_chars: BorderChars::plain(),
            border_focused: accent,
            border_unfocused: surface1,

            // General UI
            accent,
            bg: base,
            fg: text,
            fg_dim: surface2,
            fg_muted: subtext0,
            selection_bg: surface0,

            // Editor
            editor_cursor_normal: Style::new().fg(base).bg(text),
            editor_cursor_insert: Style::new().fg(text).add_modifier(Modifier::UNDERLINED),
            editor_gutter_active: Style::new().fg(text).add_modifier(Modifier::BOLD),
            editor_gutter_inactive: Style::new().fg(surface1),
            editor_gutter_separator: surface0,

            // File tree
            tree_dir_fg: accent,
            tree_file_fg: text,
            tree_symlink_fg: teal,
            tree_selected_bg: surface0,

            // Git status
            git_added: green,
            git_modified: yellow,
            git_deleted: red,
            git_conflicted: mauve,
            git_renamed: teal,
            git_untracked: subtext0,
            git_ignored: surface1,

            // Diff view
            diff_add_fg: green,
            diff_add_bg: Color::Rgb(26, 46, 26), // dark green tint
            diff_del_fg: red,
            diff_del_bg: Color::Rgb(46, 26, 30), // dark red tint
            diff_hunk_fg: teal,

            // Live Mode overlay
            live_change_bg: Color::Rgb(35, 40, 60),

            // Tab bar
            tab_active_focused: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            tab_active_unfocused: Style::new()
                .fg(text)
                .bg(surface0)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::new().fg(surface2),
            tab_live_badge: Style::new().fg(teal).add_modifier(Modifier::BOLD),

            // Status bar
            status_mode: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_info: Style::new().fg(subtext0),
            status_bg: Style::new().fg(subtext0).bg(mantle),

            // Notifications
            notif_info: green,
            notif_warn: yellow,
            notif_error: red,

            // Overlay / popup
            overlay_border: accent,
            overlay_selected: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            overlay_dir_fg: teal,
            overlay_file_fg: text,
            overlay_hint_fg: surface2,

            // Welcome screen
            welcome_title: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            welcome_text: Style::new().fg(subtext0),
        }
    }

    /// Built-in light theme.
    ///
    /// Uses a consistent RGB palette (Catppuccin Latte-inspired) for
    /// terminals with a light background.
    pub const fn light() -> Self {
        // ── Palette ──────────────────────────────────────────────
        let base = Color::Rgb(239, 241, 245); // #eff1f5  background
        let surface0 = Color::Rgb(204, 208, 218); // #ccd0da  raised surfaces
        let surface1 = Color::Rgb(188, 192, 204); // #bcc0cc  borders
        let surface2 = Color::Rgb(156, 160, 176); // #9ca0b0  dimmed text
        let subtext0 = Color::Rgb(108, 111, 133); // #6c6f85  muted text
        let text = Color::Rgb(76, 79, 105); // #4c4f69  primary text
        let accent = Color::Rgb(30, 102, 245); // #1e66f5  blue accent
        let green = Color::Rgb(64, 160, 43); // #40a02b
        let yellow = Color::Rgb(223, 142, 29); // #df8e1d
        let red = Color::Rgb(210, 15, 57); // #d20f39
        let mauve = Color::Rgb(136, 57, 239); // #8839ef
        let teal = Color::Rgb(23, 146, 153); // #179299
        let mantle = Color::Rgb(230, 233, 239); // #e6e9ef  status bar bg

        Self {
            // Borders
            border_chars: BorderChars::plain(),
            border_focused: accent,
            border_unfocused: surface1,

            // General UI
            accent,
            bg: base,
            fg: text,
            fg_dim: surface2,
            fg_muted: subtext0,
            selection_bg: surface0,

            // Editor
            editor_cursor_normal: Style::new().fg(base).bg(text),
            editor_cursor_insert: Style::new().fg(text).add_modifier(Modifier::UNDERLINED),
            editor_gutter_active: Style::new().fg(text).add_modifier(Modifier::BOLD),
            editor_gutter_inactive: Style::new().fg(surface1),
            editor_gutter_separator: surface0,

            // File tree
            tree_dir_fg: accent,
            tree_file_fg: text,
            tree_symlink_fg: teal,
            tree_selected_bg: surface0,

            // Git status
            git_added: green,
            git_modified: yellow,
            git_deleted: red,
            git_conflicted: mauve,
            git_renamed: teal,
            git_untracked: subtext0,
            git_ignored: surface1,

            // Diff view
            diff_add_fg: green,
            diff_add_bg: Color::Rgb(220, 245, 220),
            diff_del_fg: red,
            diff_del_bg: Color::Rgb(255, 225, 225),
            diff_hunk_fg: teal,

            // Live Mode overlay
            live_change_bg: Color::Rgb(220, 230, 250),

            // Tab bar
            tab_active_focused: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            tab_active_unfocused: Style::new()
                .fg(text)
                .bg(surface0)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::new().fg(surface2),
            tab_live_badge: Style::new().fg(teal).add_modifier(Modifier::BOLD),

            // Status bar
            status_mode: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_info: Style::new().fg(subtext0),
            status_bg: Style::new().fg(subtext0).bg(mantle),

            // Notifications
            notif_info: green,
            notif_warn: yellow,
            notif_error: red,

            // Overlay / popup
            overlay_border: accent,
            overlay_selected: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            overlay_dir_fg: teal,
            overlay_file_fg: text,
            overlay_hint_fg: surface2,

            // Welcome screen
            welcome_title: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            welcome_text: Style::new().fg(subtext0),
        }
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

    #[test]
    fn dark_theme_accent_matches_border_focused() {
        let t = Theme::dark();
        assert_eq!(t.accent, t.border_focused);
    }

    #[test]
    fn border_chars_plain_is_default() {
        assert_eq!(BorderChars::default(), BorderChars::plain());
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
        assert_eq!(t.tab_active_focused.fg, Some(t.accent));
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
    fn const_construction() {
        // Verify both themes can be constructed in a const context.
        const DARK: Theme = Theme::dark();
        const LIGHT: Theme = Theme::light();
        assert_eq!(DARK.accent, Color::Rgb(137, 180, 250));
        assert_eq!(LIGHT.accent, Color::Rgb(30, 102, 245));
    }

    #[test]
    fn theme_is_copy() {
        let a = Theme::dark();
        let b = a; // Copy, not move
        assert_eq!(a, b);
    }
}
