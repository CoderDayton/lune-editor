//! Line-level gutter markers derived from diffs.
//!
//! [`GutterMarks`] maps line numbers to [`GutterMark`] indicators
//! for rendering in the editor gutter column.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::service::GitService;

/// Type of gutter indicator for a single line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterMark {
    /// Line was added (not present in HEAD).
    Added,
    /// Line was modified compared to HEAD.
    Modified,
    /// One or more lines were deleted after this line.
    Deleted,
}

/// Line-level gutter marks for an open buffer.
#[derive(Clone, Debug, Default)]
pub struct GutterMarks {
    /// Mapping from 0-based line number to gutter mark.
    pub marks: HashMap<usize, GutterMark>,
}

impl GutterMarks {
    /// Get the gutter mark for a specific line (0-based).
    pub fn get(&self, line: usize) -> Option<GutterMark> {
        self.marks.get(&line).copied()
    }

    /// Whether there are any marks.
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Number of marked lines.
    pub fn len(&self) -> usize {
        self.marks.len()
    }
}

impl GitService {
    /// Compute gutter marks by diffing the given buffer content against
    /// the HEAD version of the file.
    ///
    /// `rel_path` is relative to the repo root.
    /// `buffer_content` is the current in-memory buffer text.
    pub fn gutter_marks(&self, rel_path: &Path, buffer_content: &str) -> Result<GutterMarks> {
        // Get the HEAD version of the file.
        let old_content = self.head_file_content(rel_path)?;
        let old = old_content.as_deref().unwrap_or("");

        // Diff the old content against the current buffer.
        Ok(compute_gutter_marks(old, buffer_content))
    }

    /// Read the content of a file from HEAD.
    fn head_file_content(&self, rel_path: &Path) -> Result<Option<String>> {
        let head = match self.repo().head() {
            Ok(h) => h,
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let tree = head.peel_to_tree()?;
        let entry = match tree.get_path(rel_path) {
            Ok(e) => e,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let blob = self.repo().find_blob(entry.id())?;
        let content = String::from_utf8_lossy(blob.content()).into_owned();
        Ok(Some(content))
    }
}

/// Compute gutter marks from old and new content using line-level diff.
///
/// Uses `similar` crate for efficient text diffing.
fn compute_gutter_marks(old: &str, new: &str) -> GutterMarks {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut marks = HashMap::new();
    let mut new_line: usize = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                new_line += 1;
            }
            ChangeTag::Insert => {
                // If the previous line had a deletion, this is a modification.
                // Otherwise it's a pure addition.
                if marks.get(&new_line) == Some(&GutterMark::Deleted) {
                    marks.insert(new_line, GutterMark::Modified);
                } else {
                    marks.insert(new_line, GutterMark::Added);
                }
                new_line += 1;
            }
            ChangeTag::Delete => {
                // Mark the current new_line as having a deletion.
                // If there's already an addition mark, upgrade to modified.
                marks
                    .entry(new_line)
                    .and_modify(|m| {
                        if *m == GutterMark::Added {
                            *m = GutterMark::Modified;
                        }
                    })
                    .or_insert(GutterMark::Deleted);
            }
        }
    }

    GutterMarks { marks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diff_no_marks() {
        let marks = compute_gutter_marks("hello\n", "hello\n");
        assert!(marks.is_empty());
    }

    #[test]
    fn added_lines_marked() {
        let marks = compute_gutter_marks("a\nb\n", "a\nnew\nb\n");
        assert_eq!(marks.get(1), Some(GutterMark::Added));
    }

    #[test]
    fn deleted_lines_marked() {
        let marks = compute_gutter_marks("a\nb\nc\n", "a\nc\n");
        // Deletion at line 1 (where b was removed before c).
        assert_eq!(marks.get(1), Some(GutterMark::Deleted));
    }

    #[test]
    fn modified_lines_marked() {
        let marks = compute_gutter_marks("a\nb\nc\n", "a\nB\nc\n");
        // Line 1: old "b" deleted, new "B" inserted → Modified.
        assert_eq!(marks.get(1), Some(GutterMark::Modified));
    }

    #[test]
    fn multiple_changes() {
        let marks = compute_gutter_marks("1\n2\n3\n4\n5\n", "1\nX\n3\nnew\n5\n");
        // Line 1: "2" → "X" = Modified
        assert_eq!(marks.get(1), Some(GutterMark::Modified));
        // Line 3: "4" → "new" = Modified (or added depending on diff)
        // Line 4 is unchanged "5"
        assert!(marks.len() >= 2);
    }

    #[test]
    fn new_file_all_added() {
        let marks = compute_gutter_marks("", "line1\nline2\n");
        assert_eq!(marks.get(0), Some(GutterMark::Added));
        assert_eq!(marks.get(1), Some(GutterMark::Added));
    }

    #[test]
    fn gutter_marks_len_and_empty() {
        let empty = GutterMarks::default();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let marks = compute_gutter_marks("a\n", "a\nb\n");
        assert!(!marks.is_empty());
        assert!(!marks.is_empty());
    }
}
