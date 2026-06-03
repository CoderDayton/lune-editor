//! Project-wide text search ("search in files").
//!
//! Two cheap-to-recombine halves power the search-in-files overlay:
//! [`collect_files`] walks a directory tree once to gather candidate text
//! files, and [`search_files`] scans a given file list for a literal
//! substring. Splitting them lets the UI re-run matching on every
//! keystroke without re-walking the tree.
//!
//! Matching is literal (not a regular expression) and ASCII
//! case-insensitive by default. Lowering only the ASCII range preserves
//! byte length, so match byte-offsets map straight back onto the original
//! line for accurate cursor placement.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Directory names never descended into: version-control internals, build
/// output, virtualenvs, and caches. Mirrors the filesystem watcher's list.
const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv", "__pycache__"];

/// Skip files larger than this many bytes — almost always data or
/// binaries, and scanning them would stall an interactive search.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1 << 20; // 1 MiB

/// Upper bound on candidate files gathered from one tree walk.
pub const DEFAULT_MAX_FILES: usize = 10_000;

/// Upper bound on hits returned for a single query.
pub const DEFAULT_MAX_RESULTS: usize = 500;

/// Upper bound on total file content retained for one interactive search
/// session, capping memory on trees with many large text files.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 64 << 20; // 64 MiB

/// A single match within a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Absolute path to the file containing the match.
    pub path: PathBuf,
    /// Zero-based line index of the match.
    pub line: usize,
    /// Zero-based byte column of the match start within the line.
    pub col: usize,
    /// The full match line (line terminator stripped) for display.
    pub line_text: String,
    /// Byte offset of the match start within `line_text`.
    pub match_start: usize,
    /// Byte offset of the match end within `line_text`.
    pub match_end: usize,
}

/// Result of one query: the hits plus whether the cap clipped them.
#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    /// Matches in file/line order, capped at [`SearchOptions::max_results`].
    pub hits: Vec<SearchHit>,
    /// `true` when more matches existed than the cap allowed.
    pub truncated: bool,
}

/// A text file read into memory for repeated in-memory searching.
#[derive(Clone, Debug)]
pub struct LoadedFile {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Full UTF-8 contents.
    pub text: String,
}

/// Tunable limits and matching mode for one search.
#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    /// Match case-sensitively when `true` (default `false`).
    pub case_sensitive: bool,
    /// Skip files larger than this many bytes.
    pub max_file_size: u64,
    /// Stop after this many hits, flagging the result truncated.
    pub max_results: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

/// Walk `root` depth-first and collect candidate text-file paths.
///
/// Skips [`IGNORED_DIRS`], hidden entries (leading `.`), and symlinks.
/// Capped at [`DEFAULT_MAX_FILES`]; returned paths are sorted for stable
/// output.
#[must_use]
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if files.len() >= DEFAULT_MAX_FILES {
            break;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip hidden entries and the always-ignored directories.
            if name.starts_with('.') || IGNORED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
                if files.len() >= DEFAULT_MAX_FILES {
                    break;
                }
            }
            // Symlinks fall through untouched to avoid cycles and surprises.
        }
    }

    files.sort();
    files
}

/// Normalize `query` for matching: lowered to ASCII when case-insensitive
/// (byte-length stable, so match offsets map back onto the source line).
fn normalize_needle(query: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        query.to_string()
    } else {
        query.to_ascii_lowercase()
    }
}

/// Read one file, applying the shared text-file filters: returns `None`
/// for oversized, binary (NUL-containing), unreadable, or non-UTF-8 files.
fn read_text_file(path: &Path, max_file_size: u64) -> Option<String> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > max_file_size {
            return None;
        }
    }
    let bytes = std::fs::read(path).ok()?;
    // A zero byte is a cheap, reliable "this is not text" signal.
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Scan one file's `text` for the already-normalized `needle`, pushing
/// hits into `results`. Returns `true` when the result cap was reached and
/// the caller should stop scanning further files.
fn scan_text(
    path: &Path,
    text: &str,
    needle: &str,
    opts: SearchOptions,
    results: &mut SearchResults,
) -> bool {
    for (line_idx, line) in text.lines().enumerate() {
        let hay: Cow<'_, str> = if opts.case_sensitive {
            Cow::Borrowed(line)
        } else {
            Cow::Owned(line.to_ascii_lowercase())
        };

        let mut from = 0;
        while let Some(rel) = hay[from..].find(needle) {
            let start = from + rel;
            let end = start + needle.len();
            results.hits.push(SearchHit {
                path: path.to_path_buf(),
                line: line_idx,
                col: start,
                line_text: line.to_string(),
                match_start: start,
                match_end: end,
            });
            if results.hits.len() >= opts.max_results {
                results.truncated = true;
                return true;
            }
            from = end;
            if from >= hay.len() {
                break;
            }
        }
    }
    false
}

/// Scan `files` for `query`, returning matches in file/line order.
///
/// Capped at [`SearchOptions::max_results`]. Files larger than the size
/// limit, non-text files (those containing a zero byte), and unreadable
/// files are skipped silently. An empty `query` yields no hits.
#[must_use]
pub fn search_files(files: &[PathBuf], query: &str, opts: SearchOptions) -> SearchResults {
    let mut results = SearchResults::default();
    if query.is_empty() {
        return results;
    }
    let needle = normalize_needle(query, opts.case_sensitive);
    for path in files {
        let Some(text) = read_text_file(path, opts.max_file_size) else {
            continue;
        };
        if scan_text(path, &text, &needle, opts, &mut results) {
            break;
        }
    }
    results
}

/// Read the text content of `files` into memory for repeated searching.
///
/// Applies the same filters as [`search_files`] (oversized, binary, and
/// unreadable files are skipped). Stops once the retained content would
/// exceed [`DEFAULT_MAX_TOTAL_BYTES`], bounding memory on trees with many
/// large text files.
#[must_use]
pub fn load_files(files: &[PathBuf], opts: SearchOptions) -> Vec<LoadedFile> {
    let mut loaded = Vec::new();
    let mut total: u64 = 0;
    for path in files {
        let Some(text) = read_text_file(path, opts.max_file_size) else {
            continue;
        };
        let next = total.saturating_add(text.len() as u64);
        if next > DEFAULT_MAX_TOTAL_BYTES {
            break;
        }
        total = next;
        loaded.push(LoadedFile {
            path: path.clone(),
            text,
        });
    }
    loaded
}

/// Search already-loaded file contents for `query`. Pure CPU; performs no
/// I/O, so it is cheap to call on every keystroke. Capped at
/// [`SearchOptions::max_results`]; an empty `query` yields no hits.
#[must_use]
pub fn search_loaded(files: &[LoadedFile], query: &str, opts: SearchOptions) -> SearchResults {
    let mut results = SearchResults::default();
    if query.is_empty() {
        return results;
    }
    let needle = normalize_needle(query, opts.case_sensitive);
    for file in files {
        if scan_text(&file.path, &file.text, &needle, opts, &mut results) {
            break;
        }
    }
    results
}

/// Convenience: [`collect_files`] then [`search_files`] in one call.
#[must_use]
pub fn search_workspace(root: &Path, query: &str, opts: SearchOptions) -> SearchResults {
    let files = collect_files(root);
    search_files(&files, query, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn rel_names(root: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn collect_files_skips_ignored_and_hidden() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "src/main.rs", "fn main() {}");
        write(root, "README.md", "hello");
        write(root, "target/junk.rs", "ignored");
        write(root, "node_modules/x.js", "ignored");
        write(root, ".git/config", "ignored");
        write(root, ".hidden", "ignored");

        let names = rel_names(root, &collect_files(root));
        assert!(names.contains(&"src/main.rs".to_string()));
        assert!(names.contains(&"README.md".to_string()));
        assert!(!names.iter().any(|n| n.contains("target")));
        assert!(!names.iter().any(|n| n.contains("node_modules")));
        assert!(!names.iter().any(|n| n.contains(".git")));
        assert!(!names.contains(&".hidden".to_string()));
    }

    #[test]
    fn search_finds_case_insensitive_by_default() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "Hello World\nsecond Hello line\n");

        let res = search_workspace(root, "hello", SearchOptions::default());
        assert_eq!(res.hits.len(), 2);
        assert_eq!(res.hits[0].line, 0);
        assert_eq!(res.hits[0].col, 0);
        assert_eq!(res.hits[0].match_start, 0);
        assert_eq!(res.hits[0].match_end, 5);
        assert_eq!(res.hits[0].line_text, "Hello World");
        assert_eq!(res.hits[1].line, 1);
        assert_eq!(res.hits[1].col, 7);
    }

    #[test]
    fn search_case_sensitive_respects_case() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "Hello hello HELLO");

        let opts = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        let res = search_workspace(root, "hello", opts);
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].col, 6);
    }

    #[test]
    fn search_multiple_matches_per_line() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "foo foo foo");

        let res = search_workspace(root, "foo", SearchOptions::default());
        assert_eq!(
            res.hits.iter().map(|h| h.col).collect::<Vec<_>>(),
            vec![0, 4, 8]
        );
    }

    #[test]
    fn search_skips_binary_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bin.dat"), b"foo\0foo").unwrap();
        write(root, "text.txt", "foo");

        let res = search_workspace(root, "foo", SearchOptions::default());
        assert_eq!(res.hits.len(), 1);
        assert!(res.hits[0].path.ends_with("text.txt"));
    }

    #[test]
    fn search_respects_max_results() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "x x x x x");

        let opts = SearchOptions {
            max_results: 2,
            ..Default::default()
        };
        let res = search_workspace(root, "x", opts);
        assert_eq!(res.hits.len(), 2);
        assert!(res.truncated);
    }

    #[test]
    fn search_skips_oversized_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "big.txt", "needle here");

        let opts = SearchOptions {
            max_file_size: 4,
            ..Default::default()
        };
        let res = search_workspace(root, "needle", opts);
        assert!(res.hits.is_empty());
    }

    #[test]
    fn empty_query_yields_no_hits() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "anything");

        let res = search_workspace(root, "", SearchOptions::default());
        assert!(res.hits.is_empty());
        assert!(!res.truncated);
    }

    #[test]
    fn loaded_search_matches_disk_search() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "Hello World\nsecond Hello line\n");
        fs::write(root.join("b.bin"), b"Hello\0World").unwrap();

        let files = collect_files(root);
        let loaded = load_files(&files, SearchOptions::default());
        // The binary file is filtered out of the loaded set.
        assert!(loaded.iter().all(|f| !f.path.ends_with("b.bin")));

        // In-memory search returns exactly the same hits as the disk scan.
        let disk = search_files(&files, "hello", SearchOptions::default());
        let mem = search_loaded(&loaded, "hello", SearchOptions::default());
        assert_eq!(disk.hits, mem.hits);
        assert_eq!(mem.hits.len(), 2);
    }
}
