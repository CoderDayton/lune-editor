#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_terminal_event(ct_event: &CtEvent, state: &mut AppState) -> Control<AppEvent> {
    match ct_event {
        CtEvent::Key(key_event) if key_event.kind == KeyEventKind::Press => {
            handle_key_event(key_event, state)
        }
        CtEvent::Mouse(mouse_event) => handle_mouse_event(*mouse_event, state),
        CtEvent::Resize(_, _) => Control::Changed,
        _ => Control::Continue,
    }
}

fn handle_key_event(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    if state.overlay.is_active() {
        return handle_overlay_key(key, state);
    }

    if state.root_tab == RootTab::Editor && key.code == KeyCode::Tab && key.modifiers.is_empty() {
        // Tab cycles focus when not in insert mode, or any time the editor
        // has no buffer to indent (so Tab isn't a silent no-op on an empty
        // editor — it pulls you to the file tree instead).
        if !state.vim.mode.is_insert() || state.session.active_buffer.is_none() {
            handle_focus_next_pane(state);
            return Control::Changed;
        }
    }

    if let Some(control) = handle_find_navigation_key(key, state) {
        return control;
    }

    if let Some(cmd) = contextual_key_command(key, state) {
        return Control::Event(AppEvent::Command(cmd));
    }

    if let Some(cmd) = state.keymap.lookup(key) {
        return Control::Event(AppEvent::Command(cmd.clone()));
    }

    if key.code == KeyCode::Esc {
        if state.focus.is_focused(PanelId::FileTree) || state.focus.is_focused(PanelId::GitPanel) {
            state.focus.focus(PanelId::Editor);
            return Control::Changed;
        }
        state.vim.enter_normal();
        state.vim.cmdline_clear();
        state.status_message.clear();
        return Control::Changed;
    }

    if state.root_tab == RootTab::Agents {
        return handle_agents_tab_key(key, state);
    }

    if state.focus.is_focused(PanelId::FileTree) {
        return handle_file_tree_key(key, state);
    }

    if state.focus.is_focused(PanelId::GitPanel) {
        return handle_git_panel_key(key, state);
    }

    if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        let page_size = state
            .last_editor_content_area
            .map_or(20, |a| (a.height as usize).saturating_sub(2).max(1));
        let extend = key.modifiers.contains(KeyModifiers::SHIFT);
        return apply_motion(state, |buf| {
            for _ in 0..page_size {
                match key.code {
                    KeyCode::PageUp => buf.move_up(extend),
                    _ => buf.move_down(extend),
                }
            }
        });
    }

    match state.vim.mode {
        VimMode::Insert => handle_insert_mode(key, state),
        VimMode::Normal => handle_normal_mode(key, state),
        VimMode::Visual | VimMode::VisualLine if state.vim_enabled => {
            handle_visual_mode(key, state)
        }
        VimMode::Command if state.vim_enabled => handle_vim_command_key(key, state),
        _ => {
            state.vim.enter_insert();
            handle_insert_mode(key, state)
        }
    }
}

fn contextual_key_command(key: &KeyEvent, state: &AppState) -> Option<AppCommand> {
    match (key.code, key.modifiers, state.root_tab) {
        (KeyCode::Char('n'), mods, RootTab::Agents) if mods == KeyModifiers::CONTROL => {
            Some(AppCommand::AgentSplitAuto)
        }
        (KeyCode::Char('n'), mods, RootTab::Editor) if mods == KeyModifiers::CONTROL => {
            Some(AppCommand::NewFile)
        }
        _ => None,
    }
}

fn handle_file_tree_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.file_tree.select_next(1);
            Control::Changed
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.file_tree.select_prev(1);
            Control::Changed
        }
        KeyCode::Enter => handle_file_tree_enter(state),
        KeyCode::Char('l') | KeyCode::Right => handle_file_tree_set_expanded(state, true),
        KeyCode::Char('h') | KeyCode::Left => handle_file_tree_set_expanded(state, false),
        KeyCode::Char('H') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Control::Event(AppEvent::Command(AppCommand::ToggleHiddenFiles))
        }
        KeyCode::Char('n') => Control::Event(AppEvent::Command(AppCommand::NewFile)),
        KeyCode::Char('N') => Control::Event(AppEvent::Command(AppCommand::NewDir)),
        KeyCode::Char('r') => Control::Event(AppEvent::Command(AppCommand::RenameEntry)),
        KeyCode::Char('d') => Control::Event(AppEvent::Command(AppCommand::DeleteEntry)),
        _ => Control::Continue,
    }
}

fn handle_file_tree_enter(state: &mut AppState) -> Control<AppEvent> {
    let Some((_, entry)) = state.file_tree.selected_entry().cloned() else {
        return Control::Continue;
    };

    match entry.kind {
        EntryKind::File | EntryKind::Symlink => {
            Control::Event(AppEvent::Command(AppCommand::OpenFile(entry.path)))
        }
        EntryKind::Directory { .. } => toggle_selected_dir(state),
    }
}

fn handle_file_tree_set_expanded(state: &mut AppState, expanded: bool) -> Control<AppEvent> {
    if state.file_tree.selected_is_dir() {
        if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
            if let Some(ref mut ws) = state.workspace {
                ws.set_expanded(&path, expanded);
                state.refresh_file_tree();
            }
        }
    }
    Control::Changed
}

pub(super) fn handle_focus_next_pane(state: &mut AppState) {
    if state.root_tab != RootTab::Editor {
        state.focus.set_active(PanelId::Editor);
        return;
    }

    let has_buffer = state.session.active_buffer.is_some();
    let mut panes = Vec::with_capacity(3);
    if state.layout.show_file_tree {
        panes.push(PanelId::FileTree);
    }
    // Skip the Editor pane from the cycle when there's nothing to edit —
    // Tab should never land on an empty editor.
    if has_buffer {
        panes.push(PanelId::Editor);
    }
    if state.layout.show_git_panel {
        panes.push(PanelId::GitPanel);
    }

    if panes.is_empty() {
        return;
    }

    let current = state.focus.active();
    let fallback = *panes.first().unwrap_or(&PanelId::Editor);
    let next = panes
        .iter()
        .position(|&p| p == current)
        .map_or(fallback, |idx| panes[(idx + 1) % panes.len()]);
    state.focus.set_active(next);
}

fn handle_git_panel_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.git_panel.select_next();
            Control::Changed
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.git_panel.select_prev();
            Control::Changed
        }
        KeyCode::Char('s') => Control::Event(AppEvent::Command(AppCommand::GitStage)),
        KeyCode::Char('u') => Control::Event(AppEvent::Command(AppCommand::GitUnstage)),
        KeyCode::Char('d') => Control::Event(AppEvent::Command(AppCommand::GitDiscard)),
        KeyCode::Char('c') => Control::Event(AppEvent::Command(AppCommand::GitCommit)),
        KeyCode::Char('r') => Control::Event(AppEvent::Command(AppCommand::GitRefresh)),
        KeyCode::Enter => {
            if let Some(file) = state.git_panel.selected_file().cloned() {
                let snap = state.git_port().status().load();
                if let Some(ref root) = snap.workdir_root {
                    let abs_path = root.join(&file.path);
                    return Control::Event(AppEvent::Command(AppCommand::OpenFile(abs_path)));
                }
            }
            Control::Continue
        }
        _ => Control::Continue,
    }
}

fn toggle_selected_dir(state: &mut AppState) -> Control<AppEvent> {
    if let Some(path) = state.file_tree.selected_path().map(Path::to_path_buf) {
        if let Some(ref mut ws) = state.workspace {
            ws.toggle_expanded(&path);
            state.refresh_file_tree();
        }
    }
    Control::Changed
}

#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_event(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    state.last_mouse_pos = Some((mouse.column, mouse.row));
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Root tab bar is shared across both tabs, so hit-test it before
            // dispatching to the per-tab handlers. Otherwise a click on
            // "Editor" while on the Agents tab would be swallowed by the
            // agents pane-focus logic and silently do nothing.
            if let Some(tab_area) = state.last_root_tabs_area {
                if point_in_rect(mouse.column, mouse.row, tab_area) {
                    if let Some(tab) = root_tab_hit_test(mouse.column, tab_area) {
                        state.set_root_tab(tab);
                        return Control::Changed;
                    }
                    return Control::Continue;
                }
            }
            if state.root_tab == RootTab::Agents {
                return handle_agents_mouse_down(mouse, state);
            }
            handle_mouse_click(mouse, state)
        }
        MouseEventKind::Down(MouseButton::Middle) => handle_middle_click(mouse, state),
        MouseEventKind::Drag(MouseButton::Left) => {
            if state.root_tab == RootTab::Agents {
                return handle_agents_mouse_drag(mouse, state);
            }
            if state.root_tab == RootTab::Editor {
                handle_mouse_drag(mouse, state)
            } else {
                Control::Continue
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            state.dragging_border = None;
            state.agents_tab.drag = None;
            state.block_select_anchor = None;
            Control::Continue
        }
        MouseEventKind::ScrollUp => handle_scroll(state, ScrollDir::Up),
        MouseEventKind::ScrollDown => handle_scroll(state, ScrollDir::Down),
        _ => Control::Continue,
    }
}

#[derive(Clone, Copy)]
enum ScrollDir {
    Up,
    Down,
}

/// Compute the number of lines to scroll the editor per wheel tick.
///
/// Viewport-proportional: each tick moves roughly 20% of the visible
/// area, with a floor of 3 lines so tiny viewports still feel
/// responsive, and a ceiling of half the viewport so a single tick
/// never teleports past your current context. The file's total line
/// count is intentionally ignored — a responsive feel on a 100-line
/// file and a 30,000-line file should look the same per tick.
fn editor_scroll_step(viewport_height: usize) -> usize {
    let proportional = viewport_height / 5;
    let floor = 3;
    let ceiling = (viewport_height / 2).max(floor);
    proportional.clamp(floor, ceiling)
}

/// Minimum time between rendered scroll frames. At ~30 fps we still
/// feel smooth but any queued scroll burst gets coalesced into few
/// renders instead of one per event.
const SCROLL_RENDER_INTERVAL: Duration = Duration::from_millis(33);

/// Peek at the crossterm event queue to decide whether more input is
/// already waiting behind this scroll. When a terminal sends a burst
/// of wheel events (Ghostty / kitty / wezterm with momentum) we get
/// dozens of events back-to-back; rendering every single one forces
/// the event loop to stutter behind the user's physical action.
///
/// We short-circuit to `Control::Unchanged` when more events are
/// pending AND we rendered recently. That mutates state but skips
/// the re-render, so the burst drains as fast as possible. A time
/// floor guarantees we still hit the screen at ~30 fps during long
/// inertial scrolls, preventing a "frozen" look.
fn scroll_control(state: &mut AppState) -> Control<AppEvent> {
    let more_pending = ratatui_crossterm::crossterm::event::poll(Duration::ZERO).unwrap_or(false);
    let since = state.last_scroll_render.elapsed();
    if more_pending && since < SCROLL_RENDER_INTERVAL {
        Control::Unchanged
    } else {
        state.last_scroll_render = Instant::now();
        Control::Changed
    }
}

/// Route a mouse-wheel scroll to the currently focused pane.
///
/// Focus-driven, not hover-driven: whichever pane was last focused (by
/// click or Tab cycling) receives the scroll. The editor branch scales
/// the step with viewport height via [`editor_scroll_step`]. File tree
/// and git panel move one row per tick for precision. Every branch
/// funnels through [`scroll_control`] so bursts of wheel events from
/// inertial scrolling get coalesced instead of triggering one render
/// each.
fn handle_scroll(state: &mut AppState, dir: ScrollDir) -> Control<AppEvent> {
    match state.focus.active() {
        PanelId::FileTree => {
            if state.file_tree.entries.is_empty() {
                return Control::Continue;
            }
            match dir {
                ScrollDir::Up => state.file_tree.select_prev(1),
                ScrollDir::Down => state.file_tree.select_next(1),
            }
            scroll_control(state)
        }
        PanelId::GitPanel => {
            match dir {
                ScrollDir::Up => state.git_panel.select_prev(),
                ScrollDir::Down => state.git_panel.select_next(),
            }
            scroll_control(state)
        }
        PanelId::Editor => {
            if state.root_tab != RootTab::Editor {
                return Control::Continue;
            }
            let total = state
                .active_buf()
                .map_or(0, lune_core::buffer::TextBuffer::line_count);
            let height = state
                .last_editor_content_area
                .map_or(20, |a| a.height as usize);
            let step = editor_scroll_step(height);

            // Advance an ease-out animation target instead of jumping the
            // viewport directly. The `AppEvent::Rendered` arm of `event`
            // lerps `top_line` toward this target frame by frame (driven
            // by `PollRendered`), which reads as subtle smoothing during
            // the final approach of each wheel burst.
            let max_top = total.saturating_sub(height.max(1));
            let current_target = state
                .viewport_scroll_target
                .unwrap_or(state.viewport.top_line);
            let new_target = match dir {
                ScrollDir::Up => current_target.saturating_sub(step),
                ScrollDir::Down => (current_target + step).min(max_top),
            };
            if new_target == state.viewport.top_line {
                state.viewport_scroll_target = None;
            } else {
                state.viewport_scroll_target = Some(new_target);
            }
            state.viewport_follow_cursor = false;
            scroll_control(state)
        }
        // Terminal / palette / status bar: no wheel scroll for now.
        _ => Control::Continue,
    }
}

fn set_viewport_from_scrollbar_row(
    state: &mut AppState,
    row: u16,
    content_area: Rect,
    total_lines: usize,
) {
    if let Some(top) = editor_pane::scrollbar_row_to_top_line(row, content_area, total_lines) {
        state.viewport.top_line = top;
        state.viewport_follow_cursor = false;
    }
}

fn handle_find_navigation_key(key: &KeyEvent, state: &mut AppState) -> Option<Control<AppEvent>> {
    if state.root_tab != RootTab::Editor || !state.focus.is_focused(PanelId::Editor) {
        return None;
    }
    if state.overlay.find_replace.search_state.matches.is_empty() {
        return None;
    }

    let control = match (key.code, key.modifiers) {
        (KeyCode::Char('n'), mods) if mods == KeyModifiers::CONTROL => {
            Some(super::overlay_handlers::find_next_match(state, false))
        }
        (KeyCode::Char('p'), mods) if mods == KeyModifiers::CONTROL => {
            Some(super::overlay_handlers::find_prev_match(state))
        }
        _ => None,
    }?;

    Some(control)
}

fn handle_middle_click(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    if state.root_tab != RootTab::Editor {
        return Control::Continue;
    }

    let Some(content_area) = state.last_editor_content_area else {
        return Control::Continue;
    };
    let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
    let has_git = state
        .session
        .active_buffer
        .is_some_and(|id| state.has_gutter(id));
    let Some(pos) = editor_pane::click_to_position(
        mouse.column,
        mouse.row,
        content_area,
        &state.viewport,
        total_lines,
        has_git,
    ) else {
        return Control::Continue;
    };

    state.focus.set_active(PanelId::Editor);
    super::editor_actions::handle_paste_at_position(state, pos)
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
fn handle_mouse_click(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    // Root tab bar clicks are intercepted by `handle_mouse_event` before
    // reaching this function.
    let (col, row) = (mouse.column, mouse.row);

    if state.root_tab != RootTab::Editor {
        return Control::Continue;
    }

    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .session
            .active_buffer
            .is_some_and(|id| state.has_gutter(id));
        if editor_pane::is_on_scrollbar(col, row, content_area, total_lines, has_git) {
            set_viewport_from_scrollbar_row(state, row, content_area, total_lines);
            state.focus.set_active(PanelId::Editor);
            state.dragging_border = Some(DragBorder::Scrollbar);
            return Control::Changed;
        }
    }

    if let Some(ref splits) = state.last_splits {
        if layout::is_on_left_border(splits, col) {
            state.dragging_border = Some(DragBorder::Left);
            return Control::Continue;
        }
        if layout::is_on_right_border(splits, col) {
            state.dragging_border = Some(DragBorder::Right);
            return Control::Continue;
        }

        if let Some(left_area) = splits.left {
            if point_in_rect(col, row, left_area) {
                state.focus.focus(PanelId::FileTree);
                let click_count = register_click(state, col, row, 500);
                if let Some(idx) = state.file_tree.hit_test(row, left_area) {
                    state.file_tree.selected = idx;
                    if click_count >= 2 {
                        state.last_click = None;
                        return handle_file_tree_enter(state);
                    }
                }
                return Control::Changed;
            }
        }

        if state.layout.show_git_panel {
            if let Some(right_area) = splits.right {
                if point_in_rect(col, row, right_area) {
                    state.focus.focus(PanelId::GitPanel);
                    return Control::Changed;
                }
            }
        }

        if row == splits.center.y {
            let tab_area = Rect::new(splits.center.x, splits.center.y, splits.center.width, 1);
            if let Some((idx, is_close)) = state.tab_mgr.hit_test(col, tab_area.x, tab_area.width) {
                if is_close {
                    if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                        close_tab_by_id(state, bid);
                    }
                } else if let Some(bid) = state.tab_mgr.buffer_at(idx) {
                    state.session.active_buffer = Some(bid);
                }
                return Control::Changed;
            }
        }
    }

    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .session
            .active_buffer
            .is_some_and(|id| state.has_gutter(id));
        if let Some(pos) = editor_pane::click_to_position(
            col,
            row,
            content_area,
            &state.viewport,
            total_lines,
            has_git,
        ) {
            state.focus.set_active(PanelId::Editor);
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                state.block_select_anchor = None;
                if let Some(buf) = state.active_buf_mut() {
                    let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                    let clamped_col = pos.col.min(buf.line_len_no_newline(clamped_line));
                    let _ = buf.toggle_secondary_cursor(Position::new(clamped_line, clamped_col));
                }
                state.viewport_follow_cursor = true;
                return Control::Changed;
            }
            if mouse.modifiers.contains(KeyModifiers::ALT) {
                if let Some(buf) = state.active_buf_mut() {
                    let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                    let clamped_col = pos.col.min(buf.line_len_no_newline(clamped_line));
                    let clamped = Position::new(clamped_line, clamped_col);
                    buf.set_block_selection(clamped, clamped);
                    state.block_select_anchor = Some(clamped);
                }
                state.viewport_follow_cursor = true;
                return Control::Changed;
            }
            let click_count = register_click(state, col, row, 400);
            state.block_select_anchor = None;

            if let Some(buf) = state.active_buf_mut() {
                let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                let clamped_col = pos.col.min(buf.line_len_no_newline(clamped_line));
                let clamped = Position::new(clamped_line, clamped_col);

                buf.cursor = CursorState::at(clamped);
                if click_count == 2 {
                    buf.move_word_left(false);
                    buf.move_word_right(true);
                } else if click_count >= 3 {
                    select_full_line(buf, clamped.line);
                    state.last_click = None;
                }
            }
            state.viewport_follow_cursor = true;
            return Control::Changed;
        }
    }

    Control::Continue
}

fn register_click(state: &mut AppState, col: u16, row: u16, threshold_ms: u128) -> u8 {
    let now = Instant::now();
    let count = match state.last_click {
        Some(last)
            if last.col == col
                && last.row == row
                && now.duration_since(last.at).as_millis() < threshold_ms =>
        {
            last.count.saturating_add(1).min(3)
        }
        _ => 1,
    };
    state.last_click = Some(MouseClickState {
        at: now,
        col,
        row,
        count,
    });
    count
}

fn select_full_line(buf: &mut TextBuffer, line: usize) {
    let last_line = buf.line_count().saturating_sub(1);
    let clamped = line.min(last_line);
    let start = Position::new(clamped, 0);
    let end = if clamped + 1 < buf.line_count() {
        Position::new(clamped + 1, 0)
    } else {
        Position::new(clamped, buf.line_len_no_newline(clamped))
    };
    buf.cursor = CursorState::from_selection(Selection::new(start, end));
}

fn drag_target_position(mouse: MouseEvent, state: &mut AppState) -> Option<Position> {
    let content_area = state.last_editor_content_area?;
    let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
    let has_git = state
        .session
        .active_buffer
        .is_some_and(|id| state.has_gutter(id));

    if mouse.row < content_area.y {
        state.viewport.scroll_up(1);
        state.viewport_follow_cursor = false;
    } else if mouse.row >= content_area.y + content_area.height {
        state
            .viewport
            .scroll_down(1, total_lines, content_area.height as usize);
        state.viewport_follow_cursor = false;
    }

    if mouse.column < content_area.x {
        state.viewport.left_col = state.viewport.left_col.saturating_sub(1);
        state.viewport_follow_cursor = false;
    } else if mouse.column >= content_area.x + content_area.width {
        state.viewport.left_col = state.viewport.left_col.saturating_add(1);
        state.viewport_follow_cursor = false;
    }

    let gutter = editor_pane::gutter_width(total_lines) + u16::from(has_git);
    let min_x = content_area.x.saturating_add(gutter);
    let mut max_x = content_area
        .x
        .saturating_add(content_area.width.saturating_sub(1));
    if editor_pane::is_on_scrollbar(max_x, content_area.y, content_area, total_lines, has_git) {
        max_x = max_x.saturating_sub(1);
    }
    max_x = max_x.max(min_x);
    let clamped_x = mouse.column.clamp(min_x, max_x);
    let clamped_y = mouse.row.clamp(
        content_area.y,
        content_area.y + content_area.height.saturating_sub(1),
    );

    editor_pane::click_to_position(
        clamped_x,
        clamped_y,
        content_area,
        &state.viewport,
        total_lines,
        has_git,
    )
}

fn root_tab_hit_test(col: u16, area: Rect) -> Option<RootTab> {
    let mut x = area.x;
    for (i, label) in ROOT_TAB_TITLES.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let width = label.len() as u16;
        if col >= x && col < x.saturating_add(width) {
            return Some(match i {
                0 => RootTab::Editor,
                1 => RootTab::Agents,
                _ => return None,
            });
        }
        x = x.saturating_add(width);
        if i + 1 < ROOT_TAB_TITLES.len() {
            #[allow(clippy::cast_possible_truncation)]
            let divider_width = ROOT_TAB_DIVIDER.len() as u16;
            x = x.saturating_add(divider_width);
        }
    }
    None
}

fn handle_editor_selection_drag(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let Some(pos) = drag_target_position(mouse, state) else {
        return Control::Continue;
    };
    let block_anchor = state.block_select_anchor;
    if let Some(buf) = state.active_buf_mut() {
        let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
        let clamped = Position::new(
            clamped_line,
            pos.col.min(buf.line_len_no_newline(clamped_line)),
        );
        if let Some(anchor) = block_anchor {
            buf.set_block_selection(anchor, clamped);
        } else {
            let anchor = buf.cursor.primary.anchor;
            buf.cursor.primary = Selection {
                anchor,
                head: clamped,
            };
        }
    }
    state.viewport_follow_cursor = true;
    Control::Changed
}

#[allow(clippy::cast_possible_truncation)]
fn handle_mouse_drag(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let Some(border) = state.dragging_border else {
        return handle_editor_selection_drag(mouse, state);
    };

    if matches!(border, DragBorder::Scrollbar) {
        if let Some(content_area) = state.last_editor_content_area {
            let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
            set_viewport_from_scrollbar_row(state, mouse.row, content_area, total_lines);
            return Control::Changed;
        }
        return Control::Continue;
    }

    let Some(ref splits) = state.last_splits else {
        return Control::Continue;
    };

    let total_width = splits.status.width;
    if total_width == 0 {
        return Control::Continue;
    }

    let pct = ((u32::from(mouse.column)) * 100 / u32::from(total_width)) as u16;

    match border {
        DragBorder::Left => state.layout.set_file_tree_width_pct(pct),
        DragBorder::Right => {
            let right_pct = 100u16.saturating_sub(pct);
            state.layout.set_right_panel_width_pct(right_pct);
        }
        DragBorder::Scrollbar => return Control::Continue,
    }

    Control::Changed
}

pub(super) fn handle_panel_command(cmd: &AppCommand, state: &mut AppState) -> Control<AppEvent> {
    match cmd {
        AppCommand::ToggleFileTree => {
            state.layout.toggle_file_tree();
            if state.layout.show_file_tree {
                state.focus.focus(PanelId::FileTree);
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            Control::Changed
        }
        AppCommand::ToggleGitPanel => {
            state.layout.toggle_git_panel();
            if state.layout.show_git_panel {
                state.focus.focus(PanelId::GitPanel);
                state.refresh_git();
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            Control::Changed
        }
        AppCommand::FocusNextPane => {
            handle_focus_next_pane(state);
            Control::Changed
        }
        AppCommand::OpenCommandPalette => {
            state.overlay.open_command_palette();
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::OpenFilePicker => {
            let start_dir = state.workspace.as_ref().map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                |ws| ws.root().to_path_buf(),
            );
            state.overlay.open_file_picker(&start_dir);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::OpenLanguagePicker => {
            let langs = LanguageRegistry::new().known_languages();
            state.overlay.open_language_picker(langs);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        AppCommand::OpenThemePicker => {
            let themes = state
                .theme_registry
                .list()
                .into_iter()
                .map(|(id, name)| (id.0, name.to_owned()))
                .collect();
            let current_idx = state.theme_registry.active_id().0;
            state.overlay.open_theme_picker(themes, current_idx);
            state.focus.focus(PanelId::CommandPalette);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_text(text: &str) -> AppState {
        let mut state = AppState::new();
        let id = state.session.registry.new_scratch();
        let buf = state.session.registry.get_mut(id).unwrap();
        buf.insert(Position::new(0, 0), text);
        buf.cursor = CursorState::at(Position::new(0, 0));
        state.session.active_buffer = Some(id);
        state.session.tabs.push(id);
        state.focus.set_active(PanelId::Editor);
        state
    }

    #[test]
    fn ctrl_n_cycles_to_next_search_match() {
        let mut state = state_with_text("foo bar foo");
        state.overlay.find_replace.search_state = state.active_buf().unwrap().search("foo", true);

        let result = handle_key_event(
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert_eq!(
            state.overlay.find_replace.search_state.current_match,
            Some(1)
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 8)
        );
    }

    #[test]
    fn ctrl_p_cycles_to_previous_search_match() {
        let mut state = state_with_text("foo bar foo");
        let mut search = state.active_buf().unwrap().search("foo", true);
        search.current_match = Some(1);
        state.overlay.find_replace.search_state = search;

        let result = handle_key_event(
            &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            &mut state,
        );

        assert!(matches!(result, Control::Changed));
        assert_eq!(
            state.overlay.find_replace.search_state.current_match,
            Some(0)
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.primary.head,
            Position::new(0, 0)
        );
    }

    #[test]
    fn click_editor_root_tab_from_agents_tab_switches_back() {
        // Regression: clicks on the root tab bar were being swallowed by the
        // agents mouse handler, so "Editor" was unclickable while on Agents.
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Agents);
        state.last_root_tabs_area = Some(Rect::new(0, 0, 40, 1));

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2, // inside the "Editor" label
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        let result = handle_mouse_event(click, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.root_tab, RootTab::Editor);
    }

    #[test]
    fn ctrl_n_opens_new_file_on_editor_tab() {
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Editor);

        let result = handle_key_event(
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            &mut state,
        );

        assert_eq!(
            result,
            Control::Event(AppEvent::Command(AppCommand::NewFile))
        );
    }

    #[test]
    fn ctrl_n_splits_on_agents_tab() {
        let mut state = AppState::new();
        state.set_root_tab(RootTab::Agents);

        let result = handle_key_event(
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            &mut state,
        );

        assert_eq!(
            result,
            Control::Event(AppEvent::Command(AppCommand::AgentSplitAuto))
        );
    }

    #[test]
    fn select_full_line_includes_newline_when_present() {
        let mut buf = TextBuffer::from_text("one\ntwo\n");
        select_full_line(&mut buf, 0);
        assert_eq!(
            buf.cursor.primary,
            Selection::new(Position::new(0, 0), Position::new(1, 0))
        );
    }

    #[test]
    fn register_click_tracks_triple_clicks() {
        let mut state = AppState::new();
        assert_eq!(register_click(&mut state, 10, 2, 400), 1);
        assert_eq!(register_click(&mut state, 10, 2, 400), 2);
        assert_eq!(register_click(&mut state, 10, 2, 400), 3);
    }

    #[test]
    fn triple_click_selects_full_line() {
        let mut state = state_with_text("alpha\nbeta");
        state.last_editor_content_area = Some(Rect::new(0, 0, 40, 10));

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let _ = handle_mouse_click(click, &mut state);
        let _ = handle_mouse_click(click, &mut state);
        let result = handle_mouse_click(click, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary,
            Selection::new(Position::new(0, 0), Position::new(1, 0))
        );
    }

    #[test]
    fn ctrl_click_toggles_secondary_cursor() {
        let mut state = state_with_text("alpha\nbeta");
        state.last_editor_content_area = Some(Rect::new(0, 0, 40, 10));

        let ctrl_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 6,
            row: 1,
            modifiers: KeyModifiers::CONTROL,
        };

        let first = handle_mouse_click(ctrl_click, &mut state);
        assert!(matches!(first, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.secondary,
            vec![Selection::cursor(Position::new(1, 4))]
        );

        let second = handle_mouse_click(ctrl_click, &mut state);
        assert!(matches!(second, Control::Changed));
        assert!(state.active_buf().unwrap().cursor.secondary.is_empty());
    }

    #[test]
    fn alt_drag_creates_block_selection() {
        let mut state = state_with_text("alpha\nbeta\ngamma");
        state.last_editor_content_area = Some(Rect::new(0, 0, 40, 5));

        let start = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 0,
            modifiers: KeyModifiers::ALT,
        };
        let drag = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 2,
            modifiers: KeyModifiers::ALT,
        };

        let first = handle_mouse_click(start, &mut state);
        let second = handle_mouse_drag(drag, &mut state);

        assert!(matches!(first, Control::Changed));
        assert!(matches!(second, Control::Changed));
        assert_eq!(
            state.active_buf().unwrap().cursor.primary,
            Selection::new(Position::new(0, 1), Position::new(0, 3))
        );
        assert_eq!(
            state.active_buf().unwrap().cursor.secondary,
            vec![
                Selection::new(Position::new(1, 1), Position::new(1, 3)),
                Selection::new(Position::new(2, 1), Position::new(2, 3)),
            ]
        );
    }

    #[test]
    fn drag_outside_viewport_autoscrolls_selection() {
        let mut state = state_with_text("0\n1\n2\n3\n4\n5");
        state.last_editor_content_area = Some(Rect::new(0, 0, 20, 2));

        let start = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let drag = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 2,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };

        let _ = handle_mouse_click(start, &mut state);
        let result = handle_mouse_drag(drag, &mut state);

        assert!(matches!(result, Control::Changed));
        assert_eq!(state.viewport.top_line, 1);
        assert_eq!(
            state.active_buf().unwrap().cursor.primary,
            Selection::new(Position::new(0, 0), Position::new(2, 0))
        );
    }

    #[test]
    fn scroll_sets_ease_out_target_when_editor_focused() {
        // 40-row viewport → step is 40/5 = 8 lines per tick.
        let text: String = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = state_with_text(&text);
        state.last_editor_content_area = Some(Rect::new(0, 0, 80, 40));
        state.focus.focus(PanelId::Editor);
        state.viewport.top_line = 50;

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let result = handle_mouse_event(scroll_up, &mut state);
        assert!(matches!(result, Control::Changed));
        // Wheel events set the ease-out target but don't move top_line
        // directly — the `Rendered` event drives the lerp.
        assert_eq!(state.viewport.top_line, 50);
        assert_eq!(state.viewport_scroll_target, Some(42));

        // A second tick accumulates into the same target.
        let _ = handle_mouse_event(scroll_up, &mut state);
        assert_eq!(state.viewport_scroll_target, Some(34));
    }

    #[test]
    fn editor_scroll_step_is_viewport_proportional() {
        // Floor of 3 for tiny viewports.
        assert_eq!(editor_scroll_step(0), 3);
        assert_eq!(editor_scroll_step(5), 3);
        assert_eq!(editor_scroll_step(14), 3);

        // ~20% of viewport once the viewport is big enough.
        assert_eq!(editor_scroll_step(15), 3);
        assert_eq!(editor_scroll_step(20), 4);
        assert_eq!(editor_scroll_step(30), 6);
        assert_eq!(editor_scroll_step(40), 8);
        assert_eq!(editor_scroll_step(60), 12);
        assert_eq!(editor_scroll_step(80), 16);

        // Ceiling is viewport/2 — proportional already stays under that
        // since 20% < 50%, but verify the clamp is sound for edge sizes.
        assert!(editor_scroll_step(100) <= 50);
    }

    #[test]
    fn scroll_moves_file_tree_selection_when_file_tree_focused() {
        use lune_core::workspace::{DirEntry, EntryKind};
        use std::path::PathBuf;

        let mut state = state_with_text("content");
        state.focus.focus(PanelId::FileTree);
        // Seed a handful of file tree entries so scroll has something to do.
        state.file_tree.entries = (0..6)
            .map(|i| {
                (
                    0usize,
                    DirEntry {
                        path: PathBuf::from(format!("/ws/f{i}.rs")),
                        name: format!("f{i}.rs"),
                        kind: EntryKind::File,
                        git_status: None,
                    },
                )
            })
            .collect();
        state.file_tree.selected = 0;

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        // Three ticks should advance the selection three rows (1 per tick).
        for _ in 0..3 {
            let result = handle_mouse_event(scroll_down, &mut state);
            assert!(matches!(result, Control::Changed));
        }
        assert_eq!(
            state.file_tree.selected, 3,
            "three scroll ticks should have moved the tree selection by three"
        );

        // Scrolling up three ticks should walk back up.
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..3 {
            let _ = handle_mouse_event(scroll_up, &mut state);
        }
        assert_eq!(state.file_tree.selected, 0);

        // And the editor viewport should not have moved.
        assert_eq!(state.viewport.top_line, 0);
    }

    #[test]
    fn scroll_is_ignored_when_file_tree_focused_but_empty() {
        let mut state = state_with_text("content");
        state.focus.focus(PanelId::FileTree);
        assert!(state.file_tree.entries.is_empty());

        let ev = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        let result = handle_mouse_event(ev, &mut state);
        assert!(matches!(result, Control::Continue));
    }
}
