# Changelog

All notable changes to Lune Editor are documented here.

## [Unreleased]

### Added
- Project-wide text search ("search in files") overlay with jump-to-line
  (Ctrl+Shift+F): literal, case-insensitive substring matching across the
  workspace, skipping ignored/hidden/binary files; Enter opens the file at
  the match
- Per-hunk git staging, unstaging, and discarding via unified diff patches
- Find & replace overlay with live search highlighting (Ctrl+F / Ctrl+H)
- File operation inline input dialogs (create file/dir, rename, delete)
- Toast notifications with a thick left accent bar and bold, left-aligned
  text; they slide in with a subtle overshoot and fade out on expiry
- Diff fade-in animation for newly added lines in the editor gutter
- Language selector overlay with fuzzy filter (Ctrl+L)
- Root Editor / Agents tab switcher (Ctrl+1 / Ctrl+2)
- Editor scrollbar and viewport follow-cursor behaviour
- Status bar polish: encoding, file type, AI status segments

### Changed
- Git panel now exposes per-hunk diff view with stage/discard actions
- Notification system now auto-dismisses with visual vitality decay
- Theme colors: bare hex must now be the full 6-digit form; the 3-digit
  shorthand requires an explicit `#` prefix (`#abc`), so short words made
  of hex digits (`add`, `dad`) are no longer misread as colors
- Markdown highlighting: fenced code blocks now style the ``` delimiter
  lines rather than the whole block body
- Refreshed the built-in **Lune Dark** and **Lune Light** palettes (warmer
  and more cohesive) and recolored syntax highlighting to share the same
  hues, so code and UI read as one theme
- Every popup overlay now renders through one shared modal frame for a
  uniform look; find & replace sits in the top-right corner and no longer
  dims the editor behind it
- The editor background now fills the whole window, including the gaps
  between panels, instead of the terminal's own background
- Status bar "Lune Editor" badge now uses the accent color

### Removed
- Border-character customization (the `[borders]` theme table) — it was
  never used for rendering; panels draw Ratatui's rounded borders directly

## [0.1.0] — Initial release

- Multi-buffer rope-based editor (ropey)
- Tree-sitter syntax highlighting (20+ languages)
- Optional Vim mode (Normal/Insert/Visual/V-Line)
- Native Git panel (libgit2)
- AI session manager with PTY (Claude Code, Aider, shell, etc.)
- Crash recovery and workspace persistence (sled)
- TOML themes with live switching
- ratatui TUI with tachyonfx visual effects
