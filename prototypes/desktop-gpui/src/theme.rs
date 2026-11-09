//! The theme data file (`shared/theme.json`). Every key has a default so a
//! partial file still loads; colors are `#rrggbb` or `#rrggbbaa`.

use gpui::{Hsla, Rgba};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub colors: Colors,
    pub fonts: Fonts,
    pub radius: Radius,
    pub spacing: Vec<f32>,
    pub rail: Rail,
    pub hero: Hero,
    pub motion: Motion,
    pub player: PlayerTheme,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub bg: String,
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub danger: String,
    pub scrim: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Fonts {
    pub ui: String,
    pub display: String,
    pub mono: String,
    pub size_base: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Radius {
    pub card: f32,
    pub button: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Rail {
    pub card_w: f32,
    pub card_h: f32,
    pub gap: f32,
    pub hover_scale: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Hero {
    /// Fraction of the window height.
    pub height: f32,
    /// Top → bottom scrim stops.
    pub gradient: Vec<String>,
    /// Gaussian blur radius of the backdrop, in pixels.
    pub blur: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Motion {
    pub fast_ms: f32,
    pub normal_ms: f32,
    pub easing: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PlayerTheme {
    pub controls_bg: String,
    pub seek_h: f32,
    pub autohide_ms: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            name: "builtin".into(),
            colors: Colors::default(),
            fonts: Fonts::default(),
            radius: Radius::default(),
            spacing: vec![4., 8., 12., 16., 24., 32., 48.],
            rail: Rail::default(),
            hero: Hero::default(),
            motion: Motion::default(),
            player: PlayerTheme::default(),
        }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            bg: "#06040d".into(),
            surface: "#1f1d25".into(),
            text: "#ebf3d2".into(),
            muted: "#69676e".into(),
            accent: "#7875bd".into(),
            danger: "#dc7f7f".into(),
            scrim: "#06040dcc".into(),
        }
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts { ui: "Inter".into(), display: "Fraunces".into(), mono: "JetBrains Mono".into(), size_base: 15. }
    }
}

impl Default for Radius {
    fn default() -> Self {
        Radius { card: 14., button: 10. }
    }
}

impl Default for Rail {
    fn default() -> Self {
        Rail { card_w: 200., card_h: 300., gap: 16., hover_scale: 1.06 }
    }
}

impl Default for Hero {
    fn default() -> Self {
        Hero { height: 0.62, gradient: vec!["#06040d00".into(), "#06040dff".into()], blur: 0. }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Motion { fast_ms: 120., normal_ms: 220., easing: "ease-out-cubic".into() }
    }
}

impl Default for PlayerTheme {
    fn default() -> Self {
        PlayerTheme { controls_bg: "#06040d99".into(), seek_h: 4., autohide_ms: 2500. }
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`; magenta on garbage so it is visible.
pub fn color(s: &str) -> Hsla {
    let h = s.trim().trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32 / 255.;
    let rgba = match h.len() {
        3 => {
            let c = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).unwrap_or(255) as f32 / 255.;
            Rgba { r: c(0), g: c(1), b: c(2), a: 1. }
        }
        6 => Rgba { r: p(0), g: p(2), b: p(4), a: 1. },
        8 => Rgba { r: p(0), g: p(2), b: p(4), a: p(6) },
        _ => Rgba { r: 1., g: 0., b: 1., a: 1. },
    };
    rgba.into()
}

/// The resolved, ready-to-draw theme.
#[derive(Debug, Clone)]
pub struct Palette {
    pub bg: Hsla,
    pub surface: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub danger: Hsla,
    pub scrim: Hsla,
    pub hero_stops: Vec<Hsla>,
    pub controls_bg: Hsla,
}

impl Theme {
    pub fn load(path: &std::path::Path) -> Result<Theme, String> {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn palette(&self) -> Palette {
        let c = &self.colors;
        Palette {
            bg: color(&c.bg),
            surface: color(&c.surface),
            text: color(&c.text),
            muted: color(&c.muted),
            accent: color(&c.accent),
            danger: color(&c.danger),
            scrim: color(&c.scrim),
            hero_stops: self.hero.gradient.iter().map(|s| color(s)).collect(),
            controls_bg: color(&self.player.controls_bg),
        }
    }

    /// `spacing[i]`, clamped to the scale.
    pub fn sp(&self, i: usize) -> f32 {
        self.spacing.get(i).or(self.spacing.last()).copied().unwrap_or(8.)
    }

    pub fn ease(&self, t: f32) -> f32 {
        ease(&self.motion.easing, t)
    }
}

/// Named easing curves; `spring` is a critically-damped-ish spring with a
/// small overshoot.
pub fn ease(name: &str, t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    match name {
        "linear" => t,
        "ease-in-cubic" => t * t * t,
        "ease-in-out-cubic" => {
            if t < 0.5 {
                4. * t * t * t
            } else {
                1. - (-2. * t + 2.).powi(3) / 2.
            }
        }
        "ease-out-quad" => 1. - (1. - t) * (1. - t),
        "ease-out-expo" => {
            if t >= 1. {
                1.
            } else {
                1. - 2f32.powf(-10. * t)
            }
        }
        "spring" => {
            if t >= 1. {
                1.
            } else {
                1. - (-6. * t).exp() * (8. * t).cos()
            }
        }
        // "ease-out-cubic" and anything unknown.
        _ => 1. - (1. - t).powi(3),
    }
}
