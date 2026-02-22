//! Visual effects powered by tachyonfx.
//!
//! Provides an effect management layer over tachyonfx, with pre-defined
//! effects for focus indicators, diff animations, and AI activity.

use std::fmt;

use crate::primitives::{Buffer, Color, Rect};
use tachyonfx::{Duration, Effect, EffectManager, fx};

use crate::focus::PanelId;

/// Unique identifier for managed effects.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectId {
    /// Brightness pulse when new diff hunks appear.
    #[default]
    DiffPulse,
    /// Color-cycling indicator when an AI session is actively processing.
    AiThinking,
    /// Brightness flash when a notification appears.
    Notification,
    /// Brightness flash on panel open/close transitions.
    PanelTransition(PanelId),
}

/// Configuration for individual effect types.
#[derive(Clone, Debug)]
pub struct EffectConfig {
    /// Whether this effect is enabled.
    pub enabled: bool,
    /// Base intensity (0.0–1.0).
    pub intensity: f32,
}

impl EffectConfig {
    const fn new(intensity: f32) -> Self {
        Self {
            enabled: true,
            intensity,
        }
    }
}

/// Effect definitions / configuration.
#[derive(Clone, Debug)]
pub struct EffectDefs {
    /// Master switch: when `false`, all effects are disabled.
    pub all_enabled: bool,
    /// Brightness pulse when new diff hunks arrive.
    pub diff_pulse: EffectConfig,
    /// Color-cycling AI thinking indicator on the status bar.
    pub ai_thinking: EffectConfig,
    /// Brightness flash for new notifications.
    pub notification_flash: EffectConfig,
    /// Brightness flash on panel open/close.
    pub panel_transition: EffectConfig,
}

impl Default for EffectDefs {
    fn default() -> Self {
        Self {
            all_enabled: true,
            diff_pulse: EffectConfig::new(0.25),
            ai_thinking: EffectConfig::new(0.6),
            notification_flash: EffectConfig::new(0.20),
            panel_transition: EffectConfig::new(0.15),
        }
    }
}

impl EffectDefs {
    /// Create definitions with all effects disabled (e.g., `--no-effects`).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            all_enabled: false,
            ..Self::default()
        }
    }

    /// Whether a specific effect is enabled (respects the master switch).
    const fn is_enabled(&self, config: &EffectConfig) -> bool {
        self.all_enabled && config.enabled
    }
}

/// Effect management layer.
///
/// Owns a tachyonfx `EffectManager` and provides high-level methods
/// for triggering and cancelling named effects.
pub struct LuneEffects {
    manager: EffectManager<EffectId>,
    defs: EffectDefs,
}

impl fmt::Debug for LuneEffects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LuneEffects")
            .field("defs", &self.defs)
            .field("is_running", &self.manager.is_running())
            .finish()
    }
}

impl LuneEffects {
    /// Create a new effect manager with default definitions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manager: EffectManager::default(),
            defs: EffectDefs::default(),
        }
    }

    /// Create with custom effect definitions.
    #[must_use]
    pub fn with_defs(defs: EffectDefs) -> Self {
        Self {
            manager: EffectManager::default(),
            defs,
        }
    }

    /// Whether any effects are currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.manager.is_running()
    }

    /// Process all active effects, applying them to the buffer.
    ///
    /// `elapsed` is the time since the last frame, converted to
    /// tachyonfx's `Duration` type.
    pub fn process(&mut self, elapsed: std::time::Duration, buf: &mut Buffer, area: Rect) {
        let dur: Duration = elapsed.into();
        self.manager.process_effects(dur, buf, area);
    }

    // ── Step 3: Diff pulse ────────────────────────────────────────────

    /// Start a brief brightness pulse over the editor area when new
    /// diff hunks appear. Replaces any existing diff pulse.
    pub fn start_diff_pulse(&mut self, tint: Color) {
        if !self.defs.is_enabled(&self.defs.diff_pulse) {
            return;
        }

        let intensity = self.defs.diff_pulse.intensity;
        let effect = create_diff_pulse(intensity, tint);
        self.manager.add_unique_effect(EffectId::DiffPulse, effect);
    }

    /// Cancel any running diff pulse.
    pub fn cancel_diff_pulse(&mut self) {
        self.manager.cancel_unique_effect(EffectId::DiffPulse);
    }

    // ── Step 4: AI thinking indicator ─────────────────────────────────

    /// Start the AI thinking color-cycle effect on the status bar area.
    pub fn start_ai_thinking(&mut self, accent: Color) {
        if !self.defs.is_enabled(&self.defs.ai_thinking) {
            return;
        }

        let intensity = self.defs.ai_thinking.intensity;
        let effect = create_ai_thinking(intensity, accent);
        self.manager.add_unique_effect(EffectId::AiThinking, effect);
    }

    /// Cancel the AI thinking effect.
    pub fn cancel_ai_thinking(&mut self) {
        self.manager.cancel_unique_effect(EffectId::AiThinking);
    }

    // ── Step 5: Notification flash ────────────────────────────────────

    /// Start a brief brightness flash when a notification appears.
    pub fn start_notification_flash(&mut self) {
        if !self.defs.is_enabled(&self.defs.notification_flash) {
            return;
        }

        let intensity = self.defs.notification_flash.intensity;
        let effect = create_notification_flash(intensity);
        self.manager
            .add_unique_effect(EffectId::Notification, effect);
    }

    // ── Step 6: Panel transition flash ────────────────────────────────

    /// Start a brief brightness flash when a panel opens or closes.
    pub fn start_panel_transition(&mut self, panel: PanelId, accent: Color) {
        if !self.defs.is_enabled(&self.defs.panel_transition) {
            return;
        }

        let intensity = self.defs.panel_transition.intensity;
        let effect = create_panel_transition(intensity, accent);
        self.manager
            .add_unique_effect(EffectId::PanelTransition(panel), effect);
    }

    /// Cancel any running panel transition on a specific panel.
    pub fn cancel_panel_transition(&mut self, panel: PanelId) {
        self.manager
            .cancel_unique_effect(EffectId::PanelTransition(panel));
    }

    // ── Step 7: Configuration helpers ─────────────────────────────────

    /// Disable all effects (e.g., for `--no-effects` flag).
    pub const fn disable_all(&mut self) {
        self.defs.all_enabled = false;
    }

    /// Enable all effects.
    pub const fn enable_all(&mut self) {
        self.defs.all_enabled = true;
    }

    /// Whether effects are globally enabled.
    #[must_use]
    pub const fn all_enabled(&self) -> bool {
        self.defs.all_enabled
    }

    /// Access the effect definitions (read-only).
    #[must_use]
    pub const fn defs(&self) -> &EffectDefs {
        &self.defs
    }

    /// Access the effect definitions (mutable, for toggling individual effects).
    pub const fn defs_mut(&mut self) -> &mut EffectDefs {
        &mut self.defs
    }
}

impl Default for LuneEffects {
    fn default() -> Self {
        Self::new()
    }
}

// ── Step 3: Diff pulse ──────────────────────────────────────────────────

/// Duration of the diff pulse effect in milliseconds.
const DIFF_PULSE_MS: u32 = 500;

/// Create a brightness pulse effect for the editor content area.
///
/// Uses a ping-pong `effect_fn_buf` that brightens cells toward the tint
/// color over `DIFF_PULSE_MS`, then fades back — 1 full ping-pong cycle.
fn create_diff_pulse(intensity: f32, tint: Color) -> Effect {
    let Color::Rgb(tr, tg, tb) = tint else {
        // Fallback: use a green-ish tint for non-RGB colors.
        return create_diff_pulse(intensity, Color::Rgb(60, 180, 80));
    };

    // A single pass: brighten then fade over the duration.
    fx::effect_fn_buf(
        (),
        Duration::from_millis(DIFF_PULSE_MS),
        move |_state: &mut (), ctx, buf: &mut Buffer| {
            // alpha: 0.0 → 1.0 over the duration.
            // Triangular wave: ramp up for first half, ramp down for second.
            let alpha = ctx.alpha();
            let wave = if alpha < 0.5 {
                alpha * 2.0
            } else {
                (1.0 - alpha) * 2.0
            };
            let t = intensity * wave;
            brighten_area(buf, ctx.area, tr, tg, tb, t);
        },
    )
}

// ── Step 4: AI thinking indicator ───────────────────────────────────────

/// Duration of one AI thinking color cycle in milliseconds.
const AI_THINKING_CYCLE_MS: u32 = 1200;

/// Create a repeating color-cycle effect for the AI thinking indicator.
///
/// Shifts the foreground hue continuously, creating a subtle "breathing"
/// animation on the status bar area while AI is processing.
fn create_ai_thinking(intensity: f32, accent: Color) -> Effect {
    let Color::Rgb(ar, ag, ab) = accent else {
        return create_ai_thinking(intensity, Color::Rgb(80, 130, 220));
    };

    let cycle = fx::effect_fn_buf(
        (),
        Duration::from_millis(AI_THINKING_CYCLE_MS),
        move |_state: &mut (), ctx, buf: &mut Buffer| {
            let alpha = ctx.alpha();
            // Sinusoidal breathing: smoothly oscillate intensity.
            let wave = ((alpha * std::f32::consts::TAU).sin() + 1.0) * 0.5;
            let t = intensity * wave;
            brighten_area(buf, ctx.area, ar, ag, ab, t);
        },
    );

    fx::repeating(cycle)
}

// ── Step 5: Notification flash ──────────────────────────────────────────

/// Duration of the notification flash in milliseconds.
const NOTIFICATION_FLASH_MS: u32 = 300;

/// Create a brief brightness flash for notification appearance.
///
/// Brightens toward white then fades — a single-shot effect.
fn create_notification_flash(intensity: f32) -> Effect {
    fx::effect_fn_buf(
        (),
        Duration::from_millis(NOTIFICATION_FLASH_MS),
        move |_state: &mut (), ctx, buf: &mut Buffer| {
            let alpha = ctx.alpha();
            // Quick brighten then fade: use (1 - alpha) for decay.
            let t = intensity * (1.0 - alpha);
            brighten_area(buf, ctx.area, 255, 255, 255, t);
        },
    )
}

// ── Step 6: Panel transition flash ──────────────────────────────────────

/// Duration of the panel transition flash in milliseconds.
const PANEL_TRANSITION_MS: u32 = 150;

/// Create a brief flash for panel open/close transitions.
fn create_panel_transition(intensity: f32, accent: Color) -> Effect {
    let Color::Rgb(ar, ag, ab) = accent else {
        return create_panel_transition(intensity, Color::Rgb(80, 130, 220));
    };

    fx::effect_fn_buf(
        (),
        Duration::from_millis(PANEL_TRANSITION_MS),
        move |_state: &mut (), ctx, buf: &mut Buffer| {
            let alpha = ctx.alpha();
            let t = intensity * (1.0 - alpha);
            brighten_area(buf, ctx.area, ar, ag, ab, t);
        },
    )
}

// ── Shared helpers ──────────────────────────────────────────────────────

/// Brighten all cells in an area toward a target color at the given intensity.
fn brighten_area(buf: &mut Buffer, area: Rect, tr: u8, tg: u8, tb: u8, t: f32) {
    if t <= 0.0 || area.width == 0 || area.height == 0 {
        return;
    }

    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            let cell = &mut buf[(x, y)];
            cell.fg = blend_toward(cell.fg, tr, tg, tb, t);
            cell.bg = blend_toward(cell.bg, tr, tg, tb, t * 0.5);
        }
    }
}

/// Blend a color toward the target RGB at the given intensity.
pub fn blend_toward(color: Color, tr: u8, tg: u8, tb: u8, t: f32) -> Color {
    let (sr, sg, sb) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Yellow => (255, 255, 0),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 128, 128),
        Color::LightGreen => (128, 255, 128),
        Color::LightBlue => (128, 128, 255),
        Color::LightYellow => (255, 255, 128),
        Color::LightMagenta => (255, 128, 255),
        Color::LightCyan => (128, 255, 255),
        Color::Indexed(_) => return color, // Can't blend indexed colors.
    };

    let lerp = |s: u8, d: u8, t: f32| -> u8 {
        let s_f = f32::from(s);
        let d_f = f32::from(d);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let result = (d_f - s_f).mul_add(t, s_f).clamp(0.0, 255.0) as u8;
        result
    };

    Color::Rgb(lerp(sr, tr, t), lerp(sg, tg, t), lerp(sb, tb, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_id_ordering() {
        // EffectId must be Ord for BTreeMap keying.
        let a = EffectId::PanelTransition(PanelId::Editor);
        let b = EffectId::PanelTransition(PanelId::FileTree);
        assert_ne!(a, b);
    }

    #[test]
    fn effect_id_variants_distinct() {
        let diff = EffectId::DiffPulse;
        let ai = EffectId::AiThinking;
        let notif = EffectId::Notification;
        let panel = EffectId::PanelTransition(PanelId::FileTree);
        assert_ne!(diff, ai);
        assert_ne!(ai, notif);
        assert_ne!(notif, panel);
    }

    #[test]
    fn blend_toward_no_change_at_zero() {
        let result = blend_toward(Color::Rgb(100, 100, 100), 200, 200, 200, 0.0);
        assert_eq!(result, Color::Rgb(100, 100, 100));
    }

    #[test]
    fn blend_toward_full_change_at_one() {
        let result = blend_toward(Color::Rgb(0, 0, 0), 200, 100, 50, 1.0);
        assert_eq!(result, Color::Rgb(200, 100, 50));
    }

    #[test]
    fn blend_toward_half() {
        let result = blend_toward(Color::Rgb(0, 0, 0), 200, 100, 0, 0.5);
        assert_eq!(result, Color::Rgb(100, 50, 0));
    }

    #[test]
    fn blend_indexed_returns_unchanged() {
        let result = blend_toward(Color::Indexed(42), 200, 200, 200, 0.5);
        assert_eq!(result, Color::Indexed(42));
    }

    #[test]
    fn lune_effects_default() {
        let fx = LuneEffects::new();
        assert!(!fx.is_running());
        assert!(fx.all_enabled());
    }

    // ── Step 3: Diff pulse tests ──────────────────────────────────────

    #[test]
    fn start_diff_pulse_runs() {
        let mut fx = LuneEffects::new();
        fx.start_diff_pulse(Color::Rgb(60, 180, 80));
        assert!(fx.is_running());
    }

    #[test]
    fn diff_pulse_with_non_rgb_fallback() {
        let mut fx = LuneEffects::new();
        fx.start_diff_pulse(Color::Green);
        assert!(fx.is_running());
    }

    #[test]
    fn cancel_diff_pulse() {
        let mut fx = LuneEffects::new();
        fx.start_diff_pulse(Color::Rgb(60, 180, 80));
        fx.cancel_diff_pulse();
        // Effect marked for removal — no panic.
    }

    // ── Step 4: AI thinking tests ─────────────────────────────────────

    #[test]
    fn start_ai_thinking_runs() {
        let mut fx = LuneEffects::new();
        fx.start_ai_thinking(Color::Rgb(80, 130, 220));
        assert!(fx.is_running());
    }

    #[test]
    fn cancel_ai_thinking() {
        let mut fx = LuneEffects::new();
        fx.start_ai_thinking(Color::Rgb(80, 130, 220));
        fx.cancel_ai_thinking();
    }

    #[test]
    fn ai_thinking_non_rgb_fallback() {
        let mut fx = LuneEffects::new();
        fx.start_ai_thinking(Color::Blue);
        assert!(fx.is_running());
    }

    // ── Step 5: Notification flash tests ──────────────────────────────

    #[test]
    fn start_notification_flash_runs() {
        let mut fx = LuneEffects::new();
        fx.start_notification_flash();
        assert!(fx.is_running());
    }

    // ── Step 6: Panel transition tests ────────────────────────────────

    #[test]
    fn start_panel_transition_runs() {
        let mut fx = LuneEffects::new();
        fx.start_panel_transition(PanelId::FileTree, Color::Rgb(80, 130, 220));
        assert!(fx.is_running());
    }

    #[test]
    fn cancel_panel_transition() {
        let mut fx = LuneEffects::new();
        fx.start_panel_transition(PanelId::FileTree, Color::Rgb(80, 130, 220));
        fx.cancel_panel_transition(PanelId::FileTree);
    }

    #[test]
    fn panel_transition_non_rgb_fallback() {
        let mut fx = LuneEffects::new();
        fx.start_panel_transition(PanelId::GitPanel, Color::Blue);
        assert!(fx.is_running());
    }

    // ── Step 7: Configuration tests ───────────────────────────────────

    #[test]
    fn disable_all_prevents_effects() {
        let mut fx = LuneEffects::new();
        fx.disable_all();
        assert!(!fx.all_enabled());

        // Trying to start effects should be a no-op.
        fx.start_diff_pulse(Color::Rgb(60, 180, 80));
        assert!(!fx.is_running());

        fx.start_ai_thinking(Color::Rgb(80, 130, 220));
        assert!(!fx.is_running());

        fx.start_notification_flash();
        assert!(!fx.is_running());

        fx.start_panel_transition(PanelId::FileTree, Color::Rgb(80, 130, 220));
        assert!(!fx.is_running());
    }

    #[test]
    fn enable_all_restores_effects() {
        let mut fx = LuneEffects::new();
        fx.disable_all();
        fx.enable_all();
        assert!(fx.all_enabled());

        fx.start_diff_pulse(Color::Rgb(60, 180, 80));
        assert!(fx.is_running());
    }

    #[test]
    fn disabled_defs_constructor() {
        let defs = EffectDefs::disabled();
        assert!(!defs.all_enabled);
        // Individual configs still have defaults, just globally off.
        assert!(defs.diff_pulse.enabled);
    }

    #[test]
    fn toggle_individual_effect() {
        let mut fx = LuneEffects::new();
        fx.defs_mut().diff_pulse.enabled = false;

        fx.start_diff_pulse(Color::Rgb(60, 180, 80));
        assert!(!fx.is_running());

        // Other effects still work.
        fx.start_ai_thinking(Color::Rgb(80, 130, 220));
        assert!(fx.is_running());
    }

    // ── Brighten area tests ───────────────────────────────────────────

    #[test]
    fn brighten_area_zero_intensity_is_noop() {
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        for cell in &mut buf.content {
            cell.fg = Color::Rgb(100, 100, 100);
            cell.bg = Color::Rgb(30, 30, 30);
        }
        brighten_area(&mut buf, area, 255, 255, 255, 0.0);
        assert_eq!(buf[(0u16, 0u16)].fg, Color::Rgb(100, 100, 100));
    }

    #[test]
    fn brighten_area_applies_blend() {
        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        for cell in &mut buf.content {
            cell.fg = Color::Rgb(0, 0, 0);
            cell.bg = Color::Rgb(0, 0, 0);
        }
        brighten_area(&mut buf, area, 200, 200, 200, 1.0);
        // fg should be fully shifted to (200, 200, 200).
        assert_eq!(buf[(0u16, 0u16)].fg, Color::Rgb(200, 200, 200));
        // bg at half intensity.
        assert_eq!(buf[(0u16, 0u16)].bg, Color::Rgb(100, 100, 100));
    }
}
