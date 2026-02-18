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

## Table of Contents

- [Why Lune?](#why-lune)
- [Features](#features)
- [Quick Start](#quick-start)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Keybindings](#keybindings)
- [AI Integration](#ai-integration)
- [Git Panel](#git-panel)
- [Themes](#themes)
- [Architecture](#architecture)
- [Contributing](#contributing)
- [License](#license)

---

## Why Lune?

Most terminal editors make you choose: raw speed *or* modern features. Lune doesn't.

It runs entirely in your terminal, starts instantly, and stays out of your way — while shipping a native Git panel, incremental tree-sitter highlighting, an embedded PTY terminal, and live AI assistance that sees your editor context. No plugins required.

---

## Features

- ⚡ **Zero-compromise performance** — LTO + codegen-units=1 release build; rope-based buffers handle multi-MB files without stutter
- 🌲 **Tree-sitter syntax highlighting** — incremental, O(1) amortized per keystroke; language registry is extensible
- 🤖 **Built-in AI (Live Mode)** — Claude-backed assistant with PTY session and full editor context; accept/reject suggestions inline
- 🗂 **Native Git panel** — stage, unstage, diff, and inspect status without leaving the editor (libgit2)
- 💻 **Embedded terminal** — full PTY inside a split pane; run build commands, tests, and scripts inline
- 🎨 **TOML themes** — ship-ready dark theme, easy to extend; hot-reload from `~/.config/lune-editor/themes/`
- ⌨️ **Optional Vim mode** — full Normal/Insert state machine, per-workspace toggle, remappable
- 💾 **Crash recovery + workspace restore** — sled-backed persistence saves open files, scroll offsets, and layout across sessions

---

## Quick Start

```bash
# Build from source (requires Rust 1.85+)
git clone https://github.com/user/lune-editor
cd lune-editor
cargo build --release

# Open a project
./target/release/lune ~/my-project

# Open with vim mode and a specific theme
./target/release/lune ~/my-project --vim --theme "Lune Dark"
```

First run? Press `Ctrl+P` to open the command palette and explore everything available.

---

## Installation

<details>
<summary><strong>From source</strong></summary>

**Prerequisites:** Rust 1.85+ (2024 edition), a C compiler (for libgit2/tree-sitter)

```bash
git clone https://github.com/user/lune-editor
cd lune-editor
cargo build --release
# Binary: target/release/lune

# Optional: install to PATH
cargo install --path .
```

</details>

<details>
<summary><strong>Faster local builds (recommended)</strong></summary>

Create `.cargo/config.toml` in the repo root for faster incremental compile:

```toml
[build]
# native CPU features for dev builds
rustflags = ["-C", "target-cpu=native"]

# use mold linker if available (Linux)
# [target.x86_64-unknown-linux-gnu]
# linker = "clang"
# rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

</details>

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
# File tree left · AI panel right · git panel on demand (F5)
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
# Ctrl+` to toggle Live Mode
# Select code · Ctrl+Shift+A to ask AI · accept or reject inline
```

---

## Configuration

Global config lives at `~/.config/lune-editor/config.toml`. Workspace overrides go in `.lune/config.toml` at the project root.

```toml
[editor]
tab_size = 4
use_spaces = true
vim_mode = false
word_wrap = false
line_numbers = true
relative_line_numbers = false
auto_save_interval_secs = 60
scroll_margin = 5

[ui]
show_file_tree = true
file_tree_width_pct = 20
show_ai_panel = false
right_panel_width_pct = 30
effects_enabled = true

[file_tree]
icons = true
sort_dirs_first = true
show_hidden = false

[ai]
default_client = "claude"   # any CLI AI tool: claude, aider, etc.
live_mode_enabled = false

theme = "Lune Dark"
```

**Workspace override example** (`.lune/config.toml`):
```toml
[editor]
tab_size = 2
vim_mode = true

[file_tree]
show_hidden = true
```

---

## Keybindings

Default bindings. All rebindable in `~/.config/lune-editor/keybindings.toml`.

| Key | Action |
|-----|--------|
| `Ctrl+P` | Command palette |
| `Ctrl+S` | Save file |
| `Ctrl+Shift+S` | Save all |
| `Ctrl+W` | Close tab |
| `Ctrl+Tab` | Next tab |
| `F5` | Toggle git panel |
| `F12` | Toggle AI panel |
| `Ctrl+`` ` | Toggle embedded terminal |
| `Ctrl+T` | Cycle themes |

<details>
<summary><strong>Vim mode bindings</strong></summary>

Standard Vim motions apply in Normal mode (`h j k l`, `w b e`, `gg G`, `0 $`, etc.).

| Key | Action |
|-----|--------|
| `i` | Enter Insert mode |
| `Esc` | Return to Normal mode |
| `v` | Visual mode |
| `dd` | Delete line |
| `yy` | Yank line |
| `p` | Paste |
| `/` | Search |
| `:w` | Save |
| `:q` | Quit |

</details>

<details>
<summary><strong>Custom keybindings</strong></summary>

```toml
# ~/.config/lune-editor/keybindings.toml
[normal]
"ctrl+s"       = "save"
"ctrl+shift+s" = "save_all"
"ctrl+p"       = "command_palette"
"f5"           = "toggle_git_panel"
"f12"          = "toggle_ai_panel"
"alt+1"        = "next_theme"
```

Chord bindings (`ctrl+k ctrl+0`) are supported.

</details>

---

## AI Integration

Lune ships a native AI manager that launches any CLI AI tool (`claude`, `aider`, etc.) in a PTY session so it can execute commands and see terminal output alongside your code.

**Setup:** Install your preferred CLI AI tool (`claude`, `aider`, etc.) and ensure it's on your `PATH`. Lune launches it as a subprocess — auth is handled by the client itself.

```bash
lune ~/my-project
```

**Live Mode** (`Ctrl+`` `): streams your editor context (open file, cursor position, visible lines) to the AI session in real time.

**Point query** (`Ctrl+Shift+A`): opens the AI prompt. Your current selection is automatically included as context — just type your question.

Lune launches the AI client as a subprocess in the embedded terminal using your local install (e.g. `claude`, `aider`, or any CLI tool). No API key configuration inside Lune is required — the client handles auth however it normally does.

To add a custom AI client, implement the `AiClient` trait in `crates/lune-ai/src/client.rs` and add a variant to `AiClientKind`.

---

## Git Panel

Press `F5` to open the Git panel (powered by libgit2 — no `git` binary required).

| Key | Action |
|-----|--------|
| `s` | Stage file/hunk |
| `u` | Unstage file/hunk |
| `d` | Open diff view |
| `Enter` | View file |

The diff view (`widgets/diff_view.rs`) renders inline unified diffs with syntax-highlighted context.

---

## Themes

Themes are TOML files in `~/.config/lune-editor/themes/`. The built-in **Lune Dark** theme is the default.

```toml
# ~/.config/lune-editor/themes/my-theme.toml
[colors]
background   = "#1e1e2e"
foreground   = "#cdd6f4"
cursor       = "#f5e0dc"
selection    = "#313244"
keyword      = "#cba6f7"
string       = "#a6e3a1"
comment      = "#6c7086"
function     = "#89b4fa"
type_name    = "#f9e2af"
```

Switch themes: `Ctrl+T` (cycle) or set `theme = "My Theme"` in config.

---

## Architecture

Lune is a Cargo workspace with four crates:

```
lune-editor/
├── src/main.rs           # CLI entry point, config loading
└── crates/
    ├── lune-core/        # Buffers, settings, tree-sitter, search, undo, crash recovery
    ├── lune-ui/          # ratatui TUI: event loop, widgets, vim state, themes, effects
    ├── lune-ai/          # AI manager, PTY session, Live Mode controller, client traits
    └── lune-git/         # Git service (libgit2): status, diffs, stage/unstage
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| `ratatui` | Immediate-mode TUI rendering |
| `ropey` | Efficient rope-based text buffers |
| `tree-sitter` | Incremental syntax parsing |
| `git2` | libgit2 bindings |
| `tokio` | Async runtime (AI/PTY/watcher) |
| `sled` | Embedded key-value store (workspace state) |
| `tachyonfx` | Terminal visual effects |
| `smallvec` | Stack-allocated small collections on hot paths |

**Event loop** (rat-salsa framework): crossterm input → `AppCommand` dispatch → state update → ratatui render → flush. File watcher, autosave timer, and AI PTY events are additional async sources.

**State persistence:** Open files, scroll offsets, layout, and active tab are saved to a per-workspace sled database at `~/.config/lune-editor/state/`. Dirty buffers are captured on crash and restored on next launch.

---

## Contributing

```bash
# Clone and build
git clone https://github.com/user/lune-editor
cd lune-editor
cargo build

# Run tests
cargo test --workspace

# Check before submitting
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace --release
```

**Guidelines:**
- Match existing code style; run `cargo fmt` before committing
- New widgets go in `crates/lune-ui/src/widgets/`; implement `ratatui::widgets::Widget`
- New languages: add a `tree-sitter-*` workspace dependency and register in `lune-core/src/language.rs`
- All unsafe code requires a `// SAFETY:` comment and a Miri-clean CI run
- Open an issue before large changes

---

## License

MIT — see [LICENSE](LICENSE).

---

<div align="right"><a href="#-lune-editor">↑ back to top</a></div>
