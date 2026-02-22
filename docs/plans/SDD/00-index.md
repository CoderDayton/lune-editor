# Lune Editor — SDD Implementation Plan

> Comprehensive build plan derived from [docs/SDD.md](../../SDD.md).
> Each numbered file in this directory covers one vertical slice.

## Naming

The SDD references "Tachyon Editor"; the crate is `lune-editor`. All plan files use **Lune Editor** as the canonical name.

---

## Phases

Implementation is organized into 4 phases. Each phase produces a usable (if incomplete) binary. Dependencies flow downward — later phases build on earlier ones.

### Phase 1 — Foundation (files 01–04)

Establish the runnable skeleton: workspace structure, editor core data model, event loop, and basic UI shell.

| File | Scope | Key Deliverables |
|------|-------|------------------|
| [01-project-scaffold.md](01-project-scaffold.md) | Workspace, crates, deps, CI | Cargo workspace with `crates/{ui,core,ai,git}`, dependency manifest, `cargo build` passes, basic CI |
| [02-editor-core.md](02-editor-core.md) | Buffer model, rope, undo/redo | `TextBuffer`, `BufferRegistry`, cursor/selection, undo/redo transactions, search/replace |
| [03-event-system.md](03-event-system.md) | Event loop, input, vim mode | rat-salsa loop, `AppEvent` dispatch, focus routing, vim mode state machine |
| [04-ui-layout.md](04-ui-layout.md) | VS Code–inspired layout | Root layout splits, tab bar, editor pane, status bar, panel toggle, command palette shell |

**Phase 1 exit criteria:** Open a file from CLI arg, render it in a tabbed editor pane with status bar, navigate with keyboard/mouse, edit text with undo/redo.

---

### Phase 2 — Workspace & Navigation (files 05–06)

Add file tree, workspace abstraction, and syntax highlighting so the editor is usable for real code navigation.

| File | Scope | Key Deliverables |
|------|-------|------------------|
| [05-file-tree.md](05-file-tree.md) | Workspace, file tree, file ops | `Workspace` struct, lazy directory tree widget, create/rename/delete, `notify`-based watcher |
| [06-syntax-highlighting.md](06-syntax-highlighting.md) | Syntax highlighting | Tree-sitter integration (or regex fallback), language detection, theme-aware coloring |

**Phase 2 exit criteria:** Open a directory, browse via toggleable file tree, open files into tabs, see syntax-highlighted code.

---

### Phase 3 — Git & AI (files 07–08)

Integrate version control and the embedded AI client — the two differentiating features.

| File | Scope | Key Deliverables |
|------|-------|------------------|
| [07-git-integration.md](07-git-integration.md) | Git service, gutter, staging | `GitService`, inline gutter markers, stage/unstage/commit, diff view panel |
| [08-ai-integration.md](08-ai-integration.md) | PTY manager, context, terminal | `AiSession`, embedded terminal widget, context provider, command patterns |
**Phase 3 exit criteria:** Git status in gutter, stage/commit from editor, launch Claude Code with editor context.

---

### Phase 4 — Polish & Robustness (files 10–12)

Visual effects, persistence, and comprehensive testing.

| File | Scope | Key Deliverables |
|------|-------|------------------|
| [10-effects.md](10-effects.md) | tachyonfx visual effects | Focus glow, diff animations, AI thinking indicator, effect DSL bindings |
| [11-persistence.md](11-persistence.md) | Settings, themes, keymaps | TOML config files, theme system, keymap customization, workspace state save/restore |
| [12-testing.md](12-testing.md) | Test strategy | Unit tests (buffer, diff, git), integration tests (event routing, PTY), manual scenarios |

**Phase 4 exit criteria:** Polished visual experience, persistent user settings, crash recovery, >80% unit test coverage on core modules.

---

## Crate Layout

```
lune-editor/
├── Cargo.toml              # workspace root
├── crates/
│   ├── lune-core/          # buffer, rope, undo/redo, diff, search
│   ├── lune-ui/            # ratatui widgets, layout, effects, event routing
│   ├── lune-ai/            # PTY manager, context provider
│   └── lune-git/           # Git service (libgit2/CLI wrapper)
├── src/
│   └── main.rs             # binary entry point, wires crates together
├── docs/
│   ├── SDD.md
│   └── plans/SDD/          # this directory
└── tests/                  # integration tests
```

## Dependency Graph

```
main.rs
  └─ lune-ui
       ├─ lune-core
       ├─ lune-ai
       │    └─ lune-core
       └─ lune-git
            └─ lune-core
```

## Key External Crates

| Crate | Purpose | Phase |
|-------|---------|-------|
| `ratatui` | Terminal UI framework | 1 |
| `crossterm` | Terminal backend | 1 |
| `rat-salsa` | Event loop | 1 |
| `rat-widget` | Extended widgets + rat-event | 1 |
| `ropey` | Rope data structure for buffers | 1 |
| `tachyonfx` | Post-render visual effects | 4 |
| `notify` | File system watcher | 2 |
| `tree-sitter` | Syntax highlighting | 2 |
| `git2` | libgit2 bindings | 3 |
| `portable-pty` | PTY for AI client | 3 |
| `similar` | Diff algorithm | 3 |
| `uuid` | Buffer/session IDs | 1 |
| `serde` + `toml` | Config serialization | 4 |
| `tokio` or `crossbeam` | Async/channel primitives | 1 |

## Conventions

- Each plan file follows the same structure: **Goal → Prerequisites → Types → Implementation Steps → Acceptance Criteria → Risks**.
- Implementation steps are numbered and sized to be completable in ~1 session (2–4 hours).
- Types/traits shown are illustrative; actual signatures will evolve during implementation.
- All code changes should pass `cargo clippy`, `cargo test`, and `cargo build --release` before merging.
