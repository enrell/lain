//! `lain.ui.theme@1` (D-076, D-196): the web UI palette, the host's
//! Omarchy colors when present, the built-in palette otherwise, with
//! the legibility floor (D-037). The gateway relays the result without
//! deriving anything; swapping this provider changes the served theme.
//! The JSON is Go's `theme.Palette`, unchanged: the shared token schema
//! of DESIGN-v2 §8.3 is a later, additive step.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_core::{Error, Provider};

pub use lain_api::Palette;

pub const CAP_UI_THEME: &str = "lain.ui.theme@1";
pub const PROVIDER_ID: &str = "lain-theme-omarchy";

const MAX_THEME_BYTES: u64 = 64 << 10;

/// Secondary text must clear this against the page and its fills.
pub const MIN_TEXT_CONTRAST: f64 = 4.5;
/// A separator must stand out this much from the surface it borders.
pub const MIN_LINE_CONTRAST: f64 = 1.25;

/// The built-in palette.
pub fn default_palette() -> Palette {
    let s = |v: &str| v.to_string();
    Palette {
        source: s("default"),
        mode: s("dark"),
        background: s("#08090c"),
        surface: s("#101218"),
        surface_hover: s("#171a22"),
        surface_active: s("#1e222c"),
        foreground: s("#e7e9f0"),
        muted: s("#8a90a3"),
        line: s("#1f2430"),
        accent: s("#5ce1c4"),
        accent_hover: s("#7af0d6"),
        accent_foreground: s("#05231d"),
        danger: s("#ff6b81"),
        danger_foreground: s("#2b060d"),
        success: s("#6ee7a8"),
        warning: s("#ffcf5c"),
    }
}

/// The host Omarchy colors file: `LAIN_OMARCHY_COLORS`, else the
/// default state path under the home directory.
pub fn omarchy_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LAIN_OMARCHY_COLORS")
        && !p.trim().is_empty()
    {
        return Some(PathBuf::from(p.trim()));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml"))
}

/// One palette: the colors file when it parses, the built-in palette
/// otherwise (missing, unreadable, incomplete or unsafe files alike).
pub fn derive(path: Option<&Path>) -> Palette {
    let Some(path) = path else { return default_palette() };
    let Ok(f) = std::fs::File::open(path) else { return default_palette() };
    let mut raw = Vec::new();
    if f.take(MAX_THEME_BYTES).read_to_end(&mut raw).is_err() {
        return default_palette();
    }
    match parse_omarchy(&String::from_utf8_lossy(&raw)) {
        Some(p) => apply_legibility_floor(p),
        None => default_palette(),
    }
}

fn is_hex_color(v: &str) -> bool {
    v.len() == 7 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parses an Omarchy `colors.toml`: `key = value` lines, values quoted
/// or bare with trailing comments; only `#rrggbb` colors and the mode
/// are kept. `None` without background, foreground and accent.
pub fn parse_omarchy(text: &str) -> Option<Palette> {
    let mut values = std::collections::HashMap::<String, String>::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let mut value = value.trim();
        if let Some(i) = value.find('#')
            && i > 0
            && !value.starts_with('"')
            && !value.starts_with('\'')
        {
            value = value[..i].trim();
        }
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        if key == "mode" {
            if value == "dark" || value == "light" {
                values.insert(key.into(), value.into());
            }
            continue;
        }
        if is_hex_color(value) {
            values.insert(key.into(), value.to_ascii_lowercase());
        }
    }
    let first = |keys: &[&str]| keys.iter().find_map(|k| values.get(*k).cloned());
    let background = first(&["background", "color0"])?;
    let foreground = first(&["foreground", "color7"])?;
    let accent = first(&["accent", "color4"])?;
    let fallback = default_palette();
    let or = |v: Option<String>, f: &str| v.unwrap_or_else(|| f.to_string());
    let mode = or(values.get("mode").cloned(), "dark");
    let surface = or(first(&["dark_background"]), &background);
    let surface_hover = or(first(&["lighter_background", "selection"]), &surface);
    let surface_active = or(first(&["selection", "lighter_background"]), &surface_hover);
    let muted = or(first(&["muted", "color8", "dark_foreground"]), &foreground);
    let line = or(first(&["selection", "muted", "color8"]), &muted);
    let accent_hover = or(first(&["bright_blue"]), &accent);
    let danger = or(first(&["red", "color1"]), &fallback.danger);
    let success = or(first(&["green", "color2"]), &fallback.success);
    let warning = or(first(&["yellow", "color3"]), &fallback.warning);
    Some(Palette {
        source: "omarchy".into(),
        mode,
        accent_foreground: contrast_color(&accent),
        danger_foreground: contrast_color(&danger),
        background,
        surface,
        surface_hover,
        surface_active,
        foreground,
        muted,
        line,
        accent,
        accent_hover,
        danger,
        success,
        warning,
    })
}

/// D-037: distinct, contrast-checked roles. Only muted, surface-active
/// and line move, and only when they violate the floor, so a host theme
/// that already separates them is served unchanged.
pub fn apply_legibility_floor(mut p: Palette) -> Palette {
    // A fill no text can sit on is re-derived as the smallest lift off
    // the surface, which is what the role means.
    if contrast_ratio(&p.surface_active, &p.muted) < MIN_TEXT_CONTRAST
        && contrast_ratio(&p.surface_active, &p.foreground) < MIN_TEXT_CONTRAST
    {
        p.surface_active = p.surface.clone();
    }
    p.surface_active = blend_to(&p.surface_active, &p.surface, &p.foreground, MIN_LINE_CONTRAST);
    p.muted = blend_to(&p.muted, &p.surface_active, &p.foreground, MIN_TEXT_CONTRAST);
    p.muted = blend_to(&p.muted, &p.background, &p.foreground, MIN_TEXT_CONTRAST);
    p.line = blend_to(&p.line, &p.surface, &p.foreground, MIN_LINE_CONTRAST);
    p.line = blend_to(&p.line, &p.surface_active, &p.foreground, MIN_LINE_CONTRAST);
    p
}

/// Blends `c` toward `target` until it clears `min` against `other`;
/// the pole itself is the last resort.
fn blend_to(c: &str, other: &str, target: &str, min: f64) -> String {
    if contrast_ratio(c, other) >= min {
        return c.to_string();
    }
    let mut c = c.to_string();
    for _ in 0..256 {
        let next = mix_hex(&c, target, 0.08);
        if next == c {
            break;
        }
        c = next;
        if contrast_ratio(&c, other) >= min {
            return c;
        }
    }
    target.to_string()
}

fn channel(c: &str, offset: usize) -> f64 {
    f64::from(u8::from_str_radix(c.get(offset..offset + 2).unwrap_or("00"), 16).unwrap_or(0))
}

/// `a` and `b` blended by `t` (0 keeps `a`, 1 keeps `b`).
fn mix_hex(a: &str, b: &str, t: f64) -> String {
    let mix = |o: usize| {
        let (x, y) = (channel(a, o), channel(b, o));
        (x + (y - x) * t).round() as u8
    };
    format!("#{:02x}{:02x}{:02x}", mix(1), mix(3), mix(5))
}

/// The WCAG relative luminance of a `#rrggbb` color.
fn luminance(c: &str) -> f64 {
    let comp = |o: usize| {
        let v = channel(c, o) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * comp(1) + 0.7152 * comp(3) + 0.0722 * comp(5)
}

/// The WCAG contrast ratio of two `#rrggbb` colors.
pub fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (mut la, mut lb) = (luminance(a), luminance(b));
    if la < lb {
        std::mem::swap(&mut la, &mut lb);
    }
    (la + 0.05) / (lb + 0.05)
}

fn contrast_color(c: &str) -> String {
    let l = luminance(c);
    if (l + 0.05) / 0.05 >= 1.05 / (l + 0.05) { "#000000".into() } else { "#ffffff".into() }
}

/// The provider. Input `{colors_path?}`; an empty path resolves the
/// host's Omarchy location.
pub struct Theme;

#[derive(Deserialize, Default)]
struct Input {
    #[serde(default)]
    colors_path: String,
}

impl Provider for Theme {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_UI_THEME.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, _op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_UI_THEME {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let i: Input = match input {
            Value::Null => Input::default(),
            v => serde_json::from_value(v).map_err(|_| Error::invalid("theme input required"))?,
        };
        let path = if i.colors_path.is_empty() { omarchy_path() } else { Some(PathBuf::from(i.colors_path)) };
        Ok(json!(derive(path.as_deref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLEXOKI_LIGHT: &str = r##"mode = "light"
background = "#FFFCF0"
dark_background = "#F2EFE4"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
muted = "#878580"
selection = "#CECDC3"
accent = "#205EA6"
bright_blue = "#4385BE"
red = "#D14D41"
green = "#879A39"
yellow = "#D0A215"
"##;

    fn file(tag: &str, text: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lain-theme-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("colors.toml");
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn host_palette_is_normalized() {
        let got = derive(Some(&file("light", FLEXOKI_LIGHT)));
        assert_eq!((got.source.as_str(), got.mode.as_str()), ("omarchy", "light"));
        assert_eq!((got.background.as_str(), got.surface.as_str()), ("#fffcf0", "#f2efe4"));
        assert_eq!(got.surface_hover, "#e6e4d9");
        assert_eq!((got.accent.as_str(), got.accent_hover.as_str()), ("#205ea6", "#4385be"));
        assert_eq!(got.accent_foreground, "#ffffff");
        assert_eq!((got.danger.as_str(), got.danger_foreground.as_str()), ("#d14d41", "#000000"));
        assert_eq!((got.success.as_str(), got.warning.as_str()), ("#879a39", "#d0a215"));
    }

    #[test]
    fn missing_or_unsafe_files_fall_back() {
        assert_eq!(derive(Some(Path::new("/nonexistent/colors.toml"))), default_palette());
        let bad = file("unsafe", "background = \"url(javascript:bad)\"\naccent = \"#123456\"\n");
        assert_eq!(derive(Some(&bad)).source, "default");
        assert_eq!(derive(None).background, "#08090c");
    }

    #[test]
    fn comments_and_modes() {
        let p =
            parse_omarchy("mode = light # a comment\nbackground = #101010\nforeground='#eeeeee'\ncolor4 = #4488CC\n")
                .unwrap();
        assert_eq!((p.mode.as_str(), p.background.as_str(), p.accent.as_str()), ("light", "#101010", "#4488cc"));
        assert_eq!(
            parse_omarchy("mode = sepia\nbackground = #101010\nforeground = #eeeeee\naccent = #123456\n").unwrap().mode,
            "dark"
        );
        // A bare color's own `#` is not a comment, so a trailing one spoils it.
        assert!(parse_omarchy("background = #101010 # x\nforeground = #eeeeee\naccent = #123456\n").is_none());
        assert_eq!(p.surface, "#101010", "falls back to the background");
    }

    #[test]
    fn floor_keeps_collapsed_palettes_legible() {
        let cases = [
            "mode = \"dark\"\nbackground = \"#0c0b0c\"\ndark_background = \"#090809\"\nlighter_background = \"#0c0b0c\"\nforeground = \"#FAFCFB\"\nmuted = \"#584e51\"\nselection = \"#584e51\"\naccent = \"#b59790\"\nred = \"#c38b7b\"\ngreen = \"#87a9b0\"\nyellow = \"#6b5e73\"\n",
            "mode = \"dark\"\nbackground = \"#101010\"\ndark_background = \"#101010\"\nlighter_background = \"#101010\"\nforeground = \"#eeeeee\"\nmuted = \"#808080\"\nselection = \"#808080\"\naccent = \"#4488cc\"\nred = \"#cc4444\"\ngreen = \"#44cc44\"\nyellow = \"#cccc44\"\n",
            "mode = \"dark\"\nbackground = \"#151623\"\nforeground = \"#bac5cd\"\nmuted = \"#4b4d53\"\nselection = \"#9a778a\"\naccent = \"#9a778a\"\nred = \"#9a778a\"\ngreen = \"#b4c9ab\"\nyellow = \"#8f7f62\"\n",
            FLEXOKI_LIGHT,
        ];
        for (i, c) in cases.iter().enumerate() {
            let g = derive(Some(&file(&format!("floor{i}"), c)));
            assert_eq!(g.source, "omarchy");
            assert!(contrast_ratio(&g.muted, &g.background) >= MIN_TEXT_CONTRAST, "{i}: {g:?}");
            assert!(contrast_ratio(&g.muted, &g.surface_active) >= MIN_TEXT_CONTRAST, "{i}: {g:?}");
            assert!(contrast_ratio(&g.surface_active, &g.surface) >= MIN_LINE_CONTRAST, "{i}: {g:?}");
            assert!(contrast_ratio(&g.line, &g.surface) >= MIN_LINE_CONTRAST, "{i}: {g:?}");
            assert!(contrast_ratio(&g.line, &g.surface_active) >= MIN_LINE_CONTRAST, "{i}: {g:?}");
        }
    }

    #[test]
    fn floor_leaves_separated_palettes_alone() {
        let g = derive(Some(&file(
            "separated",
            "mode = \"dark\"\nbackground = \"#0c0b0c\"\ndark_background = \"#141216\"\nlighter_background = \"#1c1a1e\"\nforeground = \"#FAFCFB\"\nmuted = \"#a8a2a4\"\nselection = \"#3a3438\"\naccent = \"#b59790\"\nred = \"#c38b7b\"\ngreen = \"#87a9b0\"\nyellow = \"#6b5e73\"\n",
        )));
        assert_eq!((g.muted.as_str(), g.surface_active.as_str()), ("#a8a2a4", "#3a3438"));
        assert!(contrast_ratio(&g.line, &g.surface_active) >= MIN_LINE_CONTRAST);
    }

    #[test]
    fn contrast_math() {
        assert!((contrast_ratio("#000000", "#ffffff") - 21.0).abs() < 1e-9);
        assert!((contrast_ratio("#777777", "#777777") - 1.0).abs() < 1e-9);
        assert_eq!(mix_hex("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(contrast_color("#ffffff"), "#000000");
        assert_eq!(contrast_color("#000000"), "#ffffff");
    }

    #[test]
    fn provider_contract() {
        let t = Theme;
        let out = t.invoke(CAP_UI_THEME, "", json!({"colors_path": "/nonexistent"})).unwrap();
        assert_eq!(serde_json::from_value::<Palette>(out).unwrap(), default_palette());
        assert!(t.invoke("lain.x@1", "", Value::Null).is_err());
        assert!(t.invoke(CAP_UI_THEME, "", json!(3)).is_err());
    }
}
