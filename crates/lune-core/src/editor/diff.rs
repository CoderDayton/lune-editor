//! Diff engine — computes line-level and character-level diffs.
//!
//! Uses the [`similar`] crate (Myers algorithm) to compare two text snapshots
//! and produce [`LiveHunk`] lists for rendering overlays.

use std::ops::Range;

use ropey::Rope;
use similar::{ChangeTag, TextDiff};

// ── Types ───────────────────────────────────────────────────────────────

/// The kind of change a hunk represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveHunkKind {
    /// New lines added (not present in baseline).
    Insertion,
    /// Lines removed from the baseline.
    Deletion,
    /// Lines changed (both deleted + inserted in the same region).
    Modification,
}

/// A single line within a diff hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveDiffLine {
    /// Whether this line is context, addition, or deletion.
    pub kind: LiveDiffLineKind,
    /// The line content (without leading +/- prefix).
    pub content: String,
}

/// Kind of a single diff line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveDiffLineKind {
    /// Unchanged context line.
    Context,
    /// Added line (present in new, not in old).
    Addition,
    /// Deleted line (present in old, not in new).
    Deletion,
}

/// A contiguous hunk of changes between baseline and disk content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveHunk {
    /// Sequential hunk identifier (0-based within the file).
    pub id: usize,
    /// Line range in the old (baseline) text (0-based, exclusive end).
    pub old_range: Range<usize>,
    /// Line range in the new (disk) text (0-based, exclusive end).
    pub new_range: Range<usize>,
    /// Classification of this hunk.
    pub kind: LiveHunkKind,
    /// All lines in this hunk (additions, deletions, and context).
    pub lines: Vec<LiveDiffLine>,
}

/// Character-level highlight span within a modified line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineHighlight {
    /// Byte range within the line content.
    pub range: Range<usize>,
    /// Whether this span was added or removed.
    pub kind: LiveDiffLineKind,
}

// ── Context lines around hunks ──────────────────────────────────────────

/// Number of unchanged context lines to include around each hunk.
const CONTEXT_LINES: usize = 3;

// ── Full diff ───────────────────────────────────────────────────────────

/// Compute a full diff between two rope contents.
///
/// Returns a list of [`LiveHunk`]s representing all changes, with
/// `CONTEXT_LINES` unchanged lines around each hunk for readability.
#[must_use]
pub fn compute_diff(old: &Rope, new: &Rope) -> Vec<LiveHunk> {
    let old_text = rope_to_string(old);
    let new_text = rope_to_string(new);
    compute_diff_str(&old_text, &new_text)
}

/// Compute a diff from string slices (avoids double allocation when
/// the caller already has strings).
#[must_use]
pub fn compute_diff_str(old: &str, new: &str) -> Vec<LiveHunk> {
    let text_diff = TextDiff::from_lines(old, new);

    // First pass: collect all changes into raw (old_line, new_line, tag) triples.
    let mut changes: Vec<(usize, usize, ChangeTag)> = Vec::new();
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for change in text_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                changes.push((old_line, new_line, ChangeTag::Equal));
                old_line += 1;
                new_line += 1;
            }
            ChangeTag::Delete => {
                changes.push((old_line, new_line, ChangeTag::Delete));
                old_line += 1;
            }
            ChangeTag::Insert => {
                changes.push((old_line, new_line, ChangeTag::Insert));
                new_line += 1;
            }
        }
    }

    // Second pass: group consecutive non-Equal changes into raw hunks,
    // then add context lines and finalize.
    let raw_groups = group_changes(&changes);

    // Collect old/new lines for content extraction.
    let old_lines = lines_inclusive(old);
    let new_lines = lines_inclusive(new);

    let mut hunks = Vec::with_capacity(raw_groups.len());

    for (hunk_id, group) in raw_groups.into_iter().enumerate() {
        let hunk = build_hunk(hunk_id, &group, &old_lines, &new_lines);
        hunks.push(hunk);
    }

    hunks
}

/// Compute character-level inline highlights for a pair of old/new lines
/// within a modified hunk. Useful for rendering word-level changes.
#[must_use]
pub fn compute_inline_highlights(old_line: &str, new_line: &str) -> Vec<InlineHighlight> {
    let diff = TextDiff::from_chars(old_line, new_line);
    let mut highlights = Vec::new();
    let mut old_byte = 0usize;
    let mut new_byte = 0usize;

    for change in diff.iter_all_changes() {
        let value = change.value();
        let byte_len = value.len();

        match change.tag() {
            ChangeTag::Equal => {
                old_byte += byte_len;
                new_byte += byte_len;
            }
            ChangeTag::Delete => {
                highlights.push(InlineHighlight {
                    range: old_byte..old_byte + byte_len,
                    kind: LiveDiffLineKind::Deletion,
                });
                old_byte += byte_len;
            }
            ChangeTag::Insert => {
                highlights.push(InlineHighlight {
                    range: new_byte..new_byte + byte_len,
                    kind: LiveDiffLineKind::Addition,
                });
                new_byte += byte_len;
            }
        }
    }

    highlights
}

// ── Incremental diff ────────────────────────────────────────────────────

/// Compute a diff incrementally when only a portion of the file changed.
///
/// `changed_range` is the line range (0-based, exclusive end) in the new
/// text that was modified. We expand the region by `CONTEXT_LINES` on each
/// side, extract the corresponding old region from `previous_hunks`, and
/// re-diff only that portion.
///
/// Falls back to full diff if the incremental result is ambiguous or if
/// no previous hunks exist.
#[must_use]
pub fn compute_diff_incremental(
    old: &Rope,
    new: &Rope,
    changed_range: Range<usize>,
    previous_hunks: &[LiveHunk],
) -> Vec<LiveHunk> {
    // If no previous hunks, just do a full diff.
    if previous_hunks.is_empty() {
        return compute_diff(old, new);
    }

    let old_line_count = count_content_lines(old);
    let new_line_count = count_content_lines(new);

    // Expand the changed range by context.
    let expanded_new_start = changed_range.start.saturating_sub(CONTEXT_LINES);
    let expanded_new_end = changed_range
        .end
        .saturating_add(CONTEXT_LINES)
        .min(new_line_count);

    // Find corresponding old range by looking at previous hunks that overlap.
    let (old_start, old_end) = find_old_range_for_new(
        expanded_new_start,
        expanded_new_end,
        previous_hunks,
        old_line_count,
    );

    // Extract sub-ropes.
    let old_sub = extract_lines(old, old_start, old_end);
    let new_sub = extract_lines(new, expanded_new_start, expanded_new_end);

    // Diff the sub-regions.
    let sub_hunks = compute_diff(&old_sub, &new_sub);

    // Offset hunk ranges back to file-level coordinates.
    let mut result = Vec::new();

    // 1. Hunks before the changed region (unchanged from previous).
    for h in previous_hunks {
        if h.new_range.end <= expanded_new_start {
            result.push(h.clone());
        }
    }

    // 2. Re-diffed hunks with adjusted ranges.
    for h in sub_hunks {
        result.push(LiveHunk {
            id: 0, // will be renumbered below
            old_range: (h.old_range.start + old_start)..(h.old_range.end + old_start),
            new_range: (h.new_range.start + expanded_new_start)
                ..(h.new_range.end + expanded_new_start),
            kind: h.kind,
            lines: h.lines,
        });
    }

    // 3. Hunks after the changed region (adjusted for any line count delta).
    append_trailing_hunks(&mut result, previous_hunks, expanded_new_end, old_end);

    // Renumber hunk IDs.
    for (i, h) in result.iter_mut().enumerate() {
        h.id = i;
    }

    // Fallback: verify by full diff. If counts diverge, use full diff.
    // This is cheap for small files and catches edge cases.
    let full = compute_diff(old, new);
    if full.len() != result.len() {
        return full;
    }

    result
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Cheaply convert a Rope to a String.
fn rope_to_string(rope: &Rope) -> String {
    let mut s = String::with_capacity(rope.len_bytes());
    for chunk in rope.chunks() {
        s.push_str(chunk);
    }
    s
}

/// Split a string into lines, including trailing `\n` in each.
fn lines_inclusive(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, byte) in s.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(&s[start..=i]);
            start = i + 1;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// Count "content lines" in a rope (excludes trailing empty line
/// that ropey adds when the content ends with `\n`).
fn count_content_lines(rope: &Rope) -> usize {
    if rope.len_chars() == 0 {
        return 0;
    }
    let total = rope.len_lines();
    let s = rope_to_string(rope);
    if s.ends_with('\n') {
        total.saturating_sub(1)
    } else {
        total
    }
}

/// A raw change group: contiguous non-Equal changes.
struct RawGroup {
    /// Line range in old text.
    old_range: Range<usize>,
    /// Line range in new text.
    new_range: Range<usize>,
    /// Whether this group has deletions.
    has_deletions: bool,
    /// Whether this group has insertions.
    has_insertions: bool,
}

/// Group consecutive non-Equal changes.
fn group_changes(changes: &[(usize, usize, ChangeTag)]) -> Vec<RawGroup> {
    let mut groups = Vec::new();
    let mut i = 0;

    while i < changes.len() {
        // Skip equal lines.
        if changes[i].2 == ChangeTag::Equal {
            i += 1;
            continue;
        }

        // Start of a change group.
        let first_change = i;
        let mut old_end = changes[i].0;
        let mut new_end = changes[i].1;
        let mut has_del = false;
        let mut has_ins = false;

        // Consume all consecutive non-Equal changes.
        while i < changes.len() && changes[i].2 != ChangeTag::Equal {
            match changes[i].2 {
                ChangeTag::Delete => {
                    has_del = true;
                    old_end = changes[i].0 + 1;
                }
                ChangeTag::Insert => {
                    has_ins = true;
                    new_end = changes[i].1 + 1;
                }
                ChangeTag::Equal => unreachable!(),
            }
            i += 1;
        }

        let old_start = changes[first_change].0;
        let new_start = changes[first_change].1;

        groups.push(RawGroup {
            old_range: old_start..old_end,
            new_range: new_start..new_end,
            has_deletions: has_del,
            has_insertions: has_ins,
        });
    }

    groups
}

/// Build a `LiveHunk` from a `RawGroup`, adding context lines.
fn build_hunk(
    hunk_id: usize,
    group: &RawGroup,
    old_lines: &[&str],
    new_lines: &[&str],
) -> LiveHunk {
    let kind = match (group.has_deletions, group.has_insertions) {
        (true, true) => LiveHunkKind::Modification,
        (false, true) => LiveHunkKind::Insertion,
        (true | false, false) => LiveHunkKind::Deletion,
    };

    // Compute context boundaries.
    let ctx_old_start = group.old_range.start.saturating_sub(CONTEXT_LINES);
    let ctx_old_end = group
        .old_range
        .end
        .min(old_lines.len())
        .saturating_add(CONTEXT_LINES)
        .min(old_lines.len());
    let ctx_new_start = group.new_range.start.saturating_sub(CONTEXT_LINES);
    let ctx_new_end = group
        .new_range
        .end
        .min(new_lines.len())
        .saturating_add(CONTEXT_LINES)
        .min(new_lines.len());

    let mut lines = Vec::new();

    // Leading context (from new side).
    let leading_end = group.new_range.start.min(new_lines.len());
    for line in &new_lines[ctx_new_start..leading_end] {
        lines.push(LiveDiffLine {
            kind: LiveDiffLineKind::Context,
            content: strip_trailing_newline(line),
        });
    }

    // Changed lines: deletions first, then insertions (unified style).
    let del_end = group.old_range.end.min(old_lines.len());
    for line in &old_lines[group.old_range.start..del_end] {
        lines.push(LiveDiffLine {
            kind: LiveDiffLineKind::Deletion,
            content: strip_trailing_newline(line),
        });
    }
    let ins_end = group.new_range.end.min(new_lines.len());
    for line in &new_lines[group.new_range.start..ins_end] {
        lines.push(LiveDiffLine {
            kind: LiveDiffLineKind::Addition,
            content: strip_trailing_newline(line),
        });
    }

    // Trailing context (from new side).
    let trailing_start = group.new_range.end.min(new_lines.len());
    for line in &new_lines[trailing_start..ctx_new_end] {
        lines.push(LiveDiffLine {
            kind: LiveDiffLineKind::Context,
            content: strip_trailing_newline(line),
        });
    }

    LiveHunk {
        id: hunk_id,
        old_range: ctx_old_start..ctx_old_end,
        new_range: ctx_new_start..ctx_new_end,
        kind,
        lines,
    }
}

/// Strip a trailing `\n` (and `\r\n`) from a line string.
fn strip_trailing_newline(s: &str) -> String {
    let trimmed = s.strip_suffix('\n').unwrap_or(s);
    trimmed.strip_suffix('\r').unwrap_or(trimmed).to_owned()
}

/// Find the old-text line range corresponding to a new-text range,
/// using previous hunk information to map coordinates.
fn find_old_range_for_new(
    new_start: usize,
    new_end: usize,
    hunks: &[LiveHunk],
    old_total: usize,
) -> (usize, usize) {
    let mut best_old_start = new_start.min(old_total);
    let mut best_old_end = new_end.min(old_total);

    // Adjust based on overlapping hunks.
    for h in hunks {
        if h.new_range.end <= new_start {
            // Hunk is entirely before our region — compute offset.
            let new_span = h.new_range.end.saturating_sub(h.new_range.start);
            let old_span = h.old_range.end.saturating_sub(h.old_range.start);
            if new_span >= old_span {
                let delta = new_span - old_span;
                best_old_start = new_start.saturating_sub(delta);
                best_old_end = new_end.saturating_sub(delta);
            } else {
                let delta = old_span - new_span;
                best_old_start = new_start.saturating_add(delta).min(old_total);
                best_old_end = new_end.saturating_add(delta).min(old_total);
            }
        } else if h.new_range.start < new_end && h.new_range.end > new_start {
            // Overlapping hunk — expand old range to cover it.
            best_old_start = best_old_start.min(h.old_range.start);
            best_old_end = best_old_end.max(h.old_range.end);
        }
    }

    (best_old_start.min(old_total), best_old_end.min(old_total))
}

/// Append hunks that come after the re-diffed region, adjusting for
/// any line count changes.
fn append_trailing_hunks(
    result: &mut Vec<LiveHunk>,
    previous_hunks: &[LiveHunk],
    expanded_new_end: usize,
    old_end: usize,
) {
    for h in previous_hunks {
        if h.new_range.start >= expanded_new_end {
            // Compute the offset between the new end and old end.
            let (adjusted_old_start, adjusted_old_end) = if expanded_new_end >= old_end {
                let delta = expanded_new_end - old_end;
                (
                    h.old_range.start.saturating_sub(delta),
                    h.old_range.end.saturating_sub(delta),
                )
            } else {
                let delta = old_end - expanded_new_end;
                (
                    h.old_range.start.saturating_add(delta),
                    h.old_range.end.saturating_add(delta),
                )
            };
            result.push(LiveHunk {
                id: 0,
                old_range: adjusted_old_start..adjusted_old_end,
                new_range: h.new_range.clone(),
                kind: h.kind,
                lines: h.lines.clone(),
            });
        }
    }
}

/// Extract a range of lines from a Rope into a new Rope.
fn extract_lines(rope: &Rope, start_line: usize, end_line: usize) -> Rope {
    let total = rope.len_lines();
    let start = start_line.min(total);
    let end = end_line.min(total);
    if start >= end || start >= total {
        return Rope::new();
    }

    let start_char = rope.line_to_char(start);
    let end_char = if end >= total {
        rope.len_chars()
    } else {
        rope.line_to_char(end)
    };

    Rope::from(rope.slice(start_char..end_char))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn identical_files_no_hunks() {
        let old = rope("hello\nworld\n");
        let new = rope("hello\nworld\n");
        let hunks = compute_diff(&old, &new);
        assert!(hunks.is_empty(), "identical files should produce no hunks");
    }

    #[test]
    fn add_only() {
        let old = rope("a\nb\n");
        let new = rope("a\nb\nc\nd\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, LiveHunkKind::Insertion);
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|l| l.kind == LiveDiffLineKind::Addition && l.content == "c"),
            "expected addition of 'c'"
        );
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|l| l.kind == LiveDiffLineKind::Addition && l.content == "d"),
            "expected addition of 'd'"
        );
    }

    #[test]
    fn delete_only() {
        let old = rope("a\nb\nc\nd\n");
        let new = rope("a\nd\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, LiveHunkKind::Deletion);
        let deletion_count = hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == LiveDiffLineKind::Deletion)
            .count();
        assert_eq!(deletion_count, 2);
    }

    #[test]
    fn modification() {
        let old = rope("a\nb\nc\n");
        let new = rope("a\nB\nc\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, LiveHunkKind::Modification);
    }

    #[test]
    fn mixed_changes() {
        let old = rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
        // Modify line 2, delete line 5, add line after 8.
        let new = rope("1\nTWO\n3\n4\n6\n7\n8\nnew\n9\n10\n");
        let hunks = compute_diff(&old, &new);
        // Should have multiple hunks (modifications spread apart).
        assert!(
            !hunks.is_empty(),
            "mixed changes should produce at least one hunk"
        );
    }

    #[test]
    fn empty_old_file() {
        let old = rope("");
        let new = rope("a\nb\nc\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, LiveHunkKind::Insertion);
    }

    #[test]
    fn empty_new_file() {
        let old = rope("a\nb\nc\n");
        let new = rope("");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, LiveHunkKind::Deletion);
    }

    #[test]
    fn both_empty() {
        let old = rope("");
        let new = rope("");
        let hunks = compute_diff(&old, &new);
        assert!(hunks.is_empty());
    }

    #[test]
    fn hunk_ids_sequential() {
        let old = rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n");
        let new = rope("1\nX\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nY\n15\n");
        let hunks = compute_diff(&old, &new);
        for (i, h) in hunks.iter().enumerate() {
            assert_eq!(h.id, i, "hunk IDs should be sequential");
        }
    }

    #[test]
    fn inline_highlights_basic() {
        let highlights = compute_inline_highlights("hello world", "hello rust");
        assert!(!highlights.is_empty(), "should detect char-level changes");
        // Should have both deletion (world) and insertion (rust).
        assert!(
            highlights
                .iter()
                .any(|h| h.kind == LiveDiffLineKind::Deletion)
        );
        assert!(
            highlights
                .iter()
                .any(|h| h.kind == LiveDiffLineKind::Addition)
        );
    }

    #[test]
    fn inline_highlights_identical() {
        let highlights = compute_inline_highlights("same text", "same text");
        assert!(
            highlights.is_empty(),
            "identical lines should have no highlights"
        );
    }

    #[test]
    fn context_lines_present() {
        // With enough surrounding lines, context should be included.
        let old = rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
        let new = rope("1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n10\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        // Should have context lines before and after the change.
        let context_count = hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == LiveDiffLineKind::Context)
            .count();
        assert!(context_count > 0, "should include context lines");
    }

    #[test]
    fn incremental_same_as_full() {
        let old = rope("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
        let new = rope("1\n2\nTHREE\n4\n5\n6\n7\n8\n9\n10\n");

        let full = compute_diff(&old, &new);
        let incremental = compute_diff_incremental(&old, &new, 2..3, &full);

        // Incremental should produce the same number of hunks.
        assert_eq!(
            full.len(),
            incremental.len(),
            "incremental should match full diff hunk count"
        );
    }

    #[test]
    fn incremental_no_previous_falls_back() {
        let old = rope("a\nb\nc\n");
        let new = rope("a\nB\nc\n");
        let hunks = compute_diff_incremental(&old, &new, 1..2, &[]);
        assert!(!hunks.is_empty());
    }

    #[test]
    fn strip_trailing_newline_works() {
        assert_eq!(strip_trailing_newline("hello\n"), "hello");
        assert_eq!(strip_trailing_newline("hello\r\n"), "hello");
        assert_eq!(strip_trailing_newline("hello"), "hello");
        assert_eq!(strip_trailing_newline(""), "");
    }

    #[test]
    fn extract_lines_basic() {
        let r = rope("0\n1\n2\n3\n4\n");
        let sub = extract_lines(&r, 1, 3);
        assert_eq!(sub.to_string(), "1\n2\n");
    }

    #[test]
    fn extract_lines_out_of_bounds() {
        let r = rope("a\nb\n");
        let sub = extract_lines(&r, 5, 10);
        assert_eq!(sub.to_string(), "");
    }

    #[test]
    fn lines_inclusive_fn() {
        let lines = lines_inclusive("a\nb\nc");
        assert_eq!(lines, vec!["a\n", "b\n", "c"]);

        let empty = lines_inclusive("");
        assert!(empty.is_empty());

        let trailing = lines_inclusive("x\n");
        assert_eq!(trailing, vec!["x\n"]);
    }

    #[test]
    fn count_content_lines_fn() {
        assert_eq!(count_content_lines(&rope("")), 0);
        assert_eq!(count_content_lines(&rope("a\n")), 1);
        assert_eq!(count_content_lines(&rope("a\nb\n")), 2);
        assert_eq!(count_content_lines(&rope("a\nb")), 2);
    }
}
