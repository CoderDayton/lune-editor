# 06 — Syntax Highlighting

> **Phase:** 2 (Workspace & Navigation)
> **Estimated effort:** 2–3 sessions (~6–8 hours)
> **Prerequisites:** [02-editor-core.md](02-editor-core.md), [04-ui-layout.md](04-ui-layout.md)

## Goal

Add syntax highlighting to the editor pane. Use tree-sitter as the primary engine (accurate, incremental parsing) with a regex fallback for unsupported languages. The highlighting layer sits between the `TextBuffer` and the rendering widget, producing styled spans for each visible line.

---

## Types & Structures

### Highlight Layer

```rust
pub trait Highlighter: Send {
    /// Re-parse after a text change.
    fn update(&mut self, buffer: &TextBuffer, edit_range: Option<(usize, usize)>);
    /// Return styled spans for a given line range.
    fn highlight_lines(&self, line_range: Range<usize>) -> Vec<HighlightedLine>;
}

pub struct HighlightedLine {
    pub line: usize,
    pub spans: Vec<StyledSpan>,
}

pub struct StyledSpan {
    pub start_col: usize,
    pub end_col: usize,
    pub style: HighlightStyle,
}

#[derive(Clone, Debug)]
pub enum HighlightStyle {
    Keyword,
    Type,
    Function,
    String,
    Comment,
    Number,
    Operator,
    Punctuation,
    Variable,
    Constant,
    Attribute,
    Namespace,
    Error,
    Default,
}
```

### Language Detection

```rust
pub struct LanguageRegistry {
    /// Map file extension → language ID
    extension_map: HashMap<String, LanguageId>,
    /// Map language ID → highlighter factory
    highlighters: HashMap<LanguageId, Box<dyn Fn() -> Box<dyn Highlighter>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(&'static str);
```

### Theme Mapping

```rust
pub struct SyntaxTheme {
    pub styles: HashMap<HighlightStyle, ratatui::style::Style>,
}
```

---

## Implementation Steps

### Step 1: Language detection

1. Create `lune-core/src/language.rs` with `LanguageId`, `LanguageRegistry`.
2. Build an extension → language map for common languages:
   - `.rs` → Rust, `.py` → Python, `.js`/`.ts` → JavaScript/TypeScript, `.md` → Markdown, `.toml` → TOML, `.json` → JSON, `.c`/`.h` → C, `.go` → Go, `.html` → HTML, `.css` → CSS, `.sh`/`.bash` → Shell.
3. Implement `LanguageRegistry::detect(path: &Path) -> Option<LanguageId>`.
4. Also support first-line detection (e.g., `#!/usr/bin/env python`).
5. **Tests:** extension mapping, shebang detection, unknown extension returns None.

### Step 2: Highlight trait and types

1. Create `lune-core/src/highlight.rs` with `Highlighter` trait, `HighlightedLine`, `StyledSpan`, `HighlightStyle`.
2. These types are UI-agnostic — they describe what to highlight, not how to render it.
3. **Tests:** basic type construction, span ordering.

### Step 3: Tree-sitter highlighter

1. Create `lune-ui/src/highlight/tree_sitter.rs` (in `lune-ui` because it depends on tree-sitter runtime).
2. Add dependencies: `tree-sitter` (runtime), and grammar crates:
   - `tree-sitter-rust`, `tree-sitter-python`, `tree-sitter-javascript`, `tree-sitter-typescript`, `tree-sitter-json`, `tree-sitter-toml`, `tree-sitter-markdown`, `tree-sitter-c`, `tree-sitter-go`, `tree-sitter-html`, `tree-sitter-css`, `tree-sitter-bash`.
3. Implement `TreeSitterHighlighter`:
   ```rust
   pub struct TreeSitterHighlighter {
       parser: Parser,
       tree: Option<Tree>,
       highlight_config: HighlightConfiguration,
       language_id: LanguageId,
   }
   ```
4. On `update()`:
   - If `edit_range` is provided, use tree-sitter's incremental parsing (`tree.edit()` + `parser.parse()`).
   - Otherwise, full re-parse.
5. On `highlight_lines()`:
   - Use tree-sitter's highlight query to iterate over captures in the requested line range.
   - Map capture names (`keyword`, `type`, `function`, `string.special`, etc.) to `HighlightStyle`.
6. **Tests:** parse a Rust snippet, verify keywords, strings, comments are tagged correctly.

### Step 4: Regex fallback highlighter

1. Create `lune-ui/src/highlight/regex_hl.rs`.
2. Implement `RegexHighlighter` for languages without tree-sitter grammars:
   ```rust
   pub struct RegexHighlighter {
       rules: Vec<(Regex, HighlightStyle)>,
   }
   ```
3. Provide generic rules: strings (double/single quoted), comments (`//`, `#`, `/* */`), numbers.
4. `highlight_lines()` applies rules per-line, resolving overlaps by first-match priority.
5. **Tests:** highlight a Python file with regex, verify strings and comments captured.

### Step 5: Syntax theme

1. Create `lune-ui/src/highlight/theme.rs` with `SyntaxTheme`.
2. Provide a default dark theme mapping:
   - `Keyword` → bold cyan
   - `Type` → yellow
   - `Function` → blue
   - `String` → green
   - `Comment` → dim gray/italic
   - `Number` → magenta
   - `Operator` → white
   - etc.
3. Implement `SyntaxTheme::resolve(HighlightStyle) -> ratatui::Style`.
4. Theme is loaded from settings (plan 11); for now, hardcode the default.
5. **Verify:** render highlighted code, visually inspect color scheme.

### Step 6: Integrate into editor pane

1. Modify `EditorPaneWidget` rendering (from plan 04):
   - Before rendering lines, call `highlighter.highlight_lines(visible_range)`.
   - Convert `StyledSpan` list to ratatui `Span` list using the theme.
   - Render each line as a `Line` composed of styled `Span`s.
2. When buffer text changes, call `highlighter.update(buffer, edit_range)`.
3. Assign a highlighter to each buffer on open based on language detection.
4. **Verify:** open a `.rs` file, see colorized syntax; edit text, see highlighting update immediately.

### Step 7: Incremental update performance

1. Ensure tree-sitter incremental parsing is wired correctly:
   - On each `EditOp`, compute the tree-sitter `InputEdit` (byte offsets).
   - Re-parse only the affected region.
2. Only re-highlight lines in the visible viewport + a small buffer (±50 lines).
3. Profile with a 10K+ line file: highlighting a single keystroke should take <5ms.
4. **Benchmark:** time `update()` + `highlight_lines()` on a large Rust file.

### Step 8: Language selector in status bar

1. Display the detected language in the status bar (right section).
2. Clicking or invoking a command opens a language picker overlay (simple list) to override detection.
3. Changing language re-assigns the highlighter for the active buffer.
4. **Verify:** status bar shows "Rust", override to "Python", highlighting changes.

---

## Acceptance Criteria

- [ ] Syntax highlighting works for at least Rust, Python, JavaScript, and Markdown
- [ ] Tree-sitter incremental parsing updates highlights within 5ms for typical edits
- [ ] Regex fallback provides basic highlighting for unrecognized languages
- [ ] Language is auto-detected from file extension and shebang
- [ ] Language can be manually overridden via status bar or command palette
- [ ] Theme colors are consistent and readable on dark terminal backgrounds
- [ ] No visible flicker or lag when editing highlighted files
- [ ] Opening a file with no known language gracefully falls back to plain text

---

## Risks

| Risk | Mitigation |
|------|-----------|
| Tree-sitter grammar crates increase binary size significantly | Feature-gate grammars; ship common ones, make others opt-in at compile time |
| Tree-sitter API changes across versions | Pin grammar versions compatible with the `tree-sitter` runtime version |
| Highlight queries may not cover all tokens | Start with upstream queries from tree-sitter repos; extend as needed |
| Regex highlighter produces incorrect results on edge cases | Document it as "best effort"; tree-sitter is the recommended path |
| Color scheme looks bad on light terminal backgrounds | Provide a light theme alternative; detect terminal background if possible |
