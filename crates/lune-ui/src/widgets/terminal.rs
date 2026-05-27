//! Embedded terminal emulator widget.
//!
//! Renders a `vt100::Screen` into a ratatui buffer, translating per-cell
//! colors and attributes to ratatui styles. Also renders the cursor
//! position and a session status header.

use crate::primitives::{
    Borders, Buffer, Color, Line, Modifier, Rect, Span, Style, Stylize, Widget,
};

use lune_ai::session::{AiSessionId, SessionState};

use crate::theme::Theme;
use crate::widgets::panel::panel_block;

/// Convert a `vt100::Color` to a `ratatui::Color`.
#[must_use]
const fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Build a ratatui `Style` from a vt100 cell's attributes.
fn cell_style(cell: &vt100::Cell) -> Style {
    let mut fg = convert_color(cell.fgcolor());
    let mut bg = convert_color(cell.bgcolor());

    // Handle inverse (swap fg/bg).
    if cell.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }

    let mut style = Style::default().fg(fg).bg(bg);

    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }

    style
}

/// Render the terminal session header bar (session name + state).
fn render_terminal_header(
    area: Rect,
    buf: &mut Buffer,
    display_name: &str,
    state: SessionState,
    theme: &Theme,
) {
    if area.height == 0 {
        return;
    }

    let state_str = match state {
        SessionState::Starting => " [Starting]",
        SessionState::Running => " [Running]",
        SessionState::Exited(0) => " [Exited]",
        SessionState::Exited(_) => " [Exited !]",
        SessionState::Error => " [Error]",
    };

    let state_color = match state {
        SessionState::Running => theme.accent,
        SessionState::Starting => Color::Yellow,
        SessionState::Exited(0) => theme.fg_dim,
        SessionState::Exited(_) | SessionState::Error => Color::Red,
    };

    let header_bg = theme.selection_bg;
    let header_style = Style::default().fg(theme.fg).bg(header_bg);

    // Fill the header row background.
    for x in area.x..area.x + area.width {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_style(header_style);
            cell.set_symbol(" ");
        }
    }

    let name_span = Span::styled(format!(" {display_name}"), header_style);
    let state_span = Span::styled(state_str, Style::default().fg(state_color).bg(header_bg));

    Line::from(vec![name_span, state_span]).render(Rect::new(area.x, area.y, area.width, 1), buf);
}

/// Render a `vt100::Screen` into the given buffer area.
///
/// Iterates the visible portion of the screen (accounting for scroll offset)
/// and translates each cell to ratatui buffer cells. Also renders the cursor
/// when the session is running and not scrolled.
pub fn render_terminal_screen(
    area: Rect,
    buf: &mut Buffer,
    screen: &vt100::Screen,
    scroll_offset: usize,
    show_cursor: bool,
    theme: &Theme,
) {
    let (screen_rows, screen_cols) = screen.size();

    // Fill background.
    let bg_style = Style::default().bg(theme.bg);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(bg_style);
                cell.set_symbol(" ");
            }
        }
    }

    // Render visible rows from the screen.
    // When scroll_offset == 0, we show the current visible screen.
    // Scrollback rendering is deferred to a future enhancement.
    if scroll_offset == 0 {
        for row_idx in 0..area.height.min(screen_rows) {
            let y = area.y + row_idx;
            let mut col_idx: u16 = 0;

            while col_idx < area.width.min(screen_cols) {
                let x = area.x + col_idx;

                if let Some(vt_cell) = screen.cell(row_idx, col_idx) {
                    let contents = vt_cell.contents();
                    let style = cell_style(vt_cell);

                    if let Some(buf_cell) = buf.cell_mut((x, y)) {
                        if contents.is_empty() {
                            buf_cell.set_symbol(" ");
                        } else {
                            buf_cell.set_symbol(contents);
                        }
                        buf_cell.set_style(style);
                    }

                    // Skip wide-continuation cells.
                    if vt_cell.is_wide() {
                        col_idx += 2;
                    } else {
                        col_idx += 1;
                    }
                } else {
                    col_idx += 1;
                }
            }
        }
    }

    // Render cursor (blinking block).
    if show_cursor && scroll_offset == 0 && !screen.hide_cursor() {
        let (cursor_row, cursor_col) = screen.cursor_position();
        let cx = area.x + cursor_col;
        let cy = area.y + cursor_row;
        if cx < area.x + area.width && cy < area.y + area.height {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                // Invert the cursor cell for visibility.
                let cursor_style = Style::default()
                    .fg(theme.bg)
                    .bg(theme.fg)
                    .add_modifier(Modifier::SLOW_BLINK);
                cell.set_style(cursor_style);
            }
        }
    }
}

/// Render the session tab bar (shown when >1 session exists).
///
/// `sessions` is a list of `(id, display_name, state)` for all sessions.
/// `active_id` is the currently focused session.
#[allow(clippy::cast_possible_truncation)]
fn render_session_tabs(
    area: Rect,
    buf: &mut Buffer,
    sessions: &[(AiSessionId, String, SessionState)],
    active_id: Option<AiSessionId>,
    theme: &Theme,
) {
    if area.height == 0 || sessions.is_empty() {
        return;
    }

    let mut x = area.x;
    for (id, name, state) in sessions {
        let is_active = Some(*id) == active_id;
        let state_mark = match state {
            SessionState::Running => "",
            SessionState::Starting => " ~",
            SessionState::Exited(0) => " ✓",
            SessionState::Exited(_) | SessionState::Error => " !",
        };
        let label = format!(" {name}{state_mark} ");
        let len = label.len() as u16;
        if x + len > area.x + area.width {
            break;
        }
        let tab_style = if is_active {
            theme.tab_active_focused
        } else {
            theme.tab_inactive
        };
        let span = Span::styled(label, tab_style);
        Line::from(span).render(Rect::new(x, area.y, len, 1), buf);
        x += len;
        // Separator
        if x < area.x + area.width {
            Line::from(Span::from("│").dim()).render(Rect::new(x, area.y, 1, 1), buf);
            x += 1;
        }
    }
}

/// Render the full AI terminal panel: optional session tabs + header + terminal screen.
///
/// `sessions` is the list of all sessions for the tab bar.
/// `session` is the currently active session to render.
pub fn render_ai_terminal(
    area: Rect,
    buf: &mut Buffer,
    sessions: &[(AiSessionId, String, SessionState)],
    session: Option<&lune_ai::AiSession>,
    theme: &Theme,
) {
    if area.height < 2 {
        return;
    }

    let multi = sessions.len() > 1;
    let tabs_height: u16 = u16::from(multi);

    match session {
        Some(session) => {
            // Tab bar (only when >1 session).
            if multi {
                let tabs_area = Rect::new(area.x, area.y, area.width, 1);
                let active_id = Some(session.id());
                render_session_tabs(tabs_area, buf, sessions, active_id, theme);
            }

            let header_y = area.y + tabs_height;
            let header_area = Rect::new(area.x, header_y, area.width, 1);
            let screen_y = header_y + 1;
            let screen_height = area.height.saturating_sub(tabs_height + 1);
            let screen_area = Rect::new(area.x, screen_y, area.width, screen_height);

            render_terminal_header(
                header_area,
                buf,
                session.kind().display_name(),
                session.state(),
                theme,
            );

            let show_cursor = session.state() == SessionState::Running;
            render_terminal_screen(
                screen_area,
                buf,
                session.screen(),
                session.scroll_offset(),
                show_cursor,
                theme,
            );
        }
        None => {
            render_no_session_placeholder(area, buf, theme);
        }
    }
}

/// Render the bottom terminal panel with Block border and tab labels.
///
/// `sessions` is the list of all sessions for the tab labels.
/// `session` is the currently active session to render.
/// `is_focused` controls the border color (focused vs unfocused).
#[allow(clippy::cast_possible_truncation)]
pub fn render_terminal_panel(
    area: Rect,
    buf: &mut Buffer,
    sessions: &[(AiSessionId, String, SessionState)],
    session: Option<&lune_ai::AiSession>,
    is_focused: bool,
    theme: &Theme,
) {
    if area.height < 2 || area.width < 4 {
        return;
    }

    // Build tab labels as the Block title.
    let title_spans = build_tab_title_spans(
        sessions,
        session.map(lune_ai::AiSession::id),
        is_focused,
        theme,
    );

    let block = panel_block(theme, is_focused, Borders::ALL).title(Line::from(title_spans));
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    match session {
        Some(session) => {
            let show_cursor = session.state() == SessionState::Running;
            render_terminal_screen(
                inner,
                buf,
                session.screen(),
                session.scroll_offset(),
                show_cursor,
                theme,
            );
        }
        None => {
            render_no_session_placeholder(inner, buf, theme);
        }
    }
}

/// Build styled `Span`s for the Block title line showing session tabs.
fn build_tab_title_spans(
    sessions: &[(AiSessionId, String, SessionState)],
    active_id: Option<AiSessionId>,
    is_focused: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    if sessions.is_empty() {
        return vec![Span::styled(" Terminal ".to_string(), Style::default())];
    }

    let mut spans = Vec::with_capacity(sessions.len() * 2 + 1);
    spans.push(Span::from(" "));
    for (i, (id, name, state)) in sessions.iter().enumerate() {
        let is_active = Some(*id) == active_id;
        let state_mark = match state {
            SessionState::Running => " [Running]",
            SessionState::Starting => " [Starting]",
            SessionState::Exited(0) => " [Exited]",
            SessionState::Exited(_) | SessionState::Error => " [Exited !]",
        };
        let label = format!("{name}{state_mark}");
        let style = if is_active {
            if is_focused {
                theme.tab_active_focused
            } else {
                theme.tab_active_unfocused
            }
        } else {
            theme.tab_inactive
        };
        spans.push(Span::styled(label, style));
        if i + 1 < sessions.len() {
            spans.push(Span::styled(" │ ".to_string(), Style::default().dim()));
        }
    }
    spans.push(Span::from(" "));
    spans
}

/// Render a placeholder when no session is active.
fn render_no_session_placeholder(area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height > 1 {
        let msg = "Press Ctrl+J to start a session";
        Line::from(Span::styled(msg, Style::default().fg(theme.fg_dim)))
            .render(Rect::new(area.x, area.y + 1, area.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buffer(w: u16, h: u16) -> (Rect, Buffer) {
        let area = Rect::new(0, 0, w, h);
        let buf = Buffer::empty(area);
        (area, buf)
    }

    #[test]
    fn convert_color_default() {
        assert_eq!(convert_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn convert_color_indexed() {
        assert_eq!(convert_color(vt100::Color::Idx(196)), Color::Indexed(196));
    }

    #[test]
    fn convert_color_rgb() {
        assert_eq!(
            convert_color(vt100::Color::Rgb(255, 128, 0)),
            Color::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn render_no_session() {
        let (area, mut buf) = make_buffer(40, 10);
        let theme = Theme::dark();
        render_ai_terminal(area, &mut buf, &[], None, &theme);

        // No sessions: should show the placeholder on row 1.
        let row1_text: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            row1_text.contains("Ctrl+J"),
            "Expected placeholder with Ctrl+J in row 1: {row1_text:?}"
        );
    }

    #[test]
    fn render_terminal_panel_with_block() {
        let (area, mut buf) = make_buffer(60, 15);
        let theme = Theme::dark();
        render_terminal_panel(area, &mut buf, &[], None, true, &theme);

        // Should contain "Terminal" in the title.
        let top_row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            top_row.contains("Terminal"),
            "Expected 'Terminal' in top row: {top_row:?}"
        );
    }

    #[test]
    fn render_terminal_screen_empty_parser() {
        let (area, mut buf) = make_buffer(40, 10);
        let theme = Theme::dark();
        let parser = vt100::Parser::new(10, 40, 100);
        render_terminal_screen(area, &mut buf, parser.screen(), 0, false, &theme);
        // Should not panic and should fill with spaces.
    }

    #[test]
    fn render_terminal_screen_with_content() {
        let (area, mut buf) = make_buffer(40, 10);
        let theme = Theme::dark();
        let mut parser = vt100::Parser::new(10, 40, 100);
        parser.process(b"Hello, World!");
        render_terminal_screen(area, &mut buf, parser.screen(), 0, true, &theme);

        // First row should contain "Hello, World!".
        let first_row: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            first_row.contains("Hello, World!"),
            "Expected 'Hello, World!' in first row: {first_row:?}"
        );
    }

    #[test]
    fn render_terminal_screen_with_colors() {
        let (area, mut buf) = make_buffer(40, 5);
        let theme = Theme::dark();
        let mut parser = vt100::Parser::new(5, 40, 100);
        // Red foreground text via ANSI escape.
        parser.process(b"\x1b[31mRed text\x1b[0m Normal");
        render_terminal_screen(area, &mut buf, parser.screen(), 0, false, &theme);

        // Check that 'R' has red color.
        if let Some(cell) = buf.cell((0, 0)) {
            let fg = cell.fg;
            // ANSI color 31 = red, which maps to Indexed(1).
            assert_eq!(fg, Color::Indexed(1), "Expected red foreground, got {fg:?}");
        }
    }

    #[test]
    fn render_tiny_area_does_not_panic() {
        let (area, mut buf) = make_buffer(5, 1);
        let theme = Theme::dark();
        render_ai_terminal(area, &mut buf, &[], None, &theme);
        // Area height < 2, should just return.
    }
}
