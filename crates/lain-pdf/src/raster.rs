//! The anti-aliased rasterizer: exact-area coverage accumulation (each
//! edge adds its signed area to the cells it crosses, a running sum per
//! row gives the winding), the canvas with optional alpha, clip masks
//! and source-over compositing with the separable blend modes.

use crate::geom::{Path, Point, Seg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

impl Blend {
    pub fn from_name(n: &str) -> Blend {
        match n {
            "Multiply" => Blend::Multiply,
            "Screen" => Blend::Screen,
            "Overlay" => Blend::Overlay,
            "Darken" => Blend::Darken,
            "Lighten" => Blend::Lighten,
            "ColorDodge" => Blend::ColorDodge,
            "ColorBurn" => Blend::ColorBurn,
            "HardLight" => Blend::HardLight,
            "SoftLight" => Blend::SoftLight,
            "Difference" => Blend::Difference,
            "Exclusion" => Blend::Exclusion,
            // Non-separable modes fall back to Normal.
            _ => Blend::Normal,
        }
    }

    fn apply(self, s: f32, d: f32) -> f32 {
        match self {
            Blend::Normal => s,
            Blend::Multiply => s * d,
            Blend::Screen => s + d - s * d,
            Blend::Overlay => Blend::HardLight.apply(d, s),
            Blend::Darken => s.min(d),
            Blend::Lighten => s.max(d),
            Blend::ColorDodge => {
                if d == 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (d / (1.0 - s)).min(1.0)
                }
            }
            Blend::ColorBurn => {
                if d >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - d) / s).min(1.0)
                }
            }
            Blend::HardLight => {
                if s <= 0.5 {
                    Blend::Multiply.apply(2.0 * s, d)
                } else {
                    Blend::Screen.apply(2.0 * s - 1.0, d)
                }
            }
            Blend::SoftLight => {
                if s <= 0.5 {
                    d - (1.0 - 2.0 * s) * d * (1.0 - d)
                } else {
                    let dd = if d <= 0.25 { ((16.0 * d - 12.0) * d + 4.0) * d } else { d.sqrt() };
                    d + (2.0 * s - 1.0) * (dd - d)
                }
            }
            Blend::Difference => (s - d).abs(),
            Blend::Exclusion => s + d - 2.0 * s * d,
        }
    }
}

/// An integer pixel rectangle `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }

    pub fn width(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }

    pub fn height(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
}

/// Coverage of a shape over a rectangle, 0..=255 per pixel.
#[derive(Debug, Clone)]
pub struct Coverage {
    pub rect: IRect,
    pub data: Vec<u8>,
}

impl Coverage {
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < self.rect.x0 || y < self.rect.y0 || x >= self.rect.x1 || y >= self.rect.y1 {
            return 0;
        }
        self.data[(y - self.rect.y0) as usize * self.rect.width() + (x - self.rect.x0) as usize]
    }
}

/// A clip: full-canvas coverage plus the bounding box of where it is
/// not zero, so work outside it is skipped.
#[derive(Debug, Clone)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
    pub bounds: IRect,
}

impl Mask {
    /// The intersection of `self` (or the whole canvas) with `cov`.
    pub fn intersect(prev: Option<&Mask>, cov: &Coverage, width: usize, height: usize) -> Mask {
        let mut data = vec![0u8; width * height];
        let r =
            cov.rect.intersect(&prev.map_or(IRect { x0: 0, y0: 0, x1: width as i32, y1: height as i32 }, |m| m.bounds));
        let (mut bx0, mut by0, mut bx1, mut by1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        if !r.is_empty() {
            for y in r.y0..r.y1 {
                let crow = (y - cov.rect.y0) as usize * cov.rect.width();
                let mrow = y as usize * width;
                for x in r.x0..r.x1 {
                    let c = cov.data[crow + (x - cov.rect.x0) as usize];
                    let v = match prev {
                        Some(m) => mul8(c, m.data[mrow + x as usize]),
                        None => c,
                    };
                    if v != 0 {
                        data[mrow + x as usize] = v;
                        bx0 = bx0.min(x);
                        by0 = by0.min(y);
                        bx1 = bx1.max(x + 1);
                        by1 = by1.max(y + 1);
                    }
                }
            }
        }
        let bounds = if bx0 == i32::MAX {
            IRect { x0: 0, y0: 0, x1: 0, y1: 0 }
        } else {
            IRect { x0: bx0, y0: by0, x1: bx1, y1: by1 }
        };
        Mask { width, height, data, bounds }
    }

    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width + x]
    }
}

#[inline]
pub fn mul8(a: u8, b: u8) -> u8 {
    let t = u32::from(a) * u32::from(b) + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// Straight-alpha RGB pixels; `alpha` is `None` for an opaque canvas
/// (a page), `Some` for a transparency group drawn off screen.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
}

impl Canvas {
    pub fn white(width: usize, height: usize) -> Canvas {
        Canvas { width, height, rgb: vec![255; width * height * 3], alpha: None }
    }

    pub fn transparent(width: usize, height: usize) -> Canvas {
        Canvas { width, height, rgb: vec![0; width * height * 3], alpha: Some(vec![0; width * height]) }
    }

    pub fn rect(&self) -> IRect {
        IRect { x0: 0, y0: 0, x1: self.width as i32, y1: self.height as i32 }
    }

    /// Composites one pixel: `src` at coverage-and-opacity `a` (0..=255).
    #[inline]
    pub fn blend_pixel(&mut self, i: usize, src: [u8; 3], a: u8, mode: Blend) {
        if a == 0 {
            return;
        }
        let p = i * 3;
        match &mut self.alpha {
            None => {
                let src = if mode == Blend::Normal {
                    src
                } else {
                    blended(mode, src, [self.rgb[p], self.rgb[p + 1], self.rgb[p + 2]])
                };
                if a == 255 {
                    self.rgb[p..p + 3].copy_from_slice(&src);
                } else {
                    for k in 0..3 {
                        let d = i32::from(self.rgb[p + k]);
                        let s = i32::from(src[k]);
                        self.rgb[p + k] = (d + ((s - d) * i32::from(a) + 127) / 255) as u8;
                    }
                }
            }
            Some(alpha) => {
                let da = u32::from(alpha[i]);
                let sa = u32::from(a);
                let src = if mode == Blend::Normal || da == 0 {
                    src
                } else {
                    // Blend against the backdrop where there is one.
                    let b = blended(mode, src, [self.rgb[p], self.rgb[p + 1], self.rgb[p + 2]]);
                    let mut out = [0u8; 3];
                    for k in 0..3 {
                        out[k] = ((u32::from(src[k]) * (255 - da) + u32::from(b[k]) * da) / 255) as u8;
                    }
                    out
                };
                let oa = sa + da * (255 - sa) / 255;
                if oa == 0 {
                    return;
                }
                for k in 0..3 {
                    let c = (u32::from(src[k]) * sa + u32::from(self.rgb[p + k]) * da * (255 - sa) / 255) / oa;
                    self.rgb[p + k] = c.min(255) as u8;
                }
                alpha[i] = oa.min(255) as u8;
            }
        }
    }

    /// Composites another canvas (a group) at opacity `alpha`, through
    /// an optional mask.
    pub fn composite(&mut self, group: &Canvas, alpha: u8, mask: Option<&Mask>, soft: Option<&Mask>, mode: Blend) {
        let n = self.width.min(group.width) * self.height.min(group.height);
        if n == 0 || self.width != group.width {
            return;
        }
        for i in 0..n {
            let ga = group.alpha.as_ref().map_or(255, |a| a[i]);
            if ga == 0 {
                continue;
            }
            let mut a = mul8(ga, alpha);
            if let Some(m) = mask {
                a = mul8(a, m.data[i]);
            }
            if let Some(s) = soft {
                a = mul8(a, s.data[i]);
            }
            let p = i * 3;
            self.blend_pixel(i, [group.rgb[p], group.rgb[p + 1], group.rgb[p + 2]], a, mode);
        }
    }
}

fn blended(mode: Blend, s: [u8; 3], d: [u8; 3]) -> [u8; 3] {
    let mut out = [0u8; 3];
    for k in 0..3 {
        let v = mode.apply(f32::from(s[k]) / 255.0, f32::from(d[k]) / 255.0);
        out[k] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    out
}

/// What a fill paints with: one colour, or a colour per pixel.
pub trait Shader {
    /// Colour and opacity at the centre of device pixel `(x, y)`.
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8);
}

pub struct Solid(pub [u8; 3]);

impl Shader for Solid {
    fn shade(&self, _x: i32, _y: i32) -> ([u8; 3], u8) {
        (self.0, 255)
    }
}

/// Paints `shader` through `cov`, at `alpha`, clipped by `clip` and the
/// soft mask `soft`.
pub fn paint(
    canvas: &mut Canvas,
    cov: &Coverage,
    shader: &dyn Shader,
    alpha: u8,
    clip: Option<&Mask>,
    soft: Option<&Mask>,
    mode: Blend,
) {
    let mut r = cov.rect.intersect(&canvas.rect());
    if let Some(m) = clip {
        r = r.intersect(&m.bounds);
    }
    if r.is_empty() || alpha == 0 {
        return;
    }
    let w = canvas.width;
    let cw = cov.rect.width();
    for y in r.y0..r.y1 {
        let crow = (y - cov.rect.y0) as usize * cw;
        for x in r.x0..r.x1 {
            let c = cov.data[crow + (x - cov.rect.x0) as usize];
            if c == 0 {
                continue;
            }
            let i = y as usize * w + x as usize;
            let mut a = mul8(c, alpha);
            if let Some(m) = clip {
                a = mul8(a, m.data[i]);
                if a == 0 {
                    continue;
                }
            }
            if let Some(s) = soft {
                a = mul8(a, s.data[i]);
            }
            let (rgb, sa) = shader.shade(x, y);
            let a = if sa == 255 { a } else { mul8(a, sa) };
            canvas.blend_pixel(i, rgb, a, mode);
        }
    }
}

// ---- flattening ----------------------------------------------------------

/// Device-space line segments of a path: curves flattened to within
/// `tol` pixels, every contour closed.
pub fn flatten(path: &Path, tol: f64) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    flatten_into(path, tol, &mut |a, b| out.push((a, b)), true);
    out
}

/// Calls `line` for each flattened segment; `close` adds the closing
/// segment of open contours (filling) or not (stroking polylines).
pub fn flatten_into(path: &Path, tol: f64, line: &mut dyn FnMut(Point, Point), close: bool) {
    let mut start = Point::default();
    let mut cur = Point::default();
    let mut open = false;
    let tol = tol.max(0.01);
    for s in &path.segs {
        match *s {
            Seg::Move(p) => {
                if open && close && cur != start {
                    line(cur, start);
                }
                start = p;
                cur = p;
                open = true;
            }
            Seg::Line(p) => {
                if !open {
                    start = cur;
                    open = true;
                }
                line(cur, p);
                cur = p;
            }
            Seg::Quad(c, p) => {
                if !open {
                    start = cur;
                    open = true;
                }
                let dd = ((cur.x - 2.0 * c.x + p.x).powi(2) + (cur.y - 2.0 * c.y + p.y).powi(2)).sqrt();
                let n = ((dd / (4.0 * tol)).sqrt().ceil() as usize).clamp(1, 200);
                let mut prev = cur;
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let mt = 1.0 - t;
                    let q = Point::new(
                        mt * mt * cur.x + 2.0 * mt * t * c.x + t * t * p.x,
                        mt * mt * cur.y + 2.0 * mt * t * c.y + t * t * p.y,
                    );
                    line(prev, q);
                    prev = q;
                }
                cur = p;
            }
            Seg::Cubic(c1, c2, p) => {
                if !open {
                    start = cur;
                    open = true;
                }
                let d1 = ((cur.x - 2.0 * c1.x + c2.x).powi(2) + (cur.y - 2.0 * c1.y + c2.y).powi(2)).sqrt();
                let d2 = ((c1.x - 2.0 * c2.x + p.x).powi(2) + (c1.y - 2.0 * c2.y + p.y).powi(2)).sqrt();
                let dd = d1.max(d2);
                let n = ((dd * 0.75 / tol).sqrt().ceil() as usize).clamp(1, 300);
                let mut prev = cur;
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let mt = 1.0 - t;
                    let (a, b, cc, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
                    let q = Point::new(
                        a * cur.x + b * c1.x + cc * c2.x + d * p.x,
                        a * cur.y + b * c1.y + cc * c2.y + d * p.y,
                    );
                    line(prev, q);
                    prev = q;
                }
                cur = p;
            }
            Seg::Close => {
                if open && cur != start {
                    line(cur, start);
                }
                cur = start;
                open = false;
            }
        }
    }
    if open && close && cur != start {
        line(cur, start);
    }
}

// ---- coverage --------------------------------------------------------------

/// The coverage of a device-space path inside `limit`.
pub fn coverage(path: &Path, rule: FillRule, limit: IRect) -> Option<Coverage> {
    if let Some(c) = rect_coverage(path, limit) {
        return Some(c);
    }
    let lines = flatten(path, 0.2);
    coverage_lines(&lines, rule, limit)
}

/// The coverage of closed line contours.
pub fn coverage_lines(lines: &[(Point, Point)], rule: FillRule, limit: IRect) -> Option<Coverage> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (a, b) in lines {
        for p in [a, b] {
            if !p.x.is_finite() || !p.y.is_finite() {
                return None;
            }
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
    }
    if x0 > x1 {
        return None;
    }
    let bbox = IRect {
        x0: clamp_i(x0.floor()),
        y0: clamp_i(y0.floor()),
        x1: clamp_i(x1.ceil()) + 1,
        y1: clamp_i(y1.ceil()) + 1,
    };
    let r = bbox.intersect(&limit);
    if r.is_empty() {
        return None;
    }
    let (w, h) = (r.width(), r.height());
    if w.saturating_mul(h) > 1 << 28 {
        return None;
    }
    let stride = w + 2;
    let mut acc = vec![0f32; stride * h];
    let (ox, oy) = (f64::from(r.x0), f64::from(r.y0));
    for (a, b) in lines {
        accumulate(&mut acc, stride, w, h, a.x - ox, a.y - oy, b.x - ox, b.y - oy);
    }
    let mut data = vec![0u8; w * h];
    for y in 0..h {
        let mut sum = 0f32;
        let row = &acc[y * stride..y * stride + stride];
        let out = &mut data[y * w..y * w + w];
        for x in 0..w {
            sum += row[x];
            let v = match rule {
                FillRule::NonZero => sum.abs().min(1.0),
                FillRule::EvenOdd => {
                    let m = sum.abs() % 2.0;
                    if m > 1.0 { 2.0 - m } else { m }
                }
            };
            out[x] = (v * 255.0 + 0.5) as u8;
        }
    }
    Some(Coverage { rect: r, data })
}

fn clamp_i(v: f64) -> i32 {
    v.clamp(-1.0e8, 1.0e8) as i32
}

/// Adds one edge's signed area to the cells it crosses (the font-rs
/// accumulation scheme). `x` is clamped into the row, so edges left of
/// the box still count fully and edges right of it not at all.
#[allow(clippy::too_many_arguments)]
fn accumulate(acc: &mut [f32], stride: usize, w: usize, h: usize, x0: f64, y0: f64, x1: f64, y1: f64) {
    if y0 == y1 {
        return;
    }
    let (dir, x0, y0, x1, y1) = if y0 < y1 { (1.0f32, x0, y0, x1, y1) } else { (-1.0f32, x1, y1, x0, y0) };
    let dxdy = (x1 - x0) / (y1 - y0);
    let hf = h as f64;
    if y1 <= 0.0 || y0 >= hf {
        return;
    }
    let (mut x, mut ys) = (x0, y0);
    if ys < 0.0 {
        x -= ys * dxdy;
        ys = 0.0;
    }
    let ye = y1.min(hf);
    let wf = w as f64;
    let mut y = ys.floor() as usize;
    let yend = ye.ceil() as usize;
    while y < yend && y < h {
        let line = y * stride;
        let yf = y as f64;
        let dy = (yf + 1.0).min(ye) - yf.max(ys);
        let xnext = x + dxdy * dy;
        let d = (dy as f32) * dir;
        let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let xa = xa.clamp(0.0, wf);
        let xb = xb.clamp(0.0, wf);
        let x0f = xa.floor();
        let x0i = x0f as usize;
        let x1c = xb.ceil();
        let x1i = x1c as usize;
        if x1i <= x0i + 1 {
            let xmf = (0.5 * (xa + xb) - x0f) as f32;
            acc[line + x0i] += d - d * xmf;
            acc[line + x0i + 1] += d * xmf;
        } else {
            let s = (1.0 / (xb - xa)) as f32;
            let x0frac = (xa - x0f) as f32;
            let a0 = 0.5 * s * (1.0 - x0frac) * (1.0 - x0frac);
            let x1frac = (xb - x1c + 1.0) as f32;
            let am = 0.5 * s * x1frac * x1frac;
            acc[line + x0i] += d * a0;
            if x1i == x0i + 2 {
                acc[line + x0i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0frac);
                acc[line + x0i + 1] += d * (a1 - a0);
                for xi in x0i + 2..x1i - 1 {
                    acc[line + xi] += d * s;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                acc[line + x1i - 1] += d * (1.0 - a2 - am);
            }
            acc[line + x1i] += d * am;
        }
        x = xnext;
        y += 1;
    }
}

/// An axis-aligned rectangle's exact coverage, without the general
/// rasterizer (page backgrounds, table rules, clip boxes).
fn rect_coverage(path: &Path, limit: IRect) -> Option<Coverage> {
    let pts = rect_points(path)?;
    let (x0, x1) = (pts[0].x.min(pts[2].x), pts[0].x.max(pts[2].x));
    let (y0, y1) = (pts[0].y.min(pts[2].y), pts[0].y.max(pts[2].y));
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
        return None;
    }
    let r = IRect { x0: clamp_i(x0.floor()), y0: clamp_i(y0.floor()), x1: clamp_i(x1.ceil()), y1: clamp_i(y1.ceil()) }
        .intersect(&limit);
    if r.is_empty() {
        return Some(Coverage { rect: IRect { x0: 0, y0: 0, x1: 0, y1: 0 }, data: Vec::new() });
    }
    let (w, h) = (r.width(), r.height());
    let span =
        |lo: f64, hi: f64, i: i32| -> f64 { (hi.min(f64::from(i) + 1.0) - lo.max(f64::from(i))).clamp(0.0, 1.0) };
    let cols: Vec<f64> = (r.x0..r.x1).map(|x| span(x0, x1, x)).collect();
    let mut data = vec![0u8; w * h];
    for (j, y) in (r.y0..r.y1).enumerate() {
        let fy = span(y0, y1, y);
        for (i, fx) in cols.iter().enumerate() {
            data[j * w + i] = (fx * fy * 255.0 + 0.5) as u8;
        }
    }
    Some(Coverage { rect: r, data })
}

/// The four corners when the path is one axis-aligned rectangle.
fn rect_points(path: &Path) -> Option<[Point; 4]> {
    let mut pts: Vec<Point> = Vec::with_capacity(5);
    for (i, s) in path.segs.iter().enumerate() {
        match *s {
            Seg::Move(p) if i == 0 => pts.push(p),
            Seg::Line(p) if i > 0 => pts.push(p),
            Seg::Close if i == path.segs.len() - 1 => {}
            _ => return None,
        }
    }
    if pts.len() == 5 && pts[4] == pts[0] {
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    let horiz = |a: Point, b: Point| (a.y - b.y).abs() < 1e-9;
    let vert = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9;
    let ok = (horiz(pts[0], pts[1]) && vert(pts[1], pts[2]) && horiz(pts[2], pts[3]) && vert(pts[3], pts[0]))
        || (vert(pts[0], pts[1]) && horiz(pts[1], pts[2]) && vert(pts[2], pts[3]) && horiz(pts[3], pts[0]));
    ok.then(|| [pts[0], pts[1], pts[2], pts[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Path {
        let mut p = Path::new();
        p.move_to(x, y);
        p.line_to(x + s, y);
        p.line_to(x + s, y + s);
        p.line_to(x, y + s);
        p.close();
        p
    }

    fn all() -> IRect {
        IRect { x0: 0, y0: 0, x1: 100, y1: 100 }
    }

    #[test]
    fn exact_area_coverage() {
        // A triangle covering half of a 10×10 box: total coverage 50 px.
        let mut p = Path::new();
        p.move_to(10.0, 10.0);
        p.line_to(20.0, 10.0);
        p.line_to(10.0, 20.0);
        p.close();
        let c = coverage(&p, FillRule::NonZero, all()).unwrap();
        let total: f64 = c.data.iter().map(|&v| f64::from(v) / 255.0).sum();
        assert!((total - 50.0).abs() < 0.5, "{total}");
        // Half-pixel rectangle edges give half coverage.
        let c = coverage(&square(10.5, 10.0, 2.0), FillRule::NonZero, all()).unwrap();
        assert_eq!((c.get(10, 10), c.get(11, 10), c.get(12, 10)), (128, 255, 128));
    }

    #[test]
    fn winding_rules() {
        // Two same-direction squares nested: nonzero fills the hole,
        // even-odd leaves it.
        let mut p = square(10.0, 10.0, 30.0);
        let inner = square(20.0, 20.0, 10.0);
        p.segs.extend(inner.segs);
        let nz = coverage(&p, FillRule::NonZero, all()).unwrap();
        let eo = coverage(&p, FillRule::EvenOdd, all()).unwrap();
        assert_eq!(nz.get(25, 25), 255);
        assert_eq!(eo.get(25, 25), 0);
        assert_eq!(eo.get(15, 15), 255);
        // A curve flattens inside its hull.
        let mut c = Path::new();
        c.move_to(0.0, 50.0);
        c.cubic_to(0.0, 0.0, 100.0, 0.0, 100.0, 50.0);
        c.close();
        let cov = coverage(&c, FillRule::NonZero, all()).unwrap();
        assert_eq!(cov.get(50, 45), 255);
        assert_eq!(cov.get(50, 5), 0);
    }

    #[test]
    fn clipping_outside_the_canvas() {
        let p = square(-50.0, -50.0, 80.0);
        let c = coverage(&p, FillRule::NonZero, all()).unwrap();
        assert_eq!(c.rect, IRect { x0: 0, y0: 0, x1: 30, y1: 30 });
        assert_eq!(c.get(0, 0), 255);
        // A diagonal shape cut by the left edge still fills.
        let mut t = Path::new();
        t.move_to(-20.0, 0.0);
        t.line_to(20.0, 40.0);
        t.line_to(-20.0, 40.0);
        t.close();
        let c = coverage(&t, FillRule::NonZero, all()).unwrap();
        assert_eq!(c.get(0, 35), 255);
        assert_eq!(c.get(10, 5), 0);
        assert!(coverage(&square(200.0, 200.0, 5.0), FillRule::NonZero, all()).is_none_or(|c| c.data.is_empty()));
    }

    #[test]
    fn painting_and_masks() {
        let mut cv = Canvas::white(20, 20);
        let cov = coverage(&square(0.0, 0.0, 10.0), FillRule::NonZero, cv.rect()).unwrap();
        let clip_cov = coverage(&square(5.0, 0.0, 15.0), FillRule::NonZero, cv.rect()).unwrap();
        let mask = Mask::intersect(None, &clip_cov, 20, 20);
        paint(&mut cv, &cov, &Solid([255, 0, 0]), 255, Some(&mask), None, Blend::Normal);
        let px = |cv: &Canvas, x: usize, y: usize| {
            [cv.rgb[(y * 20 + x) * 3], cv.rgb[(y * 20 + x) * 3 + 1], cv.rgb[(y * 20 + x) * 3 + 2]]
        };
        assert_eq!(px(&cv, 2, 2), [255, 255, 255]);
        assert_eq!(px(&cv, 7, 2), [255, 0, 0]);
        assert_eq!(px(&cv, 15, 2), [255, 255, 255]);
        // Half opacity, then multiply.
        paint(&mut cv, &cov, &Solid([0, 0, 255]), 128, None, None, Blend::Normal);
        assert_eq!(px(&cv, 2, 2), [128, 128, 255]);
        let mut cv2 = Canvas::white(20, 20);
        paint(&mut cv2, &cov, &Solid([255, 0, 0]), 255, None, None, Blend::Normal);
        paint(&mut cv2, &cov, &Solid([0, 255, 255]), 255, None, None, Blend::Multiply);
        assert_eq!(px(&cv2, 2, 2), [0, 0, 0]);
        // A transparent group composited at half opacity.
        let mut g = Canvas::transparent(20, 20);
        paint(&mut g, &cov, &Solid([0, 0, 0]), 255, None, None, Blend::Normal);
        let mut page = Canvas::white(20, 20);
        page.composite(&g, 128, None, None, Blend::Normal);
        assert_eq!(px(&page, 2, 2), [128, 128, 128]);
        assert_eq!(px(&page, 15, 15), [255, 255, 255]);
    }

    #[test]
    fn hostile_paths() {
        let mut p = Path::new();
        p.move_to(f64::NAN, 0.0);
        p.line_to(1e300, -1e300);
        assert!(coverage(&p, FillRule::NonZero, all()).is_none());
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.line_to(1e12, 1e12);
        p.line_to(0.0, 1e12);
        let _ = coverage(&p, FillRule::NonZero, all());
    }
}
