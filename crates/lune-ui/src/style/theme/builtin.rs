use crate::primitives::{Modifier, Style};
use crate::style::color::hex;

use super::Theme;

impl Theme {
    /// The built-in dark theme.
    ///
    /// A GitHub-dark structure (neutral cool-gray surfaces) carrying
    /// Gruvbox-soft warm content colors (warm off-white text, desaturated
    /// olive/amber/coral accents). Uses a consistent RGB palette for
    /// predictable rendering across terminals.
    ///
    /// All style methods on `ratatui_core::style::Style` are `const fn`
    /// in ratatui-core 0.1, so this constructor is fully const-evaluable.
    pub const fn dark() -> Self {
        // ── Palette ──────────────────────────────────────────────
        let base = hex("#16191e"); // background
        let surface0 = hex("#1f242b"); // raised surfaces
        let surface1 = hex("#2f353d"); // borders, gutters
        let surface2 = hex("#4b5563"); // dimmed text
        let subtext0 = hex("#7d8590"); // muted text
        let text = hex("#d3c6aa"); // primary text (warm off-white)
        let accent = hex("#83a6d6"); // blue accent
        let green = hex("#a7c080");
        let yellow = hex("#dbbc7f");
        let red = hex("#e67e80");
        let mauve = hex("#d699b6");
        let teal = hex("#7fbbb3");
        let mantle = hex("#0f1216"); // status bar bg

        Self {
            // Borders
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
            diff_add_bg: hex("#20271b"), // dark green tint
            diff_del_fg: red,
            diff_del_bg: hex("#2c1f1f"), // dark red tint
            diff_hunk_fg: teal,

            // Tab bar
            tab_active_focused: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            tab_active_unfocused: Style::new()
                .fg(text)
                .bg(surface0)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::new().fg(surface2),

            // Status bar
            status_mode: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_brand: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_info: Style::new().fg(subtext0),
            status_bg: Style::new().fg(subtext0).bg(mantle),

            // Notifications
            notif_success: green,
            notif_info: accent,
            notif_warn: yellow,
            notif_error: red,
            notif_bg: surface0,
            notif_fg: text,

            // Overlay / popup
            overlay_border: accent,
            overlay_selected: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            overlay_dir_fg: teal,
            overlay_file_fg: text,
            overlay_hint_fg: surface2,

            // Search highlights
            search_current_bg: hex("#b58a3f"),
            search_match_bg: hex("#4a4424"),

            // Welcome screen
            welcome_title: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            welcome_text: Style::new().fg(subtext0),
        }
    }

    /// Built-in light theme.
    ///
    /// A GitHub-light "paper" structure with Gruvbox-soft warm accents,
    /// for terminals with a light background. Consistent RGB palette.
    pub const fn light() -> Self {
        // ── Palette ──────────────────────────────────────────────
        let base = hex("#f4f0e6"); // background (warm paper)
        let surface0 = hex("#e6dfcd"); // raised surfaces
        let surface1 = hex("#d8d0bc"); // borders
        let surface2 = hex("#a89f8a"); // dimmed text
        let subtext0 = hex("#6f6957"); // muted text
        let text = hex("#3c3a32"); // primary text
        let accent = hex("#3a7bd5"); // blue accent
        let green = hex("#6c802f");
        let yellow = hex("#b07d2b");
        let red = hex("#c14a3d");
        let mauve = hex("#9a5fb0");
        let teal = hex("#4a8b80");
        let mantle = hex("#eae4d4"); // status bar bg

        Self {
            // Borders
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
            diff_add_bg: hex("#dfe9cf"),
            diff_del_fg: red,
            diff_del_bg: hex("#f3dcd6"),
            diff_hunk_fg: teal,

            // Tab bar
            tab_active_focused: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            tab_active_unfocused: Style::new()
                .fg(text)
                .bg(surface0)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::new().fg(surface2),

            // Status bar
            status_mode: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_brand: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            status_info: Style::new().fg(subtext0),
            status_bg: Style::new().fg(subtext0).bg(mantle),

            // Notifications
            notif_success: green,
            notif_info: accent,
            notif_warn: yellow,
            notif_error: red,
            notif_bg: surface0,
            notif_fg: text,

            // Overlay / popup
            overlay_border: accent,
            overlay_selected: Style::new()
                .fg(base)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            overlay_dir_fg: teal,
            overlay_file_fg: text,
            overlay_hint_fg: surface2,

            // Search highlights
            search_current_bg: hex("#e6c46a"),
            search_match_bg: hex("#ece0b0"),

            // Welcome screen
            welcome_title: Style::new().fg(accent).add_modifier(Modifier::BOLD),
            welcome_text: Style::new().fg(subtext0),
        }
    }
}
