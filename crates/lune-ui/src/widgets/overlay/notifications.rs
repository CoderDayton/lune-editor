//! Toast notification system — queue, levels, animation helpers.

use std::time::{Duration, Instant};

use crate::primitives::{Buffer, Clear, Color, Line, Modifier, Rect, Span, Style, Widget};
use crate::style::color as color_util;
use crate::theme::Theme;

use super::util::truncate_inline_text;

/// Severity level for notifications. Ordered from least to most alarming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationLevel {
    /// Positive outcome ("Saved", "Committed", "Copied").
    Success,
    /// Informational message (neutral status updates).
    Info,
    /// Warning — user should notice but nothing is broken.
    Warning,
    /// Error — something actually failed.
    Error,
}

impl NotificationLevel {
    /// Single-glyph icon shown on the left of the toast body.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Success => "✓",
            Self::Info => "●",
            Self::Warning => "⚠",
            Self::Error => "✕",
        }
    }
}

/// Centralized tuning for the notification subsystem.
///
/// Keeps the timeouts, queue caps, width, and fade window in one place
/// instead of scattering magic numbers across `notify()` / `render` /
/// `prune_notifications`.
#[derive(Clone, Copy, Debug)]
pub struct NotificationConfig {
    /// Maximum number of toasts rendered simultaneously.
    pub max_visible: usize,
    /// Hard cap on queue depth. Older toasts are dropped if exceeded.
    pub max_queue: usize,
    /// Total width of the toast frame (including borders).
    pub width: u16,
    /// Maximum body rows per toast before truncating the message.
    pub max_body_rows: u16,
    /// Time a `Success` toast lives before expiry.
    pub ttl_success: Duration,
    /// Time an `Info` toast lives before expiry.
    pub ttl_info: Duration,
    /// Time a `Warning` toast lives before expiry.
    pub ttl_warning: Duration,
    /// Time an `Error` toast lives before expiry (stays longest so the
    /// user can actually read it).
    pub ttl_error: Duration,
    /// Duration of the fade-out right before expiry.
    pub fade: Duration,
    /// Duration of the slide-and-fade-in entrance when a toast appears.
    pub enter: Duration,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            max_visible: 5,
            max_queue: 50,
            width: 46,
            max_body_rows: 3,
            ttl_success: Duration::from_millis(2500),
            ttl_info: Duration::from_millis(3500),
            ttl_warning: Duration::from_millis(6000),
            ttl_error: Duration::from_millis(10_000),
            fade: Duration::from_millis(700),
            enter: Duration::from_millis(220),
        }
    }
}

impl NotificationConfig {
    /// TTL for a specific severity level.
    #[must_use]
    pub const fn ttl_for(&self, level: NotificationLevel) -> Duration {
        match level {
            NotificationLevel::Success => self.ttl_success,
            NotificationLevel::Info => self.ttl_info,
            NotificationLevel::Warning => self.ttl_warning,
            NotificationLevel::Error => self.ttl_error,
        }
    }
}

/// A toast notification message. Equal consecutive notifications are
/// coalesced into a single entry with an incrementing `count`.
#[derive(Clone, Debug)]
pub struct Notification {
    /// The message text.
    pub message: String,
    /// Severity level.
    pub level: NotificationLevel,
    /// When the notification was (last) created. Reset on dedup so a
    /// repeated message refreshes its TTL instead of expiring early.
    pub created: Instant,
    /// When the notification first appeared. Unlike `created`, this is
    /// *not* reset on dedup — so the entrance animation plays exactly
    /// once and a rapidly-repeated message doesn't re-slide on each
    /// `×N` bump.
    pub spawned: Instant,
    /// How many times this message has been pushed consecutively.
    /// Rendered as a trailing `×N` suffix when > 1.
    pub count: u32,
}

impl Notification {
    /// Whether this notification has lived past its TTL.
    #[must_use]
    pub fn is_expired(&self, cfg: &NotificationConfig) -> bool {
        self.created.elapsed() >= cfg.ttl_for(self.level)
    }

    /// Remaining vitality in `[0.0, 1.0]`. `1.0` while fresh, linearly
    /// decaying over the final `cfg.fade` window, then `0.0` at expiry.
    #[must_use]
    pub fn vitality(&self, cfg: &NotificationConfig) -> f32 {
        let elapsed = self.created.elapsed();
        let ttl = cfg.ttl_for(self.level);
        if elapsed >= ttl {
            return 0.0;
        }
        let fade_start = ttl.saturating_sub(cfg.fade);
        if elapsed <= fade_start {
            return 1.0;
        }
        let remaining = ttl.saturating_sub(elapsed).as_secs_f32();
        let fade_secs = cfg.fade.as_secs_f32().max(f32::EPSILON);
        (remaining / fade_secs).clamp(0.0, 1.0)
    }

    /// Entrance progress in `[0.0, 1.0]`, eased out (cubic) over
    /// `cfg.enter` from `spawned`. `0.0` the instant the toast appears,
    /// `1.0` once it has fully slid and faded in. Decoupled from
    /// `vitality` (the exit fade) so the two animations never interfere.
    #[must_use]
    pub fn entrance(&self, cfg: &NotificationConfig) -> f32 {
        let elapsed = self.spawned.elapsed().as_secs_f32();
        let dur = cfg.enter.as_secs_f32().max(f32::EPSILON);
        let t = (elapsed / dur).clamp(0.0, 1.0);
        // Ease-out cubic: fast start, gentle settle.
        1.0 - (1.0 - t).powi(3)
    }

    /// Entrance progress with a slight overshoot past `1.0` before it
    /// settles — an `ease-out-back` curve over `cfg.enter` from `spawned`.
    /// Unlike [`entrance`](Self::entrance) it can momentarily exceed `1.0`;
    /// that excess drives the slide overshoot and the accent brightness pop.
    #[must_use]
    pub fn entrance_back(&self, cfg: &NotificationConfig) -> f32 {
        let elapsed = self.spawned.elapsed().as_secs_f32();
        let dur = cfg.enter.as_secs_f32().max(f32::EPSILON);
        let t = (elapsed / dur).clamp(0.0, 1.0);
        ease_out_back(t)
    }
}

fn blend_toward(color: Color, tr: u8, tg: u8, tb: u8, t: f32) -> Color {
    color_util::blend(color, Color::Rgb(tr, tg, tb), t)
}

/// Ease-out-back easing — starts slow, overshoots past 1.0, then settles.
///
/// Produces values outside `[0, 1]` (specifically > 1) near t ≈ 0.7,
/// which drives the slide-past-then-settle effect on toast entrance.
fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.701_58;
    const C3: f32 = C1 + 1.0;
    let tm1 = t - 1.0;
    (C1 * tm1).mul_add(tm1, (C3 * tm1 * tm1).mul_add(tm1, 1.0))
}

/// Horizontal x shift for a toast entrance slide, in columns.
///
/// Returns how many columns to the right of the settled anchor the toast
/// is: positive = still sliding in, 0 = settled, negative = overshot past
/// the anchor (the ease-out-back overshoot effect).
///
/// `eob` is the output of [`ease_out_back`] at the current entrance t.
#[allow(clippy::cast_possible_truncation)]
fn entrance_x_shift(eob: f32, max_shift: u16) -> i32 {
    let settled = (f32::from(max_shift) * eob).round() as i32;
    i32::from(max_shift) - settled
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn toast_x_shift(presence: f32, max_shift: u16) -> u16 {
    let p = presence.clamp(0.0, 1.0);
    ((1.0 - p) * f32::from(max_shift)).round() as u16
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn slot_advance(entrance: f32, full: u16) -> u16 {
    let e = entrance.clamp(0.0, 1.0);
    (e * f32::from(full)).round() as u16
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn render_notifications(
    area: Rect,
    buf: &mut Buffer,
    notifications: &[Notification],
    config: &NotificationConfig,
    theme: &Theme,
) {
    // Each toast is a 2-row strip (text row + a one-row base); the slot
    // adds a 1-row gap between stacked toasts.
    const TOAST_H: u16 = 2;
    const SLOT_H: u16 = TOAST_H + 1;

    if notifications.is_empty() || area.width < 16 || area.height < 5 {
        return;
    }

    let toast_w = config.width.min(area.width.saturating_sub(2));
    if toast_w < 16 {
        return;
    }

    // Anchor bottom-right with a 1-col / 1-row margin.
    let right_margin: u16 = 1;
    let bottom_margin: u16 = 1;
    let x = area.x + area.width.saturating_sub(toast_w + right_margin);
    let mut next_bottom = area.y + area.height.saturating_sub(bottom_margin);

    let visible_count = notifications.len().min(config.max_visible);
    let hidden = notifications.len().saturating_sub(visible_count);

    // Horizontal travel for the slide-in / slide-out, capped so a toast
    // mid-animation still shows something rather than vanishing.
    let max_shift = (toast_w / 4).max(2);

    // Render from newest (end of vec) upward.
    let (top_of_stack_y, top_shift) = {
        let mut top = next_bottom;
        let mut top_shift = 0u16;
        for notif in notifications.iter().rev().take(visible_count) {
            if next_bottom < area.y + TOAST_H {
                break;
            }
            let entrance = notif.entrance(config);
            let eob = notif.entrance_back(config);
            let vitality = notif.vitality(config);
            let presence = entrance.min(vitality);
            // The entrance slides in from the right edge and overshoots a
            // hair past flush (a negative shift => left of the anchor)
            // before settling; the exit slides back out to the right as
            // vitality decays. The two phases never overlap in time, so
            // summing their shifts is safe. Everything is clamped to the
            // visible area so off-screen cells are never written (direct
            // buffer indexing would otherwise panic).
            let entrance_shift = entrance_x_shift(eob, max_shift);
            let exit_shift = i32::from(toast_x_shift(vitality, max_shift));
            let shift_i = entrance_shift + exit_shift;
            let min_x = i32::from(area.x);
            let max_x = i32::from(area.x + area.width.saturating_sub(1));
            let toast_x = (i32::from(x) + shift_i).clamp(min_x, max_x) as u16;
            let avail_w = (area.x + area.width).saturating_sub(toast_x);
            let draw_w = toast_w.min(avail_w);
            let rect = Rect::new(
                toast_x,
                next_bottom.saturating_sub(TOAST_H),
                draw_w,
                TOAST_H,
            );
            // `pop` is the overshoot beyond a settled entrance; it drives a
            // one-shot brightness pulse on the accent so the toast reads as
            // "snapping" into place even though cell-grid travel is too
            // coarse to show a sub-row positional overshoot.
            let pop = (eob - 1.0).max(0.0);
            render_single_toast(rect, buf, notif, theme, presence, pop);
            top = rect.y;
            // Topmost visible toast wins the slide offset for the overflow
            // label below, so the label tracks it instead of the static anchor.
            top_shift = toast_x.saturating_sub(x);
            // The newest toast's slot grows from collapsed to full over
            // its entrance, so the stack above rises into place smoothly
            // instead of snapping up the instant the toast appears.
            let advance = slot_advance(entrance, SLOT_H).max(1);
            next_bottom = next_bottom.saturating_sub(advance);
        }
        (top, top_shift)
    };

    // "+N more" overflow indicator above the topmost visible toast.
    if hidden > 0 && top_of_stack_y > area.y {
        let label = format!("+{hidden} more");
        let label_w = u16::try_from(label.chars().count())
            .unwrap_or(u16::MAX)
            .min(toast_w);
        // Pin the label to the topmost toast's right edge as it slides,
        // clamped so it never writes past the visible area.
        let label_x = (x + top_shift + toast_w.saturating_sub(label_w))
            .min(area.x + area.width.saturating_sub(label_w));
        let label_y = top_of_stack_y.saturating_sub(1);
        let label_rect = Rect::new(label_x, label_y, label_w, 1);
        Clear.render(label_rect, buf);
        Line::from(Span::styled(label, Style::new().fg(theme.fg_muted))).render(label_rect, buf);
    }
}

fn render_single_toast(
    rect: Rect,
    buf: &mut Buffer,
    notif: &Notification,
    theme: &Theme,
    presence: f32,
    pop: f32,
) {
    // Accent bar (1) + one padding column on each side of the text.
    const BAR_AND_PAD: u16 = 2;
    const RIGHT_PAD: u16 = 1;

    if rect.width < 8 || rect.height < 2 {
        return;
    }

    let accent = match notif.level {
        NotificationLevel::Success => theme.notif_success,
        NotificationLevel::Info => theme.notif_info,
        NotificationLevel::Warning => theme.notif_warn,
        NotificationLevel::Error => theme.notif_error,
    };

    // Cross-fade every element toward the editor background by `1 - presence`
    // so the toast dissolves in and out instead of snapping on and off.
    let (br, bg, bb) = color_util::to_rgb_u8(theme.bg).unwrap_or((0, 0, 0));
    let fade = 1.0 - presence;
    let panel_bg = blend_toward(theme.notif_bg, br, bg, bb, fade);
    let text_fg = blend_toward(theme.notif_fg, br, bg, bb, fade * 0.85);
    let mut accent_fg = blend_toward(accent, br, bg, bb, fade);
    // On arrival the accent over-brightens past its resting color, then
    // settles — the visible half of the entrance "snap".
    if pop > 0.0 {
        accent_fg = color_util::lighten(accent_fg, (pop * 2.0).min(0.35));
    }

    // Opaque panel: clear, then paint the background across every cell.
    Clear.render(rect, buf);
    for dy in 0..rect.height {
        for dx in 0..rect.width {
            buf[(rect.x + dx, rect.y + dy)]
                .set_char(' ')
                .set_bg(panel_bg);
        }
    }

    // Thick left bar — `▌` (left half-block, ~4px) down the full height.
    for dy in 0..rect.height {
        buf[(rect.x, rect.y + dy)]
            .set_char('▌')
            .set_fg(accent_fg)
            .set_bg(panel_bg);
    }

    // Text area: right of the bar, one padding column on each side.
    let inner_x = rect.x + BAR_AND_PAD;
    let inner_w = rect.width.saturating_sub(BAR_AND_PAD + RIGHT_PAD);
    if inner_w == 0 {
        return;
    }

    let count_suffix = if notif.count > 1 {
        format!(" ×{}", notif.count)
    } else {
        String::new()
    };
    let count_w = u16::try_from(count_suffix.chars().count()).unwrap_or(u16::MAX);
    // Message budget = inner width minus icon (1) + gap (2) + the count.
    let max_msg_w = inner_w.saturating_sub(3 + count_w);
    let message = truncate_inline_text(&notif.message, max_msg_w as usize);
    let msg_w = u16::try_from(message.chars().count()).unwrap_or(u16::MAX);

    // Left-align the icon + message + count block against the bar, on the
    // upper row so the one-row base reads as breathing space below.
    let content_w = (3 + msg_w + count_w).min(inner_w);
    let text_x = inner_x;
    let text_y = rect.y + rect.height.saturating_sub(1) / 2;

    let accent_style = Style::new().fg(accent_fg).add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(notif.level.icon(), accent_style),
        Span::from("  "),
        Span::styled(
            message,
            Style::new().fg(text_fg).add_modifier(Modifier::BOLD),
        ),
    ];
    if !count_suffix.is_empty() {
        spans.push(Span::styled(count_suffix, accent_style));
    }
    Line::from(spans).render(Rect::new(text_x, text_y, content_w, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_single_toast_draws_bar_and_message() {
        let theme = Theme::dark();
        let notif = Notification {
            message: "Saved".to_string(),
            level: NotificationLevel::Success,
            created: Instant::now(),
            spawned: Instant::now(),
            count: 1,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        // Fully present, no entrance overshoot.
        render_single_toast(area, &mut buf, &notif, &theme, 1.0, 0.0);
        let text: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains('▌'), "toast must draw the left accent bar");
        assert!(text.contains("Saved"), "toast must show the message");
        assert!(text.contains('✓'), "success toast shows the ✓ icon");
    }
    use crate::widgets::overlay::OverlayState;
    use std::time::{Duration, Instant};

    #[test]
    fn notification_prune() {
        let mut overlay = OverlayState::default();
        overlay.notify("test", NotificationLevel::Info);
        assert_eq!(overlay.notifications.len(), 1);
        overlay.prune_notifications();
        // Should still be there (just created).
        assert_eq!(overlay.notifications.len(), 1);
    }

    #[test]
    fn notifications_collapse_multiline_messages_to_one_row() {
        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        let notification = Notification {
            message: "Clipboard was dropped very quickly after writing (9ms)\nConsider keeping `Clipboard` in more persistent state somewhere.".to_string(),
            level: NotificationLevel::Error,
            created: Instant::now(),
            spawned: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            count: 1,
        };

        let config = NotificationConfig::default();
        render_notifications(area, &mut buf, &[notification], &config, &theme);

        let rows: Vec<String> = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
                    .collect::<String>()
            })
            .collect();

        assert_eq!(
            rows.iter()
                .filter(|row| row.contains("Clipboard was dropped very quickly"))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.contains("Consider keeping `Clipboard`"))
                .count(),
            0
        );
    }

    #[test]
    fn vitality_fresh_is_one() {
        let cfg = NotificationConfig::default();
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now(),
            spawned: Instant::now(),
            count: 1,
        };
        assert!((notif.vitality(&cfg) - 1.0).abs() < 0.01);
    }

    #[test]
    fn vitality_at_expiry_is_zero() {
        let cfg = NotificationConfig::default();
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now()
                .checked_sub(std::time::Duration::from_secs(30))
                .unwrap(),
            spawned: Instant::now(),
            count: 1,
        };
        assert!(notif.vitality(&cfg) <= 0.0);
    }

    #[test]
    fn vitality_during_fade_is_between() {
        let cfg = NotificationConfig::default();
        // Info TTL is 3500ms with a 700ms fade window, so placing the
        // creation timestamp 3100ms ago puts us mid-fade.
        let fade_midpoint = cfg.ttl_info.saturating_sub(cfg.fade / 2);
        let notif = Notification {
            message: "test".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now().checked_sub(fade_midpoint).unwrap(),
            spawned: Instant::now(),
            count: 1,
        };
        let v = notif.vitality(&cfg);
        assert!(
            v > 0.0 && v < 1.0,
            "vitality during fade should be 0 < {v} < 1"
        );
    }

    #[test]
    fn notify_dedups_consecutive_identical_messages() {
        let mut overlay = OverlayState::default();
        overlay.notify("Saved", NotificationLevel::Success);
        overlay.notify("Saved", NotificationLevel::Success);
        overlay.notify("Saved", NotificationLevel::Success);
        assert_eq!(overlay.notifications.len(), 1);
        assert_eq!(overlay.notifications[0].count, 3);
    }

    #[test]
    fn notify_does_not_dedup_different_levels() {
        let mut overlay = OverlayState::default();
        overlay.notify("X", NotificationLevel::Info);
        overlay.notify("X", NotificationLevel::Warning);
        assert_eq!(overlay.notifications.len(), 2);
        assert_eq!(overlay.notifications[0].count, 1);
        assert_eq!(overlay.notifications[1].count, 1);
    }

    #[test]
    fn notify_does_not_dedup_non_adjacent_repeats() {
        let mut overlay = OverlayState::default();
        overlay.notify("A", NotificationLevel::Info);
        overlay.notify("B", NotificationLevel::Info);
        overlay.notify("A", NotificationLevel::Info);
        assert_eq!(overlay.notifications.len(), 3);
    }

    #[test]
    fn notify_enforces_max_queue_by_dropping_oldest() {
        let mut overlay = OverlayState::default();
        overlay.notification_config.max_queue = 3;
        for i in 0..6 {
            overlay.notify(format!("m{i}"), NotificationLevel::Info);
        }
        assert_eq!(overlay.notifications.len(), 3);
        assert_eq!(overlay.notifications[0].message, "m3");
        assert_eq!(overlay.notifications[2].message, "m5");
    }

    #[test]
    fn dismiss_all_clears_every_notification() {
        let mut overlay = OverlayState::default();
        overlay.notify("a", NotificationLevel::Info);
        overlay.notify("b", NotificationLevel::Warning);
        overlay.notify("c", NotificationLevel::Error);
        overlay.dismiss_all_notifications();
        assert!(overlay.notifications.is_empty());
    }

    #[test]
    fn notify_helpers_produce_expected_levels() {
        let mut overlay = OverlayState::default();
        overlay.notify_success("s");
        overlay.notify_info("i");
        overlay.notify_warn("w");
        overlay.notify_error("e");
        let levels: Vec<NotificationLevel> =
            overlay.notifications.iter().map(|n| n.level).collect();
        assert_eq!(
            levels,
            vec![
                NotificationLevel::Success,
                NotificationLevel::Info,
                NotificationLevel::Warning,
                NotificationLevel::Error,
            ]
        );
    }

    #[test]
    fn notification_level_icons_are_all_distinct() {
        let icons = [
            NotificationLevel::Success.icon(),
            NotificationLevel::Info.icon(),
            NotificationLevel::Warning.icon(),
            NotificationLevel::Error.icon(),
        ];
        let unique: std::collections::BTreeSet<_> = icons.iter().collect();
        assert_eq!(unique.len(), icons.len());
    }

    #[test]
    fn per_severity_ttl_matches_config() {
        let cfg = NotificationConfig::default();
        assert!(cfg.ttl_for(NotificationLevel::Success) < cfg.ttl_for(NotificationLevel::Info));
        assert!(cfg.ttl_for(NotificationLevel::Info) < cfg.ttl_for(NotificationLevel::Warning));
        assert!(cfg.ttl_for(NotificationLevel::Warning) < cfg.ttl_for(NotificationLevel::Error));
    }

    #[test]
    fn prune_respects_per_severity_ttl() {
        let mut overlay = OverlayState::default();
        let cfg = overlay.notification_config;
        // A Success message whose TTL has expired should be pruned.
        overlay.notifications.push(Notification {
            message: "old success".to_string(),
            level: NotificationLevel::Success,
            created: Instant::now()
                .checked_sub(cfg.ttl_for(NotificationLevel::Success) + Duration::from_secs(1))
                .unwrap(),
            spawned: Instant::now(),
            count: 1,
        });
        // An Error message created at the same ancient timestamp should
        // still be alive because Error has a much longer TTL.
        let ancient = Instant::now()
            .checked_sub(cfg.ttl_for(NotificationLevel::Success) + Duration::from_secs(1))
            .unwrap();
        overlay.notifications.push(Notification {
            message: "still-live error".to_string(),
            level: NotificationLevel::Error,
            created: ancient,
            spawned: ancient,
            count: 1,
        });
        overlay.prune_notifications();
        assert_eq!(overlay.notifications.len(), 1);
        assert_eq!(overlay.notifications[0].level, NotificationLevel::Error);
    }

    #[test]
    fn notify_at_sets_spawned_equal_to_created_on_new() {
        let mut overlay = OverlayState::default();
        let t0 = Instant::now();
        overlay.notify_at("Hi", NotificationLevel::Info, t0);
        let n = &overlay.notifications[0];
        assert_eq!(n.spawned, t0);
        assert_eq!(n.created, t0);
    }

    #[test]
    fn notify_dedup_preserves_spawned_resets_created() {
        let mut overlay = OverlayState::default();
        let t0 = Instant::now();
        overlay.notify_at("Saved", NotificationLevel::Success, t0);
        let t1 = t0 + Duration::from_millis(500);
        overlay.notify_at("Saved", NotificationLevel::Success, t1);
        assert_eq!(overlay.notifications.len(), 1);
        let n = &overlay.notifications[0];
        assert_eq!(n.count, 2);
        assert_eq!(n.spawned, t0, "entrance clock must survive dedup");
        assert_eq!(n.created, t1, "TTL clock must reset on dedup");
    }

    #[test]
    fn entrance_is_zero_when_fresh() {
        let cfg = NotificationConfig::default();
        let notif = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now(),
            spawned: Instant::now(),
            count: 1,
        };
        assert!(notif.entrance(&cfg) < 0.05, "fresh toast starts hidden");
    }

    #[test]
    fn entrance_completes_after_enter_window() {
        let cfg = NotificationConfig::default();
        let notif = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now(),
            spawned: Instant::now()
                .checked_sub(cfg.enter + Duration::from_millis(50))
                .unwrap(),
            count: 1,
        };
        assert!((notif.entrance(&cfg) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn entrance_midway_is_eased_past_half() {
        let cfg = NotificationConfig::default();
        let notif = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created: Instant::now(),
            spawned: Instant::now().checked_sub(cfg.enter / 2).unwrap(),
            count: 1,
        };
        let e = notif.entrance(&cfg);
        assert!(
            e > 0.5 && e < 1.0,
            "ease-out: halfway in time should be past halfway in progress, got {e}"
        );
    }

    #[test]
    fn toast_x_shift_full_presence_is_flush() {
        assert_eq!(toast_x_shift(1.0, 8), 0);
    }

    #[test]
    fn toast_x_shift_zero_presence_is_max() {
        assert_eq!(toast_x_shift(0.0, 8), 8);
    }

    #[test]
    fn toast_x_shift_is_proportional_and_clamped() {
        assert_eq!(toast_x_shift(0.5, 8), 4);
        assert_eq!(toast_x_shift(-1.0, 8), 8);
        assert_eq!(toast_x_shift(2.0, 8), 0);
    }

    #[test]
    fn slot_advance_grows_with_entrance() {
        assert_eq!(slot_advance(0.0, 4), 0);
        assert_eq!(slot_advance(1.0, 4), 4);
        assert_eq!(slot_advance(0.5, 4), 2);
    }

    #[test]
    fn has_active_notifications_tracks_queue() {
        let mut overlay = OverlayState::default();
        assert!(!overlay.has_active_notifications());
        overlay.notify_info("hi");
        assert!(overlay.has_active_notifications());
        overlay.dismiss_all_notifications();
        assert!(!overlay.has_active_notifications());
    }

    #[test]
    fn ease_out_back_hits_endpoints() {
        assert!((ease_out_back(0.0) - 0.0).abs() < 1e-4);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn ease_out_back_overshoots_past_one() {
        // The ease-out-back curve must overshoot past 1.0 somewhere in (0, 1).
        let peak = (0u8..=100)
            .map(|i| ease_out_back(f32::from(i) / 100.0))
            .fold(0.0f32, f32::max);
        assert!(
            peak > 1.0,
            "ease_out_back must overshoot past 1.0, got peak {peak}"
        );
    }

    #[test]
    fn entrance_x_shift_slides_in_then_overshoots() {
        // Far from settled: shoved toward the right edge.
        assert_eq!(entrance_x_shift(0.0, 8), 8);
        // Settled: flush with the anchor.
        assert_eq!(entrance_x_shift(1.0, 8), 0);
        // Overshooting past 1.0: nudged a column left of the anchor.
        assert!(entrance_x_shift(1.08, 8) < 0);
    }
}
