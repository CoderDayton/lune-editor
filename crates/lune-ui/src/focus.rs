//! Focus management for panel routing.
//!
//! The focus system tracks which panel currently receives keyboard input
//! and maintains a history stack for focus-return behavior (e.g., closing
//! the command palette returns focus to the previously focused panel).

/// Identifies a focusable panel in the UI.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PanelId {
    /// The file tree sidebar.
    FileTree,
    /// The main editor pane.
    #[default]
    Editor,
    /// The AI terminal panel.
    AiTerminal,
    /// The git panel.
    GitPanel,
    /// The command palette overlay.
    CommandPalette,
    /// The status bar (rarely focused directly).
    StatusBar,
}

/// Manages which panel is focused and supports focus history for
/// overlay-style panels.
#[derive(Clone, Debug)]
pub struct FocusManager {
    /// The currently focused panel.
    active: PanelId,
    /// Stack of previously focused panels (for focus-return).
    history: Vec<PanelId>,
}

impl FocusManager {
    /// Create a new focus manager with initial focus on the editor.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: PanelId::Editor,
            history: Vec::new(),
        }
    }

    /// Get the currently focused panel.
    #[must_use]
    pub const fn active(&self) -> PanelId {
        self.active
    }

    /// Check if a given panel is currently focused.
    #[must_use]
    pub const fn is_focused(&self, panel: PanelId) -> bool {
        // Can't use == in const fn with derive(PartialEq), so use discriminant.
        self.active as u8 == panel as u8
    }

    /// Focus a new panel, pushing the current one onto the history stack.
    pub fn focus(&mut self, panel: PanelId) {
        if self.active != panel {
            self.history.push(self.active);
            self.active = panel;
        }
    }

    /// Return focus to the previously focused panel. If the history is
    /// empty, defaults to [`PanelId::Editor`].
    pub fn focus_return(&mut self) {
        self.active = self.history.pop().unwrap_or(PanelId::Editor);
    }

    /// Set focus without pushing to history (for restoring saved state).
    pub const fn set_active(&mut self, panel: PanelId) {
        self.active = panel;
    }
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_focus_is_editor() {
        let fm = FocusManager::new();
        assert_eq!(fm.active(), PanelId::Editor);
        assert!(fm.is_focused(PanelId::Editor));
    }

    #[test]
    fn focus_changes_active() {
        let mut fm = FocusManager::new();
        fm.focus(PanelId::FileTree);
        assert_eq!(fm.active(), PanelId::FileTree);
        assert!(!fm.is_focused(PanelId::Editor));
    }

    #[test]
    fn focus_return_pops_history() {
        let mut fm = FocusManager::new();
        fm.focus(PanelId::CommandPalette);
        assert_eq!(fm.active(), PanelId::CommandPalette);

        fm.focus_return();
        assert_eq!(fm.active(), PanelId::Editor);
    }

    #[test]
    fn focus_return_empty_defaults_to_editor() {
        let mut fm = FocusManager::new();
        fm.set_active(PanelId::GitPanel);
        fm.focus_return();
        assert_eq!(fm.active(), PanelId::Editor);
    }

    #[test]
    fn focus_same_panel_does_not_push() {
        let mut fm = FocusManager::new();
        fm.focus(PanelId::Editor);
        fm.focus(PanelId::Editor);
        // History should still be empty.
        fm.focus_return();
        assert_eq!(fm.active(), PanelId::Editor);
    }

    #[test]
    fn nested_focus_return() {
        let mut fm = FocusManager::new();
        // Editor → FileTree → CommandPalette
        fm.focus(PanelId::FileTree);
        fm.focus(PanelId::CommandPalette);

        fm.focus_return(); // → FileTree
        assert_eq!(fm.active(), PanelId::FileTree);

        fm.focus_return(); // → Editor
        assert_eq!(fm.active(), PanelId::Editor);
    }
}
