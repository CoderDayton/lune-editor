//! Markdown preview overlay — rendered via `tui-markdown`.

use std::borrow::Cow;
use std::time::Duration;
use std::time::Instant;

use ratatui_core::text::Text;

use crate::primitives::{Buffer, Line, Rect, Span, Style, Widget};
use crate::theme::Theme;
use crate::widgets::modal::{Anchor, Modal, ModalState};
use lune_core::buffer::BufferId;
use lune_core::undo::RevisionId;

/// State for the markdown preview overlay.
///
/// Owns the source text and a pre-parsed `Text<'static>` rendered from
/// it at open time. Re-parsing on every frame would scan + re-allocate
/// the entire document on every keystroke; we pay the parse cost once
/// and clone the cached lines per render.
#[derive(Clone, Debug, Default)]
pub struct MarkdownPreviewState {
    /// Source markdown text — typically the active buffer's content at the
    /// moment the preview was opened. Kept for diagnostic / re-render
    /// purposes only; the render path reads `rendered`.
    pub source: String,
    /// Optional title shown in the popup frame (usually the file path).
    pub title: String,
    /// Vertical scroll offset (in rendered lines, not source lines).
    pub scroll: u16,
    /// Cached ratatui `Text` produced by `tui_markdown::from_str` at open
    /// time. `None` only on a defaulted state (before any open call).
    pub rendered: Option<Text<'static>>,
    /// Buffer the cached parse came from. Together with `source_revision`,
    /// this forms the cache key for `OverlayState::refresh_markdown_preview`.
    pub source_buffer: Option<BufferId>,
    /// Revision of `source_buffer` when the cached parse was produced.
    pub source_revision: Option<RevisionId>,
    /// Wall-clock instant of the last parse. Used by
    /// `OverlayState::refresh_markdown_preview` to debounce per-keystroke
    /// re-parses on the UI thread.
    pub last_parsed_at: Option<Instant>,
}

impl MarkdownPreviewState {
    /// Scroll down by `lines` rows, capped at a soft upper bound to avoid
    /// scrolling past the end of long rendered output. The hard clamp
    /// happens at render time once we know the rendered line count.
    pub const fn scroll_down(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_add(lines);
    }

    /// Scroll up by `lines` rows.
    pub const fn scroll_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
    }
}

/// Convert a borrowed `Text<'_>` (e.g. returned by `tui_markdown::from_str`
/// which borrows from its input) into an owned `Text<'static>` so it can
/// be cached on overlay state without a lifetime tied to the source
/// buffer.
/// Minimum interval between live re-parses of the same buffer's markdown
/// preview. `tui_markdown::from_str` runs synchronously on the UI thread
/// and `buf.text()` materializes the entire rope; without this, every
/// keystroke on a large markdown buffer would block the event loop on
/// a full re-parse. 80 ms caps the worst-case re-parse rate at ~12 Hz
/// while keeping perceived staleness below typical human reaction time.
/// Bypassed for buffer swaps and the initial open so those paths are
/// never stale.
pub(crate) const MIN_LIVE_REFRESH_INTERVAL: Duration = Duration::from_millis(80);

pub(crate) fn render_markdown_preview(
    area: Rect,
    buf: &mut Buffer,
    state: &MarkdownPreviewState,
    theme: &Theme,
) {
    use crate::primitives::Paragraph;

    // Reserve ~80% of the screen for the preview, clamped to keep margins.
    let w = area.width.saturating_mul(8) / 10;
    let h = area.height.saturating_mul(8) / 10;
    if w < 20 || h < 5 {
        return;
    }
    let title = if state.title.is_empty() {
        " Markdown Preview ".to_string()
    } else {
        format!(" {} (preview) ", state.title)
    };
    let mut modal = ModalState::new();
    modal.open();
    Modal::new(theme)
        .title(&title)
        .title_style(Style::new().fg(theme.fg).bold())
        .border_style(Style::new().fg(theme.fg_muted))
        .size_cells(w, h)
        .anchor(Anchor::Center)
        .render(area, buf, &mut modal, |_, _| {});
    let Some(inner) = modal.inner_area() else {
        return;
    };

    // Cloned from the cached parse; cheap relative to re-parsing the
    // entire source on every render frame.
    let text = state.rendered.clone().unwrap_or_default();
    Paragraph::new(text)
        .scroll((state.scroll, 0))
        .style(Style::new().fg(theme.fg).bg(theme.bg))
        .render(inner, buf);
}

pub(crate) fn parse_markdown(mut source: String) -> (String, Text<'static>) {
    const MAX_MARKDOWN_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
    if source.len() > MAX_MARKDOWN_PREVIEW_BYTES {
        let mut cut = MAX_MARKDOWN_PREVIEW_BYTES;
        while cut > 0 && !source.is_char_boundary(cut) {
            cut -= 1;
        }
        source.truncate(cut);
        source.push_str("\n\n…[truncated]…\n");
    }
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        into_static_text(tui_markdown::from_str(&source))
    }))
    .unwrap_or_else(|_| Text::from("[markdown render panicked]"));
    (source, rendered)
}

fn into_static_text(t: Text<'_>) -> Text<'static> {
    Text {
        alignment: t.alignment,
        style: t.style,
        lines: t.lines.into_iter().map(into_static_line).collect(),
    }
}

fn into_static_line(l: Line<'_>) -> Line<'static> {
    Line {
        style: l.style,
        alignment: l.alignment,
        spans: l.spans.into_iter().map(into_static_span).collect(),
    }
}

fn into_static_span(s: Span<'_>) -> Span<'static> {
    Span {
        content: Cow::Owned(s.content.into_owned()),
        style: s.style,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::overlay::OverlayState;
    use lune_core::buffer::BufferId;
    use std::time::Instant;

    #[test]
    fn markdown_refresh_skips_when_revision_unchanged() {
        let mut overlay = OverlayState::default();
        let buf_id = BufferId(lune_core::uuid::Uuid::nil());
        overlay.open_markdown_preview("# A".to_string(), "x.md".to_string(), Some((buf_id, 7)));

        let mut source_calls = 0;
        let mut title_calls = 0;
        // Backdate `last_parsed_at` so the debounce window can't be what's
        // suppressing the call — this test is about the revision-equality
        // cache hit, not the debounce path.
        overlay.markdown_preview.last_parsed_at =
            Instant::now().checked_sub(Duration::from_secs(1));
        overlay.refresh_markdown_preview(
            buf_id,
            7,
            Instant::now(),
            || {
                source_calls += 1;
                "# B".to_string()
            },
            || {
                title_calls += 1;
                "x.md".to_string()
            },
        );
        assert_eq!(source_calls, 0, "lazy source_fn must not run when cached");
        assert_eq!(title_calls, 0, "lazy title_fn must not run when cached");
        assert_eq!(overlay.markdown_preview.source, "# A");
    }

    #[test]
    fn markdown_refresh_reparses_on_revision_bump() {
        let mut overlay = OverlayState::default();
        let buf_id = BufferId(lune_core::uuid::Uuid::nil());
        overlay.open_markdown_preview(
            "# Initial".to_string(),
            "x.md".to_string(),
            Some((buf_id, 1)),
        );
        overlay.markdown_preview.scroll = 4;
        // Push last parse past the debounce window so the revision bump is
        // allowed to re-parse.
        overlay.markdown_preview.last_parsed_at =
            Instant::now().checked_sub(Duration::from_secs(1));

        let mut title_calls = 0;
        overlay.refresh_markdown_preview(
            buf_id,
            2,
            Instant::now(),
            || "# Updated content\n\nmore".to_string(),
            || {
                title_calls += 1;
                "x.md".to_string()
            },
        );
        assert_eq!(overlay.markdown_preview.source, "# Updated content\n\nmore");
        assert_eq!(overlay.markdown_preview.source_revision, Some(2));
        assert_eq!(
            overlay.markdown_preview.scroll, 4,
            "scroll position preserved across refresh"
        );
        assert_eq!(
            title_calls, 0,
            "title_fn must not run on a same-buffer revision bump"
        );
        assert_eq!(overlay.markdown_preview.title, "x.md");
    }

    #[test]
    fn markdown_refresh_noop_when_overlay_inactive() {
        let mut overlay = OverlayState::default();
        // Never opened — overlay not active.
        let buf_id = BufferId(lune_core::uuid::Uuid::nil());
        overlay.refresh_markdown_preview(
            buf_id,
            1,
            Instant::now(),
            || "should not be called".to_string(),
            || "x.md".to_string(),
        );
        assert!(overlay.markdown_preview.source.is_empty());
        assert!(overlay.markdown_preview.source_buffer.is_none());
    }

    #[test]
    fn markdown_refresh_swaps_on_different_buffer() {
        let mut overlay = OverlayState::default();
        let buf_a = BufferId(lune_core::uuid::Uuid::nil());
        let buf_b = BufferId(lune_core::uuid::Uuid::new_v4());
        overlay.open_markdown_preview("# A".to_string(), "a.md".to_string(), Some((buf_a, 1)));
        // A buffer swap must bypass the debounce even when the previous
        // parse happened a moment ago — switching files must never show
        // stale content from the wrong buffer.
        overlay.markdown_preview.last_parsed_at = Some(Instant::now());

        let mut title_calls = 0;
        overlay.refresh_markdown_preview(
            buf_b,
            1, // same revision number — different buffer must still trigger
            Instant::now(),
            || "# B".to_string(),
            || {
                title_calls += 1;
                "b.md".to_string()
            },
        );
        assert_eq!(overlay.markdown_preview.source, "# B");
        assert_eq!(overlay.markdown_preview.source_buffer, Some(buf_b));
        assert_eq!(overlay.markdown_preview.title, "b.md");
        assert_eq!(title_calls, 1, "title_fn must run on a buffer swap");
    }

    #[test]
    fn markdown_refresh_debounces_revision_bumps_on_same_buffer() {
        let mut overlay = OverlayState::default();
        let buf_id = BufferId(lune_core::uuid::Uuid::nil());
        let t0 = Instant::now();
        overlay.open_markdown_preview("# v1".to_string(), "x.md".to_string(), Some((buf_id, 1)));
        overlay.markdown_preview.last_parsed_at = Some(t0);

        // Revision bump well within the debounce window: must be skipped.
        let mut source_calls = 0;
        overlay.refresh_markdown_preview(
            buf_id,
            2,
            t0 + Duration::from_millis(10),
            || {
                source_calls += 1;
                "# v2".to_string()
            },
            || "x.md".to_string(),
        );
        assert_eq!(
            source_calls, 0,
            "within-window revision bump must be skipped"
        );
        assert_eq!(overlay.markdown_preview.source_revision, Some(1));
        assert_eq!(overlay.markdown_preview.source, "# v1");

        // Same revision bump past the debounce window: must re-parse.
        overlay.refresh_markdown_preview(
            buf_id,
            2,
            t0 + MIN_LIVE_REFRESH_INTERVAL + Duration::from_millis(1),
            || {
                source_calls += 1;
                "# v2".to_string()
            },
            || "x.md".to_string(),
        );
        assert_eq!(source_calls, 1, "past-window revision bump must re-parse");
        assert_eq!(overlay.markdown_preview.source_revision, Some(2));
        assert_eq!(overlay.markdown_preview.source, "# v2");
    }
}
