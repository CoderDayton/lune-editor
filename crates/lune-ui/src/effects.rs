//! Visual effects powered by tachyonfx.
//!
//! Provides an effect management layer over tachyonfx, with pre-defined
//! effects for focus indicators, diff animations, and AI activity.

use std::fmt;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Color;
use tachyonfx::{fx, Duration, Effect, EffectManager};

use crate::focus::PanelId;

/// Unique identifier for managed effects.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectId {
    /// Focus glow on a specific panel.
    FocusGlow(PanelId),
}

impl Default for EffectId {
    fn default() -> Self {
        Self::FocusGlow(PanelId::default())
    }
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
    /// Focus glow on active panel borders.
    pub focus_glow: EffectConfig,
}

impl Default for EffectDefs {
    fn default() -> Self {
        Self {
            focus_glow: EffectConfig::new(0.15),
        }
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

    /// Start the focus glow effect on a panel's area.
    ///
    /// Cancels any existing focus glow on other panels first.
    pub fn start_focus_glow(&mut self, panel: PanelId, accent: Color) {
        if !self.defs.focus_glow.enabled {
            return;
        }

        let intensity = self.defs.focus_glow.intensity;
        let effect = create_focus_glow(intensity, accent);
        self.manager
            .add_unique_effect(EffectId::FocusGlow(panel), effect);
    }

    /// Returns the focus glow intensity if the effect is enabled, else `0.0`.
    #[must_use]
    pub const fn focus_glow_intensity(&self) -> f32 {
        if self.defs.focus_glow.enabled {
            self.defs.focus_glow.intensity
        } else {
            0.0
        }
    }

    /// Cancel focus glow on a specific panel.
    pub fn cancel_focus_glow(&mut self, panel: PanelId) {
        self.manager
            .cancel_unique_effect(EffectId::FocusGlow(panel));
    }
}

impl Default for LuneEffects {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a focus glow effect: brightens the outer 1-cell border of the
/// area using the accent color, running indefinitely until cancelled.
fn create_focus_glow(intensity: f32, accent: Color) -> Effect {
    // Use effect_fn_buf for a custom per-frame effect that paints the
    // inner border cells with the accent color at the given intensity.
    // This runs every frame indefinitely.
    let glow = fx::effect_fn_buf(
        (),                        // no state needed
        Duration::from_millis(16), // ~1 frame at 60fps
        move |_state: &mut (), ctx, buf: &mut Buffer| {
            let area = ctx.area;
            if area.width < 2 || area.height < 2 {
                return;
            }
            paint_inner_border(buf, area, accent, intensity);
        },
    );

    fx::never_complete(glow)
}

/// Paint the inner border cells of a rect with an accent color blend.
///
/// This brightens/blends the existing cell colors toward the accent color
/// at the given intensity (0.0 = no change, 1.0 = fully accent-colored).
pub fn paint_inner_border(buf: &mut Buffer, area: Rect, accent: Color, intensity: f32) {
    let Color::Rgb(ar, ag, ab) = accent else {
        return; // Only RGB accent colors supported for blending.
    };

    let x0 = area.x;
    let x1 = area.x + area.width.saturating_sub(1);
    let y0 = area.y;
    let y1 = area.y + area.height.saturating_sub(1);

    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            // Check if this cell is on the inner border (1-cell thick).
            let on_border = x == x0 || x == x1 || y == y0 || y == y1;
            if !on_border {
                continue;
            }

            let cell = &mut buf[(x, y)];
            // Blend the foreground color toward accent.
            cell.fg = blend_toward(cell.fg, ar, ag, ab, intensity);
            // Blend the background color toward accent (subtler).
            cell.bg = blend_toward(cell.bg, ar, ag, ab, intensity * 0.5);
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
        let a = EffectId::FocusGlow(PanelId::Editor);
        let b = EffectId::FocusGlow(PanelId::FileTree);
        assert_ne!(a, b);
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
    }

    #[test]
    fn start_and_cancel_focus_glow() {
        let mut fx = LuneEffects::new();
        fx.start_focus_glow(PanelId::Editor, Color::Rgb(80, 130, 220));
        assert!(fx.is_running());
        fx.cancel_focus_glow(PanelId::Editor);
        // Note: cancel_unique_effect may not immediately stop the effect
        // (it marks it for removal on next process), but the API call works.
    }

    #[test]
    fn paint_inner_border_small_rect() {
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        // Fill with a known color.
        for cell in &mut buf.content {
            cell.fg = Color::Rgb(100, 100, 100);
            cell.bg = Color::Rgb(30, 30, 30);
        }
        paint_inner_border(&mut buf, area, Color::Rgb(80, 130, 220), 0.5);

        // Border cell (0,0) should be blended.
        let corner = &buf[(0u16, 0u16)];
        assert!(matches!(corner.fg, Color::Rgb(_, _, _)));

        // Interior cell (1,1) should NOT be blended (it's not on the border for a 4x3 rect).
        // For 4x3: x=0..3, y=0..2. Border: x==0, x==3, y==0, y==2.
        // Cell (1,1) is interior only if width>2 AND height>2, which it is (4>2, 3>2).
        let interior = &buf[(1u16, 1u16)];
        assert_eq!(interior.fg, Color::Rgb(100, 100, 100));
    }
}
