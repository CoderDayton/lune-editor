# Contributing to Lune Editor

## Prerequisites

- Rust 1.85+ (2024 edition)
- A C compiler (for tree-sitter and libgit2)

## Build

```bash
cargo build
```

## Test

```bash
cargo test --workspace
```

Snapshot tests use `insta`. To review and accept snapshot changes:

```bash
cargo insta test --workspace --accept
```

## Before submitting

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Guidelines

- Run `cargo fmt` before committing.
- New widgets go in `crates/lune-ui/src/widgets/` and implement `ratatui::widgets::Widget`.
- New languages: add a `tree-sitter-*` workspace dep and register in `crates/lune-core/src/syntax/language.rs`.
- All `unsafe` requires a `// SAFETY:` comment.
- Open an issue before starting large changes.

## Crate overview

| Crate | Responsibility |
|-------|---------------|
| `lune-core` | Buffers, undo, search, settings, workspace, crash recovery |
| `lune-ui` | ratatui TUI: event loop, widgets, vim, themes, effects |
| `lune-ai` | AI manager, PTY sessions, client abstraction |
| `lune-git` | Git service (libgit2): status, diffs, stage/unstage |

## Commit style

```
feat(scope): short description
fix(scope): short description
test: description
refactor(scope): description
```
