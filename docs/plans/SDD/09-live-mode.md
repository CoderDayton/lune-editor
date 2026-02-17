# 09 — Live Mode

> **Phase:** 3 (Git & AI)
> **Estimated effort:** 3–4 sessions (~8–12 hours)
> **Prerequisites:** [02-editor-core.md](02-editor-core.md), [05-file-tree.md](05-file-tree.md) (watcher), [07-git-integration.md](07-git-integration.md) (diff engine), [08-ai-integration.md](08-ai-integration.md)

## Goal

Implement Live Mode — the editor's flagship feature that detects AI-driven file changes in real time, computes incremental diffs, overlays them in the editor buffer, and provides accept/reject controls. This turns the editor from a passive tool into an active collaborator with the AI.

---

## Types & Structures

### Live Mode State

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveModeState {
    /// No live tracking. Files only refresh on manual reload.
    Off,
    /// Diffs shown but cursor stays where user left it.
    Preview,
    /// Cursor auto-tracks AI edits. Viewport follows changes.
    Follow,
}

pub struct LiveModeController {
    pub state: LiveModeState,
    pub tracked_buffers: HashMap<BufferId, LiveDiffState>,
    pub global_stats: LiveModeStats,
}

pub struct LiveModeStats {
    pub total_hunks_pending: usize,
    pub total_files_changed: usize,
    pub last_change_at: Option<Instant>,
}
```

### Per-Buffer Diff State

```rust
pub struct LiveDiffState {
    /// Snapshot of buffer content before AI started editing.
    pub baseline: Rope,
    /// Current on-disk content (updated by watcher).
    pub disk_content: Rope,
    /// Computed diff hunks between baseline and disk.
    pub hunks: Vec<LiveHunk>,
    /// Which hunks have been accepted/rejected.
    pub hunk_decisions: Vec<HunkDecision>,
    /// Last time this buffer's diff was updated.
    pub last_updated: Instant,
}

pub struct LiveHunk {
    pub id: usize,
    pub old_range: Range<usize>,  // line range in baseline
    pub new_range: Range<usize>,  // line range in disk_content
    pub kind: LiveHunkKind,
    pub lines: Vec<DiffLine>,     // reuse DiffLine from git module
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveHunkKind {
    Insertion,   // new lines added
    Deletion,    // lines removed
    Modification, // lines changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkDecision {
    Pending,
    Accepted,
    Rejected,
}
```

### Diff Engine

```rust
pub trait DiffEngine: Send {
    fn compute_diff(old: &Rope, new: &Rope) -> Vec<LiveHunk>;
    fn compute_diff_incremental(
        old: &Rope,
        new: &Rope,
        changed_range: Range<usize>,
        previous_hunks: &[LiveHunk],
    ) -> Vec<LiveHunk>;
}
```

---

## Implementation Steps

### Step 1: Diff engine

1. Create `lune-core/src/diff.rs`.
2. Implement `MyersDiffEngine` using the `similar` crate:
   - `compute_diff(old, new)` — full diff between two rope contents.
   - Convert `similar`'s changeset into `Vec<LiveHunk>`.
3. Implement line-level diffing (primary) and optional char-level diffing within modified hunks (for inline highlighting).
4. Handle edge cases: empty files, files with only additions, files with only deletions.
5. **Tests:** diff identical files (no hunks), add-only, delete-only, mixed changes, empty file.

### Step 2: Incremental diff

1. Implement `compute_diff_incremental`:
   - When only a small range of the file changed, re-diff only the affected region + context lines.
   - Merge results back into the full hunk list.
2. This optimization matters for Follow mode where AI streams changes rapidly.
3. Fallback: if incremental is unreliable, fall back to full diff.
4. **Tests:** modify 3 lines in a 1000-line file, verify incremental produces same result as full diff.

### Step 3: LiveModeController

1. Create `lune-ai/src/live_mode.rs` with `LiveModeController`.
2. Implement state transitions:
   - `set_state(LiveModeState)` — Off/Preview/Follow.
   - When entering Preview/Follow from Off: snapshot all open buffers' current content as baselines.
   - When entering Off: clear all diff states.
3. Implement `on_file_changed(path, new_content)`:
   - Look up the buffer by path.
   - If not tracked, ignore.
   - Update `disk_content` in `LiveDiffState`.
   - Re-compute diff (incremental if possible).
   - Update `global_stats`.
   - Emit a render event.
4. **Tests:** state transitions, file change triggers diff recomputation.

### Step 4: Watcher integration

1. Modify the file watcher (from plan 05) to integrate with Live Mode:
   - On `FsEvent::FileChanged(path)`:
     1. If Live Mode is Off → normal behavior (plan 05 Step 8).
     2. If Live Mode is Preview or Follow → read new file content, call `controller.on_file_changed()`.
2. Debounce rapid changes (AI may write multiple times per second):
   - Coalesce changes per-file within a 100ms window.
   - After debounce period, compute diff once.
3. **Verify:** start Live Mode, modify a file externally (simulating AI), see diff state update.

### Step 5: Editor overlay rendering

1. Modify `EditorPaneWidget` to render diff overlays when Live Mode is active:
   - **Added lines**: green background or green left-bar marker.
   - **Deleted lines**: red "ghost lines" shown above/below their former position with strikethrough or dim styling.
   - **Modified lines**: yellow background on changed characters (char-level diff).
2. Hunk boundaries are visually distinct (horizontal rule or spacing).
3. Each hunk shows a small inline action indicator: `[✓ Accept] [✗ Reject]` accessible via keybinding or mouse.
4. Current hunk (cursor is within it) gets a more prominent highlight.
5. **Verify:** enable Live Mode, make AI-like changes, see colored diff overlay in editor.

### Step 6: Hunk navigation

1. Implement keybindings:
   - `]c` (vim-style) or `Alt-↓` → jump to next hunk.
   - `[c` or `Alt-↑` → jump to previous hunk.
   - These work across hunks within the same file.
2. The status bar shows: `Hunk 3/7` when Live Mode is active.
3. **Verify:** navigate through hunks, cursor jumps to hunk start, status bar updates.

### Step 7: Accept/Reject per hunk

1. Implement `AppCommand::AcceptHunk`:
   - Apply the hunk's changes to the buffer (replace baseline content with disk content for that range).
   - Mark hunk as `Accepted` in `hunk_decisions`.
   - Remove the overlay for that hunk.
   - Update undo stack — accepting is a single undo-able operation.
2. Implement `AppCommand::RejectHunk`:
   - Mark hunk as `Rejected`.
   - Revert the file's section to baseline content (write to disk).
   - Remove the overlay.
3. Implement `AppCommand::AcceptAllHunks` / `RejectAllHunks`:
   - Batch operation for the entire file.
4. Keybindings:
   - `Ctrl-Shift-Y` or `,a` → Accept current hunk.
   - `Ctrl-Shift-N` or `,r` → Reject current hunk.
   - `Ctrl-Shift-A` → Accept all in file.
5. **Tests:** accept a hunk → buffer content matches disk content for that range. Reject a hunk → disk content reverts to baseline.

### Step 8: Follow mode auto-scroll

1. In Follow mode, when new changes arrive:
   - Automatically scroll the viewport to show the latest change.
   - Optionally move the cursor to the start of the newest hunk.
   - If the user manually scrolls away, temporarily pause auto-follow (resume on next change after 3s idle).
2. Visual indicator in status bar: `FOLLOW ▶` when actively tracking, `FOLLOW ⏸` when paused.
3. **Verify:** enable Follow mode, simulate streaming AI changes, viewport auto-scrolls.

### Step 9: Multi-file Live Mode

1. When AI changes multiple files:
   - Each file's tab gets a diff indicator (colored dot or hunk count badge).
   - The command palette or a dedicated "Live Changes" list shows all files with pending hunks.
   - `Ctrl-Shift-L` opens a Live Mode overview panel listing all changed files and hunk counts.
2. Accept/Reject All Files: batch operation across all tracked files.
3. **Verify:** AI modifies 3 files, see all tabs marked, navigate between them, accept all.

### Step 10: Conflict handling

1. If the user edits a region that also has a pending AI diff:
   - The hunk is marked as "conflicted".
   - Neither accept nor reject is available until the user resolves the overlap.
   - Visual: distinct color (orange/yellow border) for conflicted hunks.
2. User can manually edit to resolve, then dismiss the conflicted hunk.
3. **Verify:** edit inside a pending diff hunk, see conflict marker, resolve by editing.

---

## Acceptance Criteria

- [ ] Live Mode toggles between Off, Preview, and Follow states
- [ ] File changes are detected and diffs computed within 200ms
- [ ] Diff overlay renders correctly: additions (green), deletions (red), modifications (yellow)
- [ ] Hunk navigation (`]c`/`[c`) jumps between diff hunks
- [ ] Accept/Reject per hunk works and updates both buffer and disk
- [ ] Accept/Reject all hunks works for entire file and across files
- [ ] Follow mode auto-scrolls to latest AI changes
- [ ] User edits in a diff region create conflict markers
- [ ] Status bar shows Live Mode state and hunk count
- [ ] Performance: diff overlay renders without visible lag on files up to 10K lines

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Rapid AI writes cause diff thrashing | Aggressive debouncing (200ms); incremental diff to minimize recomputation |
| Ghost line rendering complicates viewport calculations | Track ghost lines as virtual lines in a separate overlay; don't insert into the rope |
| Accept/Reject + undo interaction complexity | Treat accept/reject as buffer edits that go through the normal undo stack |
| User edits during Follow mode create confusing state | Pause Follow on user activity; clear visual distinction between user and AI changes |
| Large diffs (AI rewrites entire file) overwhelm the overlay | Collapse large hunks with "Show N more lines" expander; limit inline rendering to reasonable size |
