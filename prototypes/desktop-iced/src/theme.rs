//! The shared theme file (`shared/theme.json`): parsed into owned values
//! the views read every frame, so a reload is just a new `Theme` value.

use std::path::Path;
use std::time::Duration;

use iced::animation::Easing;
use iced::font::{Family, Weight};
use iced::{Color, Font};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawColors {
    bg: Option<String>,
    surface: Option<String>,
    text: Option<String>,
    muted: Option<String>,
    accent: Option<String>,
    danger: Option<String>,
    scrim: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawFonts {
    ui: Option<String>,
    display: Option<String>,
    mono: Option<String>,
    size_base: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawRadius {
    card: Option<f32>,
    button: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawRail {
    card_w: Option<f32>,
    card_h: Option<f32>,
    gap: Option<f32>,
    hover_scale: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawHero {
    height: Option<f32>,
    gradient: Option<Vec<String>>,
    blur: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawMotion {
    fast_ms: Option<u64>,
    normal_ms: Option<u64>,
    easing: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawPlayer {
    controls_bg: Option<String>,
    seek_h: Option<f32>,
    autohide_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawTheme {
    name: Option<String>,
    colors: RawColors,
    fonts: RawFonts,
    radius: RawRadius,
    spacing: Option<Vec<f32>>,
    rail: RawRail,
    hero: RawHero,
    motion: RawMotion,
    player: RawPlayer,
}

/// Everything the views read. Defaults are `lain-dark`.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub bg: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub danger: Color,
    pub scrim: Color,
    pub font_ui: Font,
    pub font_ui_medium: Font,
    pub font_ui_semibold: Font,
    pub font_display: Font,
    pub font_mono: Font,
    pub size_base: f32,
    pub radius_card: f32,
    pub radius_button: f32,
    pub spacing: Vec<f32>,
    pub card_w: f32,
    pub card_h: f32,
    pub gap: f32,
    pub hover_scale: f32,
    pub hero_height: f32,
    pub hero_gradient: (Color, Color),
    pub hero_blur: f32,
    pub fast: Duration,
    pub normal: Duration,
    pub easing: Easing,
    pub controls_bg: Color,
    pub seek_h: f32,
    pub autohide: Duration,
}

impl Theme {
    /// Spacing step `i` of the theme's scale (clamped to the last step).
    pub fn sp(&self, i: usize) -> f32 {
        self.spacing.get(i).or(self.spacing.last()).copied().unwrap_or(8.0)
    }

    pub fn load(path: &Path) -> Result<Theme, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t: RawTheme = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Theme::from_raw(t))
    }

    fn from_raw(t: RawTheme) -> Theme {
        let c = |v: &Option<String>, d: &str| parse_color(v.as_deref().unwrap_or(d)).unwrap_or(Color::BLACK);
        let ui = t.fonts.ui.clone().unwrap_or_else(|| "Inter".into());
        let display = t.fonts.display.clone().unwrap_or_else(|| "Fraunces".into());
        let mono = t.fonts.mono.clone().unwrap_or_else(|| "JetBrains Mono".into());
        let fam = |name: &str| Font { family: Family::Name(intern(name)), ..Font::DEFAULT };
        let grad = t.hero.gradient.clone().unwrap_or_default();
        let g0 = grad.first().and_then(|s| parse_color(s)).unwrap_or(Color::TRANSPARENT);
        let g1 = grad.last().and_then(|s| parse_color(s)).unwrap_or(Color::BLACK);
        let easing_name = t.motion.easing.clone().unwrap_or_else(|| "ease-out-cubic".into());
        Theme {
            name: t.name.unwrap_or_else(|| "unnamed".into()),
            bg: c(&t.colors.bg, "#06040d"),
            surface: c(&t.colors.surface, "#1f1d25"),
            text: c(&t.colors.text, "#ebf3d2"),
            muted: c(&t.colors.muted, "#69676e"),
            accent: c(&t.colors.accent, "#7875bd"),
            danger: c(&t.colors.danger, "#dc7f7f"),
            scrim: c(&t.colors.scrim, "#06040dcc"),
            font_ui: fam(&ui),
            font_ui_medium: Font { weight: Weight::Medium, ..fam(&ui) },
            font_ui_semibold: Font { weight: Weight::Semibold, ..fam(&ui) },
            font_display: Font { weight: Weight::Semibold, ..fam(&display) },
            font_mono: fam(&mono),
            size_base: t.fonts.size_base.unwrap_or(15.0),
            radius_card: t.radius.card.unwrap_or(14.0),
            radius_button: t.radius.button.unwrap_or(10.0),
            spacing: t.spacing.filter(|s| !s.is_empty()).unwrap_or_else(|| vec![4., 8., 12., 16., 24., 32., 48.]),
            card_w: t.rail.card_w.unwrap_or(200.0).max(40.0),
            card_h: t.rail.card_h.unwrap_or(300.0).max(40.0),
            gap: t.rail.gap.unwrap_or(16.0),
            hover_scale: t.rail.hover_scale.unwrap_or(1.06),
            hero_height: t.hero.height.unwrap_or(0.62).clamp(0.1, 1.0),
            hero_gradient: (g0, g1),
            hero_blur: t.hero.blur.unwrap_or(0.0).max(0.0),
            fast: Duration::from_millis(t.motion.fast_ms.unwrap_or(120)),
            normal: Duration::from_millis(t.motion.normal_ms.unwrap_or(220)),
            easing: easing(&easing_name),
            controls_bg: c(&t.player.controls_bg, "#06040d99"),
            seek_h: t.player.seek_h.unwrap_or(4.0).max(1.0),
            autohide: Duration::from_millis(t.player.autohide_ms.unwrap_or(2500)),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::from_raw(RawTheme::default())
    }
}

/// `iced::Font` wants `&'static str` family names; a reload interns the
/// few names a theme has (leaks a handful of bytes per distinct name).
fn intern(name: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if let Some(s) = set.get(name) {
        return s;
    }
    let s: &'static str = Box::leak(name.to_string().into_boxed_str());
    set.insert(s);
    s
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|x| x * 17);
            Some(Color::from_rgb8(d(0)?, d(1)?, d(2)?))
        }
        6 => Some(Color::from_rgb8(v(0)?, v(2)?, v(4)?)),
        8 => Some(Color::from_rgba8(v(0)?, v(2)?, v(4)?, v(6)? as f32 / 255.0)),
        _ => None,
    }
}

/// CSS-ish names to iced's easings. `spring` is a custom curve: an
/// under-damped spring normalised to settle at t = 1 (iced's `Animation`
/// is duration-based, so the spring is time-normalised, not physical).
fn easing(name: &str) -> Easing {
    match name.trim().to_ascii_lowercase().as_str() {
        "linear" => Easing::Linear,
        "ease" | "ease-in-out" => Easing::EaseInOut,
        "ease-in" => Easing::EaseIn,
        "ease-out" => Easing::EaseOut,
        "ease-in-cubic" => Easing::EaseInCubic,
        "ease-out-cubic" => Easing::EaseOutCubic,
        "ease-in-out-cubic" => Easing::EaseInOutCubic,
        "ease-out-quart" => Easing::EaseOutQuart,
        "ease-out-expo" => Easing::EaseOutExpo,
        "ease-out-back" => Easing::EaseOutBack,
        "spring" => Easing::Custom(spring),
        _ => Easing::EaseOutCubic,
    }
}

fn spring(t: f32) -> f32 {
    if t >= 1.0 {
        return 1.0;
    }
    // zeta 0.55, settles (|x-1| < 0.5%) at about t = 1.
    let zeta = 0.55_f32;
    let omega = 9.5_f32;
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    let e = (-zeta * omega * t).exp();
    1.0 - e * ((wd * t).cos() + (zeta * omega / wd) * (wd * t).sin())
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Modification time, for the live-reload poll.
pub fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
