//! Property-based tests for Settings serialization round-trip.
//!
//! Key invariant: `deserialize(serialize(s)) == s` for any `Settings`.

use proptest::prelude::*;

use lune_core::settings::*;

// ── Strategy ──────────────────────────────────────────────────────────

/// Generate arbitrary `EditorSettings`.
fn arb_editor_settings() -> impl Strategy<Value = EditorSettings> {
    (
        (1usize..16),  // tab_size
        any::<bool>(), // use_spaces
        any::<bool>(), // word_wrap
        any::<bool>(), // line_numbers
        any::<bool>(), // relative_line_numbers
        any::<bool>(), // cursor_blink
        // NOTE: TOML has no null; absent keys deserialize as the serde
        // default (Some(60)), so None cannot roundtrip.  Always generate Some.
        (1u64..3600).prop_map(Some), // auto_save_interval_secs
        any::<bool>(),               // vim_mode
        any::<bool>(),               // mouse_enabled
        (0usize..20),                // scroll_margin
    )
        .prop_map(
            |(
                tab_size,
                use_spaces,
                word_wrap,
                line_numbers,
                relative_line_numbers,
                cursor_blink,
                auto_save_interval_secs,
                vim_mode,
                mouse_enabled,
                scroll_margin,
            )| {
                EditorSettings {
                    tab_size,
                    use_spaces,
                    word_wrap,
                    line_numbers,
                    relative_line_numbers,
                    cursor_blink,
                    auto_save_interval_secs,
                    vim_mode,
                    mouse_enabled,
                    scroll_margin,
                }
            },
        )
}

/// Generate arbitrary `UiSettings`.
fn arb_ui_settings() -> impl Strategy<Value = UiSettings> {
    (
        any::<bool>(), // show_file_tree
        (10u16..50),   // file_tree_width_pct
        any::<bool>(), // show_ai_panel
        (10u16..50),   // right_panel_width_pct
    )
        .prop_map(
            |(show_file_tree, file_tree_width_pct, show_ai_panel, right_panel_width_pct)| {
                UiSettings {
                    show_file_tree,
                    file_tree_width_pct,
                    show_ai_panel,
                    right_panel_width_pct,
                }
            },
        )
}

/// Generate arbitrary `FileTreeSettings`.
fn arb_file_tree_settings() -> impl Strategy<Value = FileTreeSettings> {
    (
        (1u16..8),     // indent_size
        any::<bool>(), // icons
        any::<bool>(), // sort_dirs_first
        any::<bool>(), // show_hidden
    )
        .prop_map(
            |(indent_size, icons, sort_dirs_first, show_hidden)| FileTreeSettings {
                indent_size,
                icons,
                sort_dirs_first,
                show_hidden,
            },
        )
}

/// Generate arbitrary `AiSettings`.
fn arb_ai_settings() -> impl Strategy<Value = AiSettings> {
    prop::string::string_regex("[a-z]{3,10}")
        .unwrap()
        .prop_map(|default_client| AiSettings { default_client })
}

/// Generate arbitrary `Settings` by composing sub-strategies.
fn arb_settings() -> impl Strategy<Value = Settings> {
    (
        arb_editor_settings(),
        arb_ui_settings(),
        arb_file_tree_settings(),
        arb_ai_settings(),
        prop::string::string_regex("[A-Za-z ]{3,20}").unwrap(),
    )
        .prop_map(|(editor, ui, file_tree, ai, theme)| Settings {
            editor,
            ui,
            file_tree,
            ai,
            theme,
        })
}

// ── Properties ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn toml_roundtrip(settings in arb_settings()) {
        let toml_str = toml::to_string_pretty(&settings)
            .expect("Failed to serialise Settings to TOML");
        let deserialized: Settings = toml::from_str(&toml_str)
            .expect("Failed to deserialise Settings from TOML");
        prop_assert_eq!(
            &settings,
            &deserialized,
            "TOML round-trip failed"
        );
    }

    #[test]
    fn merge_defaults_is_noop(settings in arb_settings()) {
        let mut merged = settings.clone();
        merged.merge_workspace(&Settings::default());
        prop_assert_eq!(
            &settings,
            &merged,
            "Merging defaults should not change settings"
        );
    }
}
