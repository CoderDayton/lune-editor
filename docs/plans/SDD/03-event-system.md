# 03 — Event System

> **Phase:** 1 (Foundation)
> **Estimated effort:** 2–3 sessions (~6–8 hours)
> **Prerequisites:** [01-project-scaffold.md](01-project-scaffold.md), [02-editor-core.md](02-editor-core.md)

## Goal

Implement the event loop using rat-salsa + crossterm, define the internal event model (`AppEvent`), build the focus-based event routing system, and implement the vim mode state machine. After this plan, the application runs an interactive TUI loop that responds to keyboard/mouse input.

---

## Types & Structures

### Application Events

```rust
/// Internal event type that unifies all input sources.
pub enum AppEvent {
    /// Keyboard input from crossterm
    Key(KeyEvent),
    /// Mouse input from crossterm
    Mouse(MouseEvent),
    /// Terminal resize
    Resize(u16, u16),
    /// File system change notification
    Fs(FsEvent),
    /// AI session event
    Ai(AiEvent),
    /// Timer tick (for animations, auto-save, etc.)
    Tick,
    /// Application-level command (from command palette, keybinding, etc.)
    Command(AppCommand),
}

pub enum FsEvent {
    FileChanged(PathBuf),
    FileCreated(PathBuf),
    FileDeleted(PathBuf),
}

pub enum AiEvent {
    Output(AiSessionId, String),
    SessionEnded(AiSessionId),
    Error(AiSessionId, String),
}
```

### Application Commands

```rust
/// High-level commands decoupled from specific keybindings.
pub enum AppCommand {
    Quit,
    Save,
    SaveAll,
    OpenFile(PathBuf),
    CloseTab,
    NextTab,
    PrevTab,
    ToggleFileTree,
    ToggleAiPanel,
    ToggleGitPanel,
    OpenCommandPalette,
    FocusPanel(PanelId),
    // Editor commands
    Undo,
    Redo,
    Find,
    Replace,
    // Vim mode
    EnterNormalMode,
    EnterInsertMode,
    EnterVisualMode,
}
```

### Focus Model

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PanelId {
    FileTree,
    Editor,
    AiTerminal,
    GitPanel,
    CommandPalette,
    StatusBar,
}

pub struct FocusManager {
    active: PanelId,
    history: Vec<PanelId>,  // for focus-return (e.g., close palette → return to editor)
}
```

### Event Outcome

```rust
/// Return value from event handlers indicating what the loop should do.
pub enum EventOutcome {
    /// Event was consumed, trigger re-render
    Consumed,
    /// Event was consumed, no re-render needed
    ConsumedNoRender,
    /// Event was not handled, propagate to parent
    NotHandled,
    /// Request application quit
    Quit,
}
```

### Vim Mode

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,  // : command line
}

pub struct VimState {
    pub mode: VimMode,
    pub count: Option<usize>,      // numeric prefix (e.g., 5j)
    pub pending_op: Option<VimOp>, // operator pending (e.g., d awaiting motion)
    pub last_command: Option<VimCommand>, // for . repeat
    pub register: char,            // active register (default ")
}

pub enum VimOp {
    Delete,
    Yank,
    Change,
    Indent,
    Outdent,
}
```

---

## Implementation Steps

### Step 1: Crossterm + rat-salsa event loop skeleton

1. In `lune-ui/src/app.rs`, create the main `App` struct:
   ```rust
   pub struct App {
       state: AppState,
       should_quit: bool,
   }
   ```
2. Implement the rat-salsa application loop:
   - Initialize crossterm (enable raw mode, alternate screen, mouse capture).
   - Create a `Terminal<CrosstermBackend>`.
   - Enter the main loop: poll events → handle → render.
   - On quit: restore terminal state.
3. Register a tick timer (e.g., 50ms) for animation and auto-save ticks.
4. **Verify:** app starts, shows blank screen, exits on `q` or `Ctrl-C`.

### Step 2: AppEvent and event conversion

1. Create `lune-ui/src/event.rs` with `AppEvent`, `FsEvent`, `AiEvent`, `AppCommand` enums.
2. Implement `From<crossterm::event::Event> for AppEvent` conversion.
3. Channel-based architecture: background threads send `AppEvent` to a single `mpsc::Receiver` consumed by the main loop.
4. **Tests:** conversion of key events, mouse events, resize events.

### Step 3: Focus manager

1. Create `lune-ui/src/focus.rs` with `FocusManager`.
2. Implement `focus(panel)` — push current to history, set new active.
3. Implement `focus_return()` — pop from history.
4. Implement `is_focused(panel)` check.
5. Focus determines which widget receives keyboard events; mouse events are routed by position.
6. **Tests:** focus transitions, return behavior, empty history edge case.

### Step 4: Event routing

1. Define a `HandleEvent` trait (or use rat-event's `HandleEvent`):
   ```rust
   pub trait HandleAppEvent {
       fn handle_event(&mut self, event: &AppEvent, ctx: &mut EventContext) -> EventOutcome;
   }
   ```
2. The root handler checks focus and dispatches:
   - If `CommandPalette` focused → route to palette.
   - Else route to `active_panel`'s handler.
   - Global keybindings (e.g., `Ctrl-Q` quit, `Ctrl-P` palette) are checked first.
3. Mouse events: determine which panel the click/scroll targets (by checking layout rects), focus that panel, then dispatch.
4. **Tests:** verify routing logic with mock handlers.

### Step 5: Keybinding system

1. Create `lune-ui/src/keybindings.rs`.
2. Define a `Keymap` as `HashMap<KeyCombo, AppCommand>` where:
   ```rust
   pub struct KeyCombo {
       pub key: KeyCode,
       pub modifiers: KeyModifiers,
   }
   ```
3. Provide a default keymap for normal mode:
   - `Ctrl-Q` → Quit
   - `Ctrl-S` → Save
   - `Ctrl-P` → OpenCommandPalette
   - `Ctrl-W` → CloseTab
   - `Ctrl-Tab` / `Ctrl-Shift-Tab` → NextTab / PrevTab
   - `Ctrl-B` → ToggleFileTree
   - `Ctrl-F` → Find
   - `Ctrl-H` → Replace
   - `Ctrl-Z` → Undo
   - `Ctrl-Y` → Redo
4. Keymap lookup happens in the event routing before panel-specific handling.
5. **Tests:** keymap lookup, modifier handling, no conflict between global and panel bindings.

### Step 6: Vim mode state machine

1. Create `lune-ui/src/vim.rs` with `VimState`, `VimMode`, `VimOp`.
2. Implement the mode transition logic:
   - Normal → Insert: `i`, `a`, `o`, `O`, `I`, `A`
   - Normal → Visual: `v`, `V`
   - Any → Normal: `Esc`
   - Normal → Command: `:`
3. Implement operator-pending mode: `d`, `y`, `c` wait for a motion.
4. Implement motions: `h`, `j`, `k`, `l`, `w`, `b`, `e`, `0`, `$`, `gg`, `G`.
5. Implement numeric prefix: accumulate digits before command.
6. Implement `.` repeat for the last change command.
7. The editor widget checks `VimState.mode` to decide input behavior:
   - Insert mode: characters are typed directly.
   - Normal mode: characters are commands.
   - Visual mode: motions extend selection.
8. **Tests:** mode transitions, `d2w` (delete 2 words), `5j` (move down 5), `.` repeat, `Esc` from all modes.

### Step 7: Wire into main.rs

1. Update `src/main.rs` to:
   - Parse CLI args (optional file path).
   - Initialize `AppState` with a `BufferRegistry`.
   - If file arg provided, open it into a buffer.
   - Start the `App` event loop.
2. **Verify:** `cargo run -- somefile.txt` opens the app, displays a blank TUI, responds to quit keybinding.

---

## Acceptance Criteria

- [ ] Application starts and enters an interactive TUI loop
- [ ] `Ctrl-C` or `Ctrl-Q` cleanly exits, restoring terminal state
- [ ] Keyboard events are properly dispatched based on focus
- [ ] Mouse clicks change focus to the targeted panel
- [ ] Vim mode transitions work: Normal ↔ Insert ↔ Visual, with Esc returning to Normal
- [ ] Vim motions move the cursor correctly (tested via unit tests against buffer state)
- [ ] Default keybindings are functional for core commands
- [ ] No panics on resize, rapid input, or edge-case key combos

---

## Risks

| Risk | Mitigation |
|------|-----------|
| rat-salsa API may differ from assumed model | Read rat-salsa source/examples carefully; adapt the event loop to its actual trait requirements |
| Vim mode is a deep rabbit hole | V1 implements a minimal subset: basic motions, insert/normal/visual, no macros/registers/marks |
| Mouse + vim coexistence edge cases | Mouse always works regardless of mode; mouse clicks implicitly enter insert mode at click position (or stay in current mode with updated cursor) |
| Event ordering under rapid input | Use crossterm's event queue directly; don't drop events |
