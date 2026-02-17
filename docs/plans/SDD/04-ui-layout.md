# 04 — UI Layout

> **Phase:** 1 (Foundation)
> **Estimated effort:** 3–4 sessions (~8–12 hours)
> **Prerequisites:** [01-project-scaffold.md](01-project-scaffold.md), [02-editor-core.md](02-editor-core.md), [03-event-system.md](03-event-system.md)

## Goal

Build the VS Code–inspired terminal UI layout: root panel splits, tab bar, editor pane (rendering a `TextBuffer`), status bar, and a command palette overlay. This is the first time the user sees rendered text on screen.

---

## Types & Structures

### Layout

```rust
pub struct LayoutState {
    pub show_file_tree: bool,
    pub show_ai_panel: bool,
    pub show_git_panel: bool,
    pub active_panel: PanelId,
    pub file_tree_width_pct: u16,    // default 20
    pub right_panel_width_pct: u16,  // default 30
}

pub struct LayoutSplits {
    pub left: Option<Rect>,    // file tree
    pub center: Rect,          // editor
    pub right: Option<Rect>,   // AI terminal / Git panel
    pub status: Rect,          // bottom status bar
}
```

### Tab Manager

```rust
pub struct TabManager {
    pub tabs: Vec<TabEntry>,
    pub active_index: usize,
}

pub struct TabEntry {
    pub buffer_id: BufferId,
    pub title: String,      // filename or "Untitled"
    pub dirty: bool,
    pub pinned: bool,
}
```

### Status Bar

```rust
pub struct StatusLineState {
    pub mode: String,        // "NORMAL", "INSERT", "VISUAL" or empty
    pub file_path: String,
    pub cursor_pos: String,  // "Ln 42, Col 13"
    pub git_branch: String,
    pub encoding: String,    // "UTF-8"
    pub ai_status: String,   // "AI: Connected" or empty
    pub file_type: String,   // "Rust", "Markdown", etc.
}
```

### Overlay

```rust
pub enum OverlayKind {
    CommandPalette,
    FindReplace,
    Notification(String),
    ConfirmDialog { message: String, on_confirm: AppCommand },
}

pub struct OverlayState {
    pub active: Option<OverlayKind>,
}
```

---

## Implementation Steps

### Step 1: Root layout computation

1. Create `lune-ui/src/layout.rs`.
2. Implement `compute_layout(area: Rect, state: &LayoutState) -> LayoutSplits`:
   - If `show_file_tree`: allocate left column at `file_tree_width_pct`.
   - If `show_ai_panel` or `show_git_panel`: allocate right column at `right_panel_width_pct`.
   - Remaining space goes to center editor.
   - Bottom 1–2 rows reserved for status bar.
3. Use ratatui `Layout::default().direction(Horizontal).constraints(...)` for column splits.
4. **Tests:** layout with all panels, with none, with only left, only right.

### Step 2: Tab bar widget

1. Create `lune-ui/src/widgets/tab_bar.rs`.
2. Render horizontally: each tab shows `[filename]` or `[filename*]` if dirty.
3. Active tab gets a distinct style (underline or inverted colors).
4. Mouse: clicking a tab switches to it. Clicking `x` on a tab closes it.
5. If tabs overflow width, show `◄ ►` scroll indicators.
6. Implement `HandleAppEvent` for tab switching on click or `Ctrl-Tab`/`Ctrl-Shift-Tab`.
7. **Verify:** render 5+ tabs, switch between them, visual distinction for active tab.

### Step 3: Editor pane widget

1. Create `lune-ui/src/widgets/editor_pane.rs`.
2. Render the active `TextBuffer`:
   - Line numbers in a left gutter (width adapts to digit count).
   - Text content with horizontal scrolling if lines exceed pane width.
   - Cursor rendered as a highlighted cell (block in normal mode, line in insert mode).
   - Selection rendered as highlighted background range.
3. Vertical scrolling: track `viewport_top_line`, scroll to keep cursor visible.
4. Mouse:
   - Click sets cursor position.
   - Click-drag creates a selection.
   - Scroll wheel adjusts viewport.
5. Implement `HandleAppEvent`:
   - In normal keybinding mode: arrow keys, Home/End, PgUp/PgDn move cursor.
   - Character input inserts text (in insert mode or non-vim mode).
   - Delegate to vim state machine when vim mode is active.
6. **Verify:** open a real source file, scroll through it, see line numbers, move cursor.

### Step 4: Status bar widget

1. Create `lune-ui/src/widgets/status_bar.rs`.
2. Render in 1 row at the bottom:
   - Left: `[MODE]` `filepath` `[dirty indicator]`
   - Center: `Ln X, Col Y`
   - Right: `git-branch` `encoding` `filetype` `ai-status`
3. Use ratatui `Paragraph` or `Line` with styled spans.
4. Update on every render from current state.
5. **Verify:** status bar shows accurate cursor position, mode, file name.

### Step 5: Overlay system (command palette)

1. Create `lune-ui/src/widgets/overlay.rs`.
2. Command palette:
   - Centered popup at ~60% width, ~40% height.
   - Text input at top for fuzzy search.
   - List of commands below, filtered as user types.
   - Enter executes selected command, Esc closes.
3. Render overlays on top of main layout (draw last in the render pass).
4. When overlay is active, it captures all keyboard input (focus override).
5. **Verify:** `Ctrl-P` opens palette, type to filter, select command, Esc closes.

### Step 6: Panel toggle and resize

1. Implement `ToggleFileTree` command: flips `show_file_tree`, triggers layout recompute.
2. Implement `ToggleAiPanel` / `ToggleGitPanel` similarly.
3. Mouse drag on panel borders resizes the split percentages.
   - Detect mouse down on border column, track drag, update percentage, clamp to min/max.
4. **Verify:** toggle panels with keybindings, drag-resize panel borders.

### Step 7: Notification system

1. Create transient notification rendering:
   - Bottom-right toast messages that auto-dismiss after N seconds.
   - Stack multiple notifications vertically.
2. Triggered by events like "File saved", "Git commit successful", errors.
3. Use the `Tick` event to decrement notification timers.
4. **Verify:** trigger a save, see notification appear and fade.

### Step 8: Wire rendering into the main loop

1. In the main app loop, call `terminal.draw(|frame| { ... })`:
   - Compute layout.
   - Render file tree placeholder (empty rect with border — real tree in plan 05).
   - Render tab bar.
   - Render editor pane with active buffer.
   - Render right panel placeholder.
   - Render status bar.
   - Render overlays on top.
2. Each render pass is stateless (immediate mode) — widgets read from `AppState`.
3. **Verify:** full layout renders correctly, resize terminal and layout adapts.

---

## Acceptance Criteria

- [ ] VS Code–like layout renders: file tree area | editor with tabs | status bar
- [ ] Tab bar shows open files, switching tabs changes the displayed buffer
- [ ] Editor pane renders text with line numbers, cursor, and selections
- [ ] Scrolling works (keyboard and mouse wheel)
- [ ] Status bar displays mode, file path, cursor position, git branch placeholder
- [ ] Command palette opens/closes and can execute basic commands
- [ ] Panel toggle keybindings show/hide sidebars
- [ ] Mouse drag resizes panel boundaries
- [ ] Terminal resize causes correct re-layout without panics
- [ ] Notifications appear and auto-dismiss

---

## Risks

| Risk | Mitigation |
|------|-----------|
| rat-widget may not have a suitable terminal emulator widget | Fall back to custom widget wrapping raw ratatui primitives; plan 08 handles the full terminal widget |
| Unicode/wide-char rendering misalignment | Use `unicode-width` crate for display width calculations; test with CJK text |
| Performance on very wide terminals or many tabs | Profile rendering; ratatui's immediate mode is generally fast, but tab bar overflow logic must be efficient |
| Mouse coordinate mapping to text position | Careful translation of click (col, row) to buffer (line, col) accounting for gutter width and scroll offset |
