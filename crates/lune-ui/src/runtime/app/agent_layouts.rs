#![allow(clippy::wildcard_imports)]

use super::*;
use crate::runtime::tiling;

fn build_agent_layout_picker_entries(state: &AppState) -> Vec<overlay::LayoutPickerEntry> {
    let mut entries: Vec<_> = tiling::PRESET_LIST
        .iter()
        .enumerate()
        .map(|(idx, info)| overlay::LayoutPickerEntry {
            label: info.name.to_string(),
            pane_count: info.pane_count,
            kind: overlay::LayoutPickerEntryKind::Preset(idx),
        })
        .collect();

    entries.extend(
        state
            .saved_agent_layouts
            .iter()
            .enumerate()
            .map(|(idx, layout)| overlay::LayoutPickerEntry {
                label: layout.name.clone(),
                pane_count: layout.pane_count(),
                kind: overlay::LayoutPickerEntryKind::Saved(idx),
            }),
    );

    entries
}

pub(super) fn agent_pane_term_size(
    pane_id: tiling::PaneId,
    state: &AppState,
) -> Option<AiTermSize> {
    if let (Some(area), Some(layout)) = (
        state.last_agents_content_area,
        state.agents_tab.layout.as_ref(),
    ) {
        if let Some((_, rect)) = layout
            .compute_rects(area)
            .into_iter()
            .find(|(id, _)| *id == pane_id)
        {
            return Some(AiTermSize::new(rect.height.max(1), rect.width.max(1)));
        }
    }

    state
        .last_agent_pane_rects
        .iter()
        .find(|(id, _)| *id == pane_id)
        .map(|(_, rect)| AiTermSize::new(rect.height.max(1), rect.width.max(1)))
}

fn agent_pane_term_sizes(state: &AppState) -> FxHashMap<tiling::PaneId, AiTermSize> {
    if let (Some(area), Some(layout)) = (
        state.last_agents_content_area,
        state.agents_tab.layout.as_ref(),
    ) {
        return layout
            .compute_rects(area)
            .into_iter()
            .map(|(pane_id, rect)| {
                (
                    pane_id,
                    AiTermSize::new(rect.height.max(1), rect.width.max(1)),
                )
            })
            .collect();
    }

    state
        .last_agent_pane_rects
        .iter()
        .map(|(pane_id, rect)| {
            (
                *pane_id,
                AiTermSize::new(rect.height.max(1), rect.width.max(1)),
            )
        })
        .collect()
}

pub(super) fn open_agent_layout_picker(state: &mut AppState) {
    open_agent_layout_picker_with_selection(None, state);
}

pub(super) fn open_agent_layout_picker_with_selection(
    selected: Option<usize>,
    state: &mut AppState,
) {
    let entries = build_agent_layout_picker_entries(state);
    state.overlay.open_layout_picker(entries);
    if let Some(selected) = selected {
        state.overlay.layout_picker.select(selected);
    }
    state.focus.focus(PanelId::CommandPalette);
}

pub(super) fn apply_agent_layout_entry(entry: &overlay::LayoutPickerEntry, state: &mut AppState) {
    let (new_pane_ids, closed_sessions) = match entry.kind {
        overlay::LayoutPickerEntryKind::Preset(idx) => state.agents_tab.apply_preset(idx),
        overlay::LayoutPickerEntryKind::Saved(idx) => {
            let Some(saved) = state.saved_agent_layouts.get(idx).cloned() else {
                state
                    .overlay
                    .notify("Saved layout not found", NotificationLevel::Warning);
                return;
            };
            state.agents_tab.apply_saved_layout(&saved)
        }
    };

    for sid in closed_sessions {
        state.ai_manager.close_session(sid);
    }
    spawn_shell_sessions_for_agent_panes(&new_pane_ids, state);
    state.set_root_tab(RootTab::Agents);
    sync_agent_layout_geometry(state);
}

fn spawn_shell_sessions_for_agent_panes(pane_ids: &[tiling::PaneId], state: &mut AppState) {
    let pane_sizes = agent_pane_term_sizes(state);

    for &pane_id in pane_ids {
        let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
        let size = pane_sizes
            .get(&pane_id)
            .copied()
            .unwrap_or_else(|| AiTermSize::new(24, 80));
        let env = std::collections::HashMap::new();
        match state
            .ai_manager
            .new_session(AiClientKind::Shell, cwd.as_deref(), &env, size)
        {
            Ok(session_id) => {
                state
                    .agents_tab
                    .register_pane(pane_id, session_id, "Shell".to_string());
            }
            Err(e) => {
                log::warn!("Failed to spawn Shell for layout pane: {e}");
                state.agents_tab.discard_pane(pane_id);
            }
        }
    }
}

pub(super) fn sync_agent_layout_geometry(state: &mut AppState) {
    let Some(content_area) = state.last_agents_content_area else {
        return;
    };

    let pane_rects = if state.agents_tab.zoomed {
        state
            .agents_tab
            .focused
            .map(|pane_id| vec![(pane_id, content_area)])
            .unwrap_or_default()
    } else {
        let Some(layout) = state.agents_tab.layout.as_ref() else {
            state.last_agent_pane_rects.clear();
            return;
        };
        layout.compute_rects(content_area)
    };

    state.last_agent_pane_rects.clear();
    state.last_agent_pane_rects.extend(
        pane_rects
            .iter()
            .copied()
            .filter(|(_, rect)| rect.width > 0 && rect.height > 0),
    );

    let resize_list: Vec<_> = pane_rects
        .iter()
        .filter_map(|(pane_id, pane_area)| {
            if pane_area.width == 0 || pane_area.height == 0 {
                return None;
            }
            let session_id = state.agents_tab.panes.get(pane_id)?.session_id;
            Some((session_id, *pane_area))
        })
        .collect();

    for (session_id, pane_area) in resize_list {
        if let Some(session) = state.ai_manager.session_mut(session_id) {
            sync_agent_session_size(session, pane_area);
        }
    }
}

pub(super) fn open_save_agent_layout_dialog(state: &mut AppState) {
    let suggested = terminal_layouts::suggest_layout_name(&state.saved_agent_layouts);
    state.overlay.open_input_dialog(
        overlay::InputDialogState::new(
            "Save Agent Layout",
            "layout name",
            overlay::InputDialogAction::SaveAgentLayout,
        )
        .with_input(suggested),
    );
    state.focus.focus(PanelId::CommandPalette);
}

pub(super) fn open_rename_agent_layout_dialog(index: usize, state: &mut AppState) {
    let Some(layout) = state.saved_agent_layouts.get(index) else {
        state
            .overlay
            .notify("Saved layout not found", NotificationLevel::Warning);
        return;
    };

    state.overlay.open_input_dialog(
        overlay::InputDialogState::new(
            "Rename Agent Layout",
            "layout name",
            overlay::InputDialogAction::RenameAgentLayout { index },
        )
        .with_input(layout.name.clone()),
    );
    state.focus.focus(PanelId::CommandPalette);
}

pub(super) fn confirm_delete_agent_layout(index: usize, state: &mut AppState) {
    let Some(layout) = state.saved_agent_layouts.get(index) else {
        state
            .overlay
            .notify("Saved layout not found", NotificationLevel::Warning);
        return;
    };

    let name = layout.name.clone();
    close_overlay(state);
    state.overlay.open_confirm(
        format!("Delete saved layout \"{name}\"?"),
        AppCommand::AgentDeleteSavedLayout(index),
    );
    state.focus.focus(PanelId::CommandPalette);
}

fn layout_picker_page_size(state: &AppState) -> usize {
    state
        .last_root_tabs_area
        .map_or(8, |area| usize::from(area.height.saturating_sub(4)).max(1))
}

pub(super) fn handle_layout_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    match key.code {
        KeyCode::Esc => {
            close_overlay(state);
            Control::Changed
        }
        KeyCode::Enter => {
            let selected = state.overlay.layout_picker.selected_entry().cloned();
            close_overlay(state);
            if let Some(entry) = selected {
                apply_agent_layout_entry(&entry, state);
            }
            Control::Changed
        }
        KeyCode::Up => {
            state.overlay.layout_picker.select_prev();
            Control::Changed
        }
        KeyCode::Down => {
            state.overlay.layout_picker.select_next();
            Control::Changed
        }
        KeyCode::Home => {
            state.overlay.layout_picker.select_first();
            Control::Changed
        }
        KeyCode::End => {
            state.overlay.layout_picker.select_last();
            Control::Changed
        }
        KeyCode::PageUp => {
            state
                .overlay
                .layout_picker
                .move_page(-1, layout_picker_page_size(state));
            Control::Changed
        }
        KeyCode::PageDown => {
            state
                .overlay
                .layout_picker
                .move_page(1, layout_picker_page_size(state));
            Control::Changed
        }
        KeyCode::Char('r' | 'R') => {
            let selected = state.overlay.layout_picker.selected_entry().cloned();
            if let Some(overlay::LayoutPickerEntry {
                kind: overlay::LayoutPickerEntryKind::Saved(index),
                ..
            }) = selected
            {
                open_rename_agent_layout_dialog(index, state);
            } else {
                state.overlay.notify(
                    "Preset layouts cannot be renamed",
                    NotificationLevel::Warning,
                );
            }
            Control::Changed
        }
        KeyCode::Delete | KeyCode::Char('d' | 'D') => {
            let selected = state.overlay.layout_picker.selected_entry().cloned();
            if let Some(overlay::LayoutPickerEntry {
                kind: overlay::LayoutPickerEntryKind::Saved(index),
                ..
            }) = selected
            {
                confirm_delete_agent_layout(index, state);
            } else {
                state.overlay.notify(
                    "Preset layouts cannot be deleted",
                    NotificationLevel::Warning,
                );
            }
            Control::Changed
        }
        KeyCode::Char('s' | 'S') => {
            if state.agents_tab.layout.is_some() {
                close_overlay(state);
                open_save_agent_layout_dialog(state);
            } else {
                state
                    .overlay
                    .notify("No agent layout to save yet", NotificationLevel::Warning);
            }
            Control::Changed
        }
        _ => Control::Continue,
    }
}
