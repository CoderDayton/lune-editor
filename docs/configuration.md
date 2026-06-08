# Configuration

Lune reads settings from TOML, layered over the built-in defaults:

1. **Global** — `~/.config/lune-editor/config.toml`
2. **Workspace** — `.lune/config.toml` at the project root (overrides the global file)
3. **CLI flags** — e.g. `--vim`, `--theme` (override both files for that run)

Every field is optional; missing keys fall back to the defaults below. Open the
global file in-app with the command palette (`Ctrl+P` → "Open Settings").

> The in-app editor rewrites `config.toml` whenever you change a setting from
> the UI (theme picker, agent palette commands, etc.). Comments and unknown keys
> are dropped on that rewrite — edit the file while Lune is closed to keep them.

## `[editor]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tab_size` | int | `4` | Spaces per tab stop. |
| `use_spaces` | bool | `true` | Insert spaces instead of tab characters. |
| `word_wrap` | bool | `false` | Wrap long lines to the viewport. |
| `line_numbers` | bool | `true` | Show the line-number gutter. |
| `relative_line_numbers` | bool | `false` | Number relative to the cursor (vim-style). |
| `cursor_blink` | bool | `true` | Blink the cursor. |
| `auto_save_interval_secs` | int | `60` | Seconds between autosaves. |
| `vim_mode` | bool | `false` | Start in Vim mode. |
| `mouse_enabled` | bool | `true` | Enable mouse input. |
| `scroll_margin` | int | `5` | Lines kept above/below the cursor when scrolling. |
| `cursor_style` | `block` \| `bar` \| `underline` | `block` | Cursor shape (ignored in Vim mode, which tracks the mode). |

## `[ui]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `show_file_tree` | bool | `true` | Show the file-tree sidebar at startup. |
| `file_tree_width_pct` | int | `20` | File-tree width, percent of terminal width. |
| `show_ai_panel` | bool | `false` | Show the AI/right panel at startup. |
| `right_panel_width_pct` | int | `30` | Right-panel width, percent of terminal width. |

## `[file_tree]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `indent_size` | int | `2` | Spaces per nesting level. |
| `icons` | bool | `true` | Show file/folder icons (needs a Nerd Font). |
| `sort_dirs_first` | bool | `true` | List directories before files. |
| `show_hidden` | bool | `false` | Show dotfiles. |

## `[ai]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_client` | string | `"claude"` | AI client launched for new sessions (any CLI tool, e.g. `claude`, `aider`, `gemini`). |

## `[agents]`

Controls how new panes are placed on the Agents tab (`Ctrl+N`).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `placement` | `fixed` \| `mouse` | `fixed` | `fixed` tiles new panes into an even, screen-capped grid; `mouse` splits the pane under the cursor based on the click position. |
| `columns_grow` | `left` \| `right` | `right` | Which side new grid columns are added to. |
| `rows_grow` | `top` \| `bottom` | `bottom` | Which side wrapped rows are added to. |
| `max_columns` | int | `0` | Columns per row. `0` = auto from screen aspect (portrait → 1, 16:9 / 21:9 → 3, 32:9 → 4). |
| `max_rows` | int | `2` | Rows the grid may wrap to before it's full. |

In `fixed` mode the grid fills a row up to `max_columns`, then wraps to the next
row up to `max_rows`; at `columns × rows` panes it's full and `Ctrl+N` is a
no-op. `Alt+\` / `Alt+-` still split manually and bypass the cap.

## Theme

The active theme is a top-level key (not a section):

```toml
theme = "Lune Dark"
```

See the [theming guide](theming.md) for built-in themes and how to write your own.

## Workspace overrides

A `.lune/config.toml` only needs the keys it changes; they replace the matching
global values, everything else stays as configured globally:

```toml
[editor]
tab_size = 2
vim_mode = true

[file_tree]
show_hidden = true
```
