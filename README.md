<div align="center">

<img src="https://raw.githubusercontent.com/CoderDayton/lune-editor/main/assets/banner.png" alt="Lune Editor" width="440">

A terminal code editor written in Rust. It opens fast, runs entirely in your terminal, and comes with the things you'd normally add through plugins: a Git panel, tree-sitter highlighting, an embedded terminal, and a way to run CLI AI tools next to your code.

[![CI](https://github.com/CoderDayton/lune-editor/actions/workflows/ci.yml/badge.svg)](https://github.com/CoderDayton/lune-editor/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Edition](https://img.shields.io/badge/edition-2024-purple)](https://doc.rust-lang.org/edition-guide/rust-2024/)

Multi-buffer editing · Vim mode · Git panel · Embedded terminal · Tree-sitter highlighting · AI sessions

</div>

---

## Features

- Multi-buffer editing on a rope-based text engine (ropey)
- Optional Vim mode with Normal, Insert, Visual, Visual Line, and Command modes
- Tree-sitter syntax highlighting for a growing list of languages, including Rust, Python, JavaScript, TypeScript, and Go
- A native Git panel: stage and unstage files, work with individual hunks, and read diffs inline in unified or side-by-side view
- Find and replace with live match highlighting
- Inline dialogs for creating, renaming, and deleting files
- A fuzzy language selector for overriding the detected language
- Notification toasts that fade out on their own
- Run CLI AI tools in an embedded terminal session, right next to your code
- Crash recovery and workspace state that persists between runs
- Two built-in themes plus any TOML themes you drop in, switchable while running

---

## Quick start

You'll need Rust 1.90 or newer (2024 edition) and a C compiler, which tree-sitter and libgit2 build against.

```bash
git clone https://github.com/CoderDayton/lune-editor
cd lune-editor
cargo build --release       # binary at target/release/lune
cargo install --path .      # optional: puts `lune` on your PATH

# open a project
lune ~/my-project

# with vim mode and a theme
lune ~/my-project --vim --theme "Lune Dark"
```

On the first run, press `Ctrl+P` to open the command palette.

---

## Usage

```
lune [OPTIONS] [PATH]...

Arguments:
  [PATH]...  File(s) or directory to open (defaults to the current directory)

Options:
      --vim            Enable vim mode
      --no-vim         Disable vim mode
      --theme <THEME>  Theme name, for example "Lune Dark"
      --config <PATH>  Use a specific config file
  -h, --help           Print help
      --version        Print version
```

### A few ways people use it

Web or Python project, with vim mode:

```bash
lune ~/my-project --vim
# file tree on the left, AI panel on the Agents tab, git panel on demand (Ctrl+G)
```

Reviewing a repo before committing:

```bash
lune /path/to/repo
# Ctrl+G opens the git panel: s to stage, u to unstage, c to commit
```

AI-assisted editing:

```bash
lune ~/my-project
# select some code, then Ctrl+K a to ask the AI about it
```

---

## Configuration

Config lives at `~/.config/lune-editor/config.toml` (or under `$XDG_CONFIG_HOME` if that's set). Per-project overrides go in `.lune/config.toml` at the workspace root. Both are TOML and layer over the built-in defaults.

```toml
[editor]
tab_size = 4
vim_mode = false
cursor_style = "block"   # block, bar, or underline

[ui]
show_file_tree = true

[ai]
default_client = "claude"

[agents]
placement = "fixed"   # fixed or mouse

theme = "Lune Dark"
```

The [configuration guide](docs/configuration.md) covers every section, field, and default, plus how per-project overrides work.

---

## Keybindings

Defaults, all rebindable in `~/.config/lune-editor/keybindings.toml`:

| Key | Action |
|-----|--------|
| `Ctrl+P` | Command palette |
| `Ctrl+S` | Save |
| `Ctrl+O` | Open file |
| `Ctrl+N` | New file |
| `Ctrl+F` | Find |
| `Ctrl+H` | Replace |
| `Ctrl+B` | Toggle file tree |
| `Ctrl+G` | Toggle Git panel |
| `Ctrl+1` / `Ctrl+2` | Editor tab / Agents tab |
| `` Ctrl+` `` | Toggle the Agents (AI) tab |
| `Ctrl+L` | Language selector |
| `Ctrl+T` | Theme selector |
| `Ctrl+Alt+V` | Toggle Vim mode |
| `F1` | Keybinding hints |

`Ctrl+K` is a leader key. Press it, then a second key: `a` to ask the AI about the selection, `r` to refactor the file, `c` to summarize changes, `s` to save all, `f` to find in files, `m` to toggle markdown preview, `n` to dismiss notifications, `w` to close the AI session.

The [keybindings guide](docs/keybindings.md) has the full list, including the Agents tab and Vim mode.

---

## AI sessions

Lune Editor can run a CLI AI tool in an embedded terminal session, so the tool can run commands and see output right next to your code.

Install a tool like Claude Code or OpenCode and make sure it's on your `PATH`. Lune Editor launches it as a subprocess, so sign-in and API keys are handled by the tool itself, not by Lune Editor.

To ask about a selection, press `Ctrl+K` then `a`, or use the command palette. That opens an AI prompt with your current selection as context.

---

## Git panel

Press `Ctrl+G` to open the Git panel.

| Key | Action |
|-----|--------|
| `s` | Stage the selected file |
| `u` | Unstage the selected file |
| `d` | Discard changes to the selected file |
| `c` | Commit |
| `r` | Refresh |
| `Enter` | Open the selected file in the editor |

The panel shows each file's diff inline, in unified or side-by-side mode. Hunk-level actions, staging, unstaging, and discarding a single hunk, are available from the command palette.

---

## Themes

Lune Editor ships with two themes: Lune Dark (the default) and Lune Light. Switch with `Ctrl+T`, or set `theme = "Lune Light"` in your config. Drop your own TOML themes in `~/.config/lune-editor/themes/`.

The [theming guide](docs/theming.md) covers the built-in themes, the config schema, and writing your own.

---

## Architecture

Lune Editor is a Cargo workspace: a small binary over five library crates.

```text
lune-editor/
├── src/main.rs            # CLI entry and config loading
└── crates/
    ├── lune-core/         # buffers, settings, search, undo, recovery, persistence
    ├── lune-ui/           # ratatui TUI: event loop, widgets, vim, themes
    ├── lune-ai/           # AI session manager and PTY sessions
    ├── lune-git/          # git service (libgit2)
    └── lune-ts-highlight/ # tree-sitter highlighting with incremental reparsing
```

The [architecture guide](docs/architecture.md) goes into the crate breakdown, key dependencies, the event loop, and how state is persisted.

---

## Documentation

- [Configuration guide](docs/configuration.md): every setting, default, and how overrides work
- [Keybindings guide](docs/keybindings.md): default bindings, Vim mode, and custom bindings
- [Theming guide](docs/theming.md): built-in themes, the config schema, and custom themes
- [Testing guide](docs/guides/testing.md): running the suite, snapshots, and property tests
- [Architecture guide](docs/architecture.md): crates, dependencies, the event loop, and persistence

---

## Contributing

```bash
cargo build
cargo test --workspace
cargo fmt --check && cargo clippy --workspace -- -D warnings   # before submitting
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, the crate map, and commit style.

---

## License

MIT. See [LICENSE](LICENSE).
