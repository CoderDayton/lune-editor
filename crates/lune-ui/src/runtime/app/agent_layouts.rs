#![allow(clippy::wildcard_imports)]

use super::*;
use crate::runtime::tiling;

fn preset_saved_tree(preset_idx: usize, pane_count: usize) -> tiling::SavedTileNode {
    let ids: Vec<tiling::PaneId> = (0..pane_count)
        .map(|i| tiling::PaneId(u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    tiling::build_preset_layout(preset_idx, &ids)
        .map_or(tiling::SavedTileNode::Leaf, |node| node.to_saved())
}

fn build_agent_layout_picker_entries(state: &AppState) -> Vec<overlay::LayoutPickerEntry> {
    let active = current_matching_saved_layout_name(state);
    let active = active.as_deref();

    let mut entries: Vec<_> = tiling::PRESET_LIST
        .iter()
        .enumerate()
        .map(|(idx, info)| overlay::LayoutPickerEntry {
            label: info.name.to_string(),
            pane_count: info.pane_count,
            kind: overlay::LayoutPickerEntryKind::Preset(idx),
            preview: preset_saved_tree(idx, info.pane_count),
            is_active: false,
        })
        .collect();

    entries.extend(
        state
            .saved_agent_layouts
            .iter()
            .enumerate()
            .map(|(idx, layout)| {
                let normalized =
                    crate::runtime::terminal_layouts::normalize_layout_name(&layout.name);
                overlay::LayoutPickerEntry {
                    label: layout.name.clone(),
                    pane_count: layout.pane_count(),
                    kind: overlay::LayoutPickerEntryKind::Saved(idx),
                    preview: layout.root.clone(),
                    is_active: active.is_some_and(|a| a == normalized.as_str()),
                }
            }),
    );

    entries
}

fn current_pane_kinds(state: &AppState) -> Vec<Option<tiling::SavedPaneKind>> {
    state
        .agents_tab
        .layout
        .as_ref()
        .map(|layout| {
            layout
                .pane_ids()
                .into_iter()
                .map(|pane_id| {
                    let pane = state.agents_tab.panes.get(&pane_id)?;
                    let session = state.ai_manager.session(pane.session_id)?;
                    Some(tiling::SavedPaneKind::from(session.kind()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn saved_layout_matches_current(
    saved: &tiling::SavedAgentLayout,
    current_root: &tiling::SavedTileNode,
    current_kinds: &[Option<tiling::SavedPaneKind>],
) -> bool {
    if saved.root != *current_root {
        return false;
    }

    if saved.pane_kinds.is_empty() {
        return true;
    }

    saved.pane_kinds.len() == current_kinds.len()
        && saved
            .pane_kinds
            .iter()
            .zip(current_kinds)
            .all(|(saved_kind, current_kind)| {
                current_kind.as_ref()
                    == Some(&saved_kind.clone().unwrap_or(tiling::SavedPaneKind::Shell))
            })
}

pub(super) fn current_matching_saved_layout_name(state: &AppState) -> Option<String> {
    let layout = state.agents_tab.layout.as_ref()?;
    let current_root = layout.to_saved();
    let current_kinds = current_pane_kinds(state);

    state
        .saved_agent_layouts
        .iter()
        .find(|saved| saved_layout_matches_current(saved, &current_root, &current_kinds))
        .map(|saved| crate::runtime::terminal_layouts::normalize_layout_name(&saved.name))
}

pub(super) fn refresh_active_saved_layout(state: &mut AppState) {
    state.agents_tab.active_saved_layout = current_matching_saved_layout_name(state);
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
    let previous_filter = matches!(
        state.overlay.active,
        Some(overlay::OverlayKind::LayoutPicker)
    )
    .then(|| state.overlay.layout_picker.filter.clone());
    let entries = build_agent_layout_picker_entries(state);
    state.overlay.open_layout_picker(entries);
    if let Some(filter) = previous_filter {
        state.overlay.layout_picker.set_filter(filter);
    }
    if let Some(selected) = selected {
        state.overlay.layout_picker.select_entry_index(selected);
    }
    state.focus.focus(PanelId::CommandPalette);
}

pub(super) fn apply_agent_layout_entry(entry: &overlay::LayoutPickerEntry, state: &mut AppState) {
    let (new_pane_ids, closed_sessions, kinds_for_new) = match entry.kind {
        overlay::LayoutPickerEntryKind::Preset(idx) => {
            let (new, closed) = state.agents_tab.apply_preset(idx);
            let kinds = vec![None; new.len()];
            (new, closed, kinds)
        }
        overlay::LayoutPickerEntryKind::Saved(idx) => {
            let Some(saved) = state.saved_agent_layouts.get(idx).cloned() else {
                state
                    .overlay
                    .notify("Saved layout not found", NotificationLevel::Warning);
                return;
            };
            // Record how many panes the tab had BEFORE applying, so we can
            // line up the pane_kinds slice with only the newly-spawned panes.
            let existing_count = state
                .agents_tab
                .layout
                .as_ref()
                .map_or(0, tiling::TileNode::pane_count);
            let (new, closed) = state.agents_tab.apply_saved_layout(&saved);
            let kinds: Vec<_> = (existing_count..existing_count + new.len())
                .map(|pos| saved.pane_kinds.get(pos).cloned().flatten())
                .collect();
            (new, closed, kinds)
        }
    };

    for sid in closed_sessions {
        state.ai_manager.close_session(sid);
    }
    spawn_sessions_for_agent_panes(&new_pane_ids, &kinds_for_new, state);
    refresh_active_saved_layout(state);
    state.set_root_tab(RootTab::Agents);
    sync_agent_layout_geometry(state);
}

fn spawn_sessions_for_agent_panes(
    pane_ids: &[tiling::PaneId],
    kinds: &[Option<tiling::SavedPaneKind>],
    state: &mut AppState,
) {
    let pane_sizes = agent_pane_term_sizes(state);

    for (i, &pane_id) in pane_ids.iter().enumerate() {
        let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
        let size = pane_sizes
            .get(&pane_id)
            .copied()
            .unwrap_or_else(|| AiTermSize::new(24, 80));
        let env = std::collections::HashMap::new();
        let client_kind = kinds
            .get(i)
            .cloned()
            .flatten()
            .map_or(AiClientKind::Shell, |saved| saved.to_client_kind());
        let display_name = client_kind.display_name().to_string();
        match state
            .ai_manager
            .new_session(client_kind, cwd.as_deref(), &env, size)
        {
            Ok(session_id) => {
                state
                    .agents_tab
                    .register_pane(pane_id, session_id, display_name);
            }
            Err(e) => {
                log::warn!("Failed to spawn {display_name} for layout pane: {e}");
                state.agents_tab.discard_pane(pane_id);
            }
        }
    }

    refresh_active_saved_layout(state);
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

/// Move the currently-selected saved layout up or down. Presets cannot be
/// reordered. Persists the new order and reopens the picker so the visible
/// selection follows the moved entry.
fn reorder_selected_saved_layout(state: &mut AppState, delta: isize) -> Control<AppEvent> {
    let Some(overlay::LayoutPickerEntry {
        kind: overlay::LayoutPickerEntryKind::Saved(saved_index),
        ..
    }) = state.overlay.layout_picker.selected_entry().cloned()
    else {
        state.overlay.notify(
            "Preset layouts cannot be reordered",
            NotificationLevel::Warning,
        );
        return Control::Changed;
    };

    let Some(new_saved_index) =
        terminal_layouts::reorder_saved_layout(&mut state.saved_agent_layouts, saved_index, delta)
    else {
        return Control::Changed;
    };

    state.persist_saved_agent_layouts();
    // Picker row index = preset_count + saved_index.
    let preset_count = tiling::PRESET_LIST.len();
    open_agent_layout_picker_with_selection(Some(preset_count + new_saved_index), state);
    Control::Changed
}

pub(super) fn handle_layout_picker_key(key: &KeyEvent, state: &mut AppState) -> Control<AppEvent> {
    use crossterm::event::KeyModifiers;

    // Alt+Up / Alt+Down — reorder saved layouts.
    if matches!(key.code, KeyCode::Up | KeyCode::Down) && key.modifiers.contains(KeyModifiers::ALT)
    {
        let delta: isize = if key.code == KeyCode::Up { -1 } else { 1 };
        return reorder_selected_saved_layout(state, delta);
    }

    // Action keys (Ctrl-modified, so they don't collide with fuzzy-filter typing).
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('r' | 'R') => return trigger_rename_selected(state),
            KeyCode::Char('d' | 'D') => return trigger_delete_selected(state),
            KeyCode::Char('s' | 'S') => return trigger_save_as_from_picker(state),
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            if state.overlay.layout_picker.filter.is_empty() {
                close_overlay(state);
            } else {
                state.overlay.layout_picker.clear_filter();
            }
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
        KeyCode::Delete => trigger_delete_selected(state),
        KeyCode::Backspace => {
            state.overlay.layout_picker.pop_filter_char();
            Control::Changed
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            state.overlay.layout_picker.push_filter_char(ch);
            Control::Changed
        }
        _ => Control::Continue,
    }
}

fn trigger_rename_selected(state: &mut AppState) -> Control<AppEvent> {
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

fn trigger_delete_selected(state: &mut AppState) -> Control<AppEvent> {
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

fn trigger_save_as_from_picker(state: &mut AppState) -> Control<AppEvent> {
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
