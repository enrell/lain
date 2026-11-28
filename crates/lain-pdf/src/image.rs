//! Image XObjects and inline images (§8.9): samples unpacked at every
//! bit depth, `/Decode`, colour conversion, stencil masks, colour-key
//! and explicit masks, soft masks, and a box filter that shrinks an
//! image to about the size it is drawn at before it is sampled.

use std::sync::Arc;

use crate::color::ColorSpace;
use crate::document::Document;
use crate::error::{PdfError, Result};
use crate::filter::Decoded;
use crate::geom::Matrix;
use crate::object::{Dict, Object};
use crate::raster::Shader;

/// Larger images are refused (about 64 megapixels).
pub const MAX_PIXELS: usize = 64 << 20;

#[derive(Debug, Clone)]
pub struct RasterImage {
    pub width: usize,
    pub height: usize,
    /// Three bytes per pixel; empty for a stencil mask.
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    /// An image mask: only `alpha` counts, painted in the fill colour.
    pub stencil: bool,
    pub interpolate: bool,
}

/// Image parameters, from an XObject's dictionary or an inline image's
/// abbreviated keys.
fn int(doc: &Document, d: &Dict, keys: &[&str]) -> Option<i64> {
    d.get_any(keys).map(|o| doc.resolve(o)).and_then(|o| o.as_i64())
}

fn flag(doc: &Document, d: &Dict, keys: &[&str]) -> bool {
    d.get_any(keys).map(|o| doc.resolve(o)).and_then(|o| o.as_bool()).unwrap_or(false)
}

/// Decodes an image. `data` is the stream's decoded bytes; `target` is
/// roughly the device size it is drawn at, so a huge scan is shrunk
/// once instead of sampled from full size per pixel.
pub fn decode(
    doc: &Document,
    d: &Dict,
    data: Decoded,
    res: Option<&Dict>,
    target: (usize, usize),
) -> Result<RasterImage> {
    let w = int(doc, d, &["Width", "W"]).unwrap_or(0);
    let h = int(doc, d, &["Height", "H"]).unwrap_or(0);
    let (mut w, mut h) = (w.max(0) as usize, h.max(0) as usize);
    // A JPEG knows its own size better than a wrong dictionary.
    if let Some(j) = data.jpeg {
        w = j.width as usize;
        h = j.height as usize;
    }
    if w == 0 || h == 0 {
        return Err(PdfError::invalid("image without size"));
    }
    if w.saturating_mul(h) > MAX_PIXELS {
        return Err(PdfError::too_large("image"));
    }
    let stencil = flag(doc, d, &["ImageMask", "IM"]);
    let interpolate = flag(doc, d, &["Interpolate", "I"]);
    let decode_arr: Vec<f64> = d
        .get_any(&["Decode", "D"])
        .map(|o| doc.resolve(o))
        .and_then(|o| o.as_array().map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect()))
        .unwrap_or_default();
    if stencil {
        let inverted = decode_arr.first().copied().unwrap_or(0.0) > decode_arr.get(1).copied().unwrap_or(1.0);
        let bits = unpack(&data.data, w, h, 1, 1);
        // Sample 0 paints (with the default `/Decode`).
        let alpha: Vec<u8> = bits.iter().map(|&b| if (b == 0) != inverted { 255 } else { 0 }).collect();
        let img = RasterImage { width: w, height: h, rgb: Vec::new(), alpha: Some(alpha), stencil: true, interpolate };
        return Ok(shrink(img, target));
    }
    let mut bpc = int(doc, d, &["BitsPerComponent", "BPC"]).unwrap_or(8) as u32;
    let cs_obj = d.get_any(&["ColorSpace", "CS"]).cloned().unwrap_or(Object::Null);
    let mut cs = if cs_obj.is_null() { ColorSpace::Gray } else { ColorSpace::parse(doc, &cs_obj, res) };
    let mut samples = data.data;
    if let Some(j) = data.jpeg {
        bpc = 8;
        // The JPEG's component count wins over a mismatched space.
        if cs.components() != usize::from(j.components) && !matches!(cs, ColorSpace::Indexed { .. }) {
            cs = match j.components {
                1 => ColorSpace::Gray,
                4 => ColorSpace::Cmyk,
                _ => ColorSpace::Rgb,
            };
        }
        // Adobe writes CMYK JPEGs inverted.
        if j.adobe && j.components == 4 {
            for s in &mut samples {
                *s = 255 - *s;
            }
        }
    }
    if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        bpc = 8;
    }
    let n = cs.components().max(1);
    let decode = if decode_arr.len() >= 2 * n { decode_arr } else { cs.default_decode(bpc) };
    let raw = unpack(&samples, w, h, n, bpc);
    let max = ((1u32 << bpc) - 1) as f64;
    let mut rgb = vec![0u8; w * h * 3];
    let default_dec = decode == cs.default_decode(bpc);
    match (&cs, n, bpc) {
        (ColorSpace::Gray, 1, 8) if default_dec => {
            for (i, &g) in raw.iter().enumerate() {
                let g = g as u8;
                rgb[i * 3..i * 3 + 3].copy_from_slice(&[g, g, g]);
            }
        }
        (ColorSpace::Rgb, 3, 8) if default_dec => {
            for (o, &v) in rgb.iter_mut().zip(raw.iter()) {
                *o = v as u8;
            }
        }
        _ if n == 1 && bpc <= 8 => {
            // One component: a table of every sample value.
            let lut: Vec<[u8; 3]> = (0..=max as u32)
                .map(|v| cs.to_rgb(&[decode[0] + f64::from(v) * (decode[1] - decode[0]) / max]))
                .collect();
            for (i, &v) in raw.iter().enumerate() {
                let c = lut[(v as usize).min(lut.len() - 1)];
                rgb[i * 3..i * 3 + 3].copy_from_slice(&c);
            }
        }
        _ => {
            let mut comps = vec![0.0; n];
            let mut last: Option<(Vec<u16>, [u8; 3])> = None;
            for i in 0..w * h {
                let px = &raw[i * n..i * n + n];
                if let Some((k, c)) = &last
                    && k.as_slice() == px
                {
                    rgb[i * 3..i * 3 + 3].copy_from_slice(c);
                    continue;
                }
                for k in 0..n {
                    comps[k] = decode[2 * k] + f64::from(px[k]) * (decode[2 * k + 1] - decode[2 * k]) / max;
                }
                let c = cs.to_rgb(&comps);
                rgb[i * 3..i * 3 + 3].copy_from_slice(&c);
                last = Some((px.to_vec(), c));
            }
        }
    }
    let mut alpha: Option<Vec<u8>> = None;
    // Colour-key masking on raw sample values (§8.9.6.4).
    if let Object::Array(key) = doc.resolve(d.get("Mask").unwrap_or(&Object::Null)) {
        let ranges: Vec<u16> =
            key.iter().filter_map(|x| doc.resolve(x).as_i64()).map(|v| v.clamp(0, 65535) as u16).collect();
        if ranges.len() >= 2 * n {
            let a: Vec<u8> = (0..w * h)
                .map(|i| {
                    let px = &raw[i * n..i * n + n];
                    let hit = (0..n).all(|k| px[k] >= ranges[2 * k] && px[k] <= ranges[2 * k + 1]);
                    if hit { 0 } else { 255 }
                })
                .collect();
            alpha = Some(a);
        }
    }
    // An explicit stencil mask (sample 1 hides) or a soft mask.
    let mask_img = match d.get("SMask").map(|o| doc.resolve(o)) {
        Some(Object::Stream(s)) => mask_from(doc, &s, false),
        _ => match d.get("Mask").map(|o| doc.resolve(o)) {
            Some(Object::Stream(s)) => mask_from(doc, &s, true),
            _ => None,
        },
    };
    if let Some((mw, mh, m)) = mask_img {
        let resized = resize_nearest(&m, mw, mh, w, h);
        alpha = Some(match alpha {
            Some(a) => a.iter().zip(resized.iter()).map(|(&x, &y)| crate::raster::mul8(x, y)).collect(),
            None => resized,
        });
    }
    Ok(shrink(RasterImage { width: w, height: h, rgb, alpha, stencil: false, interpolate }, target))
}

/// A mask image as 8-bit opacity: a soft mask's grey values, or an
/// explicit mask's inverted bits.
fn mask_from(doc: &Document, s: &Arc<crate::object::Stream>, explicit: bool) -> Option<(usize, usize, Vec<u8>)> {
    let d = &s.dict;
    let data = doc.decode_stream(s).ok()?;
    let w = int(doc, d, &["Width"]).filter(|v| *v > 0)? as usize;
    let h = int(doc, d, &["Height"]).filter(|v| *v > 0)? as usize;
    let (w, h) = match data.jpeg {
        Some(j) => (j.width as usize, j.height as usize),
        None => (w, h),
    };
    if w.saturating_mul(h) > MAX_PIXELS {
        return None;
    }
    let bpc = if data.jpeg.is_some() {
        8
    } else if explicit {
        1
    } else {
        int(doc, d, &["BitsPerComponent"]).unwrap_or(8) as u32
    };
    let bpc = if matches!(bpc, 1 | 2 | 4 | 8 | 16) { bpc } else { 8 };
    let comps = data.jpeg.map_or(1, |j| usize::from(j.components.max(1)));
    let raw = unpack(&data.data, w, h, comps, bpc);
    let max = ((1u32 << bpc) - 1) as f64;
    let dec = doc.numbers_in(d, "Decode");
    let (d0, d1) = if dec.len() >= 2 { (dec[0], dec[1]) } else { (0.0, 1.0) };
    let out: Vec<u8> = (0..w * h)
        .map(|i| {
            let v = d0 + f64::from(raw[i * comps]) / max * (d1 - d0);
            let v = if explicit { 1.0 - v } else { v };
            (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        })
        .collect();
    Some((w, h, out))
}

/// Unpacks `w × h` pixels of `n` components at `bpc` bits (rows start
/// on byte boundaries); missing data reads as zero.
pub fn unpack(data: &[u8], w: usize, h: usize, n: usize, bpc: u32) -> Vec<u16> {
    let count = w * h * n;
    let mut out = vec![0u16; count];
    let row_bytes = (w * n * bpc as usize).div_ceil(8);
    match bpc {
        8 => {
            for y in 0..h {
                let src = data.get(y * row_bytes..).unwrap_or(&[]);
                let dst = &mut out[y * w * n..(y + 1) * w * n];
                for (o, &b) in dst.iter_mut().zip(src.iter()) {
                    *o = u16::from(b);
                }
            }
        }
        16 => {
            for y in 0..h {
                for x in 0..w * n {
                    let i = y * row_bytes + x * 2;
                    let v = match (data.get(i), data.get(i + 1)) {
                        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]),
                        _ => 0,
                    };
                    out[y * w * n + x] = v;
                }
            }
        }
        _ => {
            let b = bpc as usize;
            let mask = (1u16 << b) - 1;
            for y in 0..h {
                let row = data.get(y * row_bytes..).unwrap_or(&[]);
                for x in 0..w * n {
                    let bit = x * b;
                    let Some(&byte) = row.get(bit / 8) else { break };
                    out[y * w * n + x] = (u16::from(byte) >> (8 - b - bit % 8)) & mask;
                }
            }
        }
    }
    out
}

fn resize_nearest(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; dw * dh];
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            out[y * dw + x] = src[sy * sw + sx];
        }
    }
    out
}

/// Halves the image with a box filter while it stays over twice the
/// size it is drawn at.
pub fn shrink(mut img: RasterImage, target: (usize, usize)) -> RasterImage {
    let (tw, th) = (target.0.max(1), target.1.max(1));
    while img.width >= tw * 4 && img.height >= th * 4 && img.width >= 2 && img.height >= 2 {
        img = halve(&img);
    }
    img
}

fn halve(img: &RasterImage) -> RasterImage {
    let (w, h) = (img.width / 2, img.height / 2);
    let avg4 = |a: u8, b: u8, c: u8, d: u8| ((u16::from(a) + u16::from(b) + u16::from(c) + u16::from(d) + 2) / 4) as u8;
    let mut rgb = Vec::new();
    if !img.stencil {
        rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                for k in 0..3 {
                    let p = |xx: usize, yy: usize| img.rgb[(yy * img.width + xx) * 3 + k];
                    rgb[(y * w + x) * 3 + k] =
                        avg4(p(2 * x, 2 * y), p(2 * x + 1, 2 * y), p(2 * x, 2 * y + 1), p(2 * x + 1, 2 * y + 1));
                }
            }
        }
    }
    let alpha = img.alpha.as_ref().map(|a| {
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let p = |xx: usize, yy: usize| a[yy * img.width + xx];
                out[y * w + x] =
                    avg4(p(2 * x, 2 * y), p(2 * x + 1, 2 * y), p(2 * x, 2 * y + 1), p(2 * x + 1, 2 * y + 1));
            }
        }
        out
    });
    RasterImage { width: w, height: h, rgb, alpha, stencil: img.stencil, interpolate: img.interpolate }
}

/// Samples an image placed by `ctm` (the unit square to device space).
pub struct ImageShader<'a> {
    pub img: &'a RasterImage,
    /// Device pixel to image pixel coordinates.
    inv: Matrix,
    smooth: bool,
    color: [u8; 3],
}

impl<'a> ImageShader<'a> {
    /// `ctm` maps the unit square to device pixels; `fill` colours a stencil.
    pub fn new(img: &'a RasterImage, ctm: &Matrix, fill: [u8; 3]) -> Option<ImageShader<'a>> {
        // Unit square (y up) to image pixels (y down).
        let to_img = Matrix::new(img.width as f64, 0.0, 0.0, -(img.height as f64), 0.0, img.height as f64);
        let inv = ctm.invert()?.then(&to_img);
        // Enlarged more than twice and not asked to smooth: keep pixels.
        let scale_x = (ctm.a * ctm.a + ctm.b * ctm.b).sqrt() / img.width as f64;
        let scale_y = (ctm.c * ctm.c + ctm.d * ctm.d).sqrt() / img.height as f64;
        let smooth = img.interpolate || scale_x.min(scale_y) < 2.0 || img.stencil;
        Some(ImageShader { img, inv, smooth, color: fill })
    }

    fn sample_rgb(&self, x: usize, y: usize) -> [u8; 3] {
        if self.img.stencil {
            return self.color;
        }
        let i = (y * self.img.width + x) * 3;
        [self.img.rgb[i], self.img.rgb[i + 1], self.img.rgb[i + 2]]
    }

    fn sample_a(&self, x: usize, y: usize) -> u8 {
        self.img.alpha.as_ref().map_or(255, |a| a[y * self.img.width + x])
    }
}

impl Shader for ImageShader<'_> {
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8) {
        let p = self.inv.apply(crate::geom::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
        let (w, h) = (self.img.width, self.img.height);
        if !self.smooth {
            let ix = (p.x.floor().max(0.0) as usize).min(w - 1);
            let iy = (p.y.floor().max(0.0) as usize).min(h - 1);
            return (self.sample_rgb(ix, iy), self.sample_a(ix, iy));
        }
        // Bilinear between the four nearest pixel centres.
        let fx = (p.x - 0.5).clamp(0.0, (w - 1) as f64);
        let fy = (p.y - 0.5).clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let wts = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
        let pts = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        let mut a = 0.0;
        for (k, &(px, py)) in pts.iter().enumerate() {
            a += wts[k] * f64::from(self.sample_a(px, py));
        }
        if self.img.stencil {
            return (self.color, (a + 0.5) as u8);
        }
        let mut c = [0.0f64; 3];
        for (k, &(px, py)) in pts.iter().enumerate() {
            let s = self.sample_rgb(px, py);
            for j in 0..3 {
                c[j] += wts[k] * f64::from(s[j]);
            }
        }
        ([(c[0] + 0.5) as u8, (c[1] + 0.5) as u8, (c[2] + 0.5) as u8], (a + 0.5) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacking_every_depth() {
        assert_eq!(unpack(&[0b1010_0000], 3, 1, 1, 1), vec![1, 0, 1]);
        assert_eq!(unpack(&[0b1110_0100, 0xff], 2, 2, 1, 2), vec![3, 2, 3, 3]);
        assert_eq!(unpack(&[0x12, 0x34], 2, 1, 2, 4), vec![1, 2, 3, 4]);
        assert_eq!(unpack(&[1, 2, 3, 4], 1, 1, 2, 16), vec![0x0102, 0x0304]);
        // Short data reads as zero.
        assert_eq!(unpack(&[9], 2, 2, 1, 8), vec![9, 0, 0, 0]);
    }

    #[test]
    fn shrink_halves_by_box() {
        let img = RasterImage {
            width: 8,
            height: 8,
            rgb: vec![200; 8 * 8 * 3],
            alpha: None,
            stencil: false,
            interpolate: false,
        };
        let s = shrink(img, (2, 2));
        assert_eq!((s.width, s.height), (4, 4));
        assert!(s.rgb.iter().all(|&v| v == 200));
    }

    #[test]
    fn shader_maps_the_unit_square() {
        // A 2×1 image: left red, right blue, drawn 20×10 at the origin.
        let img = RasterImage {
            width: 2,
            height: 1,
            rgb: vec![255, 0, 0, 0, 0, 255],
            alpha: None,
            stencil: false,
            interpolate: false,
        };
        let ctm = Matrix::new(20.0, 0.0, 0.0, -10.0, 0.0, 10.0);
        let s = ImageShader::new(&img, &ctm, [0, 0, 0]).unwrap();
        assert_eq!(s.shade(1, 5).0, [255, 0, 0]);
        assert_eq!(s.shade(18, 5).0, [0, 0, 255]);
    }
}
