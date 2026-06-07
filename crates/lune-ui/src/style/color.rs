//! Color bridge between ratatui and the `coolor` crate.
//!
//! Provides a single place to convert ratatui `Color` values to and from
//! `coolor::Color`, so higher-level helpers like HSL blending, luminosity
//! adjustment, WCAG contrast, and ANSI downgrades stay consistent across
//! the TUI instead of being hand-rolled in every widget.

use crate::primitives::Color;
use coolor::{AnsiColor, Color as CoolColor, Rgb};

/// Parse a hex color literal into `Color::Rgb` at compile time.
/// Accepts `#rgb`, `#rrggbb`, and the same without a leading `#`.
///
/// This is a `const fn` so it can be used inside `const fn` theme
/// constructors. Invalid input is a compile-time (or startup) panic —
/// this is intentional, since these literals are developer-controlled.
/// For fallible runtime parsing use [`parse_hex`].
///
/// # Panics
/// Panics if `s` is not a valid 3- or 6-digit hex color.
#[must_use]
pub const fn hex(s: &str) -> Color {
    let bytes = s.as_bytes();
    let (start, len) = if !bytes.is_empty() && bytes[0] == b'#' {
        (1, bytes.len() - 1)
    } else {
        (0, bytes.len())
    };
    match len {
        3 => {
            let r = dehex(bytes[start]);
            let g = dehex(bytes[start + 1]);
            let b = dehex(bytes[start + 2]);
            Color::Rgb(r * 17, g * 17, b * 17)
        }
        6 => {
            let r = dehex(bytes[start]) * 16 + dehex(bytes[start + 1]);
            let g = dehex(bytes[start + 2]) * 16 + dehex(bytes[start + 3]);
            let b = dehex(bytes[start + 4]) * 16 + dehex(bytes[start + 5]);
            Color::Rgb(r, g, b)
        }
        _ => panic!("invalid hex color length (expected 3 or 6 hex digits)"),
    }
}

const fn dehex(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("invalid hex digit"),
    }
}

/// Fallible hex color parser accepting `#rgb`, `#rrggbb`, `rgb`, `rrggbb`.
#[must_use]
pub(crate) fn parse_hex(s: &str) -> Option<Color> {
    let trimmed = s.trim();
    let s = trimmed.strip_prefix('#').unwrap_or(trimmed);
    match s.len() {
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            // Duplicate each nibble: 0xF -> 0xFF, 0x5 -> 0x55.
            Some(Color::Rgb(r * 17, g * 17, b * 17))
        }
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

/// Convert a ratatui `Color` into a `coolor::Color`.
///
/// Returns `None` for `Color::Reset`, which has no concrete value.
#[must_use]
#[allow(clippy::missing_const_for_fn)] // early return inside match is not const-fn friendly
pub fn to_coolor(color: Color) -> Option<CoolColor> {
    Some(match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => CoolColor::Rgb(Rgb { r, g, b }),
        Color::Indexed(code) => CoolColor::Ansi(AnsiColor { code }),
        Color::Black => CoolColor::Ansi(AnsiColor { code: 0 }),
        Color::Red => CoolColor::Ansi(AnsiColor { code: 1 }),
        Color::Green => CoolColor::Ansi(AnsiColor { code: 2 }),
        Color::Yellow => CoolColor::Ansi(AnsiColor { code: 3 }),
        Color::Blue => CoolColor::Ansi(AnsiColor { code: 4 }),
        Color::Magenta => CoolColor::Ansi(AnsiColor { code: 5 }),
        Color::Cyan => CoolColor::Ansi(AnsiColor { code: 6 }),
        Color::Gray => CoolColor::Ansi(AnsiColor { code: 7 }),
        Color::DarkGray => CoolColor::Ansi(AnsiColor { code: 8 }),
        Color::LightRed => CoolColor::Ansi(AnsiColor { code: 9 }),
        Color::LightGreen => CoolColor::Ansi(AnsiColor { code: 10 }),
        Color::LightYellow => CoolColor::Ansi(AnsiColor { code: 11 }),
        Color::LightBlue => CoolColor::Ansi(AnsiColor { code: 12 }),
        Color::LightMagenta => CoolColor::Ansi(AnsiColor { code: 13 }),
        Color::LightCyan => CoolColor::Ansi(AnsiColor { code: 14 }),
        Color::White => CoolColor::Ansi(AnsiColor { code: 15 }),
    })
}

/// Convert a `coolor::Color` back to ratatui as truecolor RGB.
#[must_use]
pub fn from_coolor(color: CoolColor) -> Color {
    let rgb = color.rgb();
    Color::Rgb(rgb.r, rgb.g, rgb.b)
}

/// Convert a `coolor::Color` back to ratatui as the nearest 8-bit ANSI
/// palette entry. Use this for terminals that don't advertise truecolor.
#[must_use]
pub fn from_coolor_ansi(color: CoolColor) -> Color {
    Color::Indexed(color.ansi().code)
}

/// Return the `(r, g, b)` tuple for any ratatui color, or `None` if the
/// color has no concrete value (`Color::Reset`).
#[must_use]
pub fn to_rgb_u8(color: Color) -> Option<(u8, u8, u8)> {
    to_coolor(color).map(|c| {
        let rgb = c.rgb();
        (rgb.r, rgb.g, rgb.b)
    })
}

/// Blend `from` toward `to` in HSL space using coolor's natural blend.
///
/// `t` is clamped to `[0, 1]`. `t == 0` returns `from`, `t == 1` returns
/// `to`. Colors without a concrete value are passed through unchanged.
#[must_use]
pub fn blend(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    // Short-circuit endpoints so callers get back the exact input they
    // gave us (the HSL round-trip inside coolor introduces tiny rounding
    // errors that would otherwise bite tests and animations).
    if t <= f32::EPSILON {
        return from;
    }
    if t >= 1.0 - f32::EPSILON {
        return to;
    }
    let Some(c_from) = to_coolor(from) else {
        return from;
    };
    let Some(c_to) = to_coolor(to) else {
        // `to` has no concrete RGB (e.g. `Color::Reset`); blending toward
        // nothing would erase the source, so keep `from` visible instead.
        return from;
    };
    from_coolor(CoolColor::blend(c_from, 1.0 - t, c_to, t))
}

/// Shift a color's HSL luminosity by `delta` (`[-1, 1]`). Positive values
/// lighten, negative values darken. `Color::Reset` is returned unchanged.
///
/// A `delta` of `0.0` returns `color` unchanged — avoids the tiny rounding
/// drift that the HSL round trip in `coolor` introduces on identity.
#[must_use]
pub fn adjust_luminosity(color: Color, delta: f32) -> Color {
    if delta.abs() <= f32::EPSILON {
        return color;
    }
    let Some(cc) = to_coolor(color) else {
        return color;
    };
    let mut hsl = cc.hsl();
    hsl.l = (hsl.l + delta).clamp(0.0, 1.0);
    from_coolor(CoolColor::Hsl(hsl))
}

/// Darken a color toward black by `amount` (`[0, 1]`).
#[must_use]
pub fn darken(color: Color, amount: f32) -> Color {
    adjust_luminosity(color, -amount.abs())
}

/// Lighten a color toward white by `amount` (`[0, 1]`).
#[must_use]
pub fn lighten(color: Color, amount: f32) -> Color {
    adjust_luminosity(color, amount.abs())
}

/// Perceptual luma in `[0, 1]` for a ratatui color, or `None` for `Reset`.
///
/// Uses coolor's BT.2020 coefficients over non-gamma-corrected sRGB —
/// good enough for sorting by brightness, but *not* suitable for WCAG
/// contrast; use [`relative_luminance`] for that.
#[must_use]
pub fn luma(color: Color) -> Option<f32> {
    to_coolor(color).map(CoolColor::luma)
}

/// WCAG 2.0 relative luminance in `[0, 1]`. Each sRGB channel is
/// gamma-linearized per the WCAG formula before being combined with
/// Rec. 709 coefficients.
///
/// Reference: <https://www.w3.org/TR/WCAG20/#relativeluminancedef>
#[must_use]
pub fn relative_luminance(color: Color) -> Option<f32> {
    let (r, g, b) = to_rgb_u8(color)?;
    let linearize = |c: u8| -> f32 {
        let c = f32::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = linearize(r);
    let g = linearize(g);
    let b = linearize(b);
    // Rec. 709 luminance coefficients, expressed as nested mul_add calls
    // for precision and to satisfy `clippy::suboptimal_flops`.
    Some(0.2126f32.mul_add(r, 0.7152f32.mul_add(g, 0.0722 * b)))
}

/// WCAG 2.0 contrast ratio between two colors. Values ≥ 4.5 meet AA for
/// normal text; ≥ 7.0 meet AAA. Returns `None` if either color is
/// `Color::Reset`.
#[must_use]
pub fn contrast_ratio(fg: Color, bg: Color) -> Option<f32> {
    let lfg = relative_luminance(fg)?;
    let lbg = relative_luminance(bg)?;
    let (lighter, darker) = if lfg > lbg { (lfg, lbg) } else { (lbg, lfg) };
    Some((lighter + 0.05) / (darker + 0.05))
}

/// Downgrade an `Rgb` color to its nearest indexed ANSI entry when the
/// terminal does not advertise truecolor (via `$COLORTERM`). Non-Rgb
/// colors are returned unchanged.
///
/// For testing, use [`downgrade`] directly with an explicit flag.
#[must_use]
pub fn downgrade_if_needed(color: Color) -> Color {
    downgrade(color, truecolor_supported())
}

/// Pure downgrade helper: returns `color` unchanged when `truecolor` is
/// `true`, otherwise maps `Color::Rgb` to the nearest indexed 8-bit ANSI
/// color. All non-Rgb variants are returned as-is.
#[must_use]
pub fn downgrade(color: Color, truecolor: bool) -> Color {
    if truecolor {
        return color;
    }
    match color {
        Color::Rgb(..) => to_coolor(color).map_or(color, from_coolor_ansi),
        _ => color,
    }
}

/// Shade `color` by `factor` in `[-1, 1]`.
///
/// Negative values darken toward black, positive values lighten toward
/// white. The shift happens in HSL space so hue and saturation are
/// preserved; only luminosity moves. Convenience around
/// [`adjust_luminosity`] for natural "dim by 30%" / "brighten by 30%"
/// call sites (hover, pressed, disabled states).
#[must_use]
pub fn shade(color: Color, factor: f32) -> Color {
    adjust_luminosity(color, factor.clamp(-1.0, 1.0))
}

/// Dynamically shade `color` toward a target, choosing the direction
/// automatically based on the target's luminance.
///
/// * If `toward` is brighter than `color`, the result is brighter.
/// * If `toward` is darker, the result is darker.
/// * `amount` is clamped to `[0, 1]` and controls how far to move.
///
/// Useful for state variants: `dynamic_shade(fg, bg, 0.3)` produces a
/// dimmed foreground that stays readable regardless of whether the theme
/// is light or dark.
#[must_use]
pub fn dynamic_shade(color: Color, toward: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    if amount <= f32::EPSILON {
        return color;
    }
    let (Some(a), Some(b)) = (relative_luminance(color), relative_luminance(toward)) else {
        return color;
    };
    // Direction: positive = lighten, negative = darken.
    let direction = if b > a { 1.0 } else { -1.0 };
    adjust_luminosity(color, direction * amount)
}

/// Whether the current terminal advertises truecolor via the `COLORTERM`
/// environment variable (`truecolor` or `24bit`).
#[must_use]
pub fn truecolor_supported() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| {
        let v = v.to_ascii_lowercase();
        v == "truecolor" || v == "24bit"
    })
}

// ── Color parsing ─────────────────────────────────────────────────────

/// Parse a hex color string (`"#RRGGBB"`) into a ratatui `Color`.
///
/// Also accepts named colors like `"red"`, `"blue"`, `"reset"`, etc.
pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();

    // Hex literals: #rgb and #rrggbb, with or without the leading #.
    if s.starts_with('#') || is_bare_hex(s) {
        if let Some(c) = parse_hex(s) {
            return Some(c);
        }
    }

    // CSS-style functional notation: rgb(r, g, b) and hsl(h, s%, l%).
    let lower = s.to_ascii_lowercase();
    if let Some(c) = parse_rgb_fn(&lower) {
        return Some(c);
    }
    if let Some(c) = parse_hsl_fn(&lower) {
        return Some(c);
    }

    match lower.as_str() {
        "reset" | "default" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Some(Color::DarkGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

/// Whether `s` looks like a bare (no-`#`) 6-digit hex triplet.
///
/// Only the full 6-digit form is accepted without a leading `#`. The
/// 3-digit bare form is deliberately excluded so short English words made
/// of hex digits (`add`, `dad`, ...) are not silently read as colors;
/// `#abc` still works via the explicit `#` prefix.
fn is_bare_hex(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse CSS-style `rgb(r, g, b)` with integer components in `[0, 255]`.
/// Whitespace is flexible; `rgb(255,0,0)`, `rgb(255 0 0)`, and
/// `rgb( 255 , 0 , 0 )` all parse.
fn parse_rgb_fn(lower: &str) -> Option<Color> {
    let inner = lower.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<u16>().ok()?;
    let g = parts[1].parse::<u16>().ok()?;
    let b = parts[2].parse::<u16>().ok()?;
    if r > 255 || g > 255 || b > 255 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some(Color::Rgb(r as u8, g as u8, b as u8))
}

/// Parse CSS-style `hsl(h, s%, l%)` where `h` is `[0, 360)` degrees and
/// `s`, `l` are percentages. Routes through `coolor::Hsl` so the output
/// matches the rest of the color math.
fn parse_hsl_fn(lower: &str) -> Option<Color> {
    let inner = lower.strip_prefix("hsl(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f32 = parts[0].parse().ok()?;
    let s: f32 = parts[1].trim_end_matches('%').parse().ok()?;
    let l: f32 = parts[2].trim_end_matches('%').parse().ok()?;
    if !(0.0..360.0).contains(&h) || !(0.0..=100.0).contains(&s) || !(0.0..=100.0).contains(&l) {
        return None;
    }
    let hsl = coolor::Hsl::new(h, s / 100.0, l / 100.0);
    Some(from_coolor(coolor::Color::Hsl(hsl)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color() {
        assert_eq!(parse_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("#0000FF"), Some(Color::Rgb(0, 0, 255)));
        assert_eq!(parse_color("#5082DC"), Some(Color::Rgb(80, 130, 220)));
    }

    #[test]
    fn parse_hex_shortform() {
        assert_eq!(parse_color("#fff"), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("#000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_color("#F0a"), Some(Color::Rgb(0xff, 0, 0xaa)));
    }

    #[test]
    fn parse_rgb_functional() {
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("rgb(0 128 255)"), Some(Color::Rgb(0, 128, 255)));
        assert_eq!(parse_color("RGB(12,34,56)"), Some(Color::Rgb(12, 34, 56)));
        assert_eq!(parse_color("rgb(256,0,0)"), None);
        assert_eq!(parse_color("rgb(1,2)"), None);
    }

    #[test]
    fn parse_hsl_functional() {
        // hsl(0, 100%, 50%) == pure red
        let red = parse_color("hsl(0, 100%, 50%)").unwrap();
        let Color::Rgb(r, g, b) = red else {
            panic!("expected rgb");
        };
        assert!(r > 240 && g < 15 && b < 15, "got ({r}, {g}, {b})");
        // hsl(120, 100%, 50%) == pure green
        let green = parse_color("hsl(120 100% 50%)").unwrap();
        let Color::Rgb(r, g, b) = green else {
            panic!("expected rgb");
        };
        assert!(r < 15 && g > 240 && b < 15, "got ({r}, {g}, {b})");
        // Invalid ranges rejected
        assert_eq!(parse_color("hsl(360, 50%, 50%)"), None);
        assert_eq!(parse_color("hsl(0, 150%, 50%)"), None);
    }

    #[test]
    fn parse_named_colors() {
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("Blue"), Some(Color::Blue));
        assert_eq!(parse_color("RESET"), Some(Color::Reset));
        assert_eq!(parse_color("dark_gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("light_red"), Some(Color::LightRed));
    }

    #[test]
    fn parse_invalid_color() {
        assert_eq!(parse_color("#GG0000"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("foobar"), None);
    }

    #[test]
    fn hex_parses_six_digit() {
        assert_eq!(hex("#1e1e2e"), Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(hex("cba6f7"), Color::Rgb(0xcb, 0xa6, 0xf7));
    }

    #[test]
    fn hex_parses_three_digit_shortform() {
        assert_eq!(hex("#fa5"), Color::Rgb(0xff, 0xaa, 0x55));
        assert_eq!(hex("#000"), Color::Rgb(0, 0, 0));
        assert_eq!(hex("#fff"), Color::Rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn parse_hex_rejects_invalid() {
        assert!(parse_hex("not a color").is_none());
        assert!(parse_hex("#12345").is_none());
        assert!(parse_hex("#1234567").is_none());
        assert!(parse_hex("#zzz").is_none());
    }

    #[test]
    fn to_coolor_round_trip_preserves_rgb() {
        let c = Color::Rgb(12, 34, 56);
        let rt = from_coolor(to_coolor(c).unwrap());
        assert_eq!(rt, c);
    }

    #[test]
    fn to_coolor_reset_is_none() {
        assert!(to_coolor(Color::Reset).is_none());
    }

    #[test]
    fn to_coolor_named_colors_map_to_ansi_codes() {
        assert!(matches!(
            to_coolor(Color::Black),
            Some(CoolColor::Ansi(AnsiColor { code: 0 }))
        ));
        assert!(matches!(
            to_coolor(Color::White),
            Some(CoolColor::Ansi(AnsiColor { code: 15 }))
        ));
    }

    #[test]
    fn blend_endpoints_are_exact() {
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(255, 255, 255);
        assert_eq!(blend(a, b, 0.0), a);
        assert_eq!(blend(a, b, 1.0), b);
    }

    #[test]
    fn blend_midpoint_is_in_between() {
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(200, 200, 200);
        let Color::Rgb(r, g, b_) = blend(a, b, 0.5) else {
            panic!("expected rgb");
        };
        assert!((40..=200).contains(&r));
        assert!((40..=200).contains(&g));
        assert!((40..=200).contains(&b_));
    }

    #[test]
    fn blend_handles_reset_gracefully() {
        assert_eq!(blend(Color::Reset, Color::Red, 0.5), Color::Reset);
        assert_eq!(blend(Color::Red, Color::Reset, 0.5), Color::Red);
    }

    #[test]
    fn darken_reduces_luma() {
        let c = Color::Rgb(200, 200, 200);
        let d = darken(c, 0.3);
        assert!(luma(d).unwrap() < luma(c).unwrap());
    }

    #[test]
    fn lighten_increases_luma() {
        let c = Color::Rgb(50, 50, 50);
        let l = lighten(c, 0.3);
        assert!(luma(l).unwrap() > luma(c).unwrap());
    }

    #[test]
    fn contrast_ratio_black_on_white_is_max() {
        let ratio = contrast_ratio(Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255)).unwrap();
        assert!(ratio > 20.0);
    }

    #[test]
    fn contrast_ratio_same_color_is_one() {
        let c = Color::Rgb(123, 45, 67);
        assert!((contrast_ratio(c, c).unwrap() - 1.0).abs() < 0.01);
    }

    #[test]
    fn to_rgb_u8_matches_manual_unpack() {
        assert_eq!(to_rgb_u8(Color::Rgb(9, 8, 7)), Some((9, 8, 7)));
        assert_eq!(to_rgb_u8(Color::Reset), None);
    }

    #[test]
    fn downgrade_passes_through_when_truecolor() {
        let c = Color::Rgb(12, 34, 56);
        assert_eq!(downgrade(c, true), c);
    }

    #[test]
    fn downgrade_maps_rgb_to_indexed_when_not_truecolor() {
        let c = Color::Rgb(255, 0, 0);
        assert!(matches!(downgrade(c, false), Color::Indexed(_)));
    }

    #[test]
    fn downgrade_leaves_named_colors_alone() {
        assert_eq!(downgrade(Color::Red, false), Color::Red);
        assert_eq!(downgrade(Color::Reset, false), Color::Reset);
        assert_eq!(downgrade(Color::Indexed(42), false), Color::Indexed(42));
    }

    #[test]
    fn shade_positive_lightens_negative_darkens() {
        let c = Color::Rgb(120, 120, 120);
        assert!(luma(shade(c, 0.3)).unwrap() > luma(c).unwrap());
        assert!(luma(shade(c, -0.3)).unwrap() < luma(c).unwrap());
    }

    #[test]
    fn shade_zero_is_identity() {
        let c = Color::Rgb(120, 50, 200);
        assert_eq!(shade(c, 0.0), c);
    }

    #[test]
    fn dynamic_shade_follows_target_direction() {
        let fg = Color::Rgb(220, 220, 220);
        // Dark bg → fg should darken toward the background.
        let dark_bg = Color::Rgb(20, 20, 20);
        let shaded = dynamic_shade(fg, dark_bg, 0.3);
        assert!(relative_luminance(shaded).unwrap() < relative_luminance(fg).unwrap());

        // Light bg → fg should lighten (even though it's already light).
        let fg2 = Color::Rgb(30, 30, 30);
        let light_bg = Color::Rgb(240, 240, 240);
        let shaded2 = dynamic_shade(fg2, light_bg, 0.3);
        assert!(relative_luminance(shaded2).unwrap() > relative_luminance(fg2).unwrap());
    }

    #[test]
    fn dynamic_shade_zero_amount_is_identity() {
        let c = Color::Rgb(100, 50, 200);
        assert_eq!(dynamic_shade(c, Color::Rgb(255, 255, 255), 0.0), c);
    }

    #[test]
    fn dynamic_shade_handles_reset_gracefully() {
        let c = Color::Rgb(100, 100, 100);
        assert_eq!(dynamic_shade(c, Color::Reset, 0.5), c);
        assert_eq!(dynamic_shade(Color::Reset, c, 0.5), Color::Reset);
    }
}
