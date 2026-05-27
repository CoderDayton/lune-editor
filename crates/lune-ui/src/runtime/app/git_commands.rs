#![allow(clippy::wildcard_imports)]
//! Git command handlers — all dispatches now flow through the async
//! [`GitPort`]. Synchronous feedback (status messages, OID display,
//! error toasts) comes from the next published `StatusSnapshot`,
//! which the render path observes via
//! `AppState::sync_git_panel_from_port`.

use lune_core::ports::{GitCommand, PatchLocation};

use super::*;

pub(super) fn handle_git_command(
    cmd: &AppCommand,
    state: &mut AppState,
) -> Option<Control<AppEvent>> {
    let control = match cmd {
        AppCommand::GitStage => dispatch_file_op(state, "stage"),
        AppCommand::GitUnstage => dispatch_file_op(state, "unstage"),
        AppCommand::GitCommit => handle_git_commit_prompt(state),
        AppCommand::GitDiscard => handle_git_discard_prompt(state),
        AppCommand::GitRefresh => {
            state.refresh_git();
            Control::Changed
        }
        AppCommand::GitDiscardConfirmed(path) => {
            state.git_port().dispatch(GitCommand::Discard(path.clone()));
            state.status_message = format!("Discarded: {}", path.display());
            Control::Changed
        }
        AppCommand::GitCommitConfirmed(msg) => {
            state.git_port().dispatch(GitCommand::Commit {
                message: msg.clone(),
            });
            state.status_message = "Commit dispatched…".to_string();
            Control::Changed
        }
        AppCommand::GitStageHunk => dispatch_hunk(state, "stage", PatchLocation::Index, false),
        AppCommand::GitUnstageHunk => dispatch_hunk(state, "unstage", PatchLocation::Index, true),
        AppCommand::GitDiscardHunk => dispatch_hunk(state, "discard", PatchLocation::Workdir, true),
        _ => return None,
    };
    Some(control)
}

// ── File-level ops (stage / unstage) ────────────────────────────────

fn dispatch_file_op(state: &mut AppState, kind: &str) -> Control<AppEvent> {
    let Some(file) = state.git_panel.selected_file().cloned() else {
        return Control::Continue;
    };
    let cmd = match kind {
        "stage" => GitCommand::Stage(file.path.clone()),
        "unstage" => GitCommand::Unstage(file.path.clone()),
        _ => return Control::Continue,
    };
    state.git_port().dispatch(cmd);
    let label = match kind {
        "stage" => "Staging",
        _ => "Unstaging",
    };
    state.status_message = format!("{label}: {}", file.path.display());
    Control::Changed
}

// ── Commit / discard prompts ────────────────────────────────────────

fn handle_git_commit_prompt(state: &mut AppState) -> Control<AppEvent> {
    let status = state.git_port().status().load();
    let staged_count = status.files.iter().filter(|f| f.staged).count();
    if staged_count == 0 {
        state
            .overlay
            .notify("Nothing staged to commit", NotificationLevel::Info);
        return Control::Changed;
    }

    let dialog = overlay::InputDialogState::new(
        format!("Commit ({staged_count} staged)"),
        "Enter commit message…",
        overlay::InputDialogAction::CommitMessage,
    );
    state.overlay.open_input_dialog(dialog);
    state.focus.focus(PanelId::CommandPalette);
    Control::Changed
}

fn handle_git_discard_prompt(state: &mut AppState) -> Control<AppEvent> {
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

// ── Hunk-level ops (stage_hunk / unstage_hunk / discard_hunk) ───────

fn dispatch_hunk(
    state: &mut AppState,
    label: &str,
    location: PatchLocation,
    reverse: bool,
) -> Control<AppEvent> {
    let Some((path, hunk)) = state.git_panel.diff_view.current_hunk_data() else {
        return Control::Continue;
    };
    let path = path.to_path_buf();
    let diff_patch = if reverse {
        hunk.to_reverse_patch(&path)
    } else {
        hunk.to_patch(&path)
    };
    let diff_patch = match diff_patch {
        Ok(p) => p,
        Err(e) => {
            state.status_message = format!("{label} hunk failed: {e}");
            return Control::Changed;
        }
    };
    state.git_port().dispatch(GitCommand::ApplyPatch {
        patch: diff_patch,
        location,
    });
    state.status_message = format!("{label} hunk: {}", path.display());
    Control::Changed
}
