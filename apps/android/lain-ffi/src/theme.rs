//! `lain.ui.theme@1` (docs/THEMES.md, D-249): the same parser and
//! defaults as the desktop app (`apps/desktop/src/theme.rs`), resolved
//! to what Compose draws with: ARGB colors, sizes in dp, and easing as
//! a cubic Bézier. The two shipped themes are the desktop's files,
//! compiled in.

use serde::Deserialize;

use crate::error::LainError;

pub const FORMAT: &str = "lain.ui.theme@1";

/// The shipped themes, from `apps/desktop/themes` (one source of truth).
pub const BUILTIN: &[(&str, &str)] = &[
    ("lain-dark", include_str!("../../../desktop/themes/lain-dark/theme.json")),
    ("paper-light", include_str!("../../../desktop/themes/paper-light/theme.json")),
];

/// Magenta: a color that did not parse, so the mistake shows.
const BAD: i64 = 0xFFFF_00FF;

#[derive(Deserialize)]
#[serde(default)]
struct Theme {
    format: String,
    name: String,
    author: String,
    scheme: String,
    colors: Colors,
    #[serde(rename = "type")]
    fonts: Fonts,
    radius: Radius,
    spacing: Vec<f32>,
    blur: Blur,
    motion: Motion,
    rail: Rail,
    hero: Hero,
    player: Player,
    wallpaper: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct Colors {
    bg: String,
    surface: String,
    raised: String,
    line: String,
    text: String,
    muted: String,
    accent: String,
    on_accent: String,
    danger: String,
    ok: String,
    warn: String,
    scrim: String,
    focus: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct Fonts {
    ui: String,
    display: String,
    mono: String,
    base: f32,
    scale: f32,
}

#[derive(Deserialize)]
#[serde(default)]
struct Radius {
    card: f32,
    button: f32,
    panel: f32,
}

#[derive(Deserialize)]
#[serde(default)]
struct Blur {
    hero: f32,
    panel: f32,
}

#[derive(Deserialize)]
#[serde(default)]
struct Motion {
    fast_ms: f32,
    normal_ms: f32,
    slow_ms: f32,
    easing: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct Rail {
    card_w: f32,
    card_h: f32,
    gap: f32,
    hover_scale: f32,
}

#[derive(Deserialize)]
#[serde(default)]
struct Hero {
    height: f32,
    gradient: Vec<String>,
}

#[derive(Deserialize)]
#[serde(default)]
struct Player {
    controls_bg: String,
    seek_h: f32,
    autohide_ms: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            format: FORMAT.into(),
            name: "builtin".into(),
            author: String::new(),
            scheme: "dark".into(),
            colors: Colors::default(),
            fonts: Fonts::default(),
            radius: Radius::default(),
            spacing: vec![4., 8., 12., 16., 24., 32., 48.],
            blur: Blur::default(),
            motion: Motion::default(),
            rail: Rail::default(),
            hero: Hero::default(),
            player: Player::default(),
            wallpaper: String::new(),
        }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            bg: "oklch(0.118 0.024 295.6)".into(),
            surface: "oklch(0.237 0.015 296.7)".into(),
            raised: "oklch(0.2 0.02 296 / 0.92)".into(),
            line: "oklch(0.95 0.044 118.3 / 0.1)".into(),
            text: "oklch(0.950 0.044 118.3)".into(),
            muted: "oklch(0.62 0.011 299)".into(),
            accent: "oklch(0.596 0.108 284.5)".into(),
            on_accent: "oklch(0.98 0.02 118)".into(),
            danger: "oklch(0.697 0.115 20.4)".into(),
            ok: "oklch(0.75 0.12 150)".into(),
            warn: "oklch(0.8 0.12 80)".into(),
            scrim: "oklch(0.118 0.024 295.6 / 0.8)".into(),
            focus: "oklch(0.596 0.108 284.5)".into(),
        }
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts { ui: "Inter".into(), display: "Fraunces".into(), mono: "JetBrains Mono".into(), base: 15., scale: 1.25 }
    }
}

impl Default for Radius {
    fn default() -> Self {
        Radius { card: 14., button: 10., panel: 16. }
    }
}

impl Default for Blur {
    fn default() -> Self {
        Blur { hero: 0., panel: 24. }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Motion { fast_ms: 120., normal_ms: 220., slow_ms: 420., easing: "ease-out-cubic".into() }
    }
}

impl Default for Rail {
    fn default() -> Self {
        Rail { card_w: 200., card_h: 300., gap: 16., hover_scale: 1.06 }
    }
}

impl Default for Hero {
    fn default() -> Self {
        Hero { height: 0.62, gradient: vec!["oklch(0.118 0.024 295.6 / 0)".into(), "oklch(0.118 0.024 295.6)".into()] }
    }
}

impl Default for Player {
    fn default() -> Self {
        Player { controls_bg: "oklch(0.118 0.024 295.6 / 0.6)".into(), seek_h: 4., autohide_ms: 2500. }
    }
}

/// OKLab → linear sRGB → gamma-encoded sRGB, clamped to the gamut.
fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> (f32, f32, f32) {
    let h = h_deg.to_radians();
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    let enc = |x: f32| {
        let x = x.clamp(0., 1.);
        if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1. / 2.4) - 0.055 }
    };
    (enc(r), enc(g), enc(bl))
}

fn argb(r: f32, g: f32, b: f32, a: f32) -> i64 {
    let q = |v: f32| (v.clamp(0., 1.) * 255.).round() as i64;
    (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}

/// `oklch(L C H)` / `oklch(L C H / A)` or `#rgb`, `#rrggbb`,
/// `#rrggbbaa`, as ARGB (`0xAARRGGBB`).
pub fn parse_color(s: &str) -> Result<i64, String> {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("oklch(").and_then(|r| r.strip_suffix(')')) {
        let (main, alpha) = match inner.split_once('/') {
            Some((m, a)) => (m, Some(a.trim())),
            None => (inner, None),
        };
        let num = |v: &str, pct_scale: f32| -> Result<f32, String> {
            let v = v.trim();
            let n = match v.strip_suffix('%') {
                Some(p) => p.parse::<f32>().map(|x| x / 100. * pct_scale),
                None => v.trim_end_matches("deg").parse::<f32>(),
            };
            n.ok().filter(|x| x.is_finite()).ok_or_else(|| format!("bad number {v:?}"))
        };
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(format!("{t}: oklch needs L C H"));
        }
        let (l, c, h) = (num(parts[0], 1.)?, num(parts[1], 0.4)?, num(parts[2], 360.)?);
        let a = match alpha {
            Some(a) => num(a, 1.)?.clamp(0., 1.),
            None => 1.,
        };
        let (r, g, b) = oklch_to_srgb(l, c, h);
        return Ok(argb(r, g, b, a));
    }
    let h = t.strip_prefix('#').ok_or_else(|| format!("{t}: not a color"))?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{t}: bad hex"));
    }
    let byte = |i: usize, n: usize| -> i64 {
        let v = i64::from_str_radix(&h[i..i + n], 16).unwrap_or(0);
        if n == 1 { v * 17 } else { v }
    };
    Ok(match h.len() {
        3 => (0xFF << 24) | (byte(0, 1) << 16) | (byte(1, 1) << 8) | byte(2, 1),
        6 => (0xFF << 24) | (byte(0, 2) << 16) | (byte(2, 2) << 8) | byte(4, 2),
        8 => (byte(6, 2) << 24) | (byte(0, 2) << 16) | (byte(2, 2) << 8) | byte(4, 2),
        _ => return Err(format!("{t}: bad hex")),
    })
}

/// A named curve or `cubic-bezier(...)` as four control values. Named
/// curves map to their usual Bézier fits; `spring` to a back-out.
pub fn easing(name: &str) -> Vec<f32> {
    if let Some(args) = name.trim().strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')) {
        let v: Vec<f32> = args.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 4 && v.iter().all(|x| x.is_finite()) {
            return v;
        }
    }
    match name.trim() {
        "linear" => vec![0., 0., 1., 1.],
        "ease-in-cubic" => vec![0.32, 0., 0.67, 0.],
        "ease-in-out-cubic" => vec![0.65, 0., 0.35, 1.],
        "ease-out-quad" => vec![0.5, 1., 0.89, 1.],
        "ease-out-expo" => vec![0.16, 1., 0.3, 1.],
        "spring" => vec![0.34, 1.56, 0.64, 1.],
        _ => vec![0.33, 1., 0.68, 1.],
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ThemeColors {
    pub bg: i64,
    pub surface: i64,
    pub raised: i64,
    pub line: i64,
    pub text: i64,
    pub muted: i64,
    pub accent: i64,
    pub on_accent: i64,
    pub danger: i64,
    pub ok: i64,
    pub warn: i64,
    pub scrim: i64,
    pub focus: i64,
    pub controls_bg: i64,
    /// Top → bottom scrim stops over hero art.
    pub hero_stops: Vec<i64>,
}

/// A resolved theme. Colors are ARGB; sizes are px in the file and dp
/// on Android.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ThemeTokens {
    pub name: String,
    pub author: String,
    /// `dark` or `light`.
    pub scheme: String,
    pub colors: ThemeColors,
    pub font_ui: String,
    pub font_display: String,
    pub font_mono: String,
    pub type_base: f32,
    pub type_scale: f32,
    pub radius_card: f32,
    pub radius_button: f32,
    pub radius_panel: f32,
    pub spacing: Vec<f32>,
    pub blur_hero: f32,
    pub blur_panel: f32,
    pub motion_fast_ms: f32,
    pub motion_normal_ms: f32,
    pub motion_slow_ms: f32,
    /// Cubic Bézier control values `x1, y1, x2, y2`.
    pub easing: Vec<f32>,
    pub rail_card_w: f32,
    pub rail_card_h: f32,
    pub rail_gap: f32,
    pub rail_hover_scale: f32,
    pub hero_height: f32,
    pub seek_h: f32,
    pub autohide_ms: f32,
    pub wallpaper: String,
    /// Values that did not parse (drawn magenta); shown in Settings.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BuiltinTheme {
    pub id: String,
    pub json: String,
}

/// Parses a `theme.json`. Missing keys take the defaults; a color that
/// does not parse is magenta and listed in `problems`; bad JSON or
/// another format is refused.
#[uniffi::export]
pub fn parse_theme(json: String) -> Result<ThemeTokens, LainError> {
    let t: Theme =
        serde_json::from_str(&json).map_err(|e| LainError::Invalid { detail: format!("theme.json: {e}") })?;
    if !t.format.is_empty() && t.format != FORMAT {
        return Err(LainError::Unsupported { detail: format!("theme format {:?} (want {FORMAT})", t.format) });
    }
    let mut problems = Vec::new();
    let mut col = |key: &str, s: &str| {
        parse_color(s).unwrap_or_else(|e| {
            problems.push(format!("{key}: {e}"));
            BAD
        })
    };
    let c = &t.colors;
    let colors = ThemeColors {
        bg: col("colors.bg", &c.bg),
        surface: col("colors.surface", &c.surface),
        raised: col("colors.raised", &c.raised),
        line: col("colors.line", &c.line),
        text: col("colors.text", &c.text),
        muted: col("colors.muted", &c.muted),
        accent: col("colors.accent", &c.accent),
        on_accent: col("colors.on_accent", &c.on_accent),
        danger: col("colors.danger", &c.danger),
        ok: col("colors.ok", &c.ok),
        warn: col("colors.warn", &c.warn),
        scrim: col("colors.scrim", &c.scrim),
        focus: col("colors.focus", &c.focus),
        controls_bg: col("player.controls_bg", &t.player.controls_bg),
        hero_stops: t.hero.gradient.iter().map(|s| col("hero.gradient", s)).collect(),
    };
    let wallpaper = if t.wallpaper.contains("..") || t.wallpaper.starts_with('/') || t.wallpaper.contains('\\') {
        problems.push("wallpaper: must stay inside the theme folder".into());
        String::new()
    } else {
        t.wallpaper
    };
    Ok(ThemeTokens {
        name: t.name,
        author: t.author,
        scheme: if t.scheme == "light" { "light".into() } else { "dark".into() },
        colors,
        font_ui: t.fonts.ui,
        font_display: t.fonts.display,
        font_mono: t.fonts.mono,
        type_base: t.fonts.base.max(8.),
        type_scale: t.fonts.scale.clamp(1.01, 2.),
        radius_card: t.radius.card.max(0.),
        radius_button: t.radius.button.max(0.),
        radius_panel: t.radius.panel.max(0.),
        spacing: if t.spacing.is_empty() { vec![4., 8., 12., 16., 24., 32., 48.] } else { t.spacing },
        blur_hero: t.blur.hero.max(0.),
        blur_panel: t.blur.panel.max(0.),
        motion_fast_ms: t.motion.fast_ms.max(0.),
        motion_normal_ms: t.motion.normal_ms.max(0.),
        motion_slow_ms: t.motion.slow_ms.max(0.),
        easing: easing(&t.motion.easing),
        rail_card_w: t.rail.card_w.max(40.),
        rail_card_h: t.rail.card_h.max(40.),
        rail_gap: t.rail.gap.max(0.),
        rail_hover_scale: t.rail.hover_scale.clamp(1., 1.5),
        hero_height: t.hero.height.clamp(0.1, 1.),
        seek_h: t.player.seek_h.max(1.),
        autohide_ms: t.player.autohide_ms.max(500.),
        wallpaper,
        problems,
    })
}

/// The shipped themes: `(id, theme.json)`. The app writes them to its
/// themes folder on first run so they can be edited.
#[uniffi::export]
pub fn builtin_themes() -> Vec<BuiltinTheme> {
    BUILTIN.iter().map(|(id, json)| BuiltinTheme { id: id.to_string(), json: json.to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_matches_known_hex() {
        // The owner's accent #7875bd (the desktop's own check).
        let c = parse_color("oklch(0.596 0.108 284.5)").unwrap();
        let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
        assert!((r - 0x78).abs() <= 1 && (g - 0x75).abs() <= 1 && (b - 0xbd).abs() <= 1, "{c:x}");
        assert_eq!(c >> 24, 0xFF);
        let a = parse_color("oklch(59.6% 0.108 284.5 / 50%)").unwrap();
        assert_eq!(a >> 24, 128);
        assert_eq!(parse_color("#7875bd").unwrap(), 0xFF78_75BD);
        assert_eq!(parse_color("#fff").unwrap(), 0xFFFF_FFFF);
        assert_eq!(parse_color("#11223380").unwrap(), 0x8011_2233);
        for bad in ["nope", "oklch(1 2)", "#ggg", "#12345", "oklch(a b c)", "#éé"] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn builtins_parse_cleanly() {
        let all = builtin_themes();
        assert_eq!(all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["lain-dark", "paper-light"]);
        for t in all {
            let tok = parse_theme(t.json).unwrap();
            assert!(tok.problems.is_empty(), "{}: {:?}", t.id, tok.problems);
            assert_eq!(tok.colors.hero_stops.len(), 2);
        }
        assert_eq!(parse_theme(BUILTIN[1].1.into()).unwrap().scheme, "light");
        assert_eq!(parse_theme(BUILTIN[1].1.into()).unwrap().easing, vec![0.2, 0., 0., 1.]);
    }

    #[test]
    fn partial_bad_and_foreign_themes() {
        let t = parse_theme(r##"{"colors": {"accent": "#ff0000", "bg": "blue"}, "wallpaper": "../x.png"}"##.into())
            .unwrap();
        assert_eq!(t.colors.accent, 0xFFFF_0000);
        assert_eq!(t.colors.bg, BAD);
        assert_eq!(t.problems.len(), 2, "{:?}", t.problems);
        assert_eq!((t.wallpaper.as_str(), t.radius_card, t.scheme.as_str()), ("", 14., "dark"));
        assert!(matches!(parse_theme(r#"{"format": "lain.ui.theme@2"}"#.into()), Err(LainError::Unsupported { .. })));
        assert!(matches!(parse_theme("{".into()), Err(LainError::Invalid { .. })));
        assert_eq!(easing("cubic-bezier(0.1, 0.2, 0.3)"), easing("ease-out-cubic"));
    }
}
