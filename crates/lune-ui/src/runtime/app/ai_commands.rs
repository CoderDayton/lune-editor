#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_ai_command(
    cmd: &AppCommand,
    state: &mut AppState,
) -> Option<Control<AppEvent>> {
    let control = match cmd {
        AppCommand::AiAskSelection => handle_ai_ask_selection(state),
        AppCommand::AiRefactorFile => handle_ai_refactor_file(state),
        AppCommand::AiSummarizeChanges => handle_ai_summarize_changes(state),
        AppCommand::AiOpenClientPicker => {
            if state.root_tab == RootTab::Agents {
                begin_agent_split_session(state, None)
            } else {
                state.overlay.open_ai_client_picker();
                Control::Changed
            }
        }
        AppCommand::AiNewSession(kind) => handle_ai_new_session(kind.clone(), state),
        AppCommand::AiCloseSession => {
            if let Some(id) = state.ai_manager.active_id() {
                state.ai_manager.close_session(id);
                prune_orphaned_agent_panes(state);
                if state.ai_manager.is_empty() {
                    state.focus.set_active(PanelId::Editor);
                }
            }
            Control::Changed
        }
        AppCommand::AiNextSession => {
            cycle_ai_session(1, state);
            Control::Changed
        }
        AppCommand::AiPrevSession => {
            cycle_ai_session(-1, state);
            Control::Changed
        }
        _ => return None,
    };
    Some(control)
}

fn cycle_ai_session(direction: isize, state: &mut AppState) {
    let ids: Vec<_> = state
        .ai_manager
        .session_list()
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    let Some(active) = state.ai_manager.active_id() else {
        return;
    };
    let Some(pos) = ids.iter().position(|&id| id == active) else {
        return;
    };
    let len = ids.len();
    if len == 0 {
        return;
    }
    let next = match direction {
        d if d < 0 => (pos + len - 1) % len,
        _ => (pos + 1) % len,
    };
    state.ai_manager.switch_session(ids[next]);
}

fn ai_client_from_settings(state: &AppState) -> AiClientKind {
    let cmd = state
        .cached_settings
        .as_ref()
        .map_or("claude", |s| s.ai.default_client.as_str());
    match cmd {
        "claude" => AiClientKind::ClaudeCode,
        other => AiClientKind::Custom {
            name: other.to_string(),
            command: other.to_string(),
        },
    }
}

fn start_ai_session_with_context(state: &mut AppState) {
    let kind = ai_client_from_settings(state);
    let ctx = state.collect_editor_context();
    let env = ctx.to_env_vars();
    let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
    let size = AiTermSize::default();
    let client_name = kind.display_name().to_string();
    match state
        .ai_manager
        .new_session(kind, cwd.as_deref(), &env, size)
    {
        Ok(_id) => {
            log::info!("Started AI session ({client_name}) with editor context");
        }
        Err(e) => {
            log::error!("Failed to start AI session: {e}");
            state.overlay.notify(
                format!("Failed to launch {client_name}: {e}"),
                NotificationLevel::Error,
            );
        }
    }
}

pub(super) fn handle_ai_new_session(kind: AiClientKind, state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let env = ctx.to_env_vars();
    let cwd = state.workspace.as_ref().map(|ws| ws.root().to_path_buf());
    let size = state
        .agents_tab_pending_pane
        .and_then(|pane_id| agent_pane_term_size(pane_id, state))
        .unwrap_or_default();
    let client_name = kind.display_name().to_string();
    match state
        .ai_manager
        .new_session(kind, cwd.as_deref(), &env, size)
    {
        Ok(session_id) => {
            log::info!("Started AI session: {client_name}");
            if let Some(pane_id) = state.agents_tab_pending_pane.take() {
                state
                    .agents_tab
                    .register_pane(pane_id, session_id, client_name);
                refresh_active_saved_layout(state);
            }
        }
        Err(e) => {
            log::warn!("AI session launch failed ({client_name}): {e}");
            if let Some(pane_id) = state.agents_tab_pending_pane.take() {
                state.agents_tab.discard_pane(pane_id);
                refresh_active_saved_layout(state);
            }
        }
    }
    Control::Changed
}

fn ensure_ai_session(state: &mut AppState) {
    if state.ai_manager.is_empty() {
        start_ai_session_with_context(state);
    }
}

fn send_prompt_to_ai(state: &mut AppState, prompt: &str) {
    if let Some(session) = state.ai_manager.active_session_mut() {
        if let Err(e) = session.send_input(prompt.as_bytes()) {
            log::error!("Failed to send prompt to AI: {e}");
        }
        if let Err(e) = session.send_input(b"\n") {
            log::error!("Failed to send newline to AI: {e}");
        }
    }
}

pub(super) fn handle_ai_ask_selection(state: &mut AppState) -> Control<AppEvent> {
    ensure_ai_session(state);
    Control::Changed
}

pub(super) fn handle_ai_refactor_file(state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let file_path = ctx
        .active_file
        .as_ref()
        .map(|f| f.path.display().to_string())
        .unwrap_or_default();

    if file_path.is_empty() {
        state.overlay.notify(
            "No file open — open a file first",
            NotificationLevel::Warning,
        );
        return Control::Changed;
    }

    ensure_ai_session(state);
    let prompt = format!("Refactor {file_path}");
    send_prompt_to_ai(state, &prompt);
    Control::Changed
}

pub(super) fn handle_ai_summarize_changes(state: &mut AppState) -> Control<AppEvent> {
    let ctx = state.collect_editor_context();
    let summary = ctx
        .git_status
        .as_ref()
        .map(|g| {
            use std::fmt::Write as _;
            let mut s = format!("Branch: {}\nModified files:\n", g.branch);
            for f in &g.modified_files {
                let _ = writeln!(s, "  - {}", f.display());
            }
            s
        })
        .unwrap_or_default();

    if summary.is_empty() {
        state.overlay.notify(
            "No git repository — open a workspace first",
            NotificationLevel::Warning,
        );
        return Control::Changed;
    }

    ensure_ai_session(state);
    let prompt = format!("Summarize these changes:\n{summary}");
    send_prompt_to_ai(state, &prompt);
    Control::Changed
}
