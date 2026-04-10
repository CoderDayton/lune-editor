#![allow(clippy::wildcard_imports)]

use super::*;
use crate::runtime::agents::DragState;
use crate::runtime::tiling;

pub(super) fn render_agents_tab(area: Rect, buf: &mut Buffer, state: &mut AppState) {
    state.last_splits = None;
    state.last_editor_content_area = None;
    state.last_agents_content_area = None;
    state.last_agent_pane_rects.clear();

    if area.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let content = chunks[0];
    let status = chunks[1];
    state.last_agents_content_area = Some(content);

    if state.agents_tab.is_empty() {
        render_empty_agents_tab(content, buf, state);
    } else {
        match state.agents_tab.layout.as_ref() {
            Some(_) if state.agents_tab.zoomed => {
                render_zoomed_agent_pane(content, buf, state);
            }
            Some(layout) => {
                let pane_rects = layout.compute_rects(content);
                let borders = layout.compute_borders(content);
                render_tiled_agent_panes(&pane_rects, &borders, buf, state);
            }
            None => render_degraded_agents_tab(content, buf, state),
        }
    }

    let status_state = state.build_status_line();
    status_bar::render_status_bar(status, buf, &status_state, &state.theme);
}

fn render_empty_agents_tab(content: Rect, buf: &mut Buffer, state: &AppState) {
    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(state.theme.overlay_border));
    let inner = block.inner(content);
    block.render(content, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let count_u16 = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
    let heading = ("No agent sessions yet.", Style::new().fg(state.theme.fg));
    let lines = [
        ("", Style::new().fg(state.theme.fg_muted)),
        (
            "Ctrl+N         open a new agent pane",
            Style::new().fg(state.theme.fg_muted),
        ),
        (
            "Alt+\\ / Alt+-  split vertical / horizontal",
            Style::new().fg(state.theme.fg_muted),
        ),
        (
            "Alt+j / Alt+k  focus next / prev pane",
            Style::new().fg(state.theme.fg_muted),
        ),
        (
            "Alt+x          close focused pane",
            Style::new().fg(state.theme.fg_muted),
        ),
        (
            "Alt+z          toggle zoom",
            Style::new().fg(state.theme.fg_muted),
        ),
        (
            "Alt+,          layout picker",
            Style::new().fg(state.theme.fg_muted),
        ),
    ];
    let total_height = count_u16(lines.len()).saturating_add(1);
    let start_y = inner.y + inner.height.saturating_sub(total_height) / 2;
    let text_block_width = lines
        .iter()
        .map(|(line, _)| count_u16(line.chars().count()))
        .max()
        .unwrap_or(0)
        .min(inner.width);
    let start_x = inner.x + inner.width.saturating_sub(text_block_width) / 2;
    let heading_width = count_u16(heading.0.chars().count()).min(inner.width);
    let heading_x = inner.x + inner.width.saturating_sub(heading_width) / 2;
    Line::from(heading.0)
        .style(heading.1)
        .render(Rect::new(heading_x, start_y, heading_width, 1), buf);
    for (i, (line, style)) in lines.iter().enumerate() {
        let row = start_y + 1 + u16::try_from(i).unwrap_or(u16::MAX);
        if row >= inner.y + inner.height {
            break;
        }
        Line::from(*line)
            .style(*style)
            .render(Rect::new(start_x, row, text_block_width, 1), buf);
    }
}

fn render_degraded_agents_tab(content: Rect, buf: &mut Buffer, state: &AppState) {
    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(state.theme.overlay_border));
    let inner = block.inner(content);
    block.render(content, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    Line::from("Agents layout unavailable.")
        .style(Style::new().fg(state.theme.fg))
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    if inner.height > 1 {
        Line::from("Reopen a pane or reapply a layout to recover.")
            .style(Style::new().fg(state.theme.fg_muted))
            .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }
}

fn render_zoomed_agent_pane(content: Rect, buf: &mut Buffer, state: &mut AppState) {
    let Some(session_id) = state.agents_tab.focused_session() else {
        return;
    };
    if let Some(pane_id) = state.agents_tab.focused {
        state.last_agent_pane_rects.push((pane_id, content));
    }
    if let Some(session) = state.ai_manager.session_mut(session_id) {
        sync_agent_session_size(session, content);
        let scroll = session.scroll_offset();
        let show_cursor = session.is_alive();
        terminal::render_terminal_screen(
            content,
            buf,
            session.screen(),
            scroll,
            show_cursor,
            &state.theme,
        );
    }
}

fn render_tiled_agent_panes(
    pane_rects: &[(tiling::PaneId, Rect)],
    borders: &[tiling::Border],
    buf: &mut Buffer,
    state: &mut AppState,
) {
    state.last_agent_pane_rects.clear();
    state.last_agent_pane_rects.extend(
        pane_rects
            .iter()
            .copied()
            .filter(|(_, rect)| rect.width > 0 && rect.height > 0),
    );
    let render_list: Vec<_> = pane_rects
        .iter()
        .filter_map(|(pid, area)| {
            if area.width == 0 || area.height == 0 {
                return None;
            }
            let sid = state.agents_tab.panes.get(pid)?.session_id;
            Some((*pid, sid, *area))
        })
        .collect();
    let focused = state.agents_tab.focused;
    for (pane_id, session_id, pane_area) in &render_list {
        if let Some(session) = state.ai_manager.session_mut(*session_id) {
            sync_agent_session_size(session, *pane_area);
            let show_cursor = session.is_alive() && focused == Some(*pane_id);
            let scroll = session.scroll_offset();
            terminal::render_terminal_screen(
                *pane_area,
                buf,
                session.screen(),
                scroll,
                show_cursor,
                &state.theme,
            );
        }
    }
    render_agent_split_borders(borders, buf, state);
}

fn render_agent_split_borders(borders: &[tiling::Border], buf: &mut Buffer, state: &AppState) {
    let border_style = Style::new().fg(state.theme.border_unfocused);
    for border in borders {
        let r = border.rect;
        if r.width == 0 || r.height == 0 {
            continue;
        }
        match border.direction {
            tiling::SplitDirection::Vertical => {
                for row in r.y..r.y + r.height {
                    if let Some(cell) = buf.cell_mut((r.x, row)) {
                        cell.set_char('│');
                        cell.set_style(border_style);
                    }
                }
            }
            tiling::SplitDirection::Horizontal => {
                for col in r.x..r.x + r.width {
                    if let Some(cell) = buf.cell_mut((col, r.y)) {
                        cell.set_char('─');
                        cell.set_style(border_style);
                    }
                }
            }
        }
    }
}

pub(super) fn sync_agent_session_size(session: &mut lune_ai::AiSession, area: Rect) {
    let target = AiTermSize::new(area.height.max(1), area.width.max(1));
    let current = session.screen().size();
    if current != (target.rows, target.cols) {
        if let Err(e) = session.resize(target) {
            log::warn!("Failed to resize agent session {}: {e}", session.id());
        }
    }
}

pub(super) fn handle_agents_tab_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if state.agents_tab.is_empty() {
        return Control::Continue;
    }

    if let Some(session_id) = state.agents_tab.focused_session() {
        if let Some(session) = state.ai_manager.session_mut(session_id) {
            let bytes = key_event_to_bytes(key);
            if !bytes.is_empty() {
                let _ = session.send_input(&bytes);
                return Control::Changed;
            }
        }
    }

    Control::Continue
}

pub(super) fn begin_agent_split_session(
    state: &mut AppState,
    requested: Option<(tiling::SplitDirection, tiling::SplitSide)>,
) -> Control<AppEvent> {
    if let Some(pending_id) = state.agents_tab_pending_pane {
        let pending_has_session = state.agents_tab.panes.contains_key(&pending_id);
        let pending_in_layout = state
            .agents_tab
            .layout
            .as_ref()
            .is_some_and(|layout| layout.pane_ids().contains(&pending_id));
        if pending_in_layout && !pending_has_session {
            state.overlay.open_ai_client_picker();
            return Control::Changed;
        }
        state.agents_tab_pending_pane = None;
    }

    state.set_root_tab(RootTab::Agents);

    if state.agents_tab.focused.is_none() {
        state.agents_tab.focused = state
            .agents_tab
            .layout
            .as_ref()
            .and_then(|layout| layout.pane_ids().into_iter().next());
    }

    let new_id = if state.agents_tab.is_empty() {
        Some(state.agents_tab.add_first_pane())
    } else {
        let split = match requested {
            Some((direction, side)) => choose_requested_agent_split(state, direction, side),
            None => choose_auto_agent_split(state),
        };
        let Some((direction, side)) = split else {
            state.overlay.notify(
                "Focused pane is too small to split again",
                NotificationLevel::Warning,
            );
            return Control::Changed;
        };
        state.agents_tab.split_focused_with_side(direction, side)
    };

    if let Some(pane_id) = new_id {
        refresh_active_saved_layout(state);
        state.overlay.open_ai_client_picker();
        state.agents_tab_pending_pane = Some(pane_id);
        Control::Changed
    } else {
        Control::Continue
    }
}

const fn default_agent_split() -> (tiling::SplitDirection, tiling::SplitSide) {
    (tiling::SplitDirection::Vertical, tiling::SplitSide::Second)
}

fn choose_requested_agent_split(
    state: &AppState,
    direction: tiling::SplitDirection,
    side: tiling::SplitSide,
) -> Option<(tiling::SplitDirection, tiling::SplitSide)> {
    focused_agent_pane_rect(state)
        .is_none_or(|rect| tiling::can_render_split(rect, direction))
        .then_some((direction, side))
}

fn choose_auto_agent_split(
    state: &mut AppState,
) -> Option<(tiling::SplitDirection, tiling::SplitSide)> {
    let focused_rect = pane_under_mouse(state).or_else(|| focused_agent_pane_rect(state));

    let Some(rect) = focused_rect else {
        return Some(default_agent_split());
    };

    let Some((col, row)) = state.last_mouse_pos else {
        return fallback_auto_split_for_rect(rect);
    };

    preferred_auto_split_for_rect(rect, col, row).or_else(|| fallback_auto_split_for_rect(rect))
}

const fn fallback_auto_split_for_rect(
    rect: Rect,
) -> Option<(tiling::SplitDirection, tiling::SplitSide)> {
    if tiling::can_render_split(rect, tiling::SplitDirection::Vertical) {
        Some(default_agent_split())
    } else if tiling::can_render_split(rect, tiling::SplitDirection::Horizontal) {
        Some((
            tiling::SplitDirection::Horizontal,
            tiling::SplitSide::Second,
        ))
    } else {
        None
    }
}

fn preferred_auto_split_for_rect(
    rect: Rect,
    col: u16,
    row: u16,
) -> Option<(tiling::SplitDirection, tiling::SplitSide)> {
    if !point_in_rect(col, row, rect) {
        return None;
    }

    let (direction, side) = split_from_point(rect, col, row);
    if tiling::can_render_split(rect, direction) {
        return Some((direction, side));
    }

    let alternate = match direction {
        tiling::SplitDirection::Vertical => tiling::SplitDirection::Horizontal,
        tiling::SplitDirection::Horizontal => tiling::SplitDirection::Vertical,
    };
    tiling::can_render_split(rect, alternate)
        .then_some((alternate, split_side_from_point(rect, col, row, alternate)))
}

const fn split_side_from_point(
    rect: Rect,
    col: u16,
    row: u16,
    direction: tiling::SplitDirection,
) -> tiling::SplitSide {
    match direction {
        tiling::SplitDirection::Vertical => {
            if col < rect.x + rect.width / 2 {
                tiling::SplitSide::First
            } else {
                tiling::SplitSide::Second
            }
        }
        tiling::SplitDirection::Horizontal => {
            if row < rect.y + rect.height / 2 {
                tiling::SplitSide::First
            } else {
                tiling::SplitSide::Second
            }
        }
    }
}

fn pane_under_mouse(state: &mut AppState) -> Option<Rect> {
    let (col, row) = state.last_mouse_pos?;
    let (pane_id, rect) = resolved_agent_pane_rects(state)
        .iter()
        .find(|(_, rect)| point_in_rect(col, row, *rect))
        .copied()?;
    state.agents_tab.focused = Some(pane_id);
    Some(rect)
}

fn focused_agent_pane_rect(state: &AppState) -> Option<Rect> {
    let focused = state.agents_tab.focused?;
    resolved_agent_pane_rects(state)
        .iter()
        .find(|(pane_id, _)| *pane_id == focused)
        .map(|(_, rect)| *rect)
}

fn resolved_agent_pane_rects(state: &AppState) -> Vec<(tiling::PaneId, Rect)> {
    if state.agents_tab.zoomed {
        if let Some(rects) = state
            .agents_tab
            .focused
            .zip(state.last_agents_content_area)
            .map(|(pane_id, rect)| vec![(pane_id, rect)])
        {
            return rects;
        }
    }

    if let (Some(content_area), Some(layout)) = (
        state.last_agents_content_area,
        state.agents_tab.layout.as_ref(),
    ) {
        return layout
            .compute_rects(content_area)
            .into_iter()
            .filter(|(_, rect)| rect.width > 0 && rect.height > 0)
            .collect();
    }

    state.last_agent_pane_rects.clone()
}

pub(super) fn split_from_point(
    rect: Rect,
    col: u16,
    row: u16,
) -> (tiling::SplitDirection, tiling::SplitSide) {
    if !point_in_rect(col, row, rect) {
        return (tiling::SplitDirection::Vertical, tiling::SplitSide::Second);
    }

    let center_x = rect.x + rect.width / 2;
    let center_y = rect.y + rect.height / 2;
    let dx = f64::from((i32::from(col) - i32::from(center_x)).unsigned_abs());
    let dy = f64::from((i32::from(row) - i32::from(center_y)).unsigned_abs());
    let norm_x = dx / f64::from(rect.width.max(1));
    let norm_y = dy / f64::from(rect.height.max(1));

    if norm_x >= norm_y {
        let side = if col < center_x {
            tiling::SplitSide::First
        } else {
            tiling::SplitSide::Second
        };
        (tiling::SplitDirection::Vertical, side)
    } else {
        let side = if row < center_y {
            tiling::SplitSide::First
        } else {
            tiling::SplitSide::Second
        };
        (tiling::SplitDirection::Horizontal, side)
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn handle_agents_mouse_down(
    mouse: MouseEvent,
    state: &mut AppState,
) -> Control<AppEvent> {
    let col = mouse.column;
    let row = mouse.row;

    let Some(layout) = state.agents_tab.layout.as_ref() else {
        return Control::Continue;
    };
    let Some(content_area) = state.last_agents_content_area else {
        return Control::Continue;
    };

    if !state.agents_tab.zoomed {
        if let Some((path, direction)) = layout.hit_test_border(content_area, col, row, 1) {
            state.agents_tab.drag = Some(DragState {
                split_path: path,
                direction,
            });
            return Control::Changed;
        }
    }

    let rects = state.last_agent_pane_rects.clone();
    state.agents_tab.focus_at(col, row, &rects);
    Control::Changed
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn handle_agents_mouse_drag(
    mouse: MouseEvent,
    state: &mut AppState,
) -> Control<AppEvent> {
    let Some(drag) = state.agents_tab.drag.clone() else {
        return Control::Continue;
    };
    let Some(content_area) = state.last_agents_content_area else {
        return Control::Continue;
    };

    let Some(layout) = state.agents_tab.layout.as_mut() else {
        return Control::Continue;
    };
    let Some(split_area) = layout.rect_at_path(content_area, &drag.split_path) else {
        return Control::Continue;
    };

    let Some(node) = layout.node_at_path_mut(&drag.split_path) else {
        return Control::Continue;
    };

    if let tiling::TileNode::Split {
        ratio, direction, ..
    } = node
    {
        let new_ratio = match direction {
            tiling::SplitDirection::Vertical => {
                let usable = split_area.width.saturating_sub(1);
                if usable == 0 {
                    return Control::Continue;
                }
                let offset = mouse.column.saturating_sub(split_area.x).min(usable);
                f64::from(offset) / f64::from(usable)
            }
            tiling::SplitDirection::Horizontal => {
                let usable = split_area.height.saturating_sub(1);
                if usable == 0 {
                    return Control::Continue;
                }
                let offset = mouse.row.saturating_sub(split_area.y).min(usable);
                f64::from(offset) / f64::from(usable)
            }
        };
        *ratio = new_ratio.clamp(0.1, 0.9);
    }

    Control::Changed
}

fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(ch) = key.code {
            let ctrl = (ch.to_ascii_lowercase() as u8)
                .wrapping_sub(b'a')
                .wrapping_add(1);
            return vec![ctrl];
        }
    }

    match key.code {
        KeyCode::Char(ch) => {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub(super) fn handle_ai_client_picker_key(
    key: &KeyEvent,
    state: &mut AppState,
) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            if let Some(pane_id) = state.agents_tab_pending_pane.take() {
                state.agents_tab.discard_pane(pane_id);
            }
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            state
                .overlay
                .ai_client_picker
                .selected_kind()
                .map_or(Control::Continue, |kind| {
                    close_overlay(state);
                    Control::Event(AppEvent::Command(AppCommand::AiNewSession(kind)))
                })
        }
        KeyCode::Up => {
            state.overlay.ai_client_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.ai_client_picker.select_next();
            Control::Changed
        }
        _ => Control::Continue,
    }
}
