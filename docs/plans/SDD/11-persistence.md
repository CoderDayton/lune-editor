# 11 — Persistence & Settings

> **Phase:** 4 (Polish & Robustness)
> **Estimated effort:** 2–3 sessions (~6–8 hours)
> **Prerequisites:** [04-ui-layout.md](04-ui-layout.md), [06-syntax-highlighting.md](06-syntax-highlighting.md)

## Goal

Implement the settings system: TOML-based configuration files for keymaps, themes, AI presets, and workspace state. Add crash recovery via periodic autosave of dirty buffers. Ensure the editor remembers user preferences and workspace state across sessions.

---

## Types & Structures

### Settings

```rust
pub struct Settings {
    pub editor: EditorSettings,
    pub keymap: KeymapConfig,
    pub theme: ThemeConfig,
    pub ai: AiConfig,           // from plan 08
    pub ui: UiSettings,
    pub file_tree: FileTreeConfig, // from plan 05
}

pub struct EditorSettings {
    pub tab_size: usize,         // default 4
    pub use_spaces: bool,        // default true
    pub word_wrap: bool,         // default false
    pub line_numbers: bool,      // default true
    pub relative_line_numbers: bool, // default false (vim users may want true)
    pub cursor_blink: bool,      // default true
    pub auto_save_interval_secs: Option<u64>, // default Some(60)
    pub vim_mode: bool,          // default false
    pub mouse_enabled: bool,     // default true
    pub scroll_margin: usize,    // lines to keep above/below cursor, default 5
}

pub struct UiSettings {
    pub show_file_tree: bool,
    pub file_tree_width: u16,
    pub show_ai_panel: bool,
    pub ai_panel_width: u16,
    pub effects_enabled: bool,
    pub font_ligatures: bool,    // terminal-dependent
}
```

### Theme

```rust
pub struct ThemeConfig {
    pub name: String,
    pub base: BaseTheme,
    pub syntax: HashMap<HighlightStyle, StyleDef>,
    pub ui: UiTheme,
}

pub struct BaseTheme {
    pub bg: Color,
    pub fg: Color,
    pub selection: Color,
    pub cursor: Color,
    pub line_number: Color,
    pub active_line: Color,
}

pub struct UiTheme {
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub tab_active_bg: Color,
    pub tab_inactive_bg: Color,
    pub panel_border: Color,
    pub file_tree_bg: Color,
    pub notification_bg: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
}

pub struct StyleDef {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}
```

### Keymap

```rust
pub struct KeymapConfig {
    /// Mode-specific keybindings override defaults.
    pub normal_mode: HashMap<KeyCombo, AppCommand>,
    pub vim_normal: HashMap<KeyCombo, AppCommand>,
    pub vim_insert: HashMap<KeyCombo, AppCommand>,
    pub vim_visual: HashMap<KeyCombo, AppCommand>,
    pub file_tree: HashMap<KeyCombo, AppCommand>,
    pub ai_panel: HashMap<KeyCombo, AppCommand>,
}
```

### Workspace State

```rust
pub struct WorkspaceState {
    pub root: PathBuf,
    pub open_files: Vec<PathBuf>,
    pub active_file: Option<PathBuf>,
    pub cursor_positions: HashMap<PathBuf, Position>,
    pub layout: LayoutState,
    pub last_opened: SystemTime,
}

pub struct RecentWorkspaces {
    pub entries: Vec<WorkspaceState>,
    pub max_entries: usize,  // default 20
}
```

### Crash Recovery

```rust
pub struct RecoveryState {
    pub dirty_buffers: Vec<RecoveryBuffer>,
    pub timestamp: SystemTime,
}

pub struct RecoveryBuffer {
    pub original_path: PathBuf,
    pub recovery_path: PathBuf,  // in ~/.lune-editor/recovery/
    pub content_hash: u64,
}
```

---

## Implementation Steps

### Step 1: Config directory structure

1. Define config paths:
   ```
   ~/.lune-editor/
   ├── config.toml          # main settings
   ├── keybindings.toml     # custom keybinding overrides
   ├── themes/
   │   ├── default-dark.toml
   │   └── default-light.toml
   ├── recovery/             # crash recovery snapshots
   ├── state/
   │   ├── workspaces.toml   # recent workspaces
   │   └── <workspace-hash>.toml  # per-workspace state
   └── log/                  # log files
   ```
2. Also support workspace-local config: `.lune/config.toml` overrides global settings.
3. Implement `ConfigPaths::resolve()` to handle XDG_CONFIG_HOME or platform conventions.
4. **Tests:** path resolution on different platforms.

### Step 2: Settings serialization

1. Create `lune-core/src/settings.rs` with all settings types.
2. Derive `Serialize`/`Deserialize` (serde) for all config types.
3. Implement `Settings::load(path) -> Result<Settings>`:
   - Read TOML file.
   - Merge with defaults for any missing fields.
4. Implement `Settings::save(path) -> Result<()>`.
5. Implement `Settings::default()` with sensible defaults.
6. Implement merging: global → workspace-local → CLI flags (highest priority).
7. **Tests:** round-trip serialize/deserialize, merge with partial overrides, missing file → defaults.

### Step 3: Theme system

1. Create `lune-ui/src/theme.rs`.
2. Implement `Theme::load(config: &ThemeConfig) -> Theme`:
   - Convert `StyleDef` → `ratatui::Style`.
   - Build the complete style palette.
3. Ship two built-in themes: `default-dark` and `default-light`.
4. Allow custom themes via TOML files in `~/.lune-editor/themes/`.
5. Implement `Theme::apply_to_buffer(buf: &mut ratatui::Buffer)` for global background/foreground.
6. All widgets use theme styles rather than hardcoded colors.
7. **Verify:** switch between dark and light themes, all UI elements respect the theme.

### Step 4: Keybinding customization

1. Implement `KeymapConfig::load(path)`:
   - Read keybindings TOML.
   - Merge with default keybindings (custom overrides, defaults fill gaps).
2. TOML format:
   ```toml
   [normal]
   "ctrl+s" = "save"
   "ctrl+shift+p" = "command_palette"

   [vim.normal]
   "g d" = "go_to_definition"  # multi-key sequences supported
   ```
3. Implement multi-key sequence support for vim mode (e.g., `gg`, `gc`, `ci"`).
4. Implement key combo parsing: `"ctrl+shift+a"` → `KeyCombo`.
5. **Tests:** parse key combos, merge custom + defaults, multi-key sequences.

### Step 5: Workspace state save/restore

1. On clean exit:
   - Save `WorkspaceState` for current workspace (open files, cursor positions, layout).
   - Update `RecentWorkspaces` list.
2. On startup with a workspace path:
   - Check for saved state.
   - Restore open files, cursor positions, and layout.
   - Skip files that no longer exist on disk.
3. **Verify:** open files, move cursor, exit, reopen → same files and cursor positions restored.

### Step 6: Crash recovery (autosave)

1. Implement periodic autosave (configurable interval, default 60s):
   - For each dirty buffer, write content to `~/.lune-editor/recovery/<hash>.bak`.
   - Write `RecoveryState` manifest listing all recovery files.
2. On startup, check for recovery state:
   - If recovery files exist, show a notification: "Recovered N unsaved files from previous session".
   - Open recovered buffers alongside their original paths.
   - User can accept (save) or discard (delete recovery files).
3. On clean exit, delete recovery state.
4. **Tests:** simulate dirty buffer, trigger autosave, verify recovery file exists, simulate crash recovery.

### Step 7: Settings UI

1. Add `AppCommand::OpenSettings` — opens `config.toml` in an editor buffer.
2. Add `AppCommand::OpenKeybindings` — opens `keybindings.toml`.
3. Changes to settings files trigger a reload notification.
4. Implement `Settings::reload()` — hot-reload settings without restart.
5. **Verify:** edit config.toml in the editor, save, see changes take effect.

### Step 8: CLI argument handling

1. Parse CLI args:
   - `lune <path>` — open file or directory.
   - `lune --config <path>` — custom config file.
   - `lune --theme <name>` — override theme.
   - `lune --no-effects` — disable visual effects.
   - `lune --vim` / `--no-vim` — override vim mode.
   - `lune --version` — print version.
   - `lune --help` — print usage.
2. Use `clap` or manual arg parsing.
3. CLI flags override config file settings.
4. **Tests:** parse various arg combinations.

---

## Acceptance Criteria

- [ ] Settings load from `~/.lune-editor/config.toml` with defaults for missing values
- [ ] Workspace-local `.lune/config.toml` overrides global settings
- [ ] Theme system supports dark and light built-in themes
- [ ] Custom themes can be added as TOML files
- [ ] Keybindings are customizable via TOML
- [ ] Vim multi-key sequences work in custom keybindings
- [ ] Workspace state (open files, cursors, layout) persists across sessions
- [ ] Crash recovery restores unsaved buffers on next launch
- [ ] Settings hot-reload on file save
- [ ] CLI arguments override config file settings

---

## Risks

| Risk | Mitigation |
|------|-----------|
| TOML format too limited for complex keybinding sequences | Allow JSON as alternative config format; or use a nested TOML structure |
| Config file corruption (partial write on crash) | Write to temp file then atomic rename |
| Theme color values don't render well on all terminals | Provide 16-color fallback theme; test on common terminals |
| Recovery files accumulate if editor crashes repeatedly | Limit recovery to last 5 sessions; clean up old recovery files on startup |
| Hot-reload of settings causes UI glitches | Validate new config before applying; fall back to previous on error |
