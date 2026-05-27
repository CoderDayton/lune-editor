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

    // Wrap the entire center column (tab strip + editor) in one
    // focus-aware Block with TOP|BOTTOM borders.  This puts the
    // editor's top border on row 0 of the center column — the same
    // row that file_tree's and git_panel's top borders sit on — and
    // tucks the tab strip into the first inner row, just below the
    // top border.  Vertical borders are intentionally omitted so the
    // editor doesn't double the `│` columns drawn by its neighbors.
    let block = crate::widgets::panel::panel_block(
        &state.theme,
        is_focused,
        Borders::TOP | Borders::BOTTOM,
    );
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 1 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let tab_area = chunks[0];
    let content_area = chunks[1];

    tab_bar::render_tab_bar(tab_area, buf, &state.tab_mgr, is_focused, &state.theme);

    // Compute `highlighted` with an explicit `if let` instead of a
    // closure so the returned borrow flows cleanly out of the scope
    // the compiler can reason about (the
    // closure-returning-a-borrowed-slice-from-captured-mutable-state
    // pattern sometimes yields subtle lifetime oddities — this avoids
    // that entire class of footgun).
    // Compute the gutter snapshot BEFORE taking the &mut borrow on
    // `state.highlighters` below — the borrow checker can't prove that
    // `gutter_for_render` doesn't alias `highlighters`, even though it
    // only reads disjoint fields.
    let active_gutter_owned = state
        .session
        .active_buffer
        .and_then(|id| state.gutter_for_render(id));
    let active_gutter = active_gutter_owned.as_deref();

    // `content_area` is already the editor's exact draw region
    // (post-block, post-tab-strip), so its height is the true visible
    // viewport height.
    let viewport_height = content_area.height as usize;

    let highlighted: Option<&[HighlightedLine]> = if let Some(id) = state.session.active_buffer {
        // Lazy initial parse: `open_file` defers the whole-buffer
        // `Highlighter::update` to the first render so that opening a
        // large file doesn't block the UI thread.  Subsequent renders
        // hit the `primed_highlighters` fast path and return early.
        state.ensure_highlighter_primed(id);
        let top = state.viewport.top_line.saturating_sub(50);
        let end = state.viewport.top_line + viewport_height + 50;
        state
            .highlighters
            .get_mut(&id)
            .map(|hl| hl.highlight_lines(top..end))
    } else {
        None
    };

    let text_buf = state
        .session
        .active_buffer
        .and_then(|id| state.session.registry.get(id));

    let search_state = if matches!(
        state.overlay.active,
        Some(overlay::OverlayKind::FindReplace)
    ) && state.overlay.find_replace.search_state.has_matches()
    {
        Some(&state.overlay.find_replace.search_state)
    } else {
        None
    };

    // `content_area` is the editor's exact draw region.  Store it so
    // mouse hit-testing (scrollbar, click-to-position, drag
    // autoscroll, PageUp/Down sizing, wheel clamp) operates on the
    // same coordinates as the rendered content.
    editor_pane::render_editor_pane(
        content_area,
        buf,
        text_buf,
        &mut state.viewport,
        state.viewport_follow_cursor,
        state.vim.mode,
        highlighted,
        &state.syntax_theme,
        active_gutter,
        search_state,
        &state.theme,
    );
    state.last_editor_content_area = Some(content_area);
}
