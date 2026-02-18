//! Integration test: open a file, edit it, save it, verify disk content.

mod common;

use lune_core::buffer::TextBuffer;
use lune_core::position::Position;

use common::TestWorkspace;

#[test]
fn open_edit_save_roundtrip() {
    let ws = TestWorkspace::new();
    ws.write_file("hello.txt", "Hello, World!\nSecond line.\n");

    let mut buf = TextBuffer::from_file(&ws.abs_path("hello.txt")).expect("failed to open file");

    assert_eq!(buf.line_count(), 3); // 2 lines + trailing newline = 3 rope lines
    assert!(!buf.is_dirty());

    // Insert text at beginning of line 1.
    buf.insert(Position::new(1, 0), "Inserted: ");
    assert!(buf.is_dirty());
    assert_eq!(buf.line(1).unwrap().trim_end(), "Inserted: Second line.");

    // Save and verify on disk.
    buf.save().expect("failed to save");
    assert!(!buf.is_dirty());

    let on_disk = ws.read_file("hello.txt");
    assert!(on_disk.contains("Inserted: Second line."));
    assert!(on_disk.contains("Hello, World!"));
}

#[test]
fn open_edit_undo_save_restores_original() {
    let ws = TestWorkspace::new();
    let original = "fn main() {\n    println!(\"hi\");\n}\n";
    ws.write_file("main.rs", original);

    let mut buf = TextBuffer::from_file(&ws.abs_path("main.rs")).expect("failed to open file");

    buf.insert(Position::new(0, 0), "// comment\n");
    assert!(buf.is_dirty());

    buf.undo();
    assert!(!buf.is_dirty());

    buf.save().expect("failed to save");
    let on_disk = ws.read_file("main.rs");
    assert_eq!(on_disk, original);
}

#[test]
fn open_multiple_buffers_independently() {
    let ws = TestWorkspace::new();
    ws.write_file("a.txt", "AAA\n");
    ws.write_file("b.txt", "BBB\n");

    let mut buf_a = TextBuffer::from_file(&ws.abs_path("a.txt")).unwrap();
    let mut buf_b = TextBuffer::from_file(&ws.abs_path("b.txt")).unwrap();

    // Edit buf_a, leave buf_b untouched.
    buf_a.insert(Position::new(0, 3), "aaa");
    assert!(buf_a.is_dirty());
    assert!(!buf_b.is_dirty());

    // Edit buf_b.
    buf_b.insert(Position::new(0, 3), "bbb");
    assert!(buf_b.is_dirty());

    // Save both.
    buf_a.save().unwrap();
    buf_b.save().unwrap();

    assert_eq!(ws.read_file("a.txt").trim(), "AAAaaa");
    assert_eq!(ws.read_file("b.txt").trim(), "BBBbbb");
}

#[test]
fn reload_picks_up_external_changes() {
    let ws = TestWorkspace::new();
    ws.write_file("data.txt", "original\n");

    let mut buf = TextBuffer::from_file(&ws.abs_path("data.txt")).unwrap();
    assert_eq!(buf.text().trim(), "original");

    // Simulate external edit.
    ws.write_file("data.txt", "externally modified\n");

    buf.reload().expect("failed to reload");
    assert_eq!(buf.text().trim(), "externally modified");
    assert!(!buf.is_dirty());
}
