# Testing Guide

## Automated Tests

### Running all tests

```bash
# Full suite (unit + integration + property + snapshot)
cargo test --workspace

# Long-running / ignored tests
cargo test --workspace -- --ignored

# Specific crate
cargo test -p lune-core
cargo test -p lune-ui
cargo test -p lune-git
cargo test -p lune-ai

# Snapshot tests only
cargo test -p lune-ui --test snapshot_widgets

# Property tests only (by convention, files prefixed with prop_)
cargo test -p lune-core --test prop_buffer
cargo test -p lune-core --test prop_diff
cargo test -p lune-core --test prop_search
cargo test -p lune-core --test prop_settings
```

### Updating snapshots

When intentional UI changes are made, snapshot tests will fail. To review and accept:

```bash
# Install insta CLI (one-time)
cargo install cargo-insta

# Review pending snapshots interactively
cargo insta review

# Or accept all pending snapshots
cargo insta accept --workspace

# Or manually: rename .snap.new -> .snap in crates/lune-ui/tests/snapshots/
```

### Coverage

```bash
# Install cargo-llvm-cov (one-time)
cargo install cargo-llvm-cov

# Generate HTML coverage report
cargo llvm-cov --workspace --html --open

# Generate LCOV for CI
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

### Lint and format

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

---

## Test Categories

| Category | Location | Count | Framework |
|----------|----------|-------|-----------|
| Property-based | `crates/lune-core/tests/prop_*.rs` | 18 | proptest |
| Integration | `tests/*.rs` | 31 | standard |
| UI snapshots | `crates/lune-ui/tests/snapshot_widgets.rs` | 26 | insta |
| Unit tests | `src/**/*.rs` (inline `#[cfg(test)]`) | varies | standard |

---

## Manual Test Checklist

These scenarios require human verification. Record results before tagging a release.

### 1. Large project navigation

- [ ] Open a project with 500+ files
- [ ] Browse file tree, expand/collapse directories
- [ ] Open 10+ files via file tree
- [ ] Switch between tabs — correct buffer displayed each time
- [ ] Close tabs, verify file tree selection follows

**Expected:** Smooth navigation, no lag on tab switch, file tree reflects open files.

### 2. AI interaction

- [ ] Launch an AI client session from the editor
- [ ] Send a contextual query referencing the current file
- [ ] Verify the AI receives correct file context
- [ ] Send follow-up queries, verify session continuity

**Expected:** AI client spawns in embedded PTY, context is accurate, no stale data.

### 3. Live Mode streaming

- [ ] Enable Live Mode (toggle on)
- [ ] Trigger an AI-driven file modification
- [ ] Observe diff hunks streaming into the editor in real time
- [ ] Verify auto-follow scrolls to changes

**Expected:** Diffs appear incrementally, editor stays responsive, auto-follow works.

### 4. Git workflows

- [ ] Modify a tracked file, verify gutter markers appear (added/modified lines)
- [ ] Open the git panel, see unstaged changes listed
- [ ] Stage individual files
- [ ] Write a commit message and commit
- [ ] Verify gutter markers clear after commit
- [ ] Verify git panel shows clean state

**Expected:** All git state transitions reflected in UI immediately.

### 5. Terminal compatibility

Test on each terminal emulator. Record pass/fail per terminal.

| Terminal | Rendering | Keys | Mouse | Resize |
|----------|-----------|------|-------|--------|
| kitty | | | | |
| Alacritty | | | | |
| WezTerm | | | | |
| tmux | | | | |
| macOS Terminal.app | | | | |
| Windows Terminal | | | | |

**Expected:** No visual artifacts, all keybindings work, mouse clicks register correctly.

### 6. Resize behavior

- [ ] Resize terminal rapidly while editing (drag corner)
- [ ] Resize to very small (e.g., 40x10)
- [ ] Resize back to large (e.g., 200x60)
- [ ] Verify layout reflows correctly, no panics

**Expected:** Layout adapts, no crashes, content remains intact.

### 7. Performance under load

- [ ] Open a 50,000+ line file
- [ ] Enable syntax highlighting
- [ ] Scroll through the file rapidly (hold Page Down)
- [ ] Edit near the middle of the file
- [ ] Verify git gutter markers display

**Expected:** Scrolling remains smooth (>30 fps feel), edits are instant, no perceptible lag.

### 8. Theme switching

- [ ] Start in dark theme
- [ ] Switch to light theme via command palette
- [ ] Verify all UI elements update: status bar, tab bar, editor, file tree, git panel
- [ ] Switch back to dark theme
- [ ] Verify round-trip is clean

**Expected:** Instant theme switch, no stale colors, all widgets respect theme.

### 9. Crash recovery

- [ ] Open several files and make unsaved edits
- [ ] Kill the process (`kill -9 <pid>`)
- [ ] Relaunch the editor
- [ ] Verify recovery prompt appears with correct file list
- [ ] Accept recovery, verify unsaved changes restored

**Expected:** State database preserves workspace state, recovery is seamless.

### 10. Vim and mouse coexistence

- [ ] Use vim keybindings (hjkl, dd, yy, p, /, etc.)
- [ ] Click with mouse to reposition cursor
- [ ] Select text with mouse drag
- [ ] Use vim visual mode after mouse click
- [ ] Scroll with mouse wheel, then use vim motions

**Expected:** No mode confusion, cursor state is consistent between input methods.

---

## Release verification

Before tagging a release, all of the following must pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo test --workspace -- --ignored
cargo build --release
```

Plus: at minimum, manual tests 1, 4, 6, 7, 8, 9 verified on the primary development terminal.

---

## Last verified

| Scenario | Date | Platform | Terminal | Result |
|----------|------|----------|----------|--------|
| — | — | — | — | — |
