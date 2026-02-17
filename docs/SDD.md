<img src="https://r2cdn.perplexity.ai/pplx-full-logo-primary-dark%402x.png" style="height:64px;margin-right:32px"/>

# Create an SDD.md for the following project idea:

Rust + Ratatui + rat-salsa + [rat-widget](https://docs.rs/rat-widget) + tachyonfx - Terminal Editor with built in direct AI Client support (ex. open Claude Code cli within an embedded terminal.)

Client example: Claude Code

Claude Code would have lots of context from the editor like current open file, current open tabs, current selected line and selection, and more.

The editor should be inspired by VS Code. It should have mouse control with optional vim control mode.

It will have a toggleable file tree like vscode and git integration.

Image attachment is a simple AI Generation reference or inspiration.

Special feature will be live mode: follow claude codes edits in real time and show diffs in the editor.

DELIVERABLE:
SDD.md file

```markdown
# SDD: Tachyon Editor (Rust + Ratatui AI-Integrated Terminal Editor)

## 0. Overview

Tachyon Editor is a Rust terminal-based code editor with a VS Code–inspired UX, built on Ratatui, rat-salsa, rat-widget, and tachyonfx, with first-class embedded AI client support (e.g., Claude Code) and live edit-following mode.[web:2][web:3][web:4][web:6][web:10][web:15][web:8]  
It provides a mouse-driven, optionally modal (vim-like) interface, a toggleable file tree, and Git integration, while exposing rich editor context to AI clients and rendering real-time diffs of AI-driven changes.

---

## 1. Goals and Non-Goals

### 1.1 Product Goals

- Provide a fast, fully keyboard- and mouse-operable terminal editor that feels structurally similar to VS Code (panels, tabs, status bar, command palette).  
- Integrate one or more AI coding assistants (initially Claude Code) via embedded terminals with deep context from the editor (active file, selection, open tabs, workspace metadata).  
- Implement a “Live Mode” that follows AI edits in real time and overlays diffs directly in the editor buffers.  
- Offer a predictable, extensible Rust architecture: clear separation of UI rendering, event loop, editor core, AI integration, and VCS layer.

### 1.2 Non-Goals (V1)

- No full plugin marketplace or dynamic plugin loading (V1 focuses on a small set of built-in features).  
- No full LSP implementation; basic syntax highlighting is acceptable, but language servers are deferred.  
- No remote SSH multiplexing or multi-host file management; V1 assumes local file system or a single mounted workspace.  
- No enforcement of specific AI providers; Claude Code is the default reference client, but the integration should be pluggable.

---

## 2. Technology Choices

- **Ratatui**: terminal UI framework for layout, widgets, and rendering.[web:2][web:3][web:4][web:6]  
- **rat-salsa**: application event loop and crossterm integration for structured event-driven TUIs.[web:10]  
- **rat-widget** (+ rat-widget-extra as needed): extended widgets with rat-event–based event handling for complex, interactive controls.[web:15][web:11][web:14]  
- **tachyonfx**: shader-like visual effects applied post-render for transitions, focus hints, and live diff highlighting.[web:8][web:12]  
- **Claude Code** (example AI client): external CLI run inside a pseudo-terminal, communicating via stdin/stdout and the workspace file system.

---

## 3. Target Users and Use Cases

### 3.1 Target Users

- Power terminal users who like VS Code’s mental model but prefer a TUI environment.  
- Developers heavily using AI coding assistants and wanting tighter integration than “editor + separate terminal”.  
- Users who want real-time visibility and control over AI-driven edits (diff-visualized, reversible).

### 3.2 Core Use Cases

- Open a project directory, navigate via a file tree, edit multiple files in tabs, and commit via built-in Git panel.  
- Invoke an embedded Claude Code terminal with context (current file, selection, workspace summary) to request refactors or code generation.  
- Enable Live Mode to watch AI edits stream in and see inline diff markings, with quick accept/reject operations.  
- Use vim mode for modal keybindings while retaining mouse interactions for selection, resizing, and panel toggles.

---

## 4. High-Level Architecture

### 4.1 Logical Layers

1. **UI Layer (Ratatui + rat-widget + tachyonfx)**  
   - Layouts: top-level splits (file tree, editor, AI terminal, status bar), tabs, popups, diff overlays.[web:2][web:4][web:15][web:8]  
   - Widgets: file tree, tab bar, editor buffers, embedded terminal, Git panel, command palette, notifications.[web:15]  
   - Effects: focus glow, selection highlighting, live diff animations via tachyonfx post-processing on rendered buffers.[web:8]

2. **Event Loop + Input Layer (rat-salsa + rat-event)**  
   - Centralized event loop over crossterm events and timers using rat-salsa.[web:10]  
   - Unified event handling via HandleEvent trait (rat-event) enabling composable keyboard/mouse routing to widgets.[web:14]  

3. **Editor Core**  
   - Buffer model, cursor/selection state, undo/redo, diff engine, search/replace, vim-mode state.  
   - Bridge to file system (workspace root abstraction) and Git service.

4. **AI Integration Layer**  
   - PTY manager for embedded AI client sessions (Claude Code).  
   - Context provider that collects editor state and serializes into prompts/CLI flags/env vars.  
   - Live Mode controller that tracks AI-induced file diffs and signals the UI.

5. **Persistence and Services**  
   - Settings (keymaps, themes, AI presets), recent workspaces, cached AI context summaries.  
   - Git service: status, diffs, branches, staging, committing.

### 4.2 Process Model

- Single-process Rust binary.  
- One main thread runs rat-salsa event loop and Ratatui rendering; background threads for file I/O, Git, AI PTY streams, and file-watching.  
- Communication via channels (mpsc) and shared state (Arc<Mutex/RwLock>) where needed, with strict ownership around editor buffers.

---

## 5. Core Functional Requirements

### 5.1 Editor

- Multiple buffers with tabs, each bound to a file or scratch buffer.  
- Modal input options: normal (VS Code-like) and vim mode (normal/insert/visual).  
- Mouse operations: clicking to focus panes, select text, drag pane boundaries, select tabs, right-click context menus.  
- Standard editing: insert/delete, cut/copy/paste, multi-cursor later; undo/redo with transaction grouping.  
- Search: inline find/replace within a buffer, with incremental highlight of matches.

### 5.2 Layout and VS Code–Inspired UX

- Toggleable left file explorer panel.  
- Central editor with tab strip across the top.  
- Right-hand optional panel for AI terminal and auxiliary views (e.g., Git diff).  
- Bottom status bar for mode (vim/insert), file path, Git branch, diagnostics, and AI connection state.

### 5.3 File Tree and Workspace

- Workspace: rooted at a selected directory.  
- File tree: lazily loaded directories, expand/collapse, reveal current file, basic filtering (hide dotfiles option).  
- File operations: create, rename, delete, move files/folders, with confirmation prompts.  

### 5.4 Git Integration

- Git status: branch name, ahead/behind counts, per-file status.  
- Inline gutter markers for modified, added, and removed lines.  
- Basic operations: stage/unstage (per file and per hunk), commit with message, discard changes for file/hunk.  
- Git diff view: side-by-side or inline diff against HEAD for current file.

### 5.5 AI Client Integration

- Launch embedded Claude Code (or other AI CLI) in a panel backed by a PTY.  
- Provide contextual invocation helpers:
  - “Ask about selection” command sends current selection text plus metadata.  
  - “Refactor file” command sends current file contents.  
  - “Summarize workspace changes” sends list of modified files and short diffs.  
- Maintain AI session history within embedded terminal while enabling copy/paste and scrollback.  

### 5.6 Live Mode and Diffs

- Detect file changes made by AI (through the file system) and map them to loaded buffers.  
- Compute diffs incrementally and overlay them:
  - Inserted lines: highlighted background or left-bar markers.  
  - Deleted lines: virtual diff lines in a side gutter or ghost lines view.  
  - Modified lines: inline highlight.  
- Live Mode has explicit states: Off, Preview, Follow:
  - Off: changes only refresh when files are manually reloaded.  
  - Preview: diff shown but cursor remains where user left it.  
  - Follow: cursor automatically tracks AI-driven edits as they stream in.  
- Apply/Reject controls per hunk or file (e.g., keybindings or quick actions).

---

## 6. Detailed Design

### 6.1 UI Layout and Widgets

- Use Ratatui’s constraint-based layout to define root splits: `[Left Sidebar][Main Area][Right Sidebar]` with responsive percentage widths.[web:2][web:4]  
- Implement file tree, tab bar, editor pane, AI terminal, Git panel, command palette, and status bar using rat-widget widgets with integrated event handlers.[web:15][web:11]  
- Panel visibility and layout configuration stored in an in-memory layout model and persisted in a settings file.  

**Key Structures**

```rust
struct LayoutState {
    show_file_tree: bool,
    show_ai_panel: bool,
    show_git_panel: bool,
    active_panel: PanelId,
    splits: LayoutSplits, // percentages for columns/rows
}
```

```rust
struct UiState {
    layout: LayoutState,
    tabs: TabManager,
    status: StatusLineState,
    overlays: OverlayState, // popups, command palette, notifications
}
```


### 6.2 Event Loop and Input Handling

- rat-salsa provides the central application loop that consumes events (keyboard, mouse, resize, timers) and dispatches them to the current scene.[web:10]
- rat-event’s HandleEvent trait is implemented for widgets and composite containers, enabling them to consume or propagate events and indicate re-render requirements.[web:14]

**Event Flow**

1. crossterm event arrives into rat-salsa’s loop.[web:10]
2. Event is normalized into an internal `AppEvent` (e.g., `KeyEvent`, `MouseEvent`, `FsEvent`, `AiEvent`).
3. Active scene/root widget’s `handle(&AppEvent)` is invoked; it routes events based on focus.
4. The handler returns an outcome (e.g., `Render`, `NoRender`, `SceneChange`), informing the loop whether to redraw.

Vim mode is realized via a mode state machine inside the editor widget; keybindings vary based on current mode.

### 6.3 Editor Core

**Data Model**

```rust
struct BufferId(Uuid);

struct TextBuffer {
    id: BufferId,
    rope: Rope,
    file_path: Option<PathBuf>,
    cursor: CursorState,
    selections: Vec<Selection>,
    undo_stack: UndoStack,
    redo_stack: UndoStack,
    last_saved_revision: RevisionId,
}
```

- Use a rope or similar structure to handle large files efficiently.
- Track revision IDs per save and use them for diff baselines vs. disk and vs. Git.

**Operations**

- Editing commands expressed as high-level ops (`InsertText`, `DeleteRange`, `ApplyHunk`, etc.) pushed onto the undo stack as transactions.
- Search/replace maintains a search state for incremental highlighting and navigation.
- Syntax highlighting for V1 can be regex- or tree-sitter–based but is architected as a pluggable layer on top of `TextBuffer`.


### 6.4 File Tree and Workspace

- `Workspace` encapsulates:
    - root path,
    - caches for directory listings,
    - file metadata (size, modification times, Git status).
- File tree widget queries `Workspace` for children lazily to avoid blocking on deep trees.
- File system changes are captured via a watcher thread (e.g., `notify` crate), posted to UI as `FsEvent::FileChanged(PathBuf)`.

When an open buffer’s file is modified externally (AI or Git operations), the editor core generates a diff and triggers Live Mode logic if enabled.

### 6.5 Git Integration

- `GitService` wraps libgit2/CLI interactions behind async methods: `status()`, `diff_file()`, `stage()`, `commit()`, `branches()`.
- Git panel widget renders:
    - list of modified files,
    - selection that opens per-file diff view in right sidebar or as overlay.
- Editor gutter shows Git markers based on diff hunks between current buffer content and HEAD.

Live Mode can reuse the same diff engine but with baseline set to “pre-AI edit” revision instead of HEAD.

### 6.6 Embedded AI Client Integration

**PTy Manager**

```rust
struct AiSession {
    id: AiSessionId,
    kind: AiClientKind, // ClaudeCode, Custom
    pty: PtyHandle,
    stdin_tx: Sender<String>,
    stdout_rx: Receiver<AiStreamChunk>,
    state: AiSessionState,
}
```

- Create and manage a PTY for Claude Code so it behaves exactly like a user-run terminal process.
- Stdout/stderr are streamed into a rat-widget-based terminal emulator widget, which handles scrollback and color escape sequences.[web:15]

**Context Provider**

- Collects:
    - active file content (text, language, path, cursor position, selections),
    - list of open tabs (paths, approximate sizes),
    - Git status summary (modified files),
    - optional workspace summary (e.g., directory and key files).
- Encodes this context into:
    - environment variables (e.g., `AI_CTX_CURRENT_FILE`, `AI_CTX_SELECTION`),
    - arguments/flags (`--file`, `--selection`),
    - or a small file in a temp directory read by the AI client.

**Command Patterns**

- “Ask about selection”:
    - Create temp file with selection, call `claude` (or similar) with `--context-file` and workspace root.
- “Refactor file”:
    - Provide file path and optional instruction string to AI client, which then edits the file directly on disk.

AI client remains an external process; Tachyon Editor never parses its internal protocol, only its terminal output and resulting file system changes.

### 6.7 Live Mode and Diff Handling

**Change Detection**

- File watcher or periodic polling notices changes to files in the workspace.
- When a file corresponding to an open buffer changes:

1. Read new file contents into a shadow buffer.
2. Run diff engine (e.g., Myers) between current in-editor contents and on-disk version.
3. Store resulting hunks in `LiveDiffState` associated with the buffer.

**Rendering**

- Diff hunks are overlaid in the editor widget as:
    - side gutter markers,
    - inline line/char-level highlights using Ratatui styles,
    - optional tachyonfx effects for smooth fade-in/fade-out to visually indicate new changes.[web:8]

**User Controls**

- Keybindings:
    - Toggle Live Mode (Off/Preview/Follow).
    - Next/previous hunk navigation.
    - Accept/Reject hunk or entire file.
- Accept applies the diff directly to the buffer; Reject reverts to pre-AI contents or selectively reverts hunks.
- Follow mode automatically scrolls and moves the viewport to keep the most recent AI edits in view.


### 6.8 Effects and Visuals (tachyonfx)

- tachyonfx operates on Ratatui buffers after widgets have been rendered, modifying cell colors and characters to produce shader-like effects.[web:8]
- Effects usages:
    - Highlight currently focused pane with a subtle glow or gradient border.
    - Animate Live Mode diffs with fade or pulse to differentiate AI changes from manual edits.
    - Provide transient “AI thinking” indicator in the status bar using timed effects and interpolation.[web:8][web:12]

Effects are configured via the EffectDsl to allow declarative animation definitions bound to state transitions (e.g., Live Mode entering Follow).[web:12]

---

## 7. Data Model

### 7.1 Core Types

- `AppState`
    - `ui_state: UiState`
    - `workspace: Option<Workspace>`
    - `buffers: BufferRegistry`
    - `git: Option<GitService>`
    - `ai_sessions: HashMap<AiSessionId, AiSession>`
    - `settings: Settings`
- `Settings`
    - `keymap: KeymapConfig` (normal/vim mode, custom bindings).
    - `theme: ThemeConfig` (colors consistent with Ratatui styles).
    - `ai: AiConfig` (default client, binary path, context options).
- `BufferRegistry`
    - map `BufferId -> TextBuffer`.
    - association `PathBuf -> BufferId` for open files.


### 7.2 Serialization

- Settings and workspace layout stored as TOML/YAML under `~/.tachyon-editor` or workspace-local `.tachyon`.
- Recent workspaces stored as a small MRU list with minimal metadata.
- No sensitive AI config (tokens, etc.) stored in plain text; rely on AI client’s own configuration.

---

## 8. Extensibility and Future Work

- **Plugin interface (future):**
    - Scoped to provide new commands, keybindings, and panels using an internal API or IPC.
- **Language services:**
    - LSP client for richer code intelligence; reuse existing buffer model for diagnostics and inlay hints.
- **Multi-AI support:**
    - Extend `AiClientKind` to allow multiple backends; each may define its own context encoding strategy.
- **Remote development:**
    - Workspaces over SSH/containers, potentially by proxying a file-service abstraction.

---

## 9. Non-Functional Requirements

### 9.1 Performance

- Must maintain sub-50 ms interactive latency for typical operations (cursor movement, basic edits, scrolling) on large files.
- Ratatui’s immediate-mode rendering and efficient widgets are used to avoid unnecessary redraws.[web:2][web:3][web:4]
- Background operations (Git, AI IO, file watching) must not block UI; all heavy work is asynchronous.


### 9.2 Reliability

- Autosave and crash recovery via periodic snapshots of dirty buffers to a hidden directory.
- All operations that affect disk or Git must be undoable or clearly confirmed.
- PTY failures or AI client crashes should not crash the editor; instead, show an error status and allow restart.


### 9.3 Portability

- Target: modern Unix terminals and Windows Console/Windows Terminal with crossterm backend.
- No OS-specific dependencies beyond standard Rust + crossterm + Git tooling.


### 9.4 Security and Privacy

- AI client integration relies on external tools; editor does not send code directly to remote servers.
- When helper features write context files, they are local to the machine and cleaned up after use.
- No telemetry or network communication is done by the editor core.

---

## 10. Testing and Validation

### 10.1 Automated Tests

- Unit tests:
    - Buffer operations (insert/delete, undo/redo, search/replace).
    - Diff engine and Live Mode hunk application.
    - Workspace file tree navigation and Git state mapping.
- Integration tests:
    - Event-routing correctness in rat-salsa, including mouse + vim mode co-existence.[web:10]
    - PTY lifecycle with a mock AI CLI that writes known edits to files.
    - Live Mode: sequence of external file edits and expected diff overlays.


### 10.2 Manual Scenarios

- Large project navigation and editing under continuous file changes from an AI client.
- Git workflows: stage, partial stage, commit, rollback of AI changes.
- Latency and responsiveness tests in different terminals and window sizes.
- Accessibility checks for color schemes and keybinding customization.

---

### Strategic Follow-Ups

S1 (Deepen: mechanics): Specify the buffer/diff algorithm choices and design PTY/AI integration code-level APIs (traits, modules, error handling).
S2 (Broaden: related fields): Extend design to support LSP-based code intelligence and multi-AI orchestration, including prompt-routing and tool calling.
S3 (Apply: actionable step): Define an initial Rust crate layout (`crates/ui`, `crates/core`, `crates/ai`, `crates/git`) and sketch minimal MVP tasks for the first working prototype.

```
<span style="display:none">[^1][^10][^11][^12][^13][^14][^15][^2][^3][^4][^5][^6][^7][^8][^9]</span>

<div align="center">⁂</div>

[^1]: gpt-image-1.5-high-fidelity_a_Ratatui_rust_termina.jpg
[^2]: https://ratatui.rs
[^3]: https://github.com/ratatui/ratatui
[^4]: https://docs.rs/ratatui
[^5]: https://www.reddit.com/r/rust/comments/1pw0tci/ratatui_v0300_is_released_a_rust_library_for/
[^6]: https://crates.io/crates/ratatui/0.23.0
[^7]: https://crates.io/crates/rat-widget-extra
[^8]: https://docs.rs/tachyonfx
[^9]: https://www.youtube.com/watch?v=F04kQMKwrwQ
[^10]: https://crates.io/crates/rat-salsa/0.23.0
[^11]: https://lib.rs/crates/rat-widget-extra
[^12]: https://docs.rs/tachyonfx/latest/tachyonfx/dsl/struct.EffectDsl.html
[^13]: https://stackoverflow.com/questions/79015134/piping-the-final-output-of-a-ratatui-rust-app-while-still-showing-the-tui-applic
[^14]: https://lib.rs/crates/rat-event
[^15]: https://crates.io/crates/rat-widget```

