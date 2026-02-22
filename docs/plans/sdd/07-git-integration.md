# 07 — Git Integration

> **Phase:** 3 (Git & AI)
> **Estimated effort:** 3–4 sessions (~8–12 hours)
> **Prerequisites:** [02-editor-core.md](02-editor-core.md), [04-ui-layout.md](04-ui-layout.md), [05-file-tree.md](05-file-tree.md)

## Goal

Implement the `lune-git` crate providing a `GitService` that wraps libgit2 (via `git2`), inline gutter markers in the editor, a Git panel for staging/committing, and a diff view. The editor should feel git-aware at all times.

---

## Types & Structures

### Git Service

```rust
pub struct GitService {
    repo: Repository,  // git2::Repository
    root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct GitStatus {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<GitFileStatus>,
}

#[derive(Clone, Debug)]
pub struct GitFileStatus {
    pub path: PathBuf,
    pub status: FileStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    Ignored,
}
```

### Diff Types

```rust
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: PathBuf,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
}
```

### Gutter Markers

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterMark {
    Added,
    Modified,
    Deleted,
}

/// Line-level gutter marks for an open buffer.
pub struct GutterMarks {
    pub marks: HashMap<usize, GutterMark>,  // line number → mark
}
```

---

## Implementation Steps

### Step 1: GitService basics

1. Create `crates/lune-git/src/service.rs` with `GitService`.
2. Implement `GitService::open(path)` — discover repo from workspace root using `git2::Repository::discover()`.
3. Implement `GitService::status() -> Result<GitStatus>`:
   - Get current branch name via `repo.head()`.
   - Compute ahead/behind via `repo.graph_ahead_behind()` against upstream.
   - Iterate `repo.statuses()` to build `Vec<GitFileStatus>`.
4. Implement `GitService::is_repo() -> bool`.
5. **Tests:** open a test git repo, verify branch name, file statuses.

### Step 2: Diff computation

1. Implement `GitService::diff_file(path) -> Result<FileDiff>`:
   - Diff working tree file against HEAD using `git2::Diff`.
   - Parse hunks and lines into `FileDiff` structure.
2. Implement `GitService::diff_staged(path) -> Result<FileDiff>`:
   - Diff index against HEAD.
3. Implement `GitService::diff_all() -> Result<Vec<FileDiff>>`:
   - Diff entire working tree against HEAD.
4. **Tests:** modify a tracked file, verify diff hunks match expected additions/deletions.

### Step 3: Gutter markers

1. Create `lune-git/src/gutter.rs` with `GutterMarks`.
2. Implement `GitService::gutter_marks(path, buffer_content) -> Result<GutterMarks>`:
   - Diff current buffer content (not disk!) against HEAD version.
   - Map diff hunks to line-level marks: added lines, modified lines, deleted-line indicators.
3. Cache gutter marks per buffer; invalidate on buffer edit or git state change.
4. **Verify:** modify lines in an editor buffer, see gutter marks appear.

### Step 4: Editor gutter rendering

1. Modify `EditorPaneWidget` to include a git gutter column (1 char wide, left of line numbers):
   - `Added` → green `│` or `+`
   - `Modified` → yellow `│` or `~`
   - `Deleted` → red `▾` or `-` (at the line above the deletion)
2. Gutter marks update on each render by querying cached `GutterMarks`.
3. **Verify:** edit a git-tracked file, see colored markers in the gutter.

### Step 5: Git panel widget

1. Create `lune-ui/src/widgets/git_panel.rs`.
2. Render in the right sidebar (or bottom panel) as a list:
   - Section: **Staged Changes** — list of staged files.
   - Section: **Changes** — list of unstaged modified/untracked files.
   - Each entry shows status icon + file path.
3. Keyboard/mouse:
   - Select a file → show its diff in an overlay or split view.
   - `s` on a file → stage it.
   - `u` on a staged file → unstage it.
   - `Enter` → open diff view for selected file.
4. **Verify:** see list of changed files, stage/unstage via keyboard.

### Step 6: Stage/unstage/commit

1. Implement `GitService::stage(path)` — `repo.index().add_path()` + write index.
2. Implement `GitService::unstage(path)` — reset index entry to HEAD version.
3. Implement `GitService::stage_hunk(path, hunk_index)` — apply only specific hunk to index (partial staging).
4. Implement `GitService::commit(message) -> Result<Oid>`:
   - Build tree from current index.
   - Create commit with message, author from git config.
5. Implement `GitService::discard_file(path)` — checkout HEAD version of file.
6. Implement `GitService::discard_hunk(path, hunk_index)` — reverse-apply hunk to working tree.
7. All destructive operations require confirmation (via overlay dialog).
8. **Tests:** stage a file, verify index status, commit, verify HEAD tree.

### Step 7: Diff view

1. Create `lune-ui/src/widgets/diff_view.rs`.
2. Support two modes:
   - **Inline** (unified diff): single pane with `+`/`-` lines colored.
   - **Side-by-side**: two panes, old on left, new on right, matched by line.
3. Render diff hunks with context lines, colored additions/deletions.
4. Navigation: jump between hunks, scroll through diff.
5. From the diff view, allow per-hunk stage/unstage/discard.
6. **Verify:** open diff for a modified file, see hunks, stage a single hunk.

### Step 8: Status bar integration

1. Display git branch name in the status bar.
2. Show ahead/behind counts as `↑2 ↓1` indicators.
3. Show a sync icon or warning if repo is dirty.
4. Clicking the branch name could open a branch picker (defer to future).
5. **Verify:** status bar shows current branch and sync status.

### Step 9: File tree integration

1. In the file tree (plan 05), display git status per file:
   - Color file names: green (added), yellow (modified), red (deleted), gray (ignored).
   - Propagate status to parent directories (if any child is modified, parent shows modified).
2. Update file tree git status when `GitService::status()` is refreshed.
3. **Verify:** modify files, see file tree colors update.

### Step 10: Background refresh

1. Run `GitService::status()` on a background thread, triggered by:
   - File save.
   - `FsEvent` in `.git/` directory (e.g., commit, branch switch).
   - Manual refresh command.
   - Timer (every 5s as fallback).
2. Send results to UI thread via channel; UI updates gutter, file tree, status bar, git panel.
3. **Verify:** commit from external terminal, see editor update within seconds.

---

## Acceptance Criteria

- [ ] Git branch name and status displayed in status bar
- [ ] Editor gutter shows add/modify/delete markers for git-tracked files
- [ ] Git panel lists staged and unstaged changes
- [ ] Stage, unstage, and commit operations work from within the editor
- [ ] Per-hunk staging and discarding works
- [ ] Diff view renders unified and side-by-side diffs
- [ ] File tree shows git status colors
- [ ] Git status refreshes automatically on file changes and external git operations
- [ ] Non-git directories work normally (git features gracefully disabled)
- [ ] No main-thread blocking from git operations

---

## Risks

| Risk | Mitigation |
|------|-----------|
| `git2` may have issues with some repo configurations (worktrees, submodules) | Degrade gracefully; log warnings, don't crash |
| Partial staging (per-hunk) is complex with `git2` | Fall back to `git add -p`-style patch application if git2 API is insufficient |
| Large repos with thousands of changed files | Paginate git panel; limit status query scope to workspace subdirectory if needed |
| Concurrent git operations (editor + AI + user in terminal) | Use `git2` with care around locking; retry on lock failures |
| Merge conflicts are complex to render | V1 shows conflict markers as-is; defer interactive merge resolution to future |
