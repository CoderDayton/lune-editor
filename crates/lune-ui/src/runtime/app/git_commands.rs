#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn handle_git_command(
    cmd: &AppCommand,
    state: &mut AppState,
) -> Option<Control<AppEvent>> {
    let control = match cmd {
        AppCommand::GitStage => handle_git_file_op(state, GitService::stage, "Staged"),
        AppCommand::GitUnstage => handle_git_file_op(state, GitService::unstage, "Unstaged"),
        AppCommand::GitCommit => handle_git_commit(state),
        AppCommand::GitDiscard => handle_git_discard(state),
        AppCommand::GitRefresh => {
            state.refresh_git();
            Control::Changed
        }
        AppCommand::GitDiscardConfirmed(path) => handle_git_discard_confirmed(path, state),
        AppCommand::GitCommitConfirmed(msg) => handle_git_commit_confirmed(msg, state),
        AppCommand::GitStageHunk => handle_git_hunk_op(state, "stage"),
        AppCommand::GitUnstageHunk => handle_git_hunk_op(state, "unstage"),
        AppCommand::GitDiscardHunk => handle_git_hunk_op(state, "discard"),
        _ => return None,
    };
    Some(control)
}

fn handle_git_file_op(
    state: &mut AppState,
    op: fn(&GitService, &Path) -> anyhow::Result<()>,
    label: &str,
) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };
    let Some(git) = state.open_git_service() else {
        return Control::Changed;
    };
    match op(&git, &file.path) {
        Ok(()) => {
            state.status_message = format!("{label}: {}", file.path.display());
            state.refresh_git();
        }
        Err(e) => {
            state.status_message = format!("{label} failed: {e}");
            state
                .overlay
                .notify(format!("{label} failed: {e}"), NotificationLevel::Error);
        }
    }
    Control::Changed
}

fn handle_git_commit(state: &mut AppState) -> Control<AppEvent> {
    let has_staged = state
        .git_panel
        .status
        .as_ref()
        .is_some_and(|s| s.files.iter().any(|f| f.staged));

    if !has_staged {
        state
            .overlay
            .notify("Nothing staged to commit", NotificationLevel::Info);
        return Control::Changed;
    }

    let staged_count = state
        .git_panel
        .status
        .as_ref()
        .map_or(0, |s| s.files.iter().filter(|f| f.staged).count());

    let dialog = overlay::InputDialogState::new(
        format!("Commit ({staged_count} staged)"),
        "Enter commit message…",
        overlay::InputDialogAction::CommitMessage,
    );
    state.overlay.open_input_dialog(dialog);
    state.focus.focus(PanelId::CommandPalette);
    Control::Changed
}

fn handle_git_commit_confirmed(message: &str, state: &mut AppState) -> Control<AppEvent> {
    let Some(git) = state.open_git_service() else {
        state
            .overlay
            .notify("No git repository", NotificationLevel::Error);
        return Control::Changed;
    };
    match git.commit(message) {
        Ok(oid) => {
            let hex = oid.to_string();
            let short = hex.get(..7).unwrap_or(&hex);
            state.status_message = format!("Committed {short}");
            state
                .overlay
                .notify(format!("[{short}] {message}"), NotificationLevel::Info);
            state.refresh_git();
        }
        Err(e) => {
            state.status_message = format!("Commit failed: {e}");
            state
                .overlay
                .notify(format!("Commit failed: {e}"), NotificationLevel::Error);
        }
    }
    Control::Changed
}

fn handle_git_discard(state: &mut AppState) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };

    state.overlay.open_confirm(
        format!("Discard changes to {}?", file.path.display()),
        AppCommand::GitDiscardConfirmed(file.path),
    );
    state.focus.focus(PanelId::CommandPalette);
    Control::Changed
}

fn handle_git_discard_confirmed(path: &Path, state: &mut AppState) -> Control<AppEvent> {
    let Some(git) = state.open_git_service() else {
        return Control::Changed;
    };
    match git.discard_file(path) {
        Ok(()) => {
            state.status_message = format!("Discarded: {}", path.display());
            state.refresh_git();
        }
        Err(e) => {
            state.status_message = format!("Discard failed: {e}");
            state
                .overlay
                .notify(format!("Discard failed: {e}"), NotificationLevel::Error);
        }
    }
    Control::Changed
}

fn handle_git_hunk_op(state: &mut AppState, op: &str) -> Control<AppEvent> {
    let Some((path, hunk)) = state.git_panel.diff_view.current_hunk_data() else {
        return Control::Continue;
    };
    let path = path.to_path_buf();
    let hunk = hunk.clone();
    let Some(git) = state.open_git_service() else {
        return Control::Continue;
    };
    let result = match op {
        "stage" => git.stage_hunk(&path, &hunk),
        "unstage" => git.unstage_hunk(&path, &hunk),
        "discard" => git.discard_hunk(&path, &hunk),
        _ => return Control::Continue,
    };
    match result {
        Ok(()) => {
            state.status_message = format!("{op} hunk: {}", path.display());
            state.refresh_git();
            // Re-fetch the diff through a fresh service handle so the
            // diff panel stays in sync with the new index/workdir state.
            if let Some(git) = state.open_git_service() {
                match git.diff_file(&path) {
                    Ok(Some(diff)) => state.git_panel.diff_view.set_diff(diff),
                    Ok(None) => state.git_panel.diff_view.clear(),
                    Err(e) => log::error!("Failed to re-fetch diff: {e}"),
                }
            }
        }
        Err(e) => {
            state.status_message = format!("{op} hunk failed: {e}");
            state
                .overlay
                .notify(format!("{op} hunk failed: {e}"), NotificationLevel::Error);
        }
    }
    Control::Changed
}
