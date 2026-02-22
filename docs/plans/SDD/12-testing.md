# 12 — Testing Strategy

> **Phase:** 4 (Polish & Robustness)
> **Estimated effort:** Ongoing, ~2 sessions initial setup (~6 hours), then continuous
> **Prerequisites:** All previous plans (01–11)

## Goal

Define and implement a comprehensive testing strategy covering unit tests, integration tests, and manual test scenarios. Establish test infrastructure, property-based testing for critical paths, and CI enforcement so that regressions are caught before merge.

---

## Types & Structures

### Test Utilities

```rust
/// Test helper for creating a populated buffer with known content.
pub fn test_buffer(content: &str) -> TextBuffer {
    TextBuffer::from_str(content)
}

/// Test helper for creating a temporary workspace with files.
pub struct TestWorkspace {
    pub dir: TempDir,
    pub workspace: Workspace,
}

impl TestWorkspace {
    pub fn new() -> Self { /* create temp dir, init workspace */ }
    pub fn write_file(&self, rel_path: &str, content: &str) { /* write file */ }
    pub fn init_git(&self) { /* git init, add, commit */ }
}

/// Mock AI client that writes known edits to files.
pub struct MockAiClient {
    pub edits: Vec<(PathBuf, String)>,  // (file, new_content)
    pub delay_ms: u64,
}
```

---

## Test Categories

### Category 1: Unit Tests (per-crate)

**`lune-core` tests** — the most critical, highest coverage target.

| Module | What to test | Approach |
|--------|-------------|----------|
| `position` | Ordering, selection contains, cursor predicates | Standard unit tests |
| `buffer` | CRUD ops, rope ↔ position conversion, line access | Unit tests + property tests |
| `undo` | Transaction push/pop, undo/redo round-trip, stack limit | Unit tests + property tests |
| `search` | Find matches, regex, case sensitivity, replace single/all | Unit tests |
| `registry` | Open/close/lookup, path dedup, scratch buffers | Unit tests |
| `diff` | Myers diff correctness, incremental diff, edge cases | Unit tests + property tests |
| `workspace` | Dir listing, cache invalidation, relative paths | Unit tests with temp dirs |
| `settings` | Serialize/deserialize, merge, defaults | Unit tests |

**`lune-git` tests**

| Module | What to test | Approach |
|--------|-------------|----------|
| `service` | Status, diff, stage, commit | Integration with temp git repos |
| `gutter` | Mark computation from diff hunks | Unit tests |

**`lune-ai` tests**

| Module | What to test | Approach |
|--------|-------------|----------|
| `pty` | Spawn, read/write, resize, kill | Integration with `/bin/cat` |
| `session` | Lifecycle, error handling | Integration with mock process |
| `context` | Context collection, encoding formats | Unit tests |

**`lune-ui` tests**

| Module | What to test | Approach |
|--------|-------------|----------|
| `layout` | Layout computation for various panel configs | Unit tests |
| `keybindings` | Keymap parsing, lookup, multi-key sequences | Unit tests |
| `vim` | Mode transitions, motions, operators, counts, repeat | Unit tests |
| `highlight` | Tree-sitter highlighting, regex fallback | Unit tests |

### Category 2: Property-Based Tests

Use `proptest` or `quickcheck` for:

1. **Buffer round-trip**: For any sequence of insert/delete ops, undo-all should restore original content.
2. **Diff symmetry**: `diff(A, B)` applied to A produces B; `diff(B, A)` applied to B produces A.
3. **Selection invariant**: After any cursor movement, selection anchor ≤ head or head ≤ anchor (ordered pair is always valid).
4. **Search correctness**: `search(query)` finds exactly the positions where `content.contains(query)` is true.
5. **Serialization round-trip**: For any `Settings`, `serialize(deserialize(s)) == s`.

### Category 3: Integration Tests

Located in `tests/` at workspace root.

| Test | Description |
|------|------------|
| `test_open_edit_save` | Open a file, make edits, save, verify disk content |
| `test_multi_buffer` | Open multiple files, switch tabs, verify correct buffer displayed |
| `test_file_tree_navigation` | Create workspace, expand dirs, open file from tree |
| `test_git_workflow` | Init repo, modify file, stage, commit, verify status updates |
| `test_ai_session_lifecycle` | Start mock AI process, send input, receive output, stop |
| `test_crash_recovery` | Write recovery state, simulate startup, verify recovery prompt |
| `test_settings_merge` | Global + workspace-local config merge correctly |
| `test_vim_editing` | Enter vim mode, execute command sequences, verify buffer state |

### Category 4: Manual Test Scenarios

These require human verification and should be documented as a checklist.

1. **Large project navigation**: Open a 500+ file project, browse file tree, open 10+ files, switch between them.
2. **AI interaction**: Launch Claude Code, send contextual queries, verify context is correct.
3. **Git workflows**: Stage individual hunks, commit, see gutter markers update.
4. **Terminal compatibility**: Test on kitty, alacritty, WezTerm, tmux, macOS Terminal.app, Windows Terminal.
5. **Resize behavior**: Resize terminal aggressively while editing — no panics or layout breaks.
6. **Performance under load**: Edit a 50K-line file with syntax highlighting and git gutter — must remain responsive.
7. **Theme switching**: Switch between dark and light themes — all UI elements update.
8. **Crash recovery**: Kill the process, relaunch, verify recovery prompt with correct files.
9. **Vim + mouse coexistence**: Use vim keybindings and mouse interchangeably.

---

## Implementation Steps

### Step 1: Test infrastructure

1. Add dev dependencies to workspace:
   ```toml
   [workspace.dependencies]
   proptest = "1"
   tempfile = "3"
   assert_cmd = "2"       # for CLI integration tests
   predicates = "3"       # for assertion helpers
   insta = "1"            # snapshot testing for UI rendering
   ```
2. Create `tests/common/mod.rs` with `TestWorkspace`, `test_buffer()`, `MockAiClient`.
3. Set up `insta` for snapshot testing of rendered ratatui buffers.

### Step 2: Core unit tests

1. Write tests for every public function in `lune-core`.
2. Target: >90% line coverage for `buffer.rs`, `undo.rs`, `diff.rs`.
3. Target: >80% for `search.rs`, `registry.rs`, `workspace.rs`.
4. Property tests for buffer ops, diff, and search.

### Step 3: UI snapshot tests

1. Use `insta` + ratatui's `TestBackend` to render widgets to a buffer and snapshot the output.
2. Snapshot test cases:
   - Tab bar with 5 tabs (one active, one dirty).
   - Editor pane with syntax-highlighted Rust code.
   - Status bar in various states.
   - File tree with expanded/collapsed directories.
   - Git gutter and diff view rendering.
3. Update snapshots when intentional UI changes are made.

### Step 4: Integration test suite

1. Write all integration tests from Category 3 table.
2. Each test creates a `TestWorkspace`, performs operations, asserts outcomes.
3. Git tests use `git2` to set up test repositories.
4. AI tests use `MockAiClient` that spawns a simple script.

### Step 5: CI enforcement

1. Extend `.github/workflows/ci.yml`:
   ```yaml
   - run: cargo test --workspace
   - run: cargo test --workspace -- --ignored  # long-running integration tests
   - run: cargo clippy --workspace --all-targets -- -D warnings
   - run: cargo fmt --all -- --check
   ```
2. Add coverage reporting via `cargo-tarpaulin` or `cargo-llvm-cov`:
   ```yaml
   - run: cargo tarpaulin --workspace --out xml
   - uses: codecov/codecov-action@v4
   ```
3. Set minimum coverage thresholds in CI (fail if coverage drops below 70% overall).

### Step 6: Manual test checklist

1. Create `docs/TESTING.md` with the manual test scenarios from Category 4.
2. Include expected results for each scenario.
3. Track last-verified date and terminal/platform for each scenario.

---

## Acceptance Criteria

- [ ] Unit tests exist for all public functions in `lune-core`
- [ ] Property-based tests cover buffer ops, diff engine, and serialization
- [ ] Integration tests cover: open/edit/save, git workflow, AI session, crash recovery
- [ ] UI snapshot tests cover key widget states
- [ ] CI runs all tests on every push/PR
- [ ] Coverage reporting is set up and visible
- [ ] `lune-core` has >80% line coverage
- [ ] No test depends on external network or AI service (all mocked)
- [ ] Manual test checklist is documented and current
- [ ] All tests pass on Linux; known platform issues documented

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Testing TUI apps is inherently difficult (no DOM, no accessibility tree) | Snapshot testing with `insta` + ratatui `TestBackend`; don't try to test visual appearance pixel-perfectly |
| Property tests may be slow | Limit proptest iterations in CI (256); run extended iterations nightly |
| PTY/process integration tests may be flaky | Use deterministic mock processes; retry flaky tests once in CI |
| Coverage tools may not support all Rust editions/features | Fall back to `cargo-llvm-cov` if `tarpaulin` has issues |
| Manual tests are easy to skip | Integrate into release checklist; require sign-off before version tags |
