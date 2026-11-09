//! theme.json → the `Theme` global. Every key is applied at startup and on
//! live reload (the file is polled; see `watch`).

use std::path::Path;

use serde_json::Value as J;
use slint::{Brush, Color};
use slint_interpreter::{ComponentInstance, Value};

/// What the art loader needs from the theme (it is applied on the CPU).
#[derive(Debug, Clone, PartialEq)]
pub struct ArtParams {
    pub card_w: f32,
    pub card_h: f32,
    pub hover_scale: f32,
    pub hero_blur: f32,
}

pub struct Loaded {
    pub json: J,
    pub art: ArtParams,
    pub easing: String,
    pub fonts: Vec<String>,
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: J = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    let f = |p: &str, d: f64| json.pointer(p).and_then(J::as_f64).unwrap_or(d) as f32;
    let art = ArtParams {
        card_w: f("/rail/card_w", 200.0),
        card_h: f("/rail/card_h", 300.0),
        hover_scale: f("/rail/hover_scale", 1.0),
        hero_blur: f("/hero/blur", 0.0),
    };
    let easing = json.pointer("/motion/easing").and_then(J::as_str).unwrap_or("ease-out").to_string();
    let fonts = ["ui", "display", "mono"]
        .iter()
        .filter_map(|k| json.pointer(&format!("/fonts/{k}")).and_then(J::as_str).map(str::to_string))
        .collect();
    Ok(Loaded { json, art, easing, fonts })
}

pub fn parse_color(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let n = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color::from_rgb_u8(n(0)?, n(2)?, n(4)?)),
        8 => Some(Color::from_argb_u8(n(6)?, n(0)?, n(2)?, n(4)?)),
        _ => None,
    }
}

/// Keys this build reads; anything else in the file is reported as unknown.
const KNOWN: &[&str] = &[
    "/name",
    "/colors/bg",
    "/colors/surface",
    "/colors/text",
    "/colors/muted",
    "/colors/accent",
    "/colors/danger",
    "/colors/scrim",
    "/fonts/ui",
    "/fonts/display",
    "/fonts/mono",
    "/fonts/size_base",
    "/radius/card",
    "/radius/button",
    "/spacing",
    "/rail/card_w",
    "/rail/card_h",
    "/rail/gap",
    "/rail/hover_scale",
    "/hero/height",
    "/hero/gradient",
    "/hero/blur",
    "/motion/fast_ms",
    "/motion/normal_ms",
    "/motion/easing",
    "/player/controls_bg",
    "/player/seek_h",
    "/player/autohide_ms",
];

fn leaves(v: &J, at: String, out: &mut Vec<String>) {
    match v {
        J::Object(m) => m.iter().for_each(|(k, v)| leaves(v, format!("{at}/{k}"), out)),
        _ => out.push(at),
    }
}

/// Applies the theme; returns warnings (unknown keys, bad values).
pub fn apply(inst: &ComponentInstance, t: &Loaded) -> Vec<String> {
    let mut warn = Vec::new();
    let j = &t.json;
    let set = |prop: &str, v: Value, warn: &mut Vec<String>| {
        if let Err(e) = inst.set_global_property("Theme", prop, v) {
            warn.push(format!("Theme.{prop}: {e}"));
        }
    };
    let color = |key: &str, prop: &str, warn: &mut Vec<String>| {
        if let Some(s) = j.pointer(key).and_then(J::as_str) {
            match parse_color(s) {
                Some(c) => set(prop, Value::Brush(Brush::SolidColor(c)), warn),
                None => warn.push(format!("{key}: not a #rrggbb[aa] color: {s}")),
            }
        }
    };
    let num = |key: &str, prop: &str, warn: &mut Vec<String>| {
        if let Some(n) = j.pointer(key).and_then(J::as_f64) {
            set(prop, Value::Number(n), warn);
        }
    };
    let string = |key: &str, prop: &str, warn: &mut Vec<String>| {
        if let Some(s) = j.pointer(key).and_then(J::as_str) {
            set(prop, Value::String(s.into()), warn);
        }
    };

    string("/name", "name", &mut warn);
    for c in ["bg", "surface", "text", "muted", "accent", "danger", "scrim"] {
        color(&format!("/colors/{c}"), c, &mut warn);
    }
    string("/fonts/ui", "font-ui", &mut warn);
    string("/fonts/display", "font-display", &mut warn);
    string("/fonts/mono", "font-mono", &mut warn);
    num("/fonts/size_base", "font-size", &mut warn);
    num("/radius/card", "radius-card", &mut warn);
    num("/radius/button", "radius-button", &mut warn);
    if let Some(sp) = j.pointer("/spacing").and_then(J::as_array) {
        for (i, v) in sp.iter().take(7).enumerate() {
            if let Some(n) = v.as_f64() {
                set(&format!("s{i}"), Value::Number(n), &mut warn);
            }
        }
        if sp.len() > 7 {
            warn.push(format!("/spacing: only the first 7 steps are used ({} given)", sp.len()));
        }
    }
    num("/rail/card_w", "card-w", &mut warn);
    num("/rail/card_h", "card-h", &mut warn);
    num("/rail/gap", "gap", &mut warn);
    num("/rail/hover_scale", "hover-scale", &mut warn);
    num("/hero/height", "hero-height", &mut warn);
    if let Some(g) = j.pointer("/hero/gradient").and_then(J::as_array) {
        let stops: Vec<Color> = g.iter().filter_map(|s| s.as_str().and_then(parse_color)).collect();
        if let (Some(a), Some(b)) = (stops.first(), stops.last()) {
            set("hero-top", Value::Brush(Brush::SolidColor(*a)), &mut warn);
            set("hero-bottom", Value::Brush(Brush::SolidColor(*b)), &mut warn);
        }
        if stops.len() > 2 {
            warn.push("/hero/gradient: only the first and last stops are used".into());
        }
    }
    num("/motion/fast_ms", "fast", &mut warn);
    num("/motion/normal_ms", "normal", &mut warn);
    match crate::ui::curve_index(&t.easing) {
        Some(i) => set("curve", Value::Number(i as f64), &mut warn),
        None => warn.push(format!("/motion/easing: unknown curve {:?}, kept the default", t.easing)),
    }
    color("/player/controls_bg", "controls-bg", &mut warn);
    num("/player/seek_h", "seek-h", &mut warn);
    num("/player/autohide_ms", "autohide", &mut warn);

    let mut all = Vec::new();
    leaves(j, String::new(), &mut all);
    for k in all {
        if !KNOWN.iter().any(|known| k == *known || k.starts_with(&format!("{known}/"))) {
            warn.push(format!("{k}: unknown theme key, ignored"));
        }
    }
    warn
}

/// The file's modification time, for polling.
pub fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
