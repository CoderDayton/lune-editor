//! Editor context provider for AI sessions.
//!
//! [`EditorContext`] captures a snapshot of the editor state — active file,
//! cursor position, selection, open tabs, git status — and can encode it as
//! environment variables, a temp JSON file, or CLI arguments. This context
//! is passed to AI client processes so they understand the developer's
//! current working state.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::PathBuf;

// ── Data structures ──────────────────────────────────────────────────────

/// A snapshot of the editor state for AI context injection.
#[derive(Clone, Debug, Default)]
pub struct EditorContext {
    /// Workspace root directory, if one is open.
    pub workspace_root: Option<PathBuf>,
    /// The currently focused file.
    pub active_file: Option<FileContext>,
    /// All open tabs.
    pub open_tabs: Vec<TabContext>,
    /// Git repository status summary.
    pub git_status: Option<GitStatusSummary>,
    /// Selected text region, if any.
    pub selection: Option<SelectionContext>,
}

/// Context about the actively focused file.
#[derive(Clone, Debug)]
pub struct FileContext {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Detected language identifier (e.g. "rust", "python").
    pub language: Option<String>,
    /// 1-based cursor line number.
    pub cursor_line: usize,
    /// 1-based cursor column number.
    pub cursor_col: usize,
    /// Total number of lines in the file.
    pub total_lines: usize,
}

/// Context about a single open tab.
#[derive(Clone, Debug)]
pub struct TabContext {
    /// File path (may be empty for scratch buffers).
    pub path: Option<PathBuf>,
    /// Whether the buffer has unsaved changes.
    pub dirty: bool,
}

/// The current text selection in the active file.
#[derive(Clone, Debug)]
pub struct SelectionContext {
    /// The selected text content.
    pub text: String,
    /// The file containing the selection.
    pub file_path: PathBuf,
    /// 1-based start line of the selection.
    pub start_line: usize,
    /// 1-based end line of the selection.
    pub end_line: usize,
}

/// Summary of git repository status.
#[derive(Clone, Debug)]
pub struct GitStatusSummary {
    /// Current branch name.
    pub branch: String,
    /// List of modified (unstaged) file paths relative to repo root.
    pub modified_files: Vec<PathBuf>,
}

// ── Encoding strategies ──────────────────────────────────────────────────

impl EditorContext {
    /// Encode the context as environment variables.
    ///
    /// Variable names use the `LUNE_CTX_` prefix:
    /// - `LUNE_CTX_WORKSPACE` — workspace root
    /// - `LUNE_CTX_FILE` — active file path
    /// - `LUNE_CTX_LINE` — cursor line (1-based)
    /// - `LUNE_CTX_COL` — cursor column (1-based)
    /// - `LUNE_CTX_LANGUAGE` — detected language
    /// - `LUNE_CTX_TOTAL_LINES` — total lines in file
    /// - `LUNE_CTX_SELECTION` — selected text (if any)
    /// - `LUNE_CTX_SELECTION_START` — selection start line (1-based)
    /// - `LUNE_CTX_SELECTION_END` — selection end line (1-based)
    /// - `LUNE_CTX_GIT_BRANCH` — git branch name
    /// - `LUNE_CTX_MODIFIED_FILES` — comma-separated modified file paths
    /// - `LUNE_CTX_OPEN_FILES` — comma-separated open file paths
    #[must_use]
    pub fn to_env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        if let Some(ref root) = self.workspace_root {
            env.insert("LUNE_CTX_WORKSPACE".to_string(), root.display().to_string());
        }

        if let Some(ref f) = self.active_file {
            env.insert("LUNE_CTX_FILE".to_string(), f.path.display().to_string());
            env.insert("LUNE_CTX_LINE".to_string(), f.cursor_line.to_string());
            env.insert("LUNE_CTX_COL".to_string(), f.cursor_col.to_string());
            env.insert(
                "LUNE_CTX_TOTAL_LINES".to_string(),
                f.total_lines.to_string(),
            );
            if let Some(ref lang) = f.language {
                env.insert("LUNE_CTX_LANGUAGE".to_string(), lang.clone());
            }
        }

        if let Some(ref sel) = self.selection {
            env.insert("LUNE_CTX_SELECTION".to_string(), sel.text.clone());
            env.insert(
                "LUNE_CTX_SELECTION_START".to_string(),
                sel.start_line.to_string(),
            );
            env.insert(
                "LUNE_CTX_SELECTION_END".to_string(),
                sel.end_line.to_string(),
            );
        }

        if let Some(ref git) = self.git_status {
            env.insert("LUNE_CTX_GIT_BRANCH".to_string(), git.branch.clone());
            if !git.modified_files.is_empty() {
                let files: Vec<String> = git
                    .modified_files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                env.insert("LUNE_CTX_MODIFIED_FILES".to_string(), files.join(","));
            }
        }

        let open_files: Vec<String> = self
            .open_tabs
            .iter()
            .filter_map(|t| t.path.as_ref().map(|p| p.display().to_string()))
            .collect();
        if !open_files.is_empty() {
            env.insert("LUNE_CTX_OPEN_FILES".to_string(), open_files.join(","));
        }

        env
    }

    /// Write the context as JSON to a temporary file and return its path.
    ///
    /// The caller is responsible for cleaning up the file when done.
    ///
    /// # Errors
    /// Returns an error if the temp file cannot be created or written.
    pub fn to_temp_file(&self) -> anyhow::Result<PathBuf> {
        use std::io::Write as _;

        let dir = std::env::temp_dir();
        let path = dir.join(format!("lune-ctx-{}.json", uuid::Uuid::new_v4()));
        let mut file = std::fs::File::create(&path)?;

        // Hand-rolled JSON to avoid adding serde as a dependency.
        let json = self.to_json();
        file.write_all(json.as_bytes())?;

        Ok(path)
    }

    /// Encode the context as CLI arguments.
    ///
    /// Produces flags like `--file path --line 42 --col 13 --language rust`.
    #[must_use]
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(ref root) = self.workspace_root {
            args.push("--workspace".to_string());
            args.push(root.display().to_string());
        }

        if let Some(ref f) = self.active_file {
            args.push("--file".to_string());
            args.push(f.path.display().to_string());
            args.push("--line".to_string());
            args.push(f.cursor_line.to_string());
            args.push("--col".to_string());
            args.push(f.cursor_col.to_string());
            if let Some(ref lang) = f.language {
                args.push("--language".to_string());
                args.push(lang.clone());
            }
        }

        if let Some(ref sel) = self.selection {
            args.push("--selection".to_string());
            args.push(sel.text.clone());
        }

        if let Some(ref git) = self.git_status {
            args.push("--git-branch".to_string());
            args.push(git.branch.clone());
        }

        args
    }

    /// Serialize context to a simple JSON string (no serde dependency).
    #[must_use]
    fn to_json(&self) -> String {
        let mut s = String::with_capacity(512);
        s.push('{');

        let mut first = true;
        let mut comma = |s: &mut String| {
            if first {
                first = false;
            } else {
                s.push(',');
            }
        };

        if let Some(ref root) = self.workspace_root {
            comma(&mut s);
            let _ = write!(
                s,
                "\"workspace\":\"{}\"",
                json_escape(&root.display().to_string())
            );
        }

        if let Some(ref f) = self.active_file {
            comma(&mut s);
            let _ = write!(
                s,
                "\"file\":{{\"path\":\"{}\",\"line\":{},\"col\":{},\"total_lines\":{}",
                json_escape(&f.path.display().to_string()),
                f.cursor_line,
                f.cursor_col,
                f.total_lines,
            );
            if let Some(ref lang) = f.language {
                let _ = write!(s, ",\"language\":\"{}\"", json_escape(lang));
            }
            s.push('}');
        }

        if let Some(ref sel) = self.selection {
            comma(&mut s);
            let _ = write!(
                s,
                "\"selection\":{{\"text\":\"{}\",\"file\":\"{}\",\"start_line\":{},\"end_line\":{}}}",
                json_escape(&sel.text),
                json_escape(&sel.file_path.display().to_string()),
                sel.start_line,
                sel.end_line,
            );
        }

        if !self.open_tabs.is_empty() {
            comma(&mut s);
            s.push_str("\"open_tabs\":[");
            for (i, tab) in self.open_tabs.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let path_str = tab
                    .path
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string());
                let _ = write!(
                    s,
                    "{{\"path\":\"{}\",\"dirty\":{}}}",
                    json_escape(&path_str),
                    tab.dirty,
                );
            }
            s.push(']');
        }

        if let Some(ref git) = self.git_status {
            comma(&mut s);
            let _ = write!(s, "\"git\":{{\"branch\":\"{}\"", json_escape(&git.branch));
            if !git.modified_files.is_empty() {
                s.push_str(",\"modified_files\":[");
                for (i, f) in git.modified_files.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    let _ = write!(s, "\"{}\"", json_escape(&f.display().to_string()));
                }
                s.push(']');
            }
            s.push('}');
        }

        s.push('}');
        s
    }
}

/// Minimal JSON string escaping (backslash, quote, control chars).
fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Utility: extract selected text from a buffer given a selection range.
///
/// `start` and `end` are 0-based positions. Returns the text between them.
#[must_use]
pub fn extract_selection_text(
    buffer: &lune_core::buffer::TextBuffer,
    start: lune_core::position::Position,
    end: lune_core::position::Position,
) -> String {
    let (lo, hi) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let start_idx = buffer.pos_to_char(lo);
    let end_idx = buffer.pos_to_char(hi);
    buffer.rope().slice(start_idx..end_idx).to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> EditorContext {
        EditorContext {
            workspace_root: Some(PathBuf::from("/home/user/project")),
            active_file: Some(FileContext {
                path: PathBuf::from("/home/user/project/src/main.rs"),
                language: Some("rust".to_string()),
                cursor_line: 42,
                cursor_col: 13,
                total_lines: 200,
            }),
            open_tabs: vec![
                TabContext {
                    path: Some(PathBuf::from("/home/user/project/src/main.rs")),
                    dirty: false,
                },
                TabContext {
                    path: Some(PathBuf::from("/home/user/project/src/lib.rs")),
                    dirty: true,
                },
                TabContext {
                    path: None,
                    dirty: false,
                },
            ],
            git_status: Some(GitStatusSummary {
                branch: "feature/ai".to_string(),
                modified_files: vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")],
            }),
            selection: Some(SelectionContext {
                text: "fn main() {}".to_string(),
                file_path: PathBuf::from("/home/user/project/src/main.rs"),
                start_line: 42,
                end_line: 42,
            }),
        }
    }

    #[test]
    fn env_vars_complete() {
        let ctx = sample_context();
        let env = ctx.to_env_vars();

        assert_eq!(env["LUNE_CTX_WORKSPACE"], "/home/user/project");
        assert_eq!(env["LUNE_CTX_FILE"], "/home/user/project/src/main.rs");
        assert_eq!(env["LUNE_CTX_LINE"], "42");
        assert_eq!(env["LUNE_CTX_COL"], "13");
        assert_eq!(env["LUNE_CTX_LANGUAGE"], "rust");
        assert_eq!(env["LUNE_CTX_TOTAL_LINES"], "200");
        assert_eq!(env["LUNE_CTX_SELECTION"], "fn main() {}");
        assert_eq!(env["LUNE_CTX_SELECTION_START"], "42");
        assert_eq!(env["LUNE_CTX_SELECTION_END"], "42");
        assert_eq!(env["LUNE_CTX_GIT_BRANCH"], "feature/ai");
        assert_eq!(env["LUNE_CTX_MODIFIED_FILES"], "src/main.rs,src/lib.rs");
        assert!(env["LUNE_CTX_OPEN_FILES"].contains("src/main.rs"));
        assert!(env["LUNE_CTX_OPEN_FILES"].contains("src/lib.rs"));
    }

    #[test]
    fn env_vars_minimal() {
        let ctx = EditorContext::default();
        let env = ctx.to_env_vars();
        assert!(env.is_empty());
    }

    #[test]
    fn cli_args_include_file_and_line() {
        let ctx = sample_context();
        let args = ctx.to_cli_args();

        assert!(args.contains(&"--file".to_string()));
        assert!(args.contains(&"--line".to_string()));
        assert!(args.contains(&"42".to_string()));
        assert!(args.contains(&"--col".to_string()));
        assert!(args.contains(&"13".to_string()));
        assert!(args.contains(&"--language".to_string()));
        assert!(args.contains(&"rust".to_string()));
        assert!(args.contains(&"--workspace".to_string()));
        assert!(args.contains(&"--selection".to_string()));
        assert!(args.contains(&"--git-branch".to_string()));
    }

    #[test]
    fn cli_args_empty_context() {
        let ctx = EditorContext::default();
        let args = ctx.to_cli_args();
        assert!(args.is_empty());
    }

    #[test]
    fn json_encoding() {
        let ctx = sample_context();
        let json = ctx.to_json();

        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"workspace\":\"/home/user/project\""));
        assert!(json.contains("\"line\":42"));
        assert!(json.contains("\"language\":\"rust\""));
        assert!(json.contains("\"branch\":\"feature/ai\""));
    }

    #[test]
    fn json_escapes_special_chars() {
        let ctx = EditorContext {
            active_file: Some(FileContext {
                path: PathBuf::from("/path/with \"quotes\""),
                language: None,
                cursor_line: 1,
                cursor_col: 1,
                total_lines: 1,
            }),
            ..EditorContext::default()
        };
        let json = ctx.to_json();
        assert!(json.contains("\\\"quotes\\\""));
    }

    #[test]
    fn temp_file_is_created() {
        let ctx = sample_context();
        let path = ctx.to_temp_file().expect("Failed to create temp file");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with('{'));
        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn extract_selection_from_buffer() {
        use lune_core::buffer::TextBuffer;
        use lune_core::position::Position;

        let buf = TextBuffer::from_text("hello world\nfoo bar\nbaz");
        let text = extract_selection_text(&buf, Position::new(0, 6), Position::new(1, 3));
        assert_eq!(text, "world\nfoo");
    }
}
