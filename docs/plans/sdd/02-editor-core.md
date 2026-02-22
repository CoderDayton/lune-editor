# 02 — Editor Core

> **Phase:** 1 (Foundation)
> **Estimated effort:** 3–4 sessions (~8–12 hours)
> **Prerequisites:** [01-project-scaffold.md](01-project-scaffold.md)

## Goal

Implement the `lune-core` crate: the text buffer model, cursor/selection management, undo/redo transaction system, search/replace engine, and the buffer registry. This crate has **zero UI dependencies** — it is pure data + logic.

---

## Types & Structures

### Buffer Identity

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferId(pub Uuid);

impl BufferId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}
```

### Cursor & Selection

```rust
#[derive(Clone, Debug)]
pub struct Position {
    pub line: usize,   // 0-based line index
    pub col: usize,    // 0-based byte offset within line (grapheme-aware later)
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub anchor: Position,  // where selection started
    pub head: Position,    // where cursor currently is
}

impl Selection {
    pub fn is_cursor(&self) -> bool { self.anchor == self.head }
    pub fn ordered(&self) -> (Position, Position) { /* min, max */ }
}

#[derive(Clone, Debug)]
pub struct CursorState {
    pub primary: Selection,
    pub secondary: Vec<Selection>,  // multi-cursor (future)
}
```

### Undo/Redo

```rust
pub type RevisionId = u64;

#[derive(Clone, Debug)]
pub enum EditOp {
    Insert { pos: Position, text: String },
    Delete { range: (Position, Position), deleted_text: String },
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub revision: RevisionId,
    pub ops: Vec<EditOp>,
    pub cursor_before: CursorState,
    pub cursor_after: CursorState,
}

pub struct UndoStack {
    entries: Vec<Transaction>,
    max_entries: usize,
}
```

### TextBuffer

```rust
pub struct TextBuffer {
    pub id: BufferId,
    rope: Rope,
    pub file_path: Option<PathBuf>,
    pub cursor: CursorState,
    undo_stack: UndoStack,
    redo_stack: UndoStack,
    current_revision: RevisionId,
    last_saved_revision: RevisionId,
    dirty: bool,
}
```

### BufferRegistry

```rust
pub struct BufferRegistry {
    buffers: HashMap<BufferId, TextBuffer>,
    path_index: HashMap<PathBuf, BufferId>,
}
```

### Search

```rust
pub struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub matches: Vec<(Position, Position)>,
    pub current_match: Option<usize>,
}
```

---

## Implementation Steps

### Step 1: Position and Selection types

1. Create `crates/lune-core/src/position.rs` with `Position`, `Selection`, `CursorState`.
2. Implement `Ord`, `PartialOrd` for `Position` (line-major ordering).
3. Implement `Selection::ordered()`, `Selection::is_cursor()`, `Selection::contains(pos)`.
4. **Tests:** ordering, contains checks, cursor vs. range selection.

### Step 2: BufferId and basic TextBuffer

1. Create `crates/lune-core/src/buffer.rs`.
2. Implement `TextBuffer::new()` (empty), `TextBuffer::from_str()`, `TextBuffer::from_file()`.
3. Implement read accessors: `line_count()`, `line(n)`, `text_range()`, `char_at()`.
4. Wrap `ropey::Rope` — convert between rope positions and `Position` (line, col).
5. **Tests:** create buffer, read lines, check line count, boundary conditions (empty buffer, single-line, trailing newline).

### Step 3: Edit operations

1. Implement `TextBuffer::insert(pos, text)` — insert text at position, return `EditOp`.
2. Implement `TextBuffer::delete(range)` — delete range, capture deleted text, return `EditOp`.
3. Implement `TextBuffer::replace(range, text)` — delete + insert as compound op.
4. All mutations update `dirty` flag and increment `current_revision`.
5. **Tests:** insert at start/middle/end, delete single char/range/multi-line, replace with longer/shorter text.

### Step 4: Undo/Redo

1. Create `crates/lune-core/src/undo.rs` with `UndoStack`, `Transaction`, `EditOp`.
2. Each edit operation pushes a `Transaction` onto the undo stack and clears the redo stack.
3. Implement `TextBuffer::undo()` — pop from undo, apply inverse ops, push to redo, restore cursor.
4. Implement `TextBuffer::redo()` — pop from redo, apply forward ops, push to undo, restore cursor.
5. Implement transaction grouping: `TextBuffer::begin_transaction()` / `TextBuffer::commit_transaction()` to group multiple ops (e.g., a paste or a search-replace-all is one undo step).
6. **Tests:** insert → undo → verify original state, multi-op transaction undo, redo after undo, undo stack size limit.

### Step 5: Cursor movement

1. Implement cursor movement methods on `TextBuffer`:
   - `move_left()`, `move_right()`, `move_up()`, `move_down()`
   - `move_word_left()`, `move_word_right()`
   - `move_line_start()`, `move_line_end()`
   - `move_buffer_start()`, `move_buffer_end()`
2. Each method takes a `bool extend_selection` parameter — if true, moves head but not anchor.
3. Handle edge cases: moving up from first line, right past end of line (wrap to next), etc.
4. **Tests:** movement in all directions, selection extension, word boundaries, buffer boundaries.

### Step 6: Search & Replace

1. Create `crates/lune-core/src/search.rs` with `SearchState`.
2. Implement `TextBuffer::search(query, opts)` — find all matches, populate `SearchState`.
3. Implement `TextBuffer::search_next()` / `search_prev()` — cycle through matches.
4. Implement `TextBuffer::replace_current(replacement)` — replace current match.
5. Implement `TextBuffer::replace_all(replacement)` — replace all matches as a single transaction.
6. **Tests:** simple search, case sensitivity, regex search, replace single, replace all with undo.

### Step 7: BufferRegistry

1. Create `crates/lune-core/src/registry.rs`.
2. Implement `BufferRegistry::open_file(path)` — check path_index first, return existing or create new.
3. Implement `BufferRegistry::new_scratch()` — create unbound buffer.
4. Implement `BufferRegistry::close(id)` — remove from both maps.
5. Implement `BufferRegistry::get(id)` / `get_mut(id)` — access by ID.
6. Implement `BufferRegistry::by_path(path)` — lookup by file path.
7. **Tests:** open same file twice returns same ID, close removes from both maps, scratch buffers have no path.

### Step 8: File I/O

1. Implement `TextBuffer::save()` — write rope to file_path, update `last_saved_revision`, clear dirty.
2. Implement `TextBuffer::reload()` — re-read from disk, replace rope, reset undo/redo.
3. Implement `TextBuffer::is_dirty()` — `current_revision != last_saved_revision`.
4. **Tests:** save then reload produces same content, dirty flag transitions.

### Step 9: Module structure & re-exports

1. Organize `lib.rs` with public module declarations and a prelude:
   ```rust
   pub mod buffer;
   pub mod position;
   pub mod registry;
   pub mod search;
   pub mod undo;

   pub mod prelude {
       pub use crate::buffer::{BufferId, TextBuffer};
       pub use crate::position::{CursorState, Position, Selection};
       pub use crate::registry::BufferRegistry;
   }
   ```

---

## Acceptance Criteria

- [ ] All types compile and are documented with `///` doc comments
- [ ] Unit test coverage for: position ordering, buffer CRUD, edit ops, undo/redo (including multi-op transactions), cursor movement, search/replace, registry
- [ ] `cargo test -p lune-core` passes with >80% line coverage on critical paths
- [ ] No `unsafe` code
- [ ] All public types implement `Debug`; value types implement `Clone`

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Rope ↔ Position conversion off-by-one errors | Extensive boundary tests; use `ropey`'s line/char indexing directly |
| Grapheme cluster handling (multi-byte chars) | V1 uses byte offsets within lines; add grapheme layer in a follow-up |
| Large file performance | Profile with 100K+ line files during Step 2; rope handles this natively |
| Transaction grouping complexity | Keep it simple: explicit begin/commit, no nesting in V1 |
