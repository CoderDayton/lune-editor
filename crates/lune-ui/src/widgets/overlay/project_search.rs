//! Project-wide text search ("search in files") overlay.

use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthStr;

use crate::primitives::{Buffer, Line, Modifier, Rect, Span, Style, Stylize, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};
use lune_core::project::search::{self, SearchHit, SearchOptions};

use super::util::render_hrule;

/// would match almost everything and stall the walk.
const PROJECT_SEARCH_MIN_QUERY: usize = 2;

/// State backing the project-wide search ("search in files") overlay.
///
/// File contents are loaded into memory once on [`Self::open`]; each
/// keystroke re-searches them in memory via [`Self::update_results`], so
/// typing stays responsive without re-reading the tree.
#[derive(Clone, Debug, Default)]
pub struct ProjectSearchState {
    /// Workspace root, used to show hit paths relative to it.
    pub root: PathBuf,
    /// Text file contents loaded once on open (re-searched per keystroke).
    loaded: Vec<search::LoadedFile>,
    /// Current query text.
    pub input: String,
    /// Matches for the current query.
    pub results: Vec<SearchHit>,
    /// Whether the result cap clipped the matches.
    pub truncated: bool,
    /// Index of the highlighted result.
    pub selected: usize,
    /// Scroll offset into `results`.
    pub scroll_offset: usize,
}

impl ProjectSearchState {
    /// Open at `root`: gather the candidate file list and clear any prior
    /// query and results.
    pub fn open(&mut self, root: &Path) {
        self.root = root.to_path_buf();
        self.input.clear();
        self.results.clear();
        self.truncated = false;
        self.selected = 0;
        self.scroll_offset = 0;
        self.loaded = search::load_files(&search::collect_files(root), SearchOptions::default());
    }

    /// Re-run the search against the cached file list. Queries shorter than
    /// `PROJECT_SEARCH_MIN_QUERY` clear the results.
    pub fn update_results(&mut self) {
        if self.query_len() < PROJECT_SEARCH_MIN_QUERY {
            self.results.clear();
            self.truncated = false;
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }
        let outcome = search::search_loaded(&self.loaded, &self.input, SearchOptions::default());
        self.results = outcome.hits;
        self.truncated = outcome.truncated;
        if self.results.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.results.len() - 1);
        }
        self.scroll_offset = self.scroll_offset.min(self.selected);
    }

    /// Number of characters typed (the unit the minimum-query gate and the
    /// status hint speak in — not bytes).
    #[must_use]
    fn query_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Append a character to the query and re-search.
    pub fn type_char(&mut self, ch: char) {
        self.input.push(ch);
        self.update_results();
    }

    /// Delete the last query character and re-search. Returns `false` when
    /// the query was already empty.
    pub fn backspace(&mut self) -> bool {
        if self.input.pop().is_some() {
            self.update_results();
            true
        } else {
            false
        }
    }

    /// Move the selection up, wrapping to the bottom.
    pub const fn select_prev(&mut self) {
        if !self.results.is_empty() {
            self.selected = if self.selected == 0 {
                self.results.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move the selection down, wrapping to the top.
    pub const fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
        }
    }

    /// The currently highlighted hit, if any.
    #[must_use]
    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.results.get(self.selected)
    }

    /// Scroll so the selection sits within a `viewport_height`-row window.
    pub const fn ensure_visible(&mut self, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + viewport_height {
            self.scroll_offset = self.selected - viewport_height + 1;
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn render_project_search(
    area: Rect,
    buf: &mut Buffer,
    state: &mut ProjectSearchState,
    theme: &Theme,
) {
    // The overlay enum gates visibility, so the modal opens transiently
    // for this frame; no persistent lifecycle state is needed.
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(" Search in Files ")
        .size_percent(70, 60)
        .min_size(40, 10)
        .anchor(Anchor::Top { margin: 2 })
        .footer(" ↑↓ select · Enter open · Esc close ")
        .render(area, buf, &mut modal, |inner, buf| {
            // Input line.
            let input_line = format!("> {}", state.input);
            Line::from(Span::from(input_line).bold())
                .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

            // Separator under the input.
            if inner.height > 1 {
                render_hrule(buf, inner.x, inner.y + 1, inner.width);
            }

            // Status line: how many matches, or why there are none.
            let status = if state.input.chars().count() < PROJECT_SEARCH_MIN_QUERY {
                "  type at least 2 characters".to_string()
            } else if state.results.is_empty() {
                "  no matches".to_string()
            } else if state.truncated {
                format!("  {}+ matches", state.results.len())
            } else if state.results.len() == 1 {
                "  1 match".to_string()
            } else {
                format!("  {} matches", state.results.len())
            };
            if inner.height > 2 {
                Line::from(Span::from(status).style(Style::new().fg(theme.overlay_hint_fg)))
                    .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
            }

            // Results list.
            let list_start_y = inner.y + 3;
            let list_height = inner.height.saturating_sub(3) as usize;
            state.ensure_visible(list_height);

            // Visible window into the results.
            let visible = (state.scroll_offset..state.results.len()).take(list_height);

            // First pass: widest `path:line` label among the visible rows,
            // so the code column lines up regardless of label width.
            let max_loc_w = visible
                .clone()
                .map(|i| {
                    let hit = &state.results[i];
                    let rel = hit
                        .path
                        .strip_prefix(&state.root)
                        .unwrap_or(hit.path.as_path());
                    UnicodeWidthStr::width(format!("{}:{}", rel.display(), hit.line + 1).as_str())
                })
                .max()
                .unwrap_or(0);

            for (vi, i) in visible.enumerate() {
                let y = list_start_y + vi as u16;
                if y >= inner.y + inner.height {
                    break;
                }

                let hit = &state.results[i];
                let rel = hit
                    .path
                    .strip_prefix(&state.root)
                    .unwrap_or(hit.path.as_path());
                let label = format!("{}:{}", rel.display(), hit.line + 1);
                let pad = max_loc_w.saturating_sub(UnicodeWidthStr::width(label.as_str()));
                // Two-space indent, padded label, two-space gutter.
                let location = format!("  {label}{:pad$}  ", "");

                // `match_start`/`match_end` are byte offsets into `line_text`;
                // tab\u2192space replacement is 1:1 byte-wise, so they stay valid.
                let code = hit.line_text.replace('\t', " ");
                let ms = hit.match_start.min(code.len());
                let me = hit.match_end.min(code.len()).max(ms);
                let selected = i == state.selected;
                let (loc_style, code_style) = if selected {
                    (theme.overlay_selected, theme.overlay_selected)
                } else {
                    (
                        Style::new().fg(theme.overlay_hint_fg),
                        Style::new().fg(theme.overlay_file_fg),
                    )
                };
                // Highlight the matched substring within the code column.
                let match_style = if selected {
                    theme.overlay_selected.add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
                };
                Line::from(vec![
                    Span::styled(location, loc_style),
                    Span::styled(code[..ms].to_string(), code_style),
                    Span::styled(code[ms..me].to_string(), match_style),
                    Span::styled(code[me..].to_string(), code_style),
                ])
                .render(Rect::new(inner.x, y, inner.width, 1), buf);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::{OverlayKind, OverlayState};
    use std::fs;

    #[test]
    fn project_search_opens_and_matches_above_min_query() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "needle here\nplain text\n").unwrap();

        let mut overlay = OverlayState::default();
        overlay.open_project_search(dir.path());
        assert!(matches!(overlay.active, Some(OverlayKind::ProjectSearch)));

        // One character is below the minimum query length: no results.
        overlay.project_search.type_char('n');
        assert!(overlay.project_search.results.is_empty());

        // At the minimum length the match appears.
        overlay.project_search.type_char('e');
        assert_eq!(overlay.project_search.results.len(), 1);
        assert_eq!(overlay.project_search.results[0].line, 0);
    }

    #[test]
    fn project_search_navigation_wraps() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "xx\nxx\nxx\n").unwrap();

        let mut ps = ProjectSearchState::default();
        ps.open(dir.path());
        ps.type_char('x');
        ps.type_char('x');
        assert_eq!(ps.results.len(), 3);
        assert_eq!(ps.selected, 0);

        ps.select_prev();
        assert_eq!(ps.selected, 2);
        ps.select_next();
        assert_eq!(ps.selected, 0);
    }

    #[test]
    fn project_search_backspace_below_min_clears_results() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "abcd\n").unwrap();

        let mut ps = ProjectSearchState::default();
        ps.open(dir.path());
        ps.type_char('a');
        ps.type_char('b');
        assert_eq!(ps.results.len(), 1);

        ps.backspace();
        assert_eq!(ps.input, "a");
        assert!(ps.results.is_empty());
    }

    #[test]
    fn project_search_close_resets_state() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "match me\n").unwrap();

        let mut overlay = OverlayState::default();
        overlay.open_project_search(dir.path());
        overlay.project_search.type_char('m');
        overlay.project_search.type_char('a');
        assert!(!overlay.project_search.results.is_empty());

        overlay.close();
        assert!(overlay.active.is_none());
        assert!(overlay.project_search.input.is_empty());
        assert!(overlay.project_search.results.is_empty());
    }
}
