# 10 — Visual Effects (tachyonfx)

> **Phase:** 4 (Polish & Robustness)
> **Estimated effort:** 1–2 sessions (~4–6 hours)
> **Prerequisites:** [04-ui-layout.md](04-ui-layout.md), [07-git-integration.md](07-git-integration.md)

## Goal

Integrate tachyonfx to add shader-like visual effects to the editor: focus glow on active panels, smooth diff animations, AI "thinking" indicators, and subtle transitions for state changes. Effects enhance UX without blocking or slowing the render pipeline.

---

## Types & Structures

### Effect Manager

```rust
pub struct EffectManager {
    /// Active effects, keyed by target region or purpose.
    active_effects: Vec<ActiveEffect>,
    /// Effect definitions loaded from config.
    effect_defs: EffectDefinitions,
}

pub struct ActiveEffect {
    pub id: EffectId,
    pub effect: Effect,        // tachyonfx::Effect
    pub target: EffectTarget,
    pub started_at: Instant,
    pub duration: Duration,
}

pub enum EffectTarget {
    /// Apply to the entire area of a panel.
    Panel(PanelId),
    /// Apply to specific lines in the editor.
    EditorLines { buffer_id: BufferId, lines: Range<usize> },
    /// Apply to the status bar.
    StatusBar,
    /// Apply to a specific rect.
    Rect(Rect),
}
```

### Effect Definitions

```rust
pub struct EffectDefinitions {
    pub focus_glow: EffectConfig,
    pub diff_fade_in: EffectConfig,
    pub diff_pulse: EffectConfig,
    pub ai_thinking: EffectConfig,
    pub notification_slide: EffectConfig,
    pub panel_transition: EffectConfig,
}

pub struct EffectConfig {
    pub enabled: bool,
    pub duration_ms: u64,
    pub intensity: f32,  // 0.0 – 1.0
}
```

---

## Implementation Steps

### Step 1: tachyonfx integration scaffold

1. Add `tachyonfx` dependency to `lune-ui`.
2. Create `lune-ui/src/effects.rs` with `EffectManager`.
3. Wire into the render loop:
   - After all widgets render to the ratatui `Buffer`, apply active effects.
   - tachyonfx modifies cell colors/styles in-place on the buffer.
4. `EffectManager::tick(dt)` — advance all active effects, remove expired ones.
5. **Verify:** apply a simple color wash effect to the entire screen, see it render.

### Step 2: Focus glow

1. When a panel gains focus, start a subtle glow effect on its border:
   - Use tachyonfx to brighten the border cells or add a gradient.
   - Duration: 200ms fade-in, hold until focus lost, 200ms fade-out.
2. Use `EffectDsl` to define the glow declaratively.
3. Trigger on `FocusChanged` events.
4. **Verify:** click between panels, see focus glow transition smoothly.

### Step 3: Diff animations

1. When new diff hunks appear:
   - **Fade-in**: new added lines fade from transparent to green highlight over 300ms.
   - **Pulse**: just-changed lines pulse briefly (brightness oscillation, 2 cycles over 500ms).
2. When a hunk is accepted: flash green and fade out.
3. When a hunk is rejected: flash red and fade out.
4. Use line-targeted effects via `EffectTarget::EditorLines`.
5. **Verify:** open a diff with new hunks and see animated diff appearance.

### Step 4: AI thinking indicator

1. When an AI session is actively processing (receiving output):
   - Animate a spinner or pulsing indicator in the status bar.
   - Use tachyonfx color interpolation (e.g., cycling through blue shades).
2. When AI goes idle: fade out the indicator.
3. **Verify:** start AI query, see thinking animation, see it stop when response completes.

### Step 5: Notification animations

1. Notifications slide in from the right edge (or fade in).
2. Auto-dismiss with a fade-out animation.
3. Use tachyonfx's translate or opacity effects.
4. **Verify:** trigger a save notification, see it slide in and fade out.

### Step 6: Panel toggle transitions

1. When a panel opens or closes:
   - Slide animation: panel content slides in from the edge.
   - Or: wipe effect that reveals the panel progressively.
2. Duration: 150ms (fast enough to feel responsive).
3. **Verify:** toggle file tree, see smooth transition instead of instant appear/disappear.

### Step 7: Effect configuration

1. All effects controlled via `EffectDefinitions` in settings.
2. Users can disable individual effects or all effects.
3. Respect terminal capabilities — disable effects on terminals that don't support 256+ colors.
4. Add `--no-effects` CLI flag for performance-sensitive environments.
5. **Verify:** disable effects in config, verify no effects render.

### Step 8: Performance profiling

1. Measure frame time with and without effects.
2. Effects must not add more than 2ms per frame on a 200-row terminal.
3. If effects are too expensive, reduce intensity or skip frames.
4. **Benchmark:** render loop timing with all effects active.

---

## Acceptance Criteria

- [ ] Focus glow transitions smoothly between panels
- [ ] Diffs fade in with visible animation
- [ ] Accept/reject hunks produce visual feedback (flash)
- [ ] AI thinking indicator animates in the status bar
- [ ] Notifications animate in/out
- [ ] Panel toggles have smooth transitions
- [ ] Effects can be disabled globally or individually
- [ ] Effects add <2ms per frame overhead
- [ ] No visual artifacts from effect rendering (clean cleanup on expiry)

---

## Risks

| Risk | Mitigation |
|------|-----------|
| tachyonfx API may not support all desired effects | Work within the available primitives; simplify effects if needed |
| Effects cause visual artifacts on some terminals | Test on multiple terminals (kitty, alacritty, WezTerm, tmux); provide fallbacks |
| Performance impact on slow terminals or SSH connections | Auto-detect latency; disable effects when frame time > 50ms |
| Color manipulation may conflict with theme colors | Effects operate on delta (brighten/dim) rather than absolute colors |
