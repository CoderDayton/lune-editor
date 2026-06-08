<div align="center">

# 🌙 Lune Editor

**A fast, agentic terminal editor built in Rust.**

[![CI](https://github.com/user/lune-editor/actions/workflows/ci.yml/badge.svg)](https://github.com/user/lune-editor/actions)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Edition](https://img.shields.io/badge/edition-2024-purple)](https://doc.rust-lang.org/edition-guide/rust-2024/)

*Multi-buffer editing · Vim mode · Built-in AI · Git panel · Embedded terminal · Tree-sitter syntax*

</div>

---

## Why Lune?

Most terminal editors make you choose: raw speed *or* modern features. Lune doesn't.

It runs entirely in your terminal, starts instantly, and stays out of your way — while shipping a native Git panel, incremental tree-sitter highlighting, an embedded PTY terminal, and AI assistance that sees your editor context. No plugins required.

---

## Features

- Multi-buffer editing with rope-based text engine (ropey)
- Optional Vim mode (Normal/Insert/Visual/V-Line)
- Tree-sitter syntax highlighting for 20+ languages
- Native Git panel — stage/unstage files and hunks, per-hunk diff view
- Find & replace with live search highlighting
- File operation dialogs — create, rename, delete with inline prompts
- Language selector overlay (Ctrl+L) with fuzzy filter
- Notification toasts with fade-out animations
- Visual effects via tachyonfx (diff fade-in, panel transitions, focus glow)
- Embedded AI session manager — Claude Code, Aider, Gemini, shell, etc.
- Crash recovery + workspace persistence (sled)
- TOML themes with live switching

---

## Quick Start

Requires **Rust 1.85+** (2024 edition) and a C compiler (for tree-sitter and libgit2).

```bash
git clone https://github.com/user/lune-editor
cd lune-editor
cargo build --release       # binary at target/release/lune
cargo install --path .      # optional: put `lune` on your PATH

# Open a project
lune ~/my-project

# …with vim mode and a theme
lune ~/my-project --vim --theme "Lune Dark"
```

First run? Press `Ctrl+P` for the command palette.

---

## Usage

```
lune [OPTIONS] [PATH]

Arguments:
  [PATH]  File or directory to open (defaults to current directory)

Options:
      --vim            Enable vim mode
      --theme <THEME>  Theme name (e.g. "Lune Dark")
  -h, --help           Print help
  -V, --version        Print version
```

### Common workflows

**Web / Python developer**
```bash
lune ~/my-project --vim
# File tree left · AI panel right · git panel on demand (Ctrl+G)
```

**Open-source contributor**
```bash
lune /path/to/repo
# Git panel shows staged/unstaged · press s/u to stage/unstage
# Open diff view from git panel (d key) before committing
```

**AI-assisted refactoring**
```bash
lune ~/my-project
# Ctrl+` to toggle AI panel
# Select code · Ctrl+K a to ask AI
```

---

## Configuration

Global config lives at `~/.config/lune-editor/config.toml`; per-project overrides go in `.lune/config.toml`. Both are TOML, layered over the built-in defaults.

```toml
[editor]
tab_size = 4
vim_mode = false
cursor_style = "block"   # block | bar | underline

[ui]
show_file_tree = true

[ai]
default_client = "claude"

[agents]
placement = "fixed"   # fixed | mouse

theme = "Lune Dark"
```

See the **[configuration guide](docs/configuration.md)** for every section, field, default, and workspace-override behavior.

---

## Keybindings

Defaults (all rebindable in `~/.config/lune-editor/keybindings.toml`):

| Key | Action |
|-----|--------|
| `Ctrl+P` | Command palette |
| `Ctrl+S` | Save · `Ctrl+O` open file · `Ctrl+N` new file |
| `Ctrl+F` | Find · `Ctrl+H` replace |
| `Ctrl+B` | File tree · `Ctrl+G` git panel |
| `Ctrl+1` / `Ctrl+2` | Editor / Agents tab |
| `Ctrl+T` | Select theme |
| `Ctrl+K` | Leader: then `a` ask AI · `f` find in files · `m` markdown · `s` save all |
| `Ctrl+Alt+V` | Toggle Vim mode |

See the **[keybindings guide](docs/keybindings.md)** for the full list, Agents-tab keys, Vim mode, and custom bindings.

---

## AI Integration

Lune ships a native AI manager that launches any CLI AI tool (`claude`, `opencode`, etc.) in a PTY session so it can execute commands and see terminal output alongside your code.

**Setup:** Install your preferred CLI AI tool and ensure it's on your `PATH`. Lune launches it as a subprocess — auth is handled by the client itself.

```bash
lune ~/my-project
```

**Point query** (`Ctrl+K` then `a`, or the command palette): opens the AI prompt with your current selection as context — just type your question.

Lune launches the AI client as a subprocess in the embedded terminal using your local install. No API key configuration inside Lune is required — the client handles auth however it normally does.

---

## Git Panel

Press `Ctrl+G` to open the Git panel.

| Key | Action |
|-----|--------|
| `s` | Stage file |
| `u` | Unstage file |
| `d` | Open diff view |

In the diff view, `GitStageHunk`, `GitUnstageHunk`, and `GitDiscardHunk` actions are available via the `Ctrl+P` command palette.

The diff view (`widgets/diff_view.rs`) renders inline unified diffs with syntax-highlighted context.

---

## Themes

Two themes ship with Lune — **Lune Dark** (default) and **Lune Light** —
switch with `Ctrl+T` or `theme = "Lune Light"` in your config. Drop your
own TOML themes in `~/.config/lune-editor/themes/`.

See the **[theming guide](docs/theming.md)** for the built-in themes, the
full config schema, and how to write a custom theme.

---

## Architecture

Lune is a Cargo workspace: a thin binary over four library crates.

```text
lune-editor/
├── src/main.rs        # CLI entry, config loading
└── crates/
    ├── lune-core/     # buffers, settings, tree-sitter, search, undo, recovery
    ├── lune-ui/       # ratatui TUI: event loop, widgets, vim, themes
    ├── lune-ai/       # AI manager, PTY sessions
    └── lune-git/      # git service (libgit2)
```

See the **[architecture guide](docs/architecture.md)** for the crate breakdown, key dependencies, the event loop, and state persistence.

---

## Documentation

- [Configuration guide](docs/configuration.md) — every setting, default, and workspace overrides
- [Keybindings guide](docs/keybindings.md) — default bindings, Vim mode, and custom bindings
- [Theming guide](docs/theming.md) — built-in themes, config schema, and custom themes
- [Testing guide](docs/guides/testing.md) — running the suite, snapshots, and property tests
- [Architecture guide](docs/architecture.md) — crates, dependencies, the event loop, and persistence

---

## Contributing

```bash
cargo build
cargo test --workspace
cargo fmt --check && cargo clippy --workspace -- -D warnings   # before submitting
```

See **[CONTRIBUTING.md](CONTRIBUTING.md)** for prerequisites, guidelines, the crate map, and commit style.

---

## License

MIT — see [LICENSE](LICENSE).

---

<div align="right"><a href="#-lune-editor">↑ back to top</a></div>
