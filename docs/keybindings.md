# Keybindings

All bindings are rebindable in `~/.config/lune-editor/keybindings.toml` (see
[Custom bindings](#custom-bindings)). Press `F1` for an in-app cheatsheet, and
`Ctrl+K` to open the leader menu — a which-key hint shows the next keys.

## Default bindings

### General

| Key | Action |
|-----|--------|
| `Ctrl+P` | Command palette |
| `Ctrl+Q` | Quit |
| `F1` | Toggle keybinding hints |

### Files & tabs

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save |
| `Ctrl+O` | Open file picker |
| `Ctrl+N` | New file (Editor tab) · new agent pane (Agents tab) |
| `Ctrl+W` | Close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+Shift+Tab` | Previous tab |

### Editing & search

| Key | Action |
|-----|--------|
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Ctrl+F` | Find in buffer |
| `Ctrl+H` | Find & replace |
| `Ctrl+L` | Language selector |

### Panels & view

| Key | Action |
|-----|--------|
| `Ctrl+B` | Toggle file tree |
| `Ctrl+G` | Toggle git panel |
| `Ctrl+1` | Show Editor tab |
| `Ctrl+2` | Show Agents tab |
| `` Ctrl+` `` | Toggle Agents tab |
| `Ctrl+T` | Select theme |

### Leader (`Ctrl+K`)

Press `Ctrl+K`, then a second key (a which-key hint appears in the status bar;
`Esc` cancels). These actions sit behind a leader because `Ctrl+Shift+<letter>`
can't be transmitted by legacy terminals and emulators reserve it for
copy/paste/tabs — two single-`Ctrl` presses always work.

| Then | Action |
|------|--------|
| `a` | AI: ask about selection |
| `r` | AI: refactor file |
| `c` | AI: summarize git changes |
| `s` | Save all |
| `f` | Search in files (project-wide) |
| `m` | Toggle Markdown preview |
| `n` | Dismiss notifications |
| `w` | Close AI session |

### Agents tab (pane multiplexer)

| Key | Action |
|-----|--------|
| `Ctrl+N` | New agent pane |
| `Alt+\` | Split vertical |
| `Alt+-` | Split horizontal |
| `Alt+x` | Close focused pane |
| `Alt+j` | Focus next pane |
| `Alt+k` | Focus previous pane |
| `Alt+z` | Toggle zoom |
| `Alt+,` | Layouts (apply / save) |

### AI

| Key | Action |
|-----|--------|
| `Ctrl+]` | Next AI session |
| `Ctrl+[` | Previous AI session |

Ask, refactor, summarize, and close-session are under the [`Ctrl+K`
leader](#leader-ctrlk) (and the command palette).

### Vim mode

Toggle with `Ctrl+Alt+V` (or `vim_mode = true` in config). Standard motions
apply in Normal mode (`h j k l`, `w b e`, `gg G`, `0 $`, …).

| Key | Action |
|-----|--------|
| `i` | Insert mode |
| `Esc` | Normal mode |
| `v` | Visual mode |
| `dd` | Delete line |
| `yy` | Yank line |
| `p` | Paste |
| `/` | Search |
| `:w` | Save |
| `:q` | Quit |

## Custom bindings

Put overrides in `~/.config/lune-editor/keybindings.toml`. Only the keys you list
change; everything else keeps its default.

```toml
[normal]
"ctrl+s"   = "save"
"f5"       = "toggle_git_panel"
"ctrl+k o" = "open_settings"   # a leader chord: Ctrl+K, then o
```

- **Modifiers:** `ctrl`, `shift`, `alt`, combined with `+`. Note that
  `ctrl+shift+<letter>` usually can't be transmitted by terminals — prefer a
  single `ctrl` key or a leader chord.
- **Keys:** letters, digits, `f1`–`f12`, and named keys (`enter`, `escape`,
  `tab`, `space`, `backspace`, `delete`, arrows, `minus`, `plus`, `equal`).
- **Chords:** space-separated combos, e.g. `ctrl+k g`. The first press becomes a
  leader; a which-key hint shows the continuations.

### Command names

| Group | Commands |
|-------|----------|
| Lifecycle | `quit`, `force_quit` |
| File | `save`, `save_all`, `open_file_picker` |
| Tabs | `close_tab`, `next_tab`, `prev_tab`, `show_editor_tab`, `show_agents_tab`, `toggle_agents_tab` |
| Panels | `toggle_file_tree`, `toggle_git_panel`, `command_palette`, `toggle_hidden_files`, `focus_next_pane` |
| File tree | `new_file`, `new_dir`, `rename_entry`, `delete_entry` |
| Editor | `undo`, `redo`, `find`, `replace` |
| Vim | `enter_normal_mode`, `enter_insert_mode`, `enter_visual_mode`, `toggle_vim_mode` |
| Git | `git_stage`, `git_unstage`, `git_commit`, `git_discard`, `git_refresh` |
| AI | `ai_ask_selection`, `ai_refactor_file`, `ai_summarize_changes`, `ai_open_client_picker`, `ai_close_session`, `ai_next_session`, `ai_prev_session` |
| Theme & view | `next_theme`, `prev_theme`, `open_theme_picker`, `dismiss_notifications`, `toggle_markdown_preview`, `toggle_key_hints` |
| Agent pane | `agent_split_auto`, `agent_split_vertical`, `agent_split_horizontal`, `agent_close_pane`, `agent_focus_next`, `agent_focus_prev`, `agent_toggle_zoom`, `agent_apply_layout`, `agent_save_layout`, `agent_save_layout_as` |
| Settings | `open_settings`, `open_keybindings` |
