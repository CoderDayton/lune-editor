#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn render_root_tabs(area: Rect, buf: &mut Buffer, state: &AppState) {
    let tabs = Tabs::new(ROOT_TAB_TITLES)
        .select(state.root_tab.as_index())
        .style(Style::new().fg(state.theme.fg_muted).bg(state.theme.bg))
        .highlight_style(state.theme.tab_active_focused)
        .divider(ROOT_TAB_DIVIDER)
        .padding("", "");
    tabs.render(area, buf);
}

pub(super) fn render_editor_tab(area: Rect, buf: &mut Buffer, state: &mut AppState) {
    state.last_agents_content_area = None;
    state.last_agent_pane_rects.clear();

    let splits = layout::compute_layout(area, &state.layout);
    state.last_splits = Some(splits.clone());

    if let Some(left_area) = splits.left {
        let ws_name = state.workspace.as_ref().map_or("EXPLORER", Workspace::name);
        let ft_focused = state.focus.is_focused(PanelId::FileTree);
        file_tree::render_file_tree(
            left_area,
            buf,
            &mut state.file_tree,
            ws_name,
            ft_focused,
            &state.theme,
        );
    }

    let editor_focused = state.focus.is_focused(PanelId::Editor);
    render_center(splits.center, buf, state, editor_focused);

    if let Some(right_area) = splits.right {
        if state.layout.show_git_panel {
            let gp_focused = state.focus.is_focused(PanelId::GitPanel);
            git_panel::render_git_panel(
                right_area,
                buf,
                &mut state.git_panel,
                gp_focused,
                &state.theme,
            );
        }
    }

    let status_state = state.build_status_line();
    status_bar::render_status_bar(splits.status, buf, &status_state, &state.theme);
}

fn render_center(area: Rect, buf: &mut Buffer, state: &mut AppState, is_focused: bool) {
    if area.height < 2 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let tab_area = chunks[0];
    let content_area = chunks[1];

    tab_bar::render_tab_bar(tab_area, buf, &state.tab_mgr, is_focused, &state.theme);
    state.last_editor_content_area = Some(content_area);

    let highlighted = state.active_buffer.and_then(|id| {
        let viewport_height = content_area.height as usize;
        let top = state.viewport.top_line.saturating_sub(50);
        let end = state.viewport.top_line + viewport_height + 50;
        state
            .highlighters
            .get_mut(&id)
            .map(|hl| hl.highlight_lines(top..end))
    });

    let text_buf = state.active_buffer.and_then(|id| state.registry.get(id));
    let active_gutter = state
        .active_buffer
        .and_then(|id| state.gutter_marks.get(&id));

    let search_state = if matches!(
        state.overlay.active,
        Some(overlay::OverlayKind::FindReplace)
    ) && state.overlay.find_replace.search_state.has_matches()
    {
        Some(&state.overlay.find_replace.search_state)
    } else {
        None
    };

    editor_pane::render_editor_pane(
        content_area,
        buf,
        text_buf,
        &mut state.viewport,
        state.viewport_follow_cursor,
        state.vim.mode,
        highlighted.as_deref(),
        &state.syntax_theme,
        active_gutter,
        search_state,
        &state.theme,
    );
}
