//! Generic centered modal widget.
//!
//! A modal is a centered, bordered panel drawn *above* the underlying
//! pane content. The cells inside the modal rect are cleared so the
//! body has a clean slate, and (optionally) the surrounding area is
//! dimmed to make the panel read as floating chrome rather than as
//! part of the page below.
//!
//! Animation is intentionally absent — open/close is instant, matching
//! the rest of lune's overlays.
//!
//! ```no_run
//! use lune_ui::widgets::modal::{Modal, ModalState};
//! use lune_ui::primitives::{Buffer, Rect};
//! # let theme = lune_ui::theme::Theme::dark();
//! # let area = Rect::new(0, 0, 80, 24);
//! # let mut buf = Buffer::empty(area);
//! let mut state = ModalState::new();
//! state.open();
//!
//! Modal::new(&theme)
//!     .title(" confirm ")
//!     .size_cells(50, 7)
//!     .render(area, &mut buf, &mut state, |inner, buf| {
//!         // paint body into `inner`
//!     });
//! ```

use crate::primitives::{
    Alignment, Block, BorderType, Borders, Buffer, Clear, Color, Line, Modifier, Rect, Span, Style,
    Widget,
};
use crate::theme::Theme;
use unicode_width::UnicodeWidthStr;

/// Lifecycle handle for a [`Modal`].
///
/// Stores the open/closed flag and the last-rendered inner content
/// rect — callers use the rect to clamp cursors or hit-test clicks
/// without re-running the render.
#[derive(Debug, Clone, Default)]
pub struct ModalState {
    open: bool,
    inner_area: Option<Rect>,
    overlay_rect: Option<Rect>,
}

impl ModalState {
    /// Closed modal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark open. Takes effect on the next render.
    pub const fn open(&mut self) {
        self.open = true;
    }

    /// Mark closed. Subsequent renders are no-ops; rect handles are
    /// dropped so stale-hit tests fail safe.
    pub const fn close(&mut self) {
        self.open = false;
        self.inner_area = None;
        self.overlay_rect = None;
    }

    /// `true` when the modal should be drawn this frame.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Inner content rect from the last render (after border inset),
    /// or `None` if the modal hasn't been rendered yet or the area
    /// was too small for chrome.
    #[must_use]
    pub const fn inner_area(&self) -> Option<Rect> {
        self.inner_area
    }

    /// Rect occupied by the entire modal (chrome included). Useful
    /// for click-outside-to-dismiss hit testing.
    #[must_use]
    pub const fn overlay_rect(&self) -> Option<Rect> {
        self.overlay_rect
    }
}

/// How the modal sizes itself within the parent rect.
#[derive(Debug, Clone, Copy)]
enum Sizing {
    /// Fixed cells in both dimensions, clamped to the parent area.
    Cells { width: u16, height: u16 },
    /// Percentage of the parent rect in both dimensions (each 0..=100).
    Percent { width: u16, height: u16 },
}

impl Sizing {
    fn resolve(self, area: Rect) -> (u16, u16) {
        let (raw_w, raw_h) = match self {
            Self::Cells { width, height } => (width, height),
            Self::Percent { width, height } => (
                area.width.saturating_mul(width) / 100,
                area.height.saturating_mul(height) / 100,
            ),
        };
        (raw_w.min(area.width), raw_h.min(area.height))
    }
}

/// Vertical placement of the modal within the parent area.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    /// Centered vertically (the default).
    Center,
    /// `margin` rows below the top edge — e.g. for pickers that
    /// conventionally sit in the upper third. Clamped so the modal
    /// stays fully on-screen.
    Top { margin: u16 },
}

/// Centered modal config. Combine with [`ModalState`] to render.
#[derive(Debug, Clone)]
pub struct Modal<'a> {
    title: Option<&'a str>,
    sizing: Sizing,
    border_style: Style,
    title_style: Style,
    body_bg: Color,
    backdrop: Option<Color>,
    title_alignment: Alignment,
    border_type: BorderType,
    padding: (u16, u16),
    min_size: (u16, u16),
    anchor: Anchor,
    footer: Option<&'a str>,
    footer_style: Style,
}

impl<'a> Modal<'a> {
    /// Lune-flavored defaults driven from the active [`Theme`]: rounded
    /// `accent` border, accented title, editor background as the body,
    /// and a dimmed backdrop using `theme.bg`.
    #[must_use]
    pub const fn new(theme: &Theme) -> Self {
        Self {
            title: None,
            sizing: Sizing::Cells {
                width: 50,
                height: 7,
            },
            border_style: Style::new().fg(theme.accent),
            title_style: Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            body_bg: theme.bg,
            backdrop: Some(theme.bg),
            title_alignment: Alignment::Center,
            border_type: BorderType::Rounded,
            padding: (0, 0),
            min_size: (0, 0),
            anchor: Anchor::Center,
            footer: None,
            footer_style: Style::new().fg(theme.fg_muted),
        }
    }

    /// Title rendered into the top border. Centered by default; change
    /// with [`Modal::title_alignment`].
    #[must_use]
    pub const fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Fixed size in cells, clamped to the parent area.
    #[must_use]
    pub const fn size_cells(mut self, width: u16, height: u16) -> Self {
        self.sizing = Sizing::Cells { width, height };
        self
    }

    /// Percentage of the parent area in both dimensions (each 0..=100).
    #[must_use]
    pub const fn size_percent(mut self, width: u16, height: u16) -> Self {
        self.sizing = Sizing::Percent { width, height };
        self
    }

    /// Override the border style (e.g. red border for destructive flows).
    #[must_use]
    pub const fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    /// Override the title style (defaults to bold accent).
    #[must_use]
    pub const fn title_style(mut self, style: Style) -> Self {
        self.title_style = style;
        self
    }

    /// Disable the dimmed backdrop (modal will render with the
    /// underlying content visible around it). Default: enabled.
    #[must_use]
    pub const fn no_backdrop(mut self) -> Self {
        self.backdrop = None;
        self
    }

    /// Horizontal alignment of the title within the top border.
    /// Defaults to centered.
    #[must_use]
    pub const fn title_alignment(mut self, alignment: Alignment) -> Self {
        self.title_alignment = alignment;
        self
    }

    /// Override the border line type (defaults to rounded corners).
    #[must_use]
    pub const fn border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = border_type;
        self
    }

    /// Inner padding in cells `(horizontal, vertical)` applied to the
    /// body rect so content doesn't sit flush against the border.
    /// Defaults to `(0, 0)`.
    #[must_use]
    pub const fn padding(mut self, horizontal: u16, vertical: u16) -> Self {
        self.padding = (horizontal, vertical);
        self
    }

    /// Minimum size in cells `(width, height)`, applied after the
    /// configured sizing and before clamping to the parent area, so a
    /// percentage modal can't collapse on a small terminal. Defaults to
    /// `(0, 0)`.
    #[must_use]
    pub const fn min_size(mut self, width: u16, height: u16) -> Self {
        self.min_size = (width, height);
        self
    }

    /// Vertical placement within the parent area. Defaults to centered.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// A dim hint rendered centered into the bottom border (e.g. key
    /// hints). Defaults to none.
    #[must_use]
    pub const fn footer(mut self, footer: &'a str) -> Self {
        self.footer = Some(footer);
        self
    }

    /// Override the footer style (defaults to muted foreground).
    #[must_use]
    pub const fn footer_style(mut self, style: Style) -> Self {
        self.footer_style = style;
        self
    }

    /// Render the modal chrome above the existing content in `area`
    /// and call `body` with the inner content rect. Does nothing when
    /// `state` is closed or when the resolved rect is degenerate.
    pub fn render<F>(self, area: Rect, buf: &mut Buffer, state: &mut ModalState, body: F)
    where
        F: FnOnce(Rect, &mut Buffer),
    {
        if !state.open {
            state.inner_area = None;
            state.overlay_rect = None;
            return;
        }

        let (raw_w, raw_h) = self.sizing.resolve(area);
        let w = raw_w.max(self.min_size.0).min(area.width);
        let h = raw_h.max(self.min_size.1).min(area.height);
        if w == 0 || h == 0 {
            state.inner_area = None;
            state.overlay_rect = None;
            return;
        }

        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = match self.anchor {
            Anchor::Center => area.y + (area.height.saturating_sub(h)) / 2,
            Anchor::Top { margin } => (area.y + margin).min(area.y + area.height.saturating_sub(h)),
        };
        let modal_rect = Rect::new(x, y, w, h);

        // Backdrop first — dims the surrounding area so the modal
        // visually floats above. Skipping the modal rect itself keeps
        // the chrome at full saturation.
        if let Some(base) = self.backdrop {
            dim_backdrop(buf, area, modal_rect, base);
        }

        // `Clear` resets every cell in the modal rect, guaranteeing
        // the panel reads as above the underlying content even when
        // the backdrop is disabled.
        Clear.render(modal_rect, buf);

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(self.border_style)
            .style(Style::new().bg(self.body_bg));

        if let Some(t) = self.title {
            block = block.title(
                Line::from(Span::styled(t, self.title_style)).alignment(self.title_alignment),
            );
        }

        let inner = pad_rect(block.inner(modal_rect), self.padding);
        block.render(modal_rect, buf);

        // Footer hint sits in the bottom border, below the body.
        if let Some(footer) = self.footer {
            render_bottom_label(buf, modal_rect, footer, self.footer_style);
        }

        state.overlay_rect = Some(modal_rect);
        state.inner_area = Some(inner);

        if inner.width > 0 && inner.height > 0 {
            body(inner, buf);
        }
    }
}

/// Shrink a rect by `(horizontal, vertical)` padding on each side,
/// clamped so it never underflows.
fn pad_rect(rect: Rect, pad: (u16, u16)) -> Rect {
    let px = pad.0.min(rect.width / 2);
    let py = pad.1.min(rect.height / 2);
    Rect::new(
        rect.x + px,
        rect.y + py,
        rect.width - px * 2,
        rect.height - py * 2,
    )
}

/// Render a centered, padded label into the bottom border row of `rect`
/// (between the corners), clipped to the interior width.
#[allow(clippy::cast_possible_truncation)]
fn render_bottom_label(buf: &mut Buffer, rect: Rect, text: &str, style: Style) {
    if rect.height < 2 || rect.width < 4 {
        return;
    }
    let label = format!(" {text} ");
    let avail = rect.width - 2; // exclude the two corner cells
    let label_w = UnicodeWidthStr::width(label.as_str()) as u16;
    if label_w == 0 || label_w > avail {
        return;
    }
    let x = rect.x + 1 + (avail - label_w) / 2;
    let y = rect.y + rect.height - 1;
    Line::from(Span::styled(label, style)).render(Rect::new(x, y, label_w, 1), buf);
}

/// Dim every cell in `area` *except* those inside `exclude` by setting
/// the background to `base` and blending the foreground toward `base`.
/// Mirrors the dimming style used by other lune chrome overlays so the
/// underlying glyphs stay legible as ghosts.
fn dim_backdrop(buf: &mut Buffer, area: Rect, exclude: Rect, base: Color) {
    let area = area.intersection(*buf.area());
    let dim_fg = derive_dim_fg(base);

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if x >= exclude.left()
                && x < exclude.right()
                && y >= exclude.top()
                && y < exclude.bottom()
            {
                continue;
            }
            let cell = &mut buf[(x, y)];
            cell.fg = dim_fg;
            cell.bg = base;
        }
    }
}

/// Midpoint blend toward white: `c/2 + 64` per channel. Keeps glyphs
/// visible as low-contrast ghosts on the dimmed backdrop.
fn derive_dim_fg(base: Color) -> Color {
    let (r, g, b) = color_to_rgb(base);
    Color::Rgb(r / 2 + 64, g / 2 + 64, b / 2 + 64)
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => indexed_to_rgb(i),
        Color::Reset => (0, 0, 0),
        named => indexed_to_rgb(named_to_index(named)),
    }
}

#[allow(clippy::match_same_arms)] // unmapped Color variants fall through to 0
const fn named_to_index(color: Color) -> u8 {
    match color {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        _ => 0,
    }
}

fn indexed_to_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0 => (0, 0, 0),
        1 => (128, 0, 0),
        2 => (0, 128, 0),
        3 => (128, 128, 0),
        4 => (0, 0, 128),
        5 => (128, 0, 128),
        6 => (0, 128, 128),
        7 => (192, 192, 192),
        8 => (128, 128, 128),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (0, 0, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let i = index - 16;
            let to_channel = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (
                to_channel(i / 36),
                to_channel((i % 36) / 6),
                to_channel(i % 6),
            )
        }
        232..=255 => {
            let gray = 8 + 10 * (index - 232);
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_bg(buf: &Buffer, x: u16, y: u16) -> Option<Color> {
        buf.cell((x, y)).and_then(|c| c.style().bg)
    }

    fn cell_symbol(buf: &Buffer, x: u16, y: u16) -> String {
        buf.cell((x, y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn closed_state_renders_nothing() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        let mut body_called = false;

        Modal::new(&theme)
            .title(" hello ")
            .render(area, &mut buf, &mut state, |_, _| {
                body_called = true;
            });

        assert!(!body_called, "body must not run when state is closed");
        assert!(state.inner_area().is_none());
        assert!(state.overlay_rect().is_none());
    }

    #[test]
    fn open_state_exposes_inner_area_and_invokes_body() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();
        assert!(state.is_open());

        let mut got_inner: Option<Rect> = None;
        Modal::new(&theme)
            .title(" hello ")
            .size_cells(30, 7)
            .render(area, &mut buf, &mut state, |inner, _| {
                got_inner = Some(inner);
            });

        let inner = got_inner.expect("body must run when open");
        assert_eq!(inner.width, 28); // 30 - 2 border cells
        assert_eq!(inner.height, 5); //  7 - 2 border cells
        assert_eq!(inner.x, (60 - 30) / 2 + 1);
        assert_eq!(inner.y, (20 - 7) / 2 + 1);
    }

    #[test]
    fn modal_renders_above_existing_content() {
        // Pre-fill the buffer with 'x' everywhere so any cell that
        // ends up holding something else proves the modal painted
        // over it (i.e. drew above the existing content).
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol("x");
            }
        }

        let mut state = ModalState::new();
        state.open();
        Modal::new(&theme)
            .size_cells(20, 6)
            .no_backdrop() // isolate the "above" effect from the dim
            .render(area, &mut buf, &mut state, |_, _| {});

        let rect = state.overlay_rect().expect("rendered");
        // Border corner replaced the underlying 'x'.
        assert_ne!(cell_symbol(&buf, rect.x, rect.y), "x");
        // Cell inside the modal body was cleared from 'x' to ' '.
        assert_eq!(cell_symbol(&buf, rect.x + 1, rect.y + 1), " ");
        // Cell outside the modal still carries the original glyph.
        assert_eq!(cell_symbol(&buf, 0, 0), "x");
    }

    #[test]
    fn backdrop_dims_cells_outside_modal() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme)
            .size_cells(20, 6)
            .render(area, &mut buf, &mut state, |_, _| {});

        // Corner cell sits outside the modal — backdrop bg applied.
        assert_eq!(cell_bg(&buf, 0, 0), Some(theme.bg));
        // Inner cell carries the modal body bg.
        let rect = state.overlay_rect().unwrap();
        assert_eq!(cell_bg(&buf, rect.x + 1, rect.y + 1), Some(theme.bg));
    }

    #[test]
    fn no_backdrop_leaves_surrounding_cells_untouched() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        // Stamp a unique bg so we can detect any backdrop overwrite.
        let sentinel = Color::Rgb(11, 22, 33);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_bg(sentinel);
            }
        }
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme).size_cells(20, 6).no_backdrop().render(
            area,
            &mut buf,
            &mut state,
            |_, _| {},
        );

        assert_eq!(cell_bg(&buf, 0, 0), Some(sentinel));
    }

    #[test]
    fn close_after_open_makes_subsequent_render_a_no_op() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();
        Modal::new(&theme).render(area, &mut buf, &mut state, |_, _| {});
        assert!(state.inner_area().is_some());

        state.close();
        let mut buf = Buffer::empty(area);
        let mut body_called = false;
        Modal::new(&theme).render(area, &mut buf, &mut state, |_, _| {
            body_called = true;
        });
        assert!(!body_called);
        assert!(!state.is_open());
        assert!(state.inner_area().is_none());
    }

    #[test]
    fn percent_sizing_scales_with_parent_area() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 100, 50);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme)
            .size_percent(50, 40)
            .render(area, &mut buf, &mut state, |_, _| {});

        let rect = state.overlay_rect().expect("rendered");
        assert_eq!(rect.width, 50);
        assert_eq!(rect.height, 20);
    }

    #[test]
    fn size_clamps_to_parent_area() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme)
            .size_cells(200, 200) // larger than area
            .render(area, &mut buf, &mut state, |_, _| {});

        let rect = state.overlay_rect().expect("rendered");
        assert_eq!(rect.width, 20);
        assert_eq!(rect.height, 10);
    }

    #[test]
    fn title_is_centered_by_default() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme).title("Hi").size_cells(20, 6).render(
            area,
            &mut buf,
            &mut state,
            |_, _| {},
        );

        let rect = state.overlay_rect().unwrap();
        // The cell right after the top-left corner is border line, not
        // the title — proving the title isn't left-aligned.
        assert_eq!(cell_symbol(&buf, rect.x + 1, rect.y), "─");
        let top: String = (rect.x..rect.x + rect.width)
            .map(|x| cell_symbol(&buf, x, rect.y))
            .collect();
        assert!(top.contains("Hi"), "title present on top border: {top:?}");
    }

    #[test]
    fn footer_renders_in_bottom_border() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme).size_cells(24, 6).footer("Esc").render(
            area,
            &mut buf,
            &mut state,
            |_, _| {},
        );

        let rect = state.overlay_rect().unwrap();
        let bottom: String = (rect.x..rect.x + rect.width)
            .map(|x| cell_symbol(&buf, x, rect.y + rect.height - 1))
            .collect();
        assert!(
            bottom.contains("Esc"),
            "footer on bottom border: {bottom:?}"
        );
    }

    #[test]
    fn min_size_raises_percent_below_floor() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 100, 50);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        // 10% → 10x5, floored by min_size to 30x12.
        Modal::new(&theme)
            .size_percent(10, 10)
            .min_size(30, 12)
            .render(area, &mut buf, &mut state, |_, _| {});

        let rect = state.overlay_rect().unwrap();
        assert_eq!(rect.width, 30);
        assert_eq!(rect.height, 12);
    }

    #[test]
    fn anchor_top_places_modal_below_top_edge() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 30);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        Modal::new(&theme)
            .size_cells(20, 6)
            .anchor(Anchor::Top { margin: 2 })
            .render(area, &mut buf, &mut state, |_, _| {});

        assert_eq!(state.overlay_rect().unwrap().y, 2);
    }

    #[test]
    fn padding_shrinks_inner_body_rect() {
        let theme = Theme::dark();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut state = ModalState::new();
        state.open();

        let mut got: Option<Rect> = None;
        Modal::new(&theme).size_cells(20, 8).padding(2, 1).render(
            area,
            &mut buf,
            &mut state,
            |inner, _| got = Some(inner),
        );

        // Border inset → 18x6; padding (2,1) → 14x4.
        let inner = got.unwrap();
        assert_eq!(inner.width, 14);
        assert_eq!(inner.height, 4);
    }
}
