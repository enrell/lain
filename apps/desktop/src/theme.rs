//! Themes, tier 1 of the modding model (DESIGN-v2 §8.3, `lain.ui.theme@1`,
//! documented in `docs/THEMES.md`): a theme is a FOLDER with `theme.json`
//! (tokens: OKLCH colors, a type scale, radii, blur, motion curves),
//! optional `wallpapers/` art and `icons/<name>.svg` overrides. Every key
//! has a default, so a partial file still loads. Themes live in
//! `<client dir>/themes/<id>/`; the two built-in ones are written there on
//! first run so they can be edited live, and the active one is reloaded
//! when its files change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

use gpui::{Hsla, Rgba};
use serde::Deserialize;

pub const FORMAT: &str = "lain.ui.theme@1";

/// The themes shipped inside the exe: (folder id, theme.json).
pub const BUILTIN: &[(&str, &str)] = &[
    ("lain-dark", include_str!("../themes/lain-dark/theme.json")),
    ("paper-light", include_str!("../themes/paper-light/theme.json")),
];

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub format: String,
    pub name: String,
    pub author: String,
    /// `dark` or `light`: a hint for system surfaces (title bar).
    pub scheme: String,
    pub colors: Colors,
    #[serde(rename = "type")]
    pub fonts: Fonts,
    pub radius: Radius,
    pub spacing: Vec<f32>,
    pub blur: Blur,
    pub motion: Motion,
    pub rail: Rail,
    pub hero: Hero,
    pub player: PlayerTheme,
    /// Relative to the theme folder: shown behind the connect/login
    /// screens and as the hero when an item has no art.
    pub wallpaper: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub bg: String,
    pub surface: String,
    pub raised: String,
    pub line: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub on_accent: String,
    pub danger: String,
    pub ok: String,
    pub warn: String,
    pub scrim: String,
    pub focus: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Fonts {
    pub ui: String,
    pub display: String,
    pub mono: String,
    /// The base size in px; every text size derives from it.
    pub base: f32,
    /// The ratio between steps of the scale (1.25 = major third).
    pub scale: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Radius {
    pub card: f32,
    pub button: f32,
    pub panel: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Blur {
    /// Gaussian blur of hero art, px.
    pub hero: f32,
    /// Backdrop blur of glass panels, px (no backdrop filter in GPUI:
    /// panels get the `raised` color's alpha instead).
    pub panel: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Motion {
    pub fast_ms: f32,
    pub normal_ms: f32,
    pub slow_ms: f32,
    /// A named curve or `cubic-bezier(x1, y1, x2, y2)`.
    pub easing: String,
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
    /// Top → bottom scrim stops (colors).
    pub gradient: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PlayerTheme {
    pub controls_bg: String,
    pub seek_h: f32,
    pub autohide_ms: f32,
}

impl Default for Theme {
    // Explicit, not parsed from BUILTIN: `#[serde(default)]` fills missing
    // keys from this, so parsing here would recurse.
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
            player: PlayerTheme::default(),
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

impl Default for PlayerTheme {
    fn default() -> Self {
        PlayerTheme { controls_bg: "oklch(0.118 0.024 295.6 / 0.6)".into(), seek_h: 4., autohide_ms: 2500. }
    }
}

// ---- colors -------------------------------------------------------------------

/// OKLab → linear sRGB → gamma-encoded sRGB, clamped to the gamut.
pub fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> (f32, f32, f32) {
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

/// Parses `oklch(L C H)` / `oklch(L C H / A)` (L as 0..1 or a percent,
/// A as 0..1 or a percent), or hex `#rgb`, `#rrggbb`, `#rrggbbaa`.
pub fn parse_color(s: &str) -> Result<Hsla, String> {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("oklch(").and_then(|r| r.strip_suffix(')')) {
        let (main, alpha) = match inner.split_once('/') {
            Some((m, a)) => (m, Some(a.trim())),
            None => (inner, None),
        };
        let num = |v: &str, pct_scale: f32| -> Result<f32, String> {
            let v = v.trim();
            match v.strip_suffix('%') {
                Some(p) => p.parse::<f32>().map(|x| x / 100. * pct_scale).map_err(|_| format!("bad number {v:?}")),
                None => v.trim_end_matches("deg").parse::<f32>().map_err(|_| format!("bad number {v:?}")),
            }
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
        return Ok(Rgba { r, g, b, a }.into());
    }
    let h = t.strip_prefix('#').ok_or_else(|| format!("{t}: not a color"))?;
    let p = |i: usize| -> Result<f32, String> {
        u8::from_str_radix(h.get(i..i + 2).unwrap_or(""), 16)
            .map(|v| v as f32 / 255.)
            .map_err(|_| format!("{t}: bad hex"))
    };
    let rgba = match h.len() {
        3 => {
            let c = |i: usize| -> Result<f32, String> {
                u8::from_str_radix(&h[i..i + 1].repeat(2), 16)
                    .map(|v| v as f32 / 255.)
                    .map_err(|_| format!("{t}: bad hex"))
            };
            Rgba { r: c(0)?, g: c(1)?, b: c(2)?, a: 1. }
        }
        6 => Rgba { r: p(0)?, g: p(2)?, b: p(4)?, a: 1. },
        8 => Rgba { r: p(0)?, g: p(2)?, b: p(4)?, a: p(6)? },
        _ => return Err(format!("{t}: bad hex")),
    };
    Ok(rgba.into())
}

/// The resolved, ready-to-draw colors.
#[derive(Debug, Clone)]
pub struct Palette {
    pub bg: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub line: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub scrim: Hsla,
    pub focus: Hsla,
    pub hero_stops: Vec<Hsla>,
    pub controls_bg: Hsla,
}

impl Theme {
    /// The palette and every color that failed to parse (shown in the
    /// theme picker; a bad color falls back to magenta so it is seen).
    pub fn palette(&self) -> (Palette, Vec<String>) {
        let mut errs = Vec::new();
        let mut col = |s: &str| {
            parse_color(s).unwrap_or_else(|e| {
                errs.push(e);
                Rgba { r: 1., g: 0., b: 1., a: 1. }.into()
            })
        };
        let c = &self.colors;
        let p = Palette {
            bg: col(&c.bg),
            surface: col(&c.surface),
            raised: col(&c.raised),
            line: col(&c.line),
            text: col(&c.text),
            muted: col(&c.muted),
            accent: col(&c.accent),
            on_accent: col(&c.on_accent),
            danger: col(&c.danger),
            ok: col(&c.ok),
            warn: col(&c.warn),
            scrim: col(&c.scrim),
            focus: col(&c.focus),
            hero_stops: self.hero.gradient.iter().map(|s| col(s)).collect(),
            controls_bg: col(&self.player.controls_bg),
        };
        (p, errs)
    }

    /// `spacing[i]`, clamped to the scale.
    pub fn sp(&self, i: usize) -> f32 {
        self.spacing.get(i).or(self.spacing.last()).copied().unwrap_or(8.)
    }

    /// The type scale: step 0 is the base, negative steps are smaller.
    pub fn ts(&self, step: i32) -> f32 {
        self.fonts.base.max(8.) * self.fonts.scale.clamp(1.01, 2.).powi(step)
    }

    pub fn ease(&self, t: f32) -> f32 {
        ease(&self.motion.easing, t)
    }
}

/// Named curves, `cubic-bezier(...)`, and `spring` (underdamped).
pub fn ease(name: &str, t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if let Some(args) = name.trim().strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')) {
        let v: Vec<f32> = args.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 4 {
            return cubic_bezier(v[0], v[1], v[2], v[3], t);
        }
    }
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
        _ => 1. - (1. - t).powi(3),
    }
}

/// CSS `cubic-bezier`: solve x(u) = t by Newton steps, return y(u).
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let bez = |a: f32, b: f32, u: f32| 3. * a * u * (1. - u).powi(2) + 3. * b * u * u * (1. - u) + u * u * u;
    let d = |a: f32, b: f32, u: f32| 3. * a * (1. - u).powi(2) + 6. * (b - a) * u * (1. - u) + 3. * (1. - b) * u * u;
    let mut u = t;
    for _ in 0..8 {
        let dx = d(x1, x2, u);
        if dx.abs() < 1e-5 {
            break;
        }
        u = (u - (bez(x1, x2, u) - t) / dx).clamp(0., 1.);
    }
    bez(y1, y2, u)
}

// ---- folders, install, icon overrides -----------------------------------------------

/// One theme folder found on disk.
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub id: String,
    pub dir: PathBuf,
    pub name: String,
    pub scheme: String,
    pub error: Option<String>,
}

pub fn themes_dir(client_dir: &Path) -> PathBuf {
    client_dir.join("themes")
}

/// Writes the built-in themes into `dir` unless a folder of that id is
/// already there (user edits win).
pub fn install_builtins(dir: &Path) {
    for (id, json) in BUILTIN {
        let d = dir.join(id);
        if !d.join("theme.json").exists()
            && let Err(e) = std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join("theme.json"), json))
        {
            eprintln!("theme {id}: {e}");
        }
    }
}

pub fn load_dir(dir: &Path) -> Result<Theme, String> {
    let file = dir.join("theme.json");
    let raw = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let t: Theme = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", file.display()))?;
    if !t.format.is_empty() && t.format != FORMAT {
        return Err(format!("{}: format {:?}, expected {FORMAT}", file.display(), t.format));
    }
    Ok(t)
}

pub fn list(dir: &Path) -> Vec<ThemeEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if !p.join("theme.json").is_file() {
            continue;
        }
        let id = e.file_name().to_string_lossy().into_owned();
        match load_dir(&p) {
            Ok(t) => out.push(ThemeEntry { id, dir: p, name: t.name, scheme: t.scheme, error: None }),
            Err(err) => out.push(ThemeEntry { name: id.clone(), id, dir: p, scheme: String::new(), error: Some(err) }),
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// The newest modification time under a theme folder (theme.json,
/// icons/, wallpapers/), for hot reload.
pub fn stamp(dir: &Path) -> Option<SystemTime> {
    let mut best = std::fs::metadata(dir.join("theme.json")).and_then(|m| m.modified()).ok();
    for sub in ["icons", "wallpapers"] {
        if let Ok(rd) = std::fs::read_dir(dir.join(sub)) {
            for e in rd.flatten() {
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    best = best.max(Some(m));
                }
            }
        }
    }
    best
}

/// Icon overrides of the active theme: `icons/<name>.svg` → bytes.
fn overrides() -> &'static RwLock<HashMap<String, Vec<u8>>> {
    static O: OnceLock<RwLock<HashMap<String, Vec<u8>>>> = OnceLock::new();
    O.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn set_icon_overrides(dir: &Path) -> usize {
    let mut map = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir.join("icons")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".svg")
                && let Ok(b) = std::fs::read(e.path())
            {
                map.insert(format!("icons/{name}"), b);
            }
        }
    }
    let n = map.len();
    *overrides().write().unwrap_or_else(|e| e.into_inner()) = map;
    n
}

pub fn icon_override(path: &str) -> Option<Vec<u8>> {
    overrides().read().unwrap_or_else(|e| e.into_inner()).get(path).cloned()
}

/// Installs a theme zip into `dir`: the archive holds `theme.json` at
/// its root or inside one top folder. Returns the new theme's id.
pub fn install_zip(zip: &Path, dir: &Path) -> Result<String, String> {
    let raw = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let files = crate::zip::read_all(&raw)?;
    let manifest = files
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| n.ends_with("theme.json") && n.matches('/').count() <= 1)
        .min_by_key(|n| n.len())
        .ok_or("the archive has no theme.json")?
        .to_string();
    let prefix = manifest.trim_end_matches("theme.json").to_string();
    if Path::new(&prefix).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(format!("unsafe path in archive: {manifest}"));
    }
    let theme: Theme =
        serde_json::from_slice(&files.iter().find(|(n, _)| *n == manifest).map(|(_, b)| b.clone()).unwrap_or_default())
            .map_err(|e| format!("theme.json: {e}"))?;
    if !theme.format.is_empty() && theme.format != FORMAT {
        return Err(format!("format {:?}, expected {FORMAT}", theme.format));
    }
    let stem = zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "theme".into());
    let id_src = if prefix.is_empty() { stem } else { prefix.trim_end_matches('/').to_string() };
    let id: String = id_src
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if id.is_empty() || id.starts_with('.') {
        return Err("bad theme folder name".into());
    }
    let target = dir.join(&id);
    for (name, bytes) in &files {
        let Some(rel) = name.strip_prefix(&prefix) else { continue };
        if rel.is_empty() || rel.ends_with('/') {
            continue;
        }
        // Only plain relative paths: no `..`, no absolute paths.
        let rel_path = Path::new(rel);
        if rel_path.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return Err(format!("unsafe path in archive: {rel}"));
        }
        let out = target.join(rel_path);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(id)
}

/// Reads a file of the theme folder (wallpaper) if it stays inside it.
pub fn theme_file(dir: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || Path::new(rel).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    let p = dir.join(rel);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_matches_known_hex() {
        // The owner's accent #7875bd.
        let c: Rgba = parse_color("oklch(0.596 0.108 284.5)").unwrap().into();
        let hex = |v: f32| (v * 255.).round() as i32;
        assert!((hex(c.r) - 0x78).abs() <= 1 && (hex(c.g) - 0x75).abs() <= 1 && (hex(c.b) - 0xbd).abs() <= 1, "{c:?}");
        let a: Rgba = parse_color("oklch(59.6% 0.108 284.5 / 50%)").unwrap().into();
        assert!((a.a - 0.5).abs() < 1e-4);
        assert!(parse_color("#7875bd").is_ok() && parse_color("nope").is_err() && parse_color("oklch(1 2)").is_err());
    }

    #[test]
    fn builtins_parse_cleanly() {
        for (id, json) in BUILTIN {
            let t: Theme = serde_json::from_str(json).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(t.format, FORMAT);
            assert!(t.palette().1.is_empty(), "{id}: {:?}", t.palette().1);
        }
    }

    #[test]
    fn zip_installs_into_its_folder_and_refuses_escapes() {
        let dir = std::env::temp_dir().join(format!("lain-theme-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("Neon Night.zip");
        let json = r##"{"format":"lain.ui.theme@1","name":"neon","colors":{"accent":"#ff00aa"}}"##;
        std::fs::write(&zip, crate::zip::tests::stored("neon-night/theme.json", json.as_bytes())).unwrap();
        let themes = dir.join("themes");
        let id = install_zip(&zip, &themes).unwrap();
        assert_eq!(id, "neon-night");
        assert_eq!(load_dir(&themes.join("neon-night")).unwrap().name, "neon");
        std::fs::write(&zip, crate::zip::tests::stored("../theme.json", json.as_bytes())).unwrap();
        assert!(install_zip(&zip, &themes).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bezier_ends_and_monotone() {
        assert!((ease("cubic-bezier(0.2, 0, 0, 1)", 0.) - 0.).abs() < 1e-4);
        assert!((ease("cubic-bezier(0.2, 0, 0, 1)", 1.) - 1.).abs() < 1e-4);
        assert!(ease("cubic-bezier(0.2, 0, 0, 1)", 0.5) > 0.5);
    }
}
