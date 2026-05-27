//! Welcome screen selection state + key dispatcher.
//!
//! Tracks the currently selected entry in the recent-files list shown
//! on the welcome screen, and translates navigation keys (j/k/arrows)
//! and Enter into selection moves or an `OpenFile` command.

use std::path::PathBuf;

use rat_salsa::Control;

#[cfg(test)]
use crate::primitives::KeyModifiers;
use crate::primitives::{KeyCode, KeyEvent};

use crate::runtime::event::{AppCommand, AppEvent};

/// Index-based selection cursor over a recent-files list. The list
/// length is owned by `AppState::recent_files`; this struct only owns
/// the cursor position. `clamp` reconciles after the list shrinks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WelcomeNav {
    pub selected: usize,
}

impl WelcomeNav {
    /// Advance selection by one, clamping at `len.saturating_sub(1)`.
    /// A zero-length list keeps the cursor at 0.
    pub const fn select_next(&mut self, len: usize) {
        let max = len.saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        } else if len == 0 {
            self.selected = 0;
        }
    }

    /// Retreat selection by one, saturating at 0.
    pub const fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Snap selection inside `[0, len)`. With an empty list, resets to 0.
    pub const fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

/// Dispatch a key for the welcome screen.
///
/// Returns `None` when the key is not handled (caller falls through to
/// the usual editor key chain). `welcome_shown` is the caller's
/// pre-computed "is the welcome screen currently visible & focused"
/// gate — typically `root_tab == Editor && active_buffer.is_none() &&
/// focus.is_focused(Editor)`.
#[must_use]
pub fn handle_welcome_key(
    key: &KeyEvent,
    nav: &mut WelcomeNav,
    recent: &[PathBuf],
    welcome_shown: bool,
) -> Option<Control<AppEvent>> {
    if !welcome_shown || recent.is_empty() {
        return None;
    }
    // Reconcile selection in case the list shrunk since last render.
    nav.clamp(recent.len());

    match (key.code, key.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, m) if m.is_empty() => {
            nav.select_next(recent.len());
            Some(Control::Changed)
        }
        (KeyCode::Char('k') | KeyCode::Up, m) if m.is_empty() => {
            nav.select_prev();
            Some(Control::Changed)
        }
        (KeyCode::Enter, m) if m.is_empty() => {
            let path = recent.get(nav.selected)?.clone();
            Some(Control::Event(AppEvent::Command(AppCommand::OpenFile(
                path,
            ))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_index_zero() {
        let nav = WelcomeNav::default();
        assert_eq!(nav.selected, 0);
    }

    #[test]
    fn select_next_advances_one() {
        let mut nav = WelcomeNav::default();
        nav.select_next(5);
        assert_eq!(nav.selected, 1);
    }

    #[test]
    fn select_next_clamps_at_last_index() {
        let mut nav = WelcomeNav { selected: 4 };
        nav.select_next(5);
        assert_eq!(nav.selected, 4, "must not advance past len-1");
    }

    #[test]
    fn select_next_with_empty_list_stays_zero() {
        let mut nav = WelcomeNav::default();
        nav.select_next(0);
        assert_eq!(nav.selected, 0);
    }

    #[test]
    fn select_prev_decreases_one() {
        let mut nav = WelcomeNav { selected: 3 };
        nav.select_prev();
        assert_eq!(nav.selected, 2);
    }

    #[test]
    fn select_prev_saturates_at_zero() {
        let mut nav = WelcomeNav::default();
        nav.select_prev();
        assert_eq!(nav.selected, 0, "must not underflow");
    }

    #[test]
    fn clamp_pulls_selection_into_bounds() {
        let mut nav = WelcomeNav { selected: 7 };
        nav.clamp(3);
        assert_eq!(nav.selected, 2, "clamp to len-1");
    }

    #[test]
    fn clamp_to_empty_resets_to_zero() {
        let mut nav = WelcomeNav { selected: 5 };
        nav.clamp(0);
        assert_eq!(nav.selected, 0);
    }

    #[test]
    fn clamp_leaves_in_bounds_selection_alone() {
        let mut nav = WelcomeNav { selected: 1 };
        nav.clamp(5);
        assert_eq!(nav.selected, 1);
    }

    // ── handle_welcome_key dispatcher ───────────────────────────────

    use std::path::PathBuf;

    fn recent(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn returns_none_when_welcome_not_shown() {
        let mut nav = WelcomeNav::default();
        let r = recent(&["/a", "/b"]);
        let result = handle_welcome_key(&k(KeyCode::Char('j')), &mut nav, &r, false);
        assert!(result.is_none());
        assert_eq!(nav.selected, 0, "must not mutate when gated off");
    }

    #[test]
    fn returns_none_when_recent_empty() {
        let mut nav = WelcomeNav::default();
        let result = handle_welcome_key(&k(KeyCode::Char('j')), &mut nav, &[], true);
        assert!(result.is_none());
    }

    #[test]
    fn j_advances_selection() {
        let mut nav = WelcomeNav::default();
        let r = recent(&["/a", "/b", "/c"]);
        let result = handle_welcome_key(&k(KeyCode::Char('j')), &mut nav, &r, true);
        assert!(matches!(result, Some(Control::Changed)));
        assert_eq!(nav.selected, 1);
    }

    #[test]
    fn down_arrow_is_equivalent_to_j() {
        let mut nav = WelcomeNav::default();
        let r = recent(&["/a", "/b"]);
        let result = handle_welcome_key(&k(KeyCode::Down), &mut nav, &r, true);
        assert!(matches!(result, Some(Control::Changed)));
        assert_eq!(nav.selected, 1);
    }

    #[test]
    fn k_retreats_selection() {
        let mut nav = WelcomeNav { selected: 2 };
        let r = recent(&["/a", "/b", "/c"]);
        let result = handle_welcome_key(&k(KeyCode::Char('k')), &mut nav, &r, true);
        assert!(matches!(result, Some(Control::Changed)));
        assert_eq!(nav.selected, 1);
    }

    #[test]
    fn up_arrow_is_equivalent_to_k() {
        let mut nav = WelcomeNav { selected: 1 };
        let r = recent(&["/a", "/b"]);
        let result = handle_welcome_key(&k(KeyCode::Up), &mut nav, &r, true);
        assert!(matches!(result, Some(Control::Changed)));
        assert_eq!(nav.selected, 0);
    }

    #[test]
    fn enter_emits_open_file_for_selected_path() {
        let mut nav = WelcomeNav { selected: 1 };
        let r = recent(&["/a", "/b", "/c"]);
        let result = handle_welcome_key(&k(KeyCode::Enter), &mut nav, &r, true);
        match result {
            Some(Control::Event(AppEvent::Command(AppCommand::OpenFile(p)))) => {
                assert_eq!(p, PathBuf::from("/b"));
            }
            other => panic!("expected OpenFile(/b), got {other:?}"),
        }
    }

    #[test]
    fn enter_with_stale_index_uses_clamped_path() {
        // Selection points past the end (e.g., list shrank between renders).
        // Handler must clamp before reading.
        let mut nav = WelcomeNav { selected: 99 };
        let r = recent(&["/x", "/y"]);
        let result = handle_welcome_key(&k(KeyCode::Enter), &mut nav, &r, true);
        match result {
            Some(Control::Event(AppEvent::Command(AppCommand::OpenFile(p)))) => {
                assert_eq!(p, PathBuf::from("/y"), "clamped to last index");
            }
            other => panic!("expected OpenFile(/y), got {other:?}"),
        }
    }

    #[test]
    fn unrelated_key_returns_none() {
        let mut nav = WelcomeNav::default();
        let r = recent(&["/a"]);
        let result = handle_welcome_key(&k(KeyCode::Char('x')), &mut nav, &r, true);
        assert!(result.is_none());
    }

    #[test]
    fn modified_j_is_not_handled() {
        // Ctrl+J / Shift+J etc. should fall through so they don't shadow
        // other bindings.
        let mut nav = WelcomeNav::default();
        let r = recent(&["/a", "/b"]);
        let result = handle_welcome_key(
            &KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            &mut nav,
            &r,
            true,
        );
        assert!(result.is_none());
        assert_eq!(nav.selected, 0);
    }
}
