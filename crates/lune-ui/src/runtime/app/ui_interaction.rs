#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_terminal_event(
    ct_event: &CtEvent,
    state: &mut AppState,
) -> Control<AppEvent> {
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

    if state.root_tab == RootTab::Editor
        && key.code == KeyCode::Tab
        && key.modifiers.is_empty()
        && !state.vim.mode.is_insert()
    {
        handle_focus_next_pane(state);
        return Control::Changed;
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

    let mut panes = Vec::with_capacity(3);
    if state.layout.show_file_tree {
        panes.push(PanelId::FileTree);
    }
    panes.push(PanelId::Editor);
    if state.layout.show_git_panel {
        panes.push(PanelId::GitPanel);
    }

    let current = state.focus.active();
    let next = panes
        .iter()
        .position(|&p| p == current)
        .map_or(PanelId::Editor, |idx| panes[(idx + 1) % panes.len()]);
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
                if let Some(git) = &state.git_service {
                    let abs_path = git.root().join(&file.path);
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
            if state.root_tab == RootTab::Agents {
                return handle_agents_mouse_down(mouse, state);
            }
            handle_mouse_click(mouse, state)
        }
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
            Control::Continue
        }
        MouseEventKind::ScrollUp => {
            if state.root_tab == RootTab::Editor {
                state.viewport.scroll_up(3);
                state.viewport_follow_cursor = false;
                Control::Changed
            } else {
                Control::Continue
            }
        }
        MouseEventKind::ScrollDown => {
            if state.root_tab == RootTab::Editor {
                let total = state
                    .active_buf()
                    .map_or(0, lune_core::buffer::TextBuffer::line_count);
                let height = state.last_editor_content_area.map_or(20, |a| a.height as usize);
                state.viewport.scroll_down(3, total, height);
                state.viewport_follow_cursor = false;
                Control::Changed
            } else {
                Control::Continue
            }
        }
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

#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
fn handle_mouse_click(mouse: MouseEvent, state: &mut AppState) -> Control<AppEvent> {
    let (col, row) = (mouse.column, mouse.row);
    if let Some(tab_area) = state.last_root_tabs_area {
        if point_in_rect(col, row, tab_area) {
            if let Some(tab) = root_tab_hit_test(col, tab_area) {
                state.set_root_tab(tab);
                return Control::Changed;
            }
        }
    }

    if state.root_tab != RootTab::Editor {
        return Control::Continue;
    }

    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .active_buffer
            .is_some_and(|id| state.gutter_marks.contains_key(&id));
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
        if layout::is_on_bottom_border(splits, row) {
            state.dragging_border = Some(DragBorder::Bottom);
            return Control::Continue;
        }

        if let Some(left_area) = splits.left {
            if point_in_rect(col, row, left_area) {
                state.focus.focus(PanelId::FileTree);
                let now = Instant::now();
                let is_double = state.last_click.is_some_and(|(t, c, r)| {
                    c == col && r == row && now.duration_since(t).as_millis() < 500
                });
                if let Some(idx) = state.file_tree.hit_test(row, left_area) {
                    state.file_tree.selected = idx;
                    if is_double {
                        state.last_click = None;
                        return handle_file_tree_enter(state);
                    }
                }
                state.last_click = Some((now, col, row));
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
                    state.active_buffer = Some(bid);
                }
                return Control::Changed;
            }
        }
    }

    if let Some(content_area) = state.last_editor_content_area {
        let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
        let has_git = state
            .active_buffer
            .is_some_and(|id| state.gutter_marks.contains_key(&id));
        if let Some(pos) = editor_pane::click_to_position(
            col,
            row,
            content_area,
            &state.viewport,
            total_lines,
            has_git,
        ) {
            state.focus.set_active(PanelId::Editor);

            let now = Instant::now();
            let is_double = state.last_click.is_some_and(|(t, c, r)| {
                c == col && r == row && now.duration_since(t).as_millis() < 400
            });

            if let Some(buf) = state.active_buf_mut() {
                let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
                let clamped_col = pos.col.min(buf.line_len_no_newline(clamped_line));
                let clamped = Position::new(clamped_line, clamped_col);

                buf.cursor = CursorState::at(clamped);
                if is_double {
                    buf.move_word_left(false);
                    buf.move_word_right(true);
                    state.last_click = None;
                } else {
                    state.last_click = Some((now, col, row));
                }
            }
            state.viewport_follow_cursor = true;
            return Control::Changed;
        }
    }

    Control::Continue
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
    let Some(content_area) = state.last_editor_content_area else {
        return Control::Continue;
    };
    let total_lines = state.active_buf().map_or(0, TextBuffer::line_count);
    let has_git = state
        .active_buffer
        .is_some_and(|id| state.gutter_marks.contains_key(&id));
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
    if let Some(buf) = state.active_buf_mut() {
        let clamped_line = pos.line.min(buf.line_count().saturating_sub(1));
        let clamped = Position::new(clamped_line, pos.col.min(buf.line_len_no_newline(clamped_line)));
        let anchor = buf.cursor.primary.anchor;
        buf.cursor.primary = Selection {
            anchor,
            head: clamped,
        };
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
        DragBorder::Bottom => {
            let total_height = splits.status.y + splits.status.height;
            if total_height > 0 {
                let bottom_pct = ((u32::from(total_height.saturating_sub(mouse.row))) * 100
                    / u32::from(total_height)) as u16;
                state.layout.set_bottom_panel_height_pct(bottom_pct);
            }
        }
        DragBorder::Scrollbar => return Control::Continue,
    }

    Control::Changed
}

pub(super) fn handle_panel_command(
    cmd: &AppCommand,
    state: &mut AppState,
) -> Control<AppEvent> {
    match cmd {
        AppCommand::ToggleFileTree => {
            state.layout.toggle_file_tree();
            if state.layout.show_file_tree {
                state.focus.focus(PanelId::FileTree);
            } else {
                state.focus.set_active(PanelId::Editor);
            }
            state
                .effects
                .start_panel_transition(PanelId::FileTree, state.theme.accent);
            Control::Changed
        }
        AppCommand::ToggleTerminal => {
            state.layout.show_bottom_panel = false;
            state.overlay.notify(
                "PTY panel is temporarily removed from the UI",
                NotificationLevel::Info,
            );
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
            state
                .effects
                .start_panel_transition(PanelId::GitPanel, state.theme.accent);
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
