//! UI snapshot tests for all Lune Editor widgets.
//!
//! Each test renders a widget into a fixed-size ratatui `Buffer`, extracts
//! the text content row-by-row, and compares against an `insta` snapshot.
//! Snapshots live in `crates/lune-ui/tests/snapshots/`.
//!
//! Run `cargo insta review` to accept/reject changes after modifying render code.

use std::path::PathBuf;

use lune_core::buffer::TextBuffer;
use lune_core::ports::{FileEntry, FileState, StatusSnapshot};
use lune_core::workspace::{DirEntry, EntryKind, FileStatus};
use lune_git::diff::{DiffHunk, DiffLine, DiffLineKind, FileDiff};

use lune_ui::highlight::theme::SyntaxTheme;
use lune_ui::primitives::{Buffer, Rect};
use lune_ui::theme::Theme;
use lune_ui::vim::VimMode;
use lune_ui::widgets::diff_view::{DiffViewMode, DiffViewState, render_diff_view};
use lune_ui::widgets::editor_pane::{ViewportState, render_editor_pane};
use lune_ui::widgets::file_tree::{FileTreeState, render_file_tree};
use lune_ui::widgets::git_panel::{GitPanelState, render_git_panel};
use lune_ui::widgets::overlay::{OverlayState, render_overlay};
use lune_ui::widgets::status_bar::{StatusLineState, render_status_bar};
use lune_ui::widgets::tab_bar::{TabEntry, TabManager, render_tab_bar};
use throbber_widgets_tui::ThrobberState;

// ── Helpers ───────────────────────────────────────────────────────────

/// Extract the text content from a ratatui `Buffer` as a multi-line string.
///
/// Each row becomes one line in the output. Trailing spaces on each row
/// are preserved to show exact column alignment. This gives a readable
/// snapshot that captures what the user would see (ignoring styles).
fn buffer_to_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);

    for y in area.y..area.y + area.height {
        let mut row = String::with_capacity(area.width as usize);
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell((x, y)) {
                row.push_str(cell.symbol());
            } else {
                row.push(' ');
            }
        }
        // Trim trailing spaces for cleaner snapshots, but preserve meaningful whitespace.
        let trimmed = row.trim_end();
        lines.push(trimmed.to_string());
    }

    lines.join("\n")
}

fn make_buffer_id() -> lune_core::prelude::BufferId {
    lune_core::prelude::BufferId::new()
}

// ── Status bar snapshots ──────────────────────────────────────────────

#[test]
fn snapshot_status_bar_normal_mode() {
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let status = StatusLineState {
        mode: VimMode::Normal,
        file_path: "src/main.rs".to_string(),
        dirty: false,
        cursor_line: 42,
        cursor_col: 15,
        git_branch: "main".to_string(),
        encoding: "UTF-8",
        file_type: "Rust".to_string(),
        ..StatusLineState::default()
    };

    let mut throbber = ThrobberState::default();
    render_status_bar(area, &mut buf, &status, &theme, &mut throbber);
    insta::assert_snapshot!("status_bar_normal", buffer_to_text(&buf));
}

#[test]
fn snapshot_status_bar_insert_dirty() {
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let status = StatusLineState {
        mode: VimMode::Insert,
        file_path: "lib.rs".to_string(),
        dirty: true,
        cursor_line: 100,
        cursor_col: 1,
        git_branch: "feature/tests".to_string(),
        encoding: "UTF-8",
        file_type: "Rust".to_string(),
        ai_status: "AI: Connected".to_string(),
        ..StatusLineState::default()
    };

    let mut throbber = ThrobberState::default();
    render_status_bar(area, &mut buf, &status, &theme, &mut throbber);
    insta::assert_snapshot!("status_bar_insert_dirty", buffer_to_text(&buf));
}

#[test]
fn snapshot_status_bar_with_message() {
    let area = Rect::new(0, 0, 60, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let status = StatusLineState {
        mode: VimMode::Normal,
        file_path: "ignored_because_message_set.rs".to_string(),
        message: "File saved successfully".to_string(),
        cursor_line: 10,
        cursor_col: 5,
        encoding: "UTF-8",
        ..StatusLineState::default()
    };

    let mut throbber = ThrobberState::default();
    render_status_bar(area, &mut buf, &status, &theme, &mut throbber);
    insta::assert_snapshot!("status_bar_with_message", buffer_to_text(&buf));
}

// ── Tab bar snapshots ─────────────────────────────────────────────────

#[test]
fn snapshot_tab_bar_empty() {
    let area = Rect::new(0, 0, 60, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();
    let mgr = TabManager::default();

    render_tab_bar(area, &mut buf, &mgr, true, &theme);
    insta::assert_snapshot!("tab_bar_empty", buffer_to_text(&buf));
}

#[test]
fn snapshot_tab_bar_single_active() {
    let area = Rect::new(0, 0, 60, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut mgr = TabManager::default();
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "main.rs".to_string(),
        dirty: false,
        pinned: false,
    });
    mgr.active_index = 0;

    render_tab_bar(area, &mut buf, &mgr, true, &theme);
    insta::assert_snapshot!("tab_bar_single_active", buffer_to_text(&buf));
}

#[test]
fn snapshot_tab_bar_multiple_tabs() {
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut mgr = TabManager::default();
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "main.rs".to_string(),
        dirty: false,
        pinned: false,
    });
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "lib.rs".to_string(),
        dirty: true,
        pinned: false,
    });
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "utils.rs".to_string(),
        dirty: false,
        pinned: false,
    });
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "config.rs".to_string(),
        dirty: true,
        pinned: false,
    });
    mgr.active_index = 1;

    render_tab_bar(area, &mut buf, &mgr, true, &theme);
    insta::assert_snapshot!("tab_bar_multiple_tabs", buffer_to_text(&buf));
}

#[test]
fn snapshot_tab_bar_unfocused() {
    let area = Rect::new(0, 0, 60, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut mgr = TabManager::default();
    mgr.tabs.push(TabEntry {
        buffer_id: make_buffer_id(),
        title: "test.rs".to_string(),
        dirty: false,
        pinned: false,
    });
    mgr.active_index = 0;

    render_tab_bar(area, &mut buf, &mgr, false, &theme);
    insta::assert_snapshot!("tab_bar_unfocused", buffer_to_text(&buf));
}

// ── Editor pane snapshots ─────────────────────────────────────────────

#[test]
fn snapshot_editor_pane_welcome() {
    // Height 17 keeps the snapshot deterministic — the tip-of-the-day
    // row only renders when area.height >= 18 and rotates daily.
    let area = Rect::new(0, 0, 60, 17);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();
    let syntax_theme = SyntaxTheme::dark();
    let mut viewport = ViewportState::default();

    render_editor_pane(
        area,
        &mut buf,
        None, // No buffer → welcome screen
        &mut viewport,
        true,
        VimMode::Normal,
        None,
        &syntax_theme,
        None,
        None,
        &theme,
        4,
        None,
    );

    insta::assert_snapshot!("editor_pane_welcome", buffer_to_text(&buf));
}

#[test]
fn snapshot_editor_pane_with_content() {
    let area = Rect::new(0, 0, 50, 10);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();
    let syntax_theme = SyntaxTheme::dark();
    let mut viewport = ViewportState::default();

    let text_buf = TextBuffer::from_text("fn main() {\n    println!(\"Hello\");\n}\n");

    render_editor_pane(
        area,
        &mut buf,
        Some(&text_buf),
        &mut viewport,
        true,
        VimMode::Normal,
        None,
        &syntax_theme,
        None,
        None,
        &theme,
        4,
        None,
    );

    insta::assert_snapshot!("editor_pane_with_content", buffer_to_text(&buf));
}

#[test]
fn snapshot_editor_pane_insert_mode() {
    let area = Rect::new(0, 0, 50, 8);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();
    let syntax_theme = SyntaxTheme::dark();
    let mut viewport = ViewportState::default();

    let text_buf = TextBuffer::from_text("line 1\nline 2\nline 3\nline 4\nline 5\n");

    render_editor_pane(
        area,
        &mut buf,
        Some(&text_buf),
        &mut viewport,
        true,
        VimMode::Insert,
        None,
        &syntax_theme,
        None,
        None,
        &theme,
        4,
        None,
    );

    insta::assert_snapshot!("editor_pane_insert_mode", buffer_to_text(&buf));
}

// ── File tree snapshots ───────────────────────────────────────────────

fn make_file_tree_entries() -> Vec<(usize, DirEntry)> {
    vec![
        (
            0,
            DirEntry {
                path: PathBuf::from("/project/src"),
                name: "src".to_string(),
                kind: EntryKind::Directory { expanded: true },
                git_status: None,
            },
        ),
        (
            1,
            DirEntry {
                path: PathBuf::from("/project/src/main.rs"),
                name: "main.rs".to_string(),
                kind: EntryKind::File,
                git_status: Some(FileStatus::Modified),
            },
        ),
        (
            1,
            DirEntry {
                path: PathBuf::from("/project/src/lib.rs"),
                name: "lib.rs".to_string(),
                kind: EntryKind::File,
                git_status: None,
            },
        ),
        (
            1,
            DirEntry {
                path: PathBuf::from("/project/src/utils.rs"),
                name: "utils.rs".to_string(),
                kind: EntryKind::File,
                git_status: Some(FileStatus::Added),
            },
        ),
        (
            0,
            DirEntry {
                path: PathBuf::from("/project/tests"),
                name: "tests".to_string(),
                kind: EntryKind::Directory { expanded: false },
                git_status: None,
            },
        ),
        (
            0,
            DirEntry {
                path: PathBuf::from("/project/Cargo.toml"),
                name: "Cargo.toml".to_string(),
                kind: EntryKind::File,
                git_status: Some(FileStatus::Untracked),
            },
        ),
        (
            0,
            DirEntry {
                path: PathBuf::from("/project/README.md"),
                name: "README.md".to_string(),
                kind: EntryKind::File,
                git_status: None,
            },
        ),
    ]
}

#[test]
fn snapshot_file_tree_focused() {
    let area = Rect::new(0, 0, 30, 10);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = FileTreeState::new();
    state.entries = make_file_tree_entries();
    state.selected = 1; // main.rs selected

    render_file_tree(area, &mut buf, &mut state, "lune-editor", true, &theme);
    insta::assert_snapshot!("file_tree_focused", buffer_to_text(&buf));
}

#[test]
fn snapshot_file_tree_unfocused() {
    let area = Rect::new(0, 0, 30, 10);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = FileTreeState::new();
    state.entries = make_file_tree_entries();
    state.selected = 0; // src directory selected

    render_file_tree(area, &mut buf, &mut state, "lune-editor", false, &theme);
    insta::assert_snapshot!("file_tree_unfocused", buffer_to_text(&buf));
}

#[test]
fn snapshot_file_tree_empty() {
    let area = Rect::new(0, 0, 30, 8);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = FileTreeState::new();

    render_file_tree(area, &mut buf, &mut state, "my-project", true, &theme);
    insta::assert_snapshot!("file_tree_empty", buffer_to_text(&buf));
}

// ── Diff view snapshots ──────────────────────────────────────────────

fn make_test_diff() -> FileDiff {
    FileDiff {
        path: PathBuf::from("src/main.rs"),
        hunks: vec![
            DiffHunk {
                header: "@@ -1,5 +1,6 @@".to_owned(),
                old_start: 1,
                old_count: 5,
                new_start: 1,
                new_count: 6,
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "fn main() {\n".to_owned(),
                        old_lineno: Some(1),
                        new_lineno: Some(1),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Deletion,
                        content: "    println!(\"old\");\n".to_owned(),
                        old_lineno: Some(2),
                        new_lineno: None,
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Addition,
                        content: "    println!(\"new\");\n".to_owned(),
                        old_lineno: None,
                        new_lineno: Some(2),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Addition,
                        content: "    println!(\"extra\");\n".to_owned(),
                        old_lineno: None,
                        new_lineno: Some(3),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "    let x = 42;\n".to_owned(),
                        old_lineno: Some(3),
                        new_lineno: Some(4),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "}\n".to_owned(),
                        old_lineno: Some(4),
                        new_lineno: Some(5),
                        no_newline_eof: false,
                    },
                ],
            },
            DiffHunk {
                header: "@@ -10,3 +11,4 @@".to_owned(),
                old_start: 10,
                old_count: 3,
                new_start: 11,
                new_count: 4,
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "fn helper() {\n".to_owned(),
                        old_lineno: Some(10),
                        new_lineno: Some(11),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Addition,
                        content: "    // TODO: implement\n".to_owned(),
                        old_lineno: None,
                        new_lineno: Some(12),
                        no_newline_eof: false,
                    },
                    DiffLine {
                        kind: DiffLineKind::Context,
                        content: "}\n".to_owned(),
                        old_lineno: Some(11),
                        new_lineno: Some(13),
                        no_newline_eof: false,
                    },
                ],
            },
        ],
    }
}

#[test]
fn snapshot_diff_view_unified() {
    let area = Rect::new(0, 0, 60, 16);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = DiffViewState::default();
    state.set_diff(make_test_diff());

    render_diff_view(area, &mut buf, &state, &theme);
    insta::assert_snapshot!("diff_view_unified", buffer_to_text(&buf));
}

#[test]
fn snapshot_diff_view_side_by_side() {
    let area = Rect::new(0, 0, 80, 16);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = DiffViewState::default();
    state.set_diff(make_test_diff());
    state.mode = DiffViewMode::SideBySide;

    render_diff_view(area, &mut buf, &state, &theme);
    insta::assert_snapshot!("diff_view_side_by_side", buffer_to_text(&buf));
}

#[test]
fn snapshot_diff_view_empty() {
    let area = Rect::new(0, 0, 50, 5);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let state = DiffViewState::default();

    render_diff_view(area, &mut buf, &state, &theme);
    insta::assert_snapshot!("diff_view_empty", buffer_to_text(&buf));
}

// ── Git panel snapshots ──────────────────────────────────────────────

fn make_git_status() -> std::sync::Arc<StatusSnapshot> {
    std::sync::Arc::new(StatusSnapshot {
        branch: Some("main".to_owned()),
        files: vec![
            FileEntry {
                path: PathBuf::from("src/main.rs"),
                state: FileState::Modified,
                staged: true,
            },
            FileEntry {
                path: PathBuf::from("src/new_file.rs"),
                state: FileState::Added,
                staged: true,
            },
            FileEntry {
                path: PathBuf::from("src/lib.rs"),
                state: FileState::Modified,
                staged: false,
            },
            FileEntry {
                path: PathBuf::from("temp.txt"),
                state: FileState::Untracked,
                staged: false,
            },
        ],
        ..Default::default()
    })
}

#[test]
fn snapshot_git_panel_with_changes() {
    let area = Rect::new(0, 0, 35, 12);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = GitPanelState::new();
    state.update_status(make_git_status());

    render_git_panel(area, &mut buf, &mut state, true, &theme);
    insta::assert_snapshot!("git_panel_with_changes", buffer_to_text(&buf));
}

#[test]
fn snapshot_git_panel_unfocused() {
    let area = Rect::new(0, 0, 35, 12);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = GitPanelState::new();
    state.update_status(make_git_status());

    render_git_panel(area, &mut buf, &mut state, false, &theme);
    insta::assert_snapshot!("git_panel_unfocused", buffer_to_text(&buf));
}

#[test]
fn snapshot_git_panel_empty() {
    let area = Rect::new(0, 0, 35, 8);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut state = GitPanelState::new();

    render_git_panel(area, &mut buf, &mut state, true, &theme);
    insta::assert_snapshot!("git_panel_empty", buffer_to_text(&buf));
}

// ── Overlay snapshots ─────────────────────────────────────────────────

#[test]
fn snapshot_overlay_command_palette() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_command_palette();

    render_overlay(area, &mut buf, &mut overlay, &theme);
    insta::assert_snapshot!("overlay_command_palette", buffer_to_text(&buf));
}

#[test]
fn snapshot_overlay_command_palette_filtered() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_command_palette();
    // Type "save" to filter
    overlay.command_palette.type_char('s');
    overlay.command_palette.type_char('a');
    overlay.command_palette.type_char('v');

    render_overlay(area, &mut buf, &mut overlay, &theme);
    insta::assert_snapshot!("overlay_command_palette_filtered", buffer_to_text(&buf));
}

#[test]
fn snapshot_overlay_confirm_dialog() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_confirm(
        "Discard unsaved changes?",
        lune_ui::event::AppCommand::CloseTab,
    );

    render_overlay(area, &mut buf, &mut overlay, &theme);
    insta::assert_snapshot!("overlay_confirm_dialog", buffer_to_text(&buf));
}

#[test]
fn snapshot_overlay_inactive() {
    let area = Rect::new(0, 0, 60, 20);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();

    render_overlay(area, &mut buf, &mut overlay, &theme);
    insta::assert_snapshot!("overlay_inactive", buffer_to_text(&buf));
}

#[test]
fn snapshot_overlay_key_hints() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_key_hints();

    render_overlay(area, &mut buf, &mut overlay, &theme);
    let text = buffer_to_text(&buf);
    assert!(
        text.contains("Keybindings"),
        "key hints overlay must show 'Keybindings' title"
    );
    assert!(
        text.contains("Ctrl+S"),
        "key hints overlay must show Ctrl+S binding"
    );
    assert!(
        text.contains("Save"),
        "key hints overlay must show 'Save' label"
    );
    insta::assert_snapshot!("overlay_key_hints", text);
}

#[test]
fn snapshot_overlay_key_hints_filtered() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_key_hints();
    for ch in "save".chars() {
        overlay.key_hints.push_filter(ch);
    }

    render_overlay(area, &mut buf, &mut overlay, &theme);
    let text = buffer_to_text(&buf);
    assert!(
        text.contains("filter: save"),
        "filtered key hints must show filter query in title"
    );
    assert!(
        text.contains("Save all"),
        "filtered key hints must include 'Save all' match"
    );
    insta::assert_snapshot!("overlay_key_hints_filtered", text);
}

#[test]
fn snapshot_overlay_markdown_preview() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.open_markdown_preview(
        "# Title\n\nBody **bold** and *italic*.\n\n- one\n- two\n".to_string(),
        "README.md".to_string(),
        None,
    );

    render_overlay(area, &mut buf, &mut overlay, &theme);
    let text = buffer_to_text(&buf);
    assert!(
        text.contains("README.md (preview)"),
        "markdown preview must show filename in title"
    );
    assert!(
        text.contains("# Title"),
        "markdown preview must render the heading"
    );
    assert!(
        text.contains("- one"),
        "markdown preview must render list items"
    );
    insta::assert_snapshot!("overlay_markdown_preview", text);
}

#[test]
fn snapshot_overlay_image_loading() {
    // Loading placeholder — no decode dispatched, just an empty state
    // forced into the Loading status with a frame title.
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.image_preview.path = Some(PathBuf::from("/tmp/lune-loading.png"));
    overlay.image_preview.status = lune_ui::widgets::overlay::ImagePreviewStatus::Loading;
    overlay.image_preview.generation = 1;
    overlay.active = Some(lune_ui::widgets::overlay::OverlayKind::ImagePreview);

    render_overlay(area, &mut buf, &mut overlay, &theme);
    let text = buffer_to_text(&buf);
    assert!(
        text.contains("lune-loading.png"),
        "image loading overlay must show filename in title"
    );
    assert!(
        text.contains("Decoding"),
        "image loading overlay must show decoding status"
    );
    insta::assert_snapshot!("overlay_image_loading", text);
}

#[test]
fn snapshot_overlay_image_failed() {
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let mut overlay = OverlayState::default();
    overlay.image_preview.path = Some(PathBuf::from("/tmp/lune-broken.png"));
    overlay.image_preview.error = Some("decode: invalid magic bytes".to_string());
    overlay.image_preview.status = lune_ui::widgets::overlay::ImagePreviewStatus::Failed;
    overlay.image_preview.generation = 1;
    overlay.active = Some(lune_ui::widgets::overlay::OverlayKind::ImagePreview);

    render_overlay(area, &mut buf, &mut overlay, &theme);
    let text = buffer_to_text(&buf);
    assert!(
        text.contains("lune-broken.png"),
        "image failed overlay must show filename in title"
    );
    assert!(
        text.contains("Failed to render image"),
        "image failed overlay must show failure message"
    );
    assert!(
        text.contains("decode: invalid magic bytes"),
        "image failed overlay must include the error detail"
    );
    insta::assert_snapshot!("overlay_image_failed", text);
}

#[test]
fn snapshot_status_bar_ai_busy() {
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::dark();

    let status = StatusLineState {
        mode: VimMode::Normal,
        file_path: "src/main.rs".to_string(),
        dirty: false,
        cursor_line: 10,
        cursor_col: 5,
        git_branch: "main".to_string(),
        encoding: "UTF-8",
        ai_status: "Working".to_string(),
        ai_busy: true,
        ..StatusLineState::default()
    };

    let mut throbber = ThrobberState::default();
    render_status_bar(area, &mut buf, &status, &theme, &mut throbber);
    insta::assert_snapshot!("status_bar_ai_busy", buffer_to_text(&buf));
}

// ── Light theme snapshots ─────────────────────────────────────────────

#[test]
fn snapshot_status_bar_light_theme() {
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    let theme = Theme::light();

    let status = StatusLineState {
        mode: VimMode::Normal,
        file_path: "src/main.rs".to_string(),
        cursor_line: 1,
        cursor_col: 1,
        git_branch: "main".to_string(),
        encoding: "UTF-8",
        file_type: "Rust".to_string(),
        ..StatusLineState::default()
    };

    let mut throbber = ThrobberState::default();
    render_status_bar(area, &mut buf, &status, &theme, &mut throbber);
    insta::assert_snapshot!("status_bar_light_theme", buffer_to_text(&buf));
}

#[test]
fn snapshot_file_tree_light_theme() {
    let area = Rect::new(0, 0, 30, 10);
    let mut buf = Buffer::empty(area);
    let theme = Theme::light();

    let mut state = FileTreeState::new();
    state.entries = make_file_tree_entries();
    state.selected = 2; // lib.rs selected

    render_file_tree(area, &mut buf, &mut state, "lune-editor", true, &theme);
    insta::assert_snapshot!("file_tree_light_theme", buffer_to_text(&buf));
}
