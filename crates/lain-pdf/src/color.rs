//! Colour spaces (§8.6): device, CIE-based (treated by component count;
//! Lab converted), Indexed, Separation, DeviceN and Pattern, all mapped
//! to 8-bit sRGB-ish values for the rasterizer.

use crate::document::Document;
use crate::function::Function;
use crate::object::{Dict, Object};

#[derive(Debug, Clone)]
pub enum ColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Lab {
        white: [f64; 3],
        range: [f64; 4],
    },
    Indexed {
        base: Box<ColorSpace>,
        hival: usize,
        lookup: Vec<u8>,
    },
    /// Separation and DeviceN: tint components through a function into
    /// the alternate space. `none` paints nothing (the `None` colorant).
    Tinted {
        n: usize,
        alt: Box<ColorSpace>,
        tint: Option<Function>,
        none: bool,
    },
    Pattern {
        under: Option<Box<ColorSpace>>,
    },
}

impl ColorSpace {
    /// The colour space an object names, through the resources'
    /// `/ColorSpace` dictionary when it is a resource name.
    pub fn parse(doc: &Document, o: &Object, resources: Option<&Dict>) -> ColorSpace {
        ColorSpace::parse_depth(doc, o, resources, 0).unwrap_or(ColorSpace::Gray)
    }

    fn parse_depth(doc: &Document, o: &Object, res: Option<&Dict>, depth: usize) -> Option<ColorSpace> {
        if depth > 8 {
            return None;
        }
        let o = doc.resolve(o);
        match &o {
            Object::Name(n) => match n.as_str() {
                "DeviceGray" | "G" | "CalGray" => Some(ColorSpace::Gray),
                "DeviceRGB" | "RGB" | "CalRGB" => Some(ColorSpace::Rgb),
                "DeviceCMYK" | "CMYK" => Some(ColorSpace::Cmyk),
                "Pattern" => Some(ColorSpace::Pattern { under: None }),
                other => {
                    let named = res.and_then(|r| doc.dict_in(r, "ColorSpace")).map(|cs| doc.get_in(&cs, other))?;
                    if named.is_null() {
                        return None;
                    }
                    ColorSpace::parse_depth(doc, &named, res, depth + 1)
                }
            },
            Object::Array(a) => {
                let kind = doc.resolve(a.first()?).as_name()?.to_string();
                let arg = |i: usize| a.get(i).map(|x| doc.resolve(x)).unwrap_or(Object::Null);
                match kind.as_str() {
                    "DeviceGray" | "CalGray" | "G" => Some(ColorSpace::Gray),
                    "DeviceRGB" | "CalRGB" | "RGB" => Some(ColorSpace::Rgb),
                    "DeviceCMYK" | "CMYK" => Some(ColorSpace::Cmyk),
                    "ICCBased" => {
                        let s = arg(1);
                        let d = s.as_dict()?;
                        if let Some(alt) = d.get("Alternate")
                            && let Some(cs) = ColorSpace::parse_depth(doc, alt, res, depth + 1)
                        {
                            return Some(cs);
                        }
                        Some(match doc.get_in(d, "N").as_i64().unwrap_or(3) {
                            1 => ColorSpace::Gray,
                            4 => ColorSpace::Cmyk,
                            _ => ColorSpace::Rgb,
                        })
                    }
                    "Lab" => {
                        let d = arg(1);
                        let d = d.as_dict()?;
                        let w = doc.numbers_in(d, "WhitePoint");
                        let r = doc.numbers_in(d, "Range");
                        let white = if w.len() >= 3 { [w[0], w[1], w[2]] } else { [0.9505, 1.0, 1.089] };
                        let range =
                            if r.len() >= 4 { [r[0], r[1], r[2], r[3]] } else { [-100.0, 100.0, -100.0, 100.0] };
                        Some(ColorSpace::Lab { white, range })
                    }
                    "Indexed" | "I" => {
                        let base = ColorSpace::parse_depth(doc, &arg(1), res, depth + 1)?;
                        let hival = arg(2).as_i64().unwrap_or(0).clamp(0, 255) as usize;
                        let lookup = match arg(3) {
                            Object::String(s) => s.to_vec(),
                            Object::Stream(s) => doc.decode_stream(&s).ok()?.data,
                            _ => Vec::new(),
                        };
                        Some(ColorSpace::Indexed { base: Box::new(base), hival, lookup })
                    }
                    "Separation" | "DeviceN" => {
                        let names = arg(1);
                        let n = match &names {
                            Object::Array(v) => v.len().max(1),
                            _ => 1,
                        };
                        let none = match &names {
                            Object::Name(x) => x.as_str() == "None",
                            Object::Array(v) => v.iter().all(|x| x.as_name() == Some("None")),
                            _ => false,
                        };
                        let alt = ColorSpace::parse_depth(doc, &arg(2), res, depth + 1).unwrap_or(ColorSpace::Gray);
                        let tint = Function::parse(doc, &arg(3));
                        Some(ColorSpace::Tinted { n, alt: Box::new(alt), tint, none })
                    }
                    "Pattern" => {
                        let under =
                            a.get(1).and_then(|u| ColorSpace::parse_depth(doc, u, res, depth + 1)).map(Box::new);
                        Some(ColorSpace::Pattern { under })
                    }
                    _ => None,
                }
            }
            Object::Stream(_) | Object::Dict(_) => None,
            _ => None,
        }
    }

    pub fn components(&self) -> usize {
        match self {
            ColorSpace::Gray => 1,
            ColorSpace::Rgb | ColorSpace::Lab { .. } => 3,
            ColorSpace::Cmyk => 4,
            ColorSpace::Indexed { .. } => 1,
            ColorSpace::Tinted { n, .. } => *n,
            ColorSpace::Pattern { under } => under.as_ref().map_or(0, |u| u.components()),
        }
    }

    /// The initial colour after `cs`/`CS` (§8.6.8).
    pub fn initial(&self) -> Vec<f64> {
        match self {
            ColorSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            ColorSpace::Lab { range, .. } => {
                vec![0.0, 0.0f64.clamp(range[0], range[1]), 0.0f64.clamp(range[2], range[3])]
            }
            ColorSpace::Tinted { n, .. } => vec![1.0; *n],
            other => vec![0.0; other.components()],
        }
    }

    /// The image `/Decode` default for `bpc` bits per component.
    pub fn default_decode(&self, bpc: u32) -> Vec<f64> {
        match self {
            ColorSpace::Indexed { .. } => vec![0.0, ((1u32 << bpc.min(16)) - 1) as f64],
            ColorSpace::Lab { range, .. } => vec![0.0, 100.0, range[0], range[1], range[2], range[3]],
            other => [0.0, 1.0].repeat(other.components().max(1)),
        }
    }

    /// Whether painting in this space leaves the page untouched.
    pub fn invisible(&self) -> bool {
        matches!(self, ColorSpace::Tinted { none: true, .. })
    }

    pub fn to_rgb(&self, c: &[f64]) -> [u8; 3] {
        let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match self {
            ColorSpace::Gray => {
                let g = to8(at(0));
                [g, g, g]
            }
            ColorSpace::Rgb => [to8(at(0)), to8(at(1)), to8(at(2))],
            ColorSpace::Cmyk => cmyk_to_rgb(at(0), at(1), at(2), at(3)),
            ColorSpace::Lab { white, .. } => lab_to_rgb(at(0), at(1), at(2), white),
            ColorSpace::Indexed { base, hival, lookup } => {
                let i = (at(0).round().max(0.0) as usize).min(*hival);
                let n = base.components();
                let comps: Vec<f64> =
                    (0..n).map(|k| f64::from(lookup.get(i * n + k).copied().unwrap_or(0)) / 255.0).collect();
                // A Lab base stores its components scaled to its range.
                let comps = match base.as_ref() {
                    ColorSpace::Lab { range, .. } => vec![
                        comps[0] * 100.0,
                        range[0] + comps[1] * (range[1] - range[0]),
                        range[2] + comps[2] * (range[3] - range[2]),
                    ],
                    _ => comps,
                };
                base.to_rgb(&comps)
            }
            ColorSpace::Tinted { alt, tint, .. } => match tint {
                Some(f) => {
                    let out = f.eval(c);
                    alt.to_rgb(&out)
                }
                // No usable tint transform: darker for more tint.
                None => {
                    let g = to8(1.0 - at(0));
                    [g, g, g]
                }
            },
            ColorSpace::Pattern { under } => under.as_ref().map_or([0, 0, 0], |u| u.to_rgb(c)),
        }
    }
}

fn to8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// CMYK to RGB through a quadratic fit of a typical press profile (the
/// polynomial pdf.js uses), much closer to print than `1 − c − k`.
pub fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> [u8; 3] {
    let (c, m, y, k) = (c.clamp(0.0, 1.0), m.clamp(0.0, 1.0), y.clamp(0.0, 1.0), k.clamp(0.0, 1.0));
    let r = 255.0
        + c * (-4.387332384609988 * c + 54.48615194189176 * m + 18.82290502165302 * y + 212.25662451639585 * k
            - 285.2331026137004)
        + m * (1.7149763477362134 * m - 5.6096736904047315 * y - 17.873870861415444 * k - 5.497006427196366)
        + y * (-2.5217340131683033 * y - 21.248923337353073 * k + 17.5119270841813)
        - k * (21.86122147463605 * k + 189.48180835922747);
    let g = 255.0
        + c * (8.841041422036149 * c + 60.118027045597366 * m + 6.871425592049007 * y + 31.159100130055922 * k
            - 79.2970844816548)
        + m * (-15.310361306967817 * m + 17.575251261109482 * y + 131.35250912493976 * k - 190.9453302588951)
        + y * (4.444339102852739 * y + 9.8632861493405 * k - 24.86741582555878)
        - k * (20.737325471181034 * k + 187.80453709719578);
    let b = 255.0
        + c * (0.8842522430003296 * c + 8.078677503112928 * m + 30.89978309703729 * y
            - 0.23883238689178934 * k
            - 14.183576799673286)
        + m * (10.49593273432072 * m + 63.02378494754052 * y + 50.606957656360734 * k - 112.23884253719248)
        + y * (0.03296041114873217 * y + 115.60384449646641 * k - 193.58209356861505)
        - k * (22.33816807309886 * k + 180.12613974708367);
    [r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8]
}

fn lab_to_rgb(l: f64, a: f64, b: f64, white: &[f64; 3]) -> [u8; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let g = |t: f64| if t >= 6.0 / 29.0 { t * t * t } else { 108.0 / 841.0 * (t - 4.0 / 29.0) };
    let (x, y, z) = (white[0] * g(fx), white[1] * g(fy), white[2] * g(fz));
    // XYZ (D65-ish) to linear sRGB, then the sRGB curve.
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let gg = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let bb = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    let enc = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        to8(s)
    };
    [enc(r), enc(gg), enc(bb)]
}

/// Luminosity (§11.6.5.2 for soft masks): Rec. 601 weights.
pub fn luminosity(rgb: [u8; 3]) -> u8 {
    ((u32::from(rgb[0]) * 77 + u32::from(rgb[1]) * 151 + u32::from(rgb[2]) * 28) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_conversions() {
        assert_eq!(ColorSpace::Gray.to_rgb(&[0.5]), [128, 128, 128]);
        assert_eq!(ColorSpace::Rgb.to_rgb(&[1.0, 0.0, 2.0]), [255, 0, 255]);
        assert_eq!(cmyk_to_rgb(0.0, 0.0, 0.0, 0.0), [255, 255, 255]);
        let k = cmyk_to_rgb(0.0, 0.0, 0.0, 1.0);
        assert!(k.iter().all(|&v| v < 70), "{k:?}");
        let c = cmyk_to_rgb(1.0, 0.0, 0.0, 0.0);
        assert!(c[0] < 30 && c[2] > 200, "{c:?}");
        let white = lab_to_rgb(100.0, 0.0, 0.0, &[0.9505, 1.0, 1.089]);
        assert!(white.iter().all(|&v| v > 250), "{white:?}");
        let idx = ColorSpace::Indexed { base: Box::new(ColorSpace::Rgb), hival: 1, lookup: vec![255, 0, 0, 0, 0, 255] };
        assert_eq!(idx.to_rgb(&[1.0]), [0, 0, 255]);
        assert_eq!(idx.to_rgb(&[9.0]), [0, 0, 255]);
        assert_eq!(ColorSpace::Cmyk.initial(), vec![0.0, 0.0, 0.0, 1.0]);
    }
}
