#![allow(clippy::wildcard_imports)]

use super::*;
use lune_core::settings::CursorStyle;

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

    let mut splits = layout::compute_layout(area, &state.layout);
    // Drop the file-tree frame by the same offset the editor frame uses
    // (tab strip + gap) so the two panes' top and bottom borders align.
    if let Some(left) = splits.left.as_mut() {
        let shift = EDITOR_FRAME_TOP_OFFSET.min(left.height);
        left.y += shift;
        left.height -= shift;
    }
    state.last_splits = Some(splits.clone());

    if let Some(left_area) = splits.left {
        // Uppercase the workspace name so the panel title reads like a
        // header (e.g. "LUNE-EDITOR"), matching the "EXPLORER" fallback.
        let ws_name = state
            .workspace
            .as_ref()
            .map_or_else(|| "EXPLORER".to_string(), |w| w.name().to_uppercase());
        let ft_focused = state.focus.is_focused(PanelId::FileTree);
        file_tree::render_file_tree(
            left_area,
            buf,
            &mut state.file_tree,
            &ws_name,
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
    if status_state.ai_busy || status_state.git_busy {
        state.throbber_state.calc_next();
    }
    status_bar::render_status_bar(
        splits.status,
        buf,
        &status_state,
        &state.theme,
        &mut state.throbber_state,
    );
}

/// Rows reserved above the editor frame inside the center column: the
/// tab strip plus an optional gap of `EDITOR_FRAME_TOP_OFFSET - 1` rows.
/// The file-tree frame is dropped by the same amount in
/// `render_editor_tab` so the two panes' top borders stay aligned.
const EDITOR_FRAME_TOP_OFFSET: u16 = 1;

fn render_center(area: Rect, buf: &mut Buffer, state: &mut AppState, is_focused: bool) {
    if area.height <= EDITOR_FRAME_TOP_OFFSET {
        return;
    }

    // The tab strip sits *above* the editor frame.  Split the center
    // column into the tab row, an optional gap (`EDITOR_FRAME_TOP_OFFSET
    // - 1` rows), and the bordered editor box below; the box keeps all
    // four borders so the editor reads as its own pane (the file_tree's
    // right `│` and this box's left `│` sit in adjacent columns — a
    // deliberate `││` seam).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(EDITOR_FRAME_TOP_OFFSET - 1),
            Constraint::Min(0),
        ])
        .split(area);

    let tab_area = outer[0];
    let editor_box = outer[2];

    // Tabs sit in row 0; `outer[1]` is the optional gap above the frame.
    tab_bar::render_tab_bar(tab_area, buf, &state.tab_mgr, is_focused, &state.theme);

    let block = crate::widgets::panel::panel_block(&state.theme, is_focused, Borders::ALL);
    let content_area = block.inner(editor_box);
    block.render(editor_box, buf);

    if content_area.height < 1 {
        return;
    }

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

    // Snapshot recent-files paths BEFORE the `&mut state.highlighters`
    // borrow below; reconcile the welcome selection cursor against the
    // current list length and hide it when the editor is unfocused.
    let recent_paths = state.recent_paths();
    state.welcome_nav.clamp(recent_paths.len());
    let welcome_selected = if recent_paths.is_empty() || !is_focused {
        None
    } else {
        Some(state.welcome_nav.selected)
    };

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
    let tab_size = state
        .cached_settings
        .as_ref()
        .map_or(4, |s| s.editor.tab_size);
    // In vim mode the cursor shape tracks the active mode (block in
    // Normal/Visual/Command, underline in Insert); otherwise it follows
    // the configured `editor.cursor_style` (default block).
    let cursor_style = if state.vim_enabled {
        match state.vim.mode {
            VimMode::Insert => CursorStyle::Underline,
            _ => CursorStyle::Block,
        }
    } else {
        state
            .cached_settings
            .as_ref()
            .map_or(CursorStyle::Block, |s| s.editor.cursor_style)
    };
    let welcome = editor_pane::WelcomeInfo {
        recent_files: &recent_paths,
        selected: welcome_selected,
    };
    editor_pane::render_editor_pane(
        content_area,
        buf,
        text_buf,
        &mut state.viewport,
        state.viewport_follow_cursor,
        cursor_style,
        highlighted,
        &state.syntax_theme,
        active_gutter,
        search_state,
        &state.theme,
        tab_size,
        Some(&welcome),
    );
    state.last_editor_content_area = Some(content_area);
}
