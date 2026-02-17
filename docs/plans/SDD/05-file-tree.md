# 05 — File Tree & Workspace

> **Phase:** 2 (Workspace & Navigation)
> **Estimated effort:** 2–3 sessions (~6–8 hours)
> **Prerequisites:** [04-ui-layout.md](04-ui-layout.md)

## Goal

Implement the `Workspace` abstraction, a lazily-loaded file tree widget in the left sidebar, file operations (create/rename/delete/move), and a `notify`-based file system watcher that feeds `FsEvent` into the event loop.

---

## Types & Structures

### Workspace

```rust
pub struct Workspace {
    pub root: PathBuf,
    pub name: String,  // last component of root path
    tree_cache: HashMap<PathBuf, Vec<DirEntry>>,
    git_status_cache: HashMap<PathBuf, FileStatus>,
    watcher_tx: Option<Sender<FsEvent>>,
}

pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
    pub git_status: Option<FileStatus>,
}

pub enum EntryKind {
    File,
    Directory { expanded: bool },
    Symlink,
}
```

### File Tree Widget

```rust
pub struct FileTreeWidget {
    pub workspace: Arc<RwLock<Workspace>>,
    pub scroll_offset: usize,
    pub selected_index: usize,
    pub filter: Option<String>,  // hide dotfiles, etc.
    pub show_hidden: bool,
}

pub struct FileTreeConfig {
    pub indent_size: u16,        // spaces per nesting level
    pub icons: bool,             // nerd font icons
    pub sort_dirs_first: bool,
}
```

### File Operations

```rust
pub enum FileOp {
    CreateFile(PathBuf),
    CreateDir(PathBuf),
    Rename { from: PathBuf, to: PathBuf },
    Delete(PathBuf),
    Move { from: PathBuf, to: PathBuf },
}
```

---

## Implementation Steps

### Step 1: Workspace struct

1. Create `lune-core/src/workspace.rs` with `Workspace`, `DirEntry`, `EntryKind`.
2. Implement `Workspace::open(root: PathBuf)` — validate directory exists, set name.
3. Implement `Workspace::list_dir(path)` — returns sorted `Vec<DirEntry>` (dirs first, then files, alphabetical). Cache results in `tree_cache`.
4. Implement `Workspace::invalidate(path)` — clear cache for a path (called on `FsEvent`).
5. Implement `Workspace::relative_path(abs_path)` — returns path relative to root.
6. **Tests:** list a temp directory, verify sorting, caching behavior, invalidation.

### Step 2: File system watcher

1. Create `lune-core/src/watcher.rs`.
2. Implement `FileWatcher::new(root, event_tx)`:
   - Use `notify::recommended_watcher` with recursive mode.
   - Debounce events (100ms) to coalesce rapid writes.
   - Convert `notify` events to `FsEvent` and send via channel.
3. Implement `FileWatcher::stop()` — drop the watcher.
4. Filter out events for ignored paths (`.git/`, `target/`, etc.).
5. **Tests:** create/modify/delete a file in a temp dir, verify events arrive.

### Step 3: File tree widget — rendering

1. Create `lune-ui/src/widgets/file_tree.rs`.
2. Render as a vertical list within the left sidebar `Rect`:
   - Each entry indented by nesting level × `indent_size`.
   - Directories show `▶` (collapsed) or `▼` (expanded) prefix.
   - Files show a type icon if `icons` enabled, else just the name.
   - Selected entry gets a highlight bar.
   - Git status shown as a colored character suffix: `M` (modified), `A` (added), `?` (untracked).
3. Scrolling: if tree exceeds panel height, scroll to keep `selected_index` visible.
4. **Verify:** render a real project directory, see tree structure.

### Step 4: File tree widget — interaction

1. Keyboard:
   - `j`/`k` or `↑`/`↓` — move selection.
   - `Enter` on directory — toggle expand/collapse (lazy-load children on first expand).
   - `Enter` on file — emit `AppCommand::OpenFile(path)`.
   - `h`/`l` or `←`/`→` — collapse/expand directory.
   - `/` — start inline filter.
2. Mouse:
   - Click selects entry.
   - Double-click on file opens it.
   - Double-click on directory toggles it.
3. Implement `HandleAppEvent` for the file tree widget.
4. **Verify:** navigate tree, open files into editor tabs, expand/collapse directories.

### Step 5: File operations

1. Implement commands in the file tree context:
   - `n` — new file: prompt for name in an inline text input, create file at selected directory.
   - `N` — new directory: same but `mkdir`.
   - `r` — rename: inline edit of the selected entry's name.
   - `d` — delete: confirmation dialog, then `fs::remove_file` / `remove_dir_all`.
   - `m` — move: prompt for destination path.
2. All operations are performed via `Workspace::execute(FileOp)` which does the I/O and invalidates cache.
3. File operations that affect open buffers trigger buffer reload or path update.
4. **Verify:** create a file, rename it, see it reflected in tree and buffer title.

### Step 6: "Reveal in File Tree"

1. Implement `AppCommand::RevealInFileTree(path)`:
   - Expand all ancestor directories of the target path.
   - Scroll to and select the target entry.
2. Trigger on: clicking the file path in the status bar, or a keybinding from the editor.
3. **Verify:** open a deeply nested file, invoke reveal, see tree scrolled to it.

### Step 7: Dotfile and ignore filtering

1. Implement `FileTreeConfig::show_hidden` toggle (`Ctrl-H` in file tree).
2. Respect `.gitignore` patterns when filtering (reuse `git2` or a simple glob matcher).
3. Always hide `.git/` directory itself.
4. **Verify:** toggle hidden files, verify `.env` and dotfiles appear/disappear.

### Step 8: Wire watcher into event loop

1. On workspace open, start `FileWatcher`.
2. `FsEvent::FileChanged(path)`:
   - If path matches an open buffer → check if buffer is dirty. If clean, auto-reload. If dirty, show notification "File changed on disk".
   - Invalidate workspace cache for parent directory.
   - If Live Mode is active, delegate to Live Mode handler (plan 09).
3. `FsEvent::FileCreated/FileDeleted`:
   - Invalidate cache for parent directory.
   - If deleted file has an open buffer, mark buffer as orphaned.
4. **Verify:** modify a file externally, see tree update and buffer reload/notification.

---

## Acceptance Criteria

- [ ] Workspace loads and displays a file tree in the left sidebar
- [ ] Directories expand/collapse lazily (no upfront full-tree scan)
- [ ] Files open into editor tabs on Enter/double-click
- [ ] File operations (create, rename, delete) work with confirmation where destructive
- [ ] File system watcher detects external changes and updates tree + buffers
- [ ] "Reveal in File Tree" scrolls to and selects the target file
- [ ] Hidden file toggle works
- [ ] Tree scrolls properly for large directories (1000+ entries)
- [ ] No blocking I/O on the main thread — directory listing is fast or background-loaded

---

## Risks

| Risk | Mitigation |
|------|-----------|
| `notify` event storms on large `git checkout` or AI bulk edits | Debounce aggressively (200ms); batch invalidations |
| Deep directory trees cause slow initial expand | Lazy loading already handles this; add a depth limit for initial scan |
| File rename affects open buffers' path references | `BufferRegistry` updates `path_index` on rename; buffer retains its `BufferId` |
| `.gitignore` parsing complexity | Use the `ignore` crate (from ripgrep) rather than rolling our own |
