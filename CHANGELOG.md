# Changelog

All notable changes to Lune Editor are documented here.

## [Unreleased]

### Added
- Per-hunk git staging, unstaging, and discarding via unified diff patches
- Find & replace overlay with live search highlighting (Ctrl+F / Ctrl+H)
- File operation inline input dialogs (create file/dir, rename, delete)
- Notification fade-out animation with block-character progress bar
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

## [0.1.0] — Initial release

- Multi-buffer rope-based editor (ropey)
- Tree-sitter syntax highlighting (20+ languages)
- Optional Vim mode (Normal/Insert/Visual/V-Line)
- Native Git panel (libgit2)
- AI session manager with PTY (Claude Code, Aider, shell, etc.)
- Crash recovery and workspace persistence (sled)
- TOML themes with live switching
- ratatui TUI with tachyonfx visual effects
