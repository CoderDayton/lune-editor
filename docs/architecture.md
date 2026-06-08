# Architecture

Lune is a Cargo workspace: a thin `lune-editor` binary over four library crates.

```text
lune-editor/
├── src/main.rs        # CLI entry point, config loading, runtime wiring
└── crates/
    ├── lune-core/     # buffers, undo, search, settings, workspace, crash recovery
    ├── lune-ui/       # ratatui TUI: event loop, widgets, vim, themes, effects
    ├── lune-ai/       # AI manager, PTY sessions, client abstraction
    └── lune-git/      # git service (libgit2): status, diffs, stage/unstage
```

## Crates

| Crate | Responsibility |
|-------|----------------|
| `lune-core` | Rope-based text buffers, undo/redo, search, tree-sitter parsing, settings, workspace and crash-recovery state. No UI. |
| `lune-ui` | The terminal app — event loop, widgets, vim state machine, theming, visual effects. Depends on the other three crates. |
| `lune-ai` | AI session manager: launches CLI AI tools in PTYs and exposes their output as sessions. |
| `lune-git` | Git operations over libgit2: status, diffs, per-hunk staging. |

## Key dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | Immediate-mode TUI rendering |
| `ropey` | Rope-based text buffers |
| `tree-sitter` | Incremental syntax parsing |
| `git2` | libgit2 bindings |
| `tokio` | Async runtime (AI / PTY / file watcher) |
| `sled` | Embedded key-value store (workspace state) |
| `tachyonfx` | Terminal visual effects |
| `smallvec` | Stack-allocated small collections on hot paths |

## Event loop

Built on the rat-salsa framework:

```text
crossterm input → AppCommand dispatch → state update → ratatui render → flush
```

The file watcher, autosave timer, and AI PTY output are additional async event
sources feeding the same loop.

## State persistence

Open files, scroll offsets, layout, and the active tab are saved to a
per-workspace `sled` database under `~/.config/lune-editor/state/`. Dirty buffers
are captured on crash and restored on the next launch.
