# 08 — AI Integration

> **Phase:** 3 (Git & AI)
> **Estimated effort:** 3–4 sessions (~8–12 hours)
> **Prerequisites:** [03-event-system.md](03-event-system.md), [04-ui-layout.md](04-ui-layout.md)

## Goal

Implement the `lune-ai` crate: a PTY manager for running AI CLI tools (Claude Code as the reference client), a context provider that serializes editor state, an embedded terminal emulator widget, and command patterns for contextual AI invocation.

---

## Types & Structures

### AI Session

```rust
pub type AiSessionId = Uuid;

pub struct AiSession {
    pub id: AiSessionId,
    pub kind: AiClientKind,
    pub pty: PtyHandle,
    pub stdin_tx: Sender<Vec<u8>>,
    pub stdout_rx: Receiver<Vec<u8>>,
    pub state: AiSessionState,
    pub started_at: Instant,
}

#[derive(Clone, Debug)]
pub enum AiClientKind {
    ClaudeCode,
    Custom { name: String, command: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiSessionState {
    Starting,
    Running,
    Idle,       // process alive but no activity
    Error,
    Terminated,
}
```

### PTY Handle

```rust
pub struct PtyHandle {
    child: Box<dyn portable_pty::Child + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    reader: Box<dyn std::io::Read + Send>,
    writer: Box<dyn std::io::Write + Send>,
    size: PtySize,
}

pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}
```

### Context Provider

```rust
pub struct EditorContext {
    pub workspace_root: Option<PathBuf>,
    pub active_file: Option<FileContext>,
    pub open_tabs: Vec<TabContext>,
    pub git_status: Option<GitStatusSummary>,
    pub selection: Option<SelectionContext>,
}

pub struct FileContext {
    pub path: PathBuf,
    pub language: Option<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub total_lines: usize,
}

pub struct TabContext {
    pub path: PathBuf,
    pub dirty: bool,
}

pub struct SelectionContext {
    pub text: String,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
}

pub struct GitStatusSummary {
    pub branch: String,
    pub modified_files: Vec<PathBuf>,
}
```

### AI Config

```rust
pub struct AiConfig {
    pub default_client: AiClientKind,
    pub claude_code_path: PathBuf,  // default: "claude"
    pub context_mode: ContextMode,
    pub auto_context: bool,         // automatically provide context on invocation
}

pub enum ContextMode {
    /// Pass context via environment variables
    EnvVars,
    /// Write context to a temp file, pass path as argument
    TempFile,
    /// Pass as CLI arguments
    CliArgs,
}
```

---

## Implementation Steps

### Step 1: PTY manager

1. Create `crates/lune-ai/src/pty.rs` with `PtyHandle`, `PtySize`.
2. Implement `PtyHandle::spawn(command, args, env, size) -> Result<PtyHandle>`:
   - Use `portable-pty` to create a pseudo-terminal pair.
   - Spawn the child process with the given command, args, and environment.
   - Return handles for reading/writing.
3. Implement `PtyHandle::resize(rows, cols)` — resize the PTY.
4. Implement `PtyHandle::write(data)` — send bytes to stdin.
5. Implement `PtyHandle::kill()` — terminate the child process.
6. Implement `PtyHandle::is_alive() -> bool`.
7. **Tests:** spawn `echo hello`, read output, verify "hello\n".

### Step 2: AiSession lifecycle

1. Create `crates/lune-ai/src/session.rs` with `AiSession`.
2. Implement `AiSession::start(kind, env, size) -> Result<AiSession>`:
   - Resolve command path based on `AiClientKind`.
   - Spawn PTY with the command.
   - Start a reader thread that sends `AiEvent::Output` to the event channel.
   - Monitor for process exit → send `AiEvent::SessionEnded`.
3. Implement `AiSession::send_input(text)` — write to PTY stdin.
4. Implement `AiSession::stop()` — kill process, clean up.
5. Implement `AiSession::resize(rows, cols)` — propagate to PTY.
6. **Tests:** start a session with `/bin/cat`, send input, read output back.

### Step 3: Session manager

1. Create `crates/lune-ai/src/manager.rs`:
   ```rust
   pub struct AiManager {
       sessions: HashMap<AiSessionId, AiSession>,
       active_session: Option<AiSessionId>,
       config: AiConfig,
       event_tx: Sender<AppEvent>,
   }
   ```
2. Implement `new_session()`, `close_session(id)`, `get_active()`, `switch_session(id)`.
3. Support multiple concurrent sessions (e.g., one Claude Code, one custom tool).
4. **Tests:** create two sessions, switch between them, close one.

### Step 4: Context provider

1. Create `crates/lune-ai/src/context.rs` with `EditorContext` and conversion methods.
2. Implement `EditorContext::collect(app_state: &AppState) -> EditorContext`:
   - Read active buffer's file path, cursor, language.
   - Read open tabs from `TabManager`.
   - Read selection text from active buffer.
   - Read git branch/status from `GitService`.
3. Implement encoding strategies:
   - `to_env_vars() -> HashMap<String, String>`:
     ```
     LUNE_CTX_FILE=/path/to/file.rs
     LUNE_CTX_LINE=42
     LUNE_CTX_COL=13
     LUNE_CTX_LANGUAGE=rust
     LUNE_CTX_SELECTION=<selected text>
     LUNE_CTX_WORKSPACE=/project/root
     LUNE_CTX_GIT_BRANCH=main
     LUNE_CTX_MODIFIED_FILES=file1.rs,file2.rs
     ```
   - `to_temp_file() -> Result<PathBuf>`: write JSON context to a temp file.
   - `to_cli_args() -> Vec<String>`: convert to `--file`, `--line`, etc.
4. **Tests:** collect context from a mock state, verify env var encoding.

### Step 5: Embedded terminal widget

1. Create `lune-ui/src/widgets/terminal.rs`.
2. Implement a VT100/xterm-compatible terminal emulator widget:
   - Parse ANSI escape sequences from PTY output.
   - Maintain a character grid (rows × cols) with styling per cell.
   - Handle: cursor movement, color codes (256-color + truecolor), scrolling, line wrapping.
   - Scrollback buffer (configurable, default 10000 lines).
3. Render the terminal grid within the AI panel's `Rect` using ratatui.
4. Keyboard input in the terminal widget is forwarded to the PTY stdin.
5. Mouse: scroll for scrollback, click could be passed through if the AI client supports it.
6. **Option:** If a suitable terminal emulator crate exists (e.g., `vt100`, `alacritty_terminal`), wrap it instead of implementing from scratch.
7. **Verify:** run `htop` or `vim` inside the embedded terminal — they should render correctly.

### Step 6: AI command patterns

1. Create `lune-ui/src/ai_commands.rs` with predefined AI invocation patterns:
   - **"Ask about selection"**:
     1. Collect `SelectionContext`.
     2. Open/focus AI panel.
     3. Start session (if not running) with context env vars.
     4. Send prompt: `"Explain this code:\n\n<selection_text>"` to the AI stdin.
   - **"Refactor file"**:
     1. Collect `FileContext`.
     2. Send: `"Refactor <filepath>: <user instruction>"`.
   - **"Summarize changes"**:
     1. Collect `GitStatusSummary`.
     2. Send: `"Summarize these changes:\n<file list with short diffs>"`.
   - **"Ask question"** (free-form):
     1. Open AI panel with context.
     2. Focus input — user types their own prompt.
2. Register these as `AppCommand` variants and bind to keybindings:
   - `Ctrl-Shift-A` → Ask about selection.
   - `Ctrl-Shift-R` → Refactor file.
   - `Ctrl-Shift-G` → Summarize changes.
   - `` Ctrl-` `` → Toggle AI panel / focus AI terminal.
3. **Verify:** select code, invoke "Ask about selection", see Claude Code receive the context.

### Step 7: AI panel UI

1. The AI panel occupies the right sidebar (toggled with keybinding).
2. Layout:
   - Top: session status bar (client name, session state, uptime).
   - Middle: terminal emulator widget (main area).
   - Bottom: optional quick-action buttons (Ask, Refactor, Summarize).
3. Multiple sessions shown as tabs within the AI panel.
4. Panel resizing works (drag left border).
5. **Verify:** AI panel renders, terminal shows Claude Code output, switch between sessions.

### Step 8: Session persistence across panel toggles

1. Hiding the AI panel does NOT terminate the session.
2. Re-showing the panel restores the terminal state (scrollback preserved).
3. Terminal resizes to match the new panel dimensions.
4. **Verify:** start AI session, hide panel, show panel — session is still running.

### Step 9: Error handling and resilience

1. If AI client binary is not found: show error notification, not a crash.
2. If session crashes: update state to `Error`, show error in terminal widget, offer "Restart" action.
3. If PTY read fails: log error, mark session as `Terminated`.
4. Timeout: if a session is unresponsive for >30s, show a warning indicator.
5. **Verify:** start session with invalid binary path, see graceful error. Kill AI process externally, see error state.

---

## Acceptance Criteria

- [ ] Claude Code (or any CLI) launches in an embedded PTY terminal
- [ ] Terminal emulator renders ANSI colors, cursor movement, and scrollback
- [ ] Editor context (file, selection, cursor, tabs, git) is passed to AI client
- [ ] "Ask about selection" command works end-to-end
- [ ] Multiple AI sessions can run concurrently
- [ ] AI panel toggles without killing sessions
- [ ] AI client crash does not crash the editor
- [ ] Terminal resizes correctly when panel is resized
- [ ] Keyboard input is properly forwarded to the AI process

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Terminal emulation is extremely complex (full xterm compat) | Use an existing crate (`vt100` or `alacritty_terminal` as a library); don't build from scratch |
| `portable-pty` may not work on all platforms | Test on Linux first; Windows PTY (ConPTY) has known quirks — document limitations |
| Claude Code's context format may change | Context provider is abstracted behind traits; format is configuration-driven |
| ANSI escape sequence parsing edge cases | Use battle-tested parser; focus on 256-color + basic sequences for V1 |
| AI client may produce enormous output (huge diffs) | Cap terminal scrollback; offer "clear" command; paginate if needed |
