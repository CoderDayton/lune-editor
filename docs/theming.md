# Theming

A Lune theme is a TOML file that starts from a built-in base and overrides
only the colors you care about. Drop it in
`~/.config/lune-editor/themes/` and it appears in the theme switcher.

## Built-in themes

Two themes ship with Lune and are always available:

- **Lune Dark** (default) — neutral cool-gray surfaces with warm,
  desaturated content colors.
- **Lune Light** — a warm "paper" background using the same accent family.

## Switching themes

- **Cycle:** `Ctrl+T`
- **Config:** set `theme = "Lune Light"` in `~/.config/lune-editor/config.toml`
  (or `.lune/config.toml` in a workspace)
- **One-off:** `lune --theme "Lune Light" <path>`

## Writing a custom theme

Create `~/.config/lune-editor/themes/my-theme.toml`. `name` is required;
`base` selects which built-in to start from (`"dark"` is the default).
Every table is optional — anything you omit keeps the base value.

```toml
name = "My Theme"
base = "dark"              # or "light"

[colors]
accent       = "#83a6d6"   # focused borders, active tab, links
bg           = "#16191e"   # editor background
fg           = "#d3c6aa"   # primary text
fg_dim       = "#4b5563"
fg_muted     = "#7d8590"
selection_bg = "#1f242b"
border_focused   = "#83a6d6"
border_unfocused = "#2f353d"

[syntax]
# Each entry is a style: { fg = "...", bg = "...", modifiers = "bold,italic" }
keyword  = { fg = "#e67e80" }
type     = { fg = "#dbbc7f" }
function = { fg = "#83a6d6" }
string   = { fg = "#a7c080" }
comment  = { fg = "#7d8590", modifiers = "italic" }
number   = { fg = "#d699b6" }
```

### Color formats

A color is one of:

- a hex literal — `#RRGGBB`, or `#RGB` shorthand (bare 6-digit hex without
  the `#` is also accepted; 3-digit shorthand requires the `#`);
- a CSS function — `rgb(131, 166, 214)` or `hsl(214, 50%, 68%)`;
- a named color — `red`, `blue`, `reset`, etc.

### Overridable tables

Beyond `[colors]` and `[syntax]`, a theme can override:

| Table | What it styles |
|-------|----------------|
| `[editor]` | cursor and gutter styles |
| `[file_tree]` | directory / file / symlink / selection colors |
| `[git]` | added / modified / deleted / conflicted / … markers |
| `[diff]` | diff add/delete foregrounds and background tints |
| `[tabs]` | active / inactive tab styles |
| `[status_bar]` | mode, brand, info, and bar styles |
| `[notifications]` | toast `success`/`info`/`warn`/`error` plus `bg`/`fg` |
| `[overlay]` | popup border, selection, and hint colors |
| `[welcome]` | welcome-screen title and text |

Style entries (anything under `[syntax]`, `[editor]`, `[tabs]`,
`[status_bar]`, `[welcome]`) take `fg`, `bg`, and a `modifiers` string —
any comma-separated combination of `bold`, `dim`, `italic`, `underlined`,
`reversed`, `hidden`, and `strikethrough`.
