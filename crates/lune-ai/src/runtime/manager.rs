//! AI session manager.
//!
//! Manages multiple concurrent AI sessions, tracks the active session,
//! and provides a unified API for session lifecycle operations.

use std::collections::HashMap;
use std::path::Path;

use crate::pty::TermSize;
use crate::session::{AiClientKind, AiSession, AiSessionId, SessionState};

/// Manages multiple AI sessions.
pub struct AiManager {
    /// All sessions, keyed by ID.
    sessions: HashMap<AiSessionId, AiSession>,
    /// The currently active (focused) session ID.
    active: Option<AiSessionId>,
}

impl AiManager {
    /// Create a new, empty session manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            active: None,
        }
    }

    /// Start a new session and make it active.
    ///
    /// # Errors
    /// Returns an error if the session cannot be started.
    pub fn new_session(
        &mut self,
        kind: AiClientKind,
        cwd: Option<&Path>,
        env: &HashMap<String, String>,
        size: TermSize,
    ) -> anyhow::Result<AiSessionId> {
        let session = AiSession::start(kind, cwd, env, size)?;
        let id = session.id();
        self.sessions.insert(id, session);
        self.active = Some(id);
        Ok(id)
    }

    /// Get a reference to the active session.
    #[must_use]
    pub fn active_session(&self) -> Option<&AiSession> {
        self.active.and_then(|id| self.sessions.get(&id))
    }

    /// Get a mutable reference to the active session.
    pub fn active_session_mut(&mut self) -> Option<&mut AiSession> {
        self.active.and_then(|id| self.sessions.get_mut(&id))
    }

    /// Get the active session ID.
    #[must_use]
    pub const fn active_id(&self) -> Option<AiSessionId> {
        self.active
    }

    /// Get a reference to a session by ID.
    #[must_use]
    pub fn session(&self, id: AiSessionId) -> Option<&AiSession> {
        self.sessions.get(&id)
    }

    /// Get a mutable reference to a session by ID.
    pub fn session_mut(&mut self, id: AiSessionId) -> Option<&mut AiSession> {
        self.sessions.get_mut(&id)
    }

    /// Switch the active session.
    ///
    /// Returns `false` if the session ID doesn't exist.
    pub fn switch_session(&mut self, id: AiSessionId) -> bool {
        if self.sessions.contains_key(&id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    /// Close a session by ID. If it's the active session, switches to
    /// another session or sets active to `None`.
    pub fn close_session(&mut self, id: AiSessionId) {
        if let Some(mut session) = self.sessions.remove(&id) {
            if session.state() == SessionState::Running {
                let _ = session.stop();
            }
        }
        if self.active == Some(id) {
            // Switch to the first remaining session, or None.
            self.active = self.sessions.keys().next().copied();
        }
    }

    /// Poll all sessions for events. Returns `true` if any session had output.
    pub fn poll_all(&mut self) -> bool {
        let mut changed = false;
        for session in self.sessions.values_mut() {
            if session.poll_events() {
                changed = true;
            }
        }
        changed
    }

    /// Get the number of sessions.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Check if there are any sessions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Iterator over all session IDs and their display names.
    pub fn session_list(&self) -> Vec<(AiSessionId, String, SessionState)> {
        self.sessions
            .values()
            .map(|s| (s.id(), s.kind().display_name().to_string(), s.state()))
            .collect()
    }

    /// Resize all sessions to a new terminal size.
    pub fn resize_all(&mut self, size: TermSize) {
        for session in self.sessions.values_mut() {
            if let Err(e) = session.resize(size) {
                log::warn!("Failed to resize session {}: {e}", session.id());
            }
        }
    }

    /// Close all sessions.
    pub fn close_all(&mut self) {
        let ids: Vec<_> = self.sessions.keys().copied().collect();
        for id in ids {
            self.close_session(id);
        }
    }
}

impl Default for AiManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager_with_shell() -> AiManager {
        let mut mgr = AiManager::new();
        mgr.new_session(
            AiClientKind::Shell,
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        )
        .expect("Failed to start shell session");
        mgr
    }

    #[test]
    fn new_manager_is_empty() {
        let mgr = AiManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.session_count(), 0);
        assert!(mgr.active_session().is_none());
        assert!(mgr.active_id().is_none());
    }

    #[test]
    fn start_session_makes_active() {
        let mgr = make_manager_with_shell();
        assert_eq!(mgr.session_count(), 1);
        assert!(!mgr.is_empty());
        assert!(mgr.active_session().is_some());
        assert!(mgr.active_id().is_some());
    }

    #[test]
    fn multiple_sessions() {
        let mut mgr = AiManager::new();
        let id1 = mgr
            .new_session(
                AiClientKind::Shell,
                None,
                &HashMap::new(),
                TermSize::new(24, 80),
            )
            .unwrap();
        let id2 = mgr
            .new_session(
                AiClientKind::Shell,
                None,
                &HashMap::new(),
                TermSize::new(24, 80),
            )
            .unwrap();

        assert_eq!(mgr.session_count(), 2);
        // Last started session is active.
        assert_eq!(mgr.active_id(), Some(id2));

        // Switch to first session.
        assert!(mgr.switch_session(id1));
        assert_eq!(mgr.active_id(), Some(id1));

        // Switch to non-existent session fails.
        assert!(!mgr.switch_session(uuid::Uuid::new_v4()));

        mgr.close_all();
    }

    #[test]
    fn close_session_switches_active() {
        let mut mgr = AiManager::new();
        let id1 = mgr
            .new_session(
                AiClientKind::Shell,
                None,
                &HashMap::new(),
                TermSize::new(24, 80),
            )
            .unwrap();
        let _id2 = mgr
            .new_session(
                AiClientKind::Shell,
                None,
                &HashMap::new(),
                TermSize::new(24, 80),
            )
            .unwrap();

        // Close the active session (id2).
        let active = mgr.active_id().unwrap();
        mgr.close_session(active);

        assert_eq!(mgr.session_count(), 1);
        // Should have switched to the remaining session.
        assert!(mgr.active_id().is_some());

        // Close the last one.
        mgr.close_session(id1);
        assert!(mgr.is_empty());
        assert!(mgr.active_id().is_none());
    }

    #[test]
    fn session_list_returns_all() {
        let mut mgr = AiManager::new();
        let _id1 = mgr
            .new_session(
                AiClientKind::Shell,
                None,
                &HashMap::new(),
                TermSize::new(24, 80),
            )
            .unwrap();
        let _id2 = mgr.new_session(
            AiClientKind::ClaudeCode,
            None,
            &HashMap::new(),
            TermSize::new(24, 80),
        );
        // ClaudeCode might fail if 'claude' isn't installed — that's fine.

        let list = mgr.session_list();
        // At least the shell session should be in the list.
        assert!(!list.is_empty());

        mgr.close_all();
    }

    #[test]
    fn poll_all_works() {
        let mut mgr = make_manager_with_shell();

        // Send something and wait.
        if let Some(session) = mgr.active_session_mut() {
            session.send_input(b"echo poll_test\n").unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(300));

        let changed = mgr.poll_all();
        assert!(changed);

        mgr.close_all();
    }
}
