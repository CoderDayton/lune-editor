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

use ratatui_core::style::{Color, Modifier, Style};

// ── Border characters ─────────────────────────────────────────────────

/// A set of Unicode border-drawing characters.
///
/// The default set uses rounded corners which produce a softer look than
/// sharp box-drawing characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorderChars {
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
    pub vertical: char,
    pub horizontal: char,
}

impl BorderChars {
    /// Rounded border character set: `╭╮╰╯│─`.
    #[inline]
    pub const fn rounded() -> Self {
        Self {
            top_left: '╭',
            top_right: '╮',
            bottom_left: '╰',
            bottom_right: '╯',
            vertical: '│',
            horizontal: '─',
        }
    }
}

impl Default for BorderChars {
    fn default() -> Self {
        Self::rounded()
    }
}

// ── Theme ─────────────────────────────────────────────────────────────

/// Centralized design token set for the entire Lune Editor UI.
///
/// Every color, style, and border character used by widgets is stored here.
/// Construct with [`Theme::dark()`] for the built-in dark theme, or build
/// a custom instance for alternative color schemes.
#[derive(Clone, Debug)]
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
    /// All style methods on `ratatui_core::style::Style` are `const fn`
    /// in ratatui-core 0.1, so this constructor is fully const-evaluable.
    pub const fn dark() -> Self {
        let accent = Color::Rgb(80, 130, 220);

        Self {
            // Borders
            border_chars: BorderChars::rounded(),
            border_focused: accent,
            border_unfocused: Color::DarkGray,

            // General UI
            accent,
            bg: Color::Reset,
            fg: Color::White,
            fg_dim: Color::DarkGray,
            fg_muted: Color::Gray,
            selection_bg: Color::DarkGray,

            // Editor
            editor_cursor_normal: Style::new().add_modifier(Modifier::REVERSED),
            editor_cursor_insert: Style::new()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED),
            editor_gutter_active: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            editor_gutter_inactive: Style::new().fg(Color::DarkGray),
            editor_gutter_separator: Color::DarkGray,

            // File tree
            tree_dir_fg: Color::Blue,
            tree_file_fg: Color::White,
            tree_symlink_fg: Color::Cyan,
            tree_selected_bg: Color::DarkGray,

            // Git status
            git_added: Color::Green,
            git_modified: Color::Yellow,
            git_deleted: Color::Red,
            git_conflicted: Color::Magenta,
            git_renamed: Color::Cyan,
            git_untracked: Color::Gray,
            git_ignored: Color::DarkGray,

            // Diff view
            diff_add_fg: Color::Green,
            diff_add_bg: Color::Rgb(0, 40, 0),
            diff_del_fg: Color::Red,
            diff_del_bg: Color::Rgb(40, 0, 0),
            diff_hunk_fg: Color::Cyan,

            // Live Mode overlay
            live_change_bg: Color::Rgb(30, 40, 60),

            // Tab bar
            tab_active_focused: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            tab_active_unfocused: Style::new()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED),
            tab_inactive: Style::new().fg(Color::DarkGray),

            // Status bar
            status_mode: Style::new()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED),
            status_info: Style::new().add_modifier(Modifier::DIM),
            status_bg: Style::new().add_modifier(Modifier::REVERSED),

            // Notifications
            notif_info: Color::Green,
            notif_warn: Color::Yellow,
            notif_error: Color::Red,

            // Overlay / popup
            overlay_border: Color::White,
            overlay_selected: Style::new()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED),
            overlay_dir_fg: Color::Cyan,
            overlay_file_fg: Color::White,
            overlay_hint_fg: Color::DarkGray,

            // Welcome screen
            welcome_title: Style::new().add_modifier(Modifier::BOLD),
            welcome_text: Style::new().add_modifier(Modifier::DIM),
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
    fn border_chars_rounded_is_default() {
        assert_eq!(BorderChars::default(), BorderChars::rounded());
    }

    #[test]
    fn default_theme_is_dark() {
        let default = Theme::default();
        let dark = Theme::dark();
        // Spot-check a few fields — full equality requires PartialEq on Style.
        assert_eq!(default.accent, dark.accent);
        assert_eq!(default.bg, dark.bg);
        assert_eq!(default.git_added, dark.git_added);
        assert_eq!(default.overlay_border, dark.overlay_border);
    }

    #[test]
    fn editor_cursor_normal_is_reversed() {
        let t = Theme::dark();
        assert!(t
            .editor_cursor_normal
            .add_modifier
            .contains(Modifier::REVERSED));
    }

    #[test]
    fn editor_cursor_insert_is_underlined_white() {
        let t = Theme::dark();
        assert_eq!(t.editor_cursor_insert.fg, Some(Color::White));
        assert!(t
            .editor_cursor_insert
            .add_modifier
            .contains(Modifier::UNDERLINED));
    }

    #[test]
    fn gutter_active_is_bold_white() {
        let t = Theme::dark();
        assert_eq!(t.editor_gutter_active.fg, Some(Color::White));
        assert!(t.editor_gutter_active.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tab_active_focused_uses_accent_color() {
        let t = Theme::dark();
        assert_eq!(t.tab_active_focused.fg, Some(t.accent));
        assert!(t.tab_active_focused.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn status_mode_is_bold_reversed() {
        let t = Theme::dark();
        assert!(t.status_mode.add_modifier.contains(Modifier::BOLD));
        assert!(t.status_mode.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn diff_colors_are_distinct() {
        let t = Theme::dark();
        assert_ne!(t.diff_add_fg, t.diff_del_fg);
        assert_ne!(t.diff_add_bg, t.diff_del_bg);
    }

    #[test]
    fn const_construction() {
        // Verify the theme can be constructed in a const context.
        const THEME: Theme = Theme::dark();
        assert_eq!(THEME.accent, Color::Rgb(80, 130, 220));
    }
}
