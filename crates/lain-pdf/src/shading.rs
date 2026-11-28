//! Shadings (§8.7.4.5): function-based, axial and radial ones evaluated
//! per pixel through a lookup table, and the mesh shadings (free-form
//! and lattice triangles, Coons and tensor patches) drawn as Gouraud
//! triangles into an off-screen buffer.

use crate::color::ColorSpace;
use crate::document::Document;
use crate::function::Function;
use crate::geom::{Matrix, Point};
use crate::object::{Dict, Object};
use crate::raster::{IRect, Shader};

const LUT: usize = 1024;
const MAX_TRIANGLES: usize = 2_000_000;

/// A Gouraud triangle: three points with their colour components.
type Triangle = [(Point, Vec<f64>); 3];

#[derive(Clone)]
pub struct Shading {
    pub kind: i64,
    pub cs: ColorSpace,
    pub background: Option<[u8; 3]>,
    pub bbox: Option<[f64; 4]>,
    dict: Dict,
    stream: Option<std::sync::Arc<crate::object::Stream>>,
    function: Option<Function>,
}

impl Shading {
    pub fn parse(doc: &Document, o: &Object, res: Option<&Dict>) -> Option<Shading> {
        let o = doc.resolve(o);
        let d = o.as_dict()?.clone();
        let kind = doc.get_in(&d, "ShadingType").as_i64()?;
        let cs = ColorSpace::parse(doc, d.get("ColorSpace").unwrap_or(&Object::Null), res);
        let function = d.get("Function").and_then(|f| Function::parse(doc, f));
        let bg = doc.numbers_in(&d, "Background");
        let background = (bg.len() >= cs.components() && !bg.is_empty()).then(|| cs.to_rgb(&bg));
        let b = doc.numbers_in(&d, "BBox");
        let bbox = (b.len() == 4).then(|| [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])]);
        Some(Shading { kind, cs, background, bbox, stream: o.as_stream().cloned(), dict: d, function })
    }

    fn color(&self, comps: &[f64]) -> [u8; 3] {
        match &self.function {
            Some(f) => self.cs.to_rgb(&f.eval(comps)),
            None => self.cs.to_rgb(comps),
        }
    }

    /// A shader for this shading under `ctm` (shading space to device).
    /// Mesh shadings are drawn into a buffer covering `area`.
    pub fn shader(&self, doc: &Document, ctm: &Matrix, area: IRect) -> Option<Box<dyn Shader>> {
        let inv = ctm.invert()?;
        match self.kind {
            1 => {
                let dom = non_empty(doc.numbers_in(&self.dict, "Domain"), vec![0.0, 1.0, 0.0, 1.0]);
                let m = doc.numbers_in(&self.dict, "Matrix");
                let m = if m.len() == 6 { Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5]) } else { Matrix::IDENTITY };
                let inv = m.then(ctm).invert()?;
                Some(Box::new(FunctionShader { sh: self.clone(), inv, dom }))
            }
            2 | 3 => {
                let c = doc.numbers_in(&self.dict, "Coords");
                let need = if self.kind == 2 { 4 } else { 6 };
                if c.len() < need {
                    return None;
                }
                let dom = non_empty(doc.numbers_in(&self.dict, "Domain"), vec![0.0, 1.0]);
                let ext: Vec<bool> =
                    doc.array_in(&self.dict, "Extend").iter().map(|o| o.as_bool().unwrap_or(false)).collect();
                let extend = [ext.first().copied().unwrap_or(false), ext.get(1).copied().unwrap_or(false)];
                let lut: Vec<[u8; 3]> = (0..LUT)
                    .map(|i| {
                        let t = dom[0] + (dom[1] - dom[0]) * i as f64 / (LUT - 1) as f64;
                        self.color(&[t])
                    })
                    .collect();
                Some(Box::new(GradientShader {
                    radial: self.kind == 3,
                    c,
                    extend,
                    lut,
                    inv,
                    background: self.background,
                }))
            }
            4..=7 => {
                let tris = self.triangles(doc)?;
                Some(Box::new(MeshShader::draw(self, &tris, ctm, area)))
            }
            _ => None,
        }
    }

    /// Mesh shadings as triangles of `(point, components)`.
    fn triangles(&self, doc: &Document) -> Option<Vec<Triangle>> {
        let s = self.stream.as_ref()?;
        let data = doc.decode_stream(s).ok()?.data;
        let d = &self.dict;
        let bpcoord = doc.get_in(d, "BitsPerCoordinate").as_i64().unwrap_or(0) as u32;
        let bpcomp = doc.get_in(d, "BitsPerComponent").as_i64().unwrap_or(0) as u32;
        let bpflag = doc.get_in(d, "BitsPerFlag").as_i64().unwrap_or(0) as u32;
        let decode = doc.numbers_in(d, "Decode");
        let ncomp = if self.function.is_some() { 1 } else { self.cs.components() };
        if !(1..=32).contains(&bpcoord) || !(1..=16).contains(&bpcomp) || decode.len() < 4 + 2 * ncomp {
            return None;
        }
        let mut r = Bits { data: &data, pos: 0 };
        let rd = |r: &mut Bits, bits: u32, lo: f64, hi: f64| -> Option<f64> {
            let v = r.read(bits)?;
            let max = if bits == 32 { u32::MAX as f64 } else { ((1u64 << bits) - 1) as f64 };
            Some(lo + v as f64 * (hi - lo) / max)
        };
        let vertex = |r: &mut Bits| -> Option<(Point, Vec<f64>)> {
            let x = rd(r, bpcoord, decode[0], decode[1])?;
            let y = rd(r, bpcoord, decode[2], decode[3])?;
            let mut c = Vec::with_capacity(ncomp);
            for k in 0..ncomp {
                c.push(rd(r, bpcomp, decode[4 + 2 * k], decode[5 + 2 * k])?);
            }
            Some((Point::new(x, y), c))
        };
        let mut tris: Vec<[(Point, Vec<f64>); 3]> = Vec::new();
        match self.kind {
            4 => {
                let mut prev: Option<[(Point, Vec<f64>); 3]> = None;
                while tris.len() < MAX_TRIANGLES {
                    let Some(f) = r.read(bpflag.max(1)) else { break };
                    let Some(v) = vertex(&mut r) else { break };
                    r.align();
                    let t = match (f, prev.take()) {
                        (1, Some([_, b, c])) => [b, c.clone(), v],
                        (2, Some([a, _, c])) => [a, c.clone(), v],
                        _ => {
                            // A new triangle: two more vertices (flags ignored).
                            let mut vs = vec![v];
                            for _ in 0..2 {
                                let _ = r.read(bpflag.max(1));
                                let Some(v) = vertex(&mut r) else { break };
                                r.align();
                                vs.push(v);
                            }
                            if vs.len() < 3 {
                                break;
                            }
                            let c = vs.pop()?;
                            let b = vs.pop()?;
                            let a = vs.pop()?;
                            [a, b, c]
                        }
                    };
                    tris.push(t.clone());
                    prev = Some(t);
                }
            }
            5 => {
                let per_row = doc.get_in(d, "VerticesPerRow").as_i64().unwrap_or(0).clamp(2, 1 << 16) as usize;
                let mut rows: Vec<Vec<(Point, Vec<f64>)>> = Vec::new();
                'rows: loop {
                    let mut row = Vec::with_capacity(per_row);
                    for _ in 0..per_row {
                        let Some(v) = vertex(&mut r) else { break 'rows };
                        row.push(v);
                    }
                    rows.push(row);
                    if rows.len() * per_row * 2 > MAX_TRIANGLES {
                        break;
                    }
                }
                for w in rows.windows(2) {
                    for i in 0..per_row - 1 {
                        tris.push([w[0][i].clone(), w[0][i + 1].clone(), w[1][i].clone()]);
                        tris.push([w[0][i + 1].clone(), w[1][i + 1].clone(), w[1][i].clone()]);
                    }
                }
            }
            6 | 7 => {
                let npts = if self.kind == 6 { 12 } else { 16 };
                let mut prev: Option<(Vec<Point>, Vec<Vec<f64>>)> = None;
                while tris.len() < MAX_TRIANGLES {
                    let Some(f) = r.read(bpflag.max(1)) else { break };
                    let (np, nc) = if f == 0 { (npts, 4) } else { (npts - 4, 2) };
                    let mut pts = Vec::with_capacity(np);
                    for _ in 0..np {
                        let (Some(x), Some(y)) =
                            (rd(&mut r, bpcoord, decode[0], decode[1]), rd(&mut r, bpcoord, decode[2], decode[3]))
                        else {
                            break;
                        };
                        pts.push(Point::new(x, y));
                    }
                    let mut cols = Vec::with_capacity(nc);
                    for _ in 0..nc {
                        let mut c = Vec::with_capacity(ncomp);
                        for k in 0..ncomp {
                            let Some(v) = rd(&mut r, bpcomp, decode[4 + 2 * k], decode[5 + 2 * k]) else { break };
                            c.push(v);
                        }
                        cols.push(c);
                    }
                    r.align();
                    if pts.len() < np || cols.len() < nc || cols.iter().any(|c| c.len() < ncomp) {
                        break;
                    }
                    let (pts, cols) = match (f, prev.as_ref()) {
                        (0, _) => (pts, cols),
                        (_, Some((pp, pc))) => {
                            // Edges shared with the previous patch (§8.7.4.5.7).
                            let (shared, sc): (Vec<Point>, [Vec<f64>; 2]) = match f {
                                1 => (vec![pp[3], pp[4], pp[5], pp[6]], [pc[1].clone(), pc[2].clone()]),
                                2 => (vec![pp[6], pp[7], pp[8], pp[9]], [pc[2].clone(), pc[3].clone()]),
                                _ => (vec![pp[9], pp[10], pp[11], pp[0]], [pc[3].clone(), pc[0].clone()]),
                            };
                            let mut p = shared;
                            p.extend(pts);
                            let mut c = sc.to_vec();
                            c.extend(cols);
                            (p, c)
                        }
                        _ => break,
                    };
                    let grid = tensor_grid(&pts, self.kind == 7);
                    patch_triangles(&grid, &cols, &mut tris);
                    prev = Some((pts, cols));
                }
            }
            _ => return None,
        }
        Some(tris)
    }
}

fn non_empty(v: Vec<f64>, d: Vec<f64>) -> Vec<f64> {
    if v.is_empty() { d } else { v }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = v << 1 | u64::from(byte >> (7 - self.pos % 8) & 1);
            self.pos += 1;
        }
        Some(v)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// The 4×4 control points of a patch (Coons patches get their four
/// inner points computed, §8.7.4.5.7), indexed `[row][col]` with row 0
/// along the first edge.
fn tensor_grid(p: &[Point], tensor: bool) -> [[Point; 4]; 4] {
    // Boundary order: p00 p01 p02 p03 p13 p23 p33 p32 p31 p30 p20 p10.
    let mut g = [[Point::default(); 4]; 4];
    let border = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3), (3, 2), (3, 1), (3, 0), (2, 0), (1, 0)];
    for (i, &(r, c)) in border.iter().enumerate() {
        g[r][c] = p[i];
    }
    if tensor && p.len() >= 16 {
        g[1][1] = p[12];
        g[1][2] = p[13];
        g[2][2] = p[14];
        g[2][1] = p[15];
    } else {
        // The implicit inner points of a Coons patch.
        let comb = |t: [(f64, Point); 9]| {
            let (mut x, mut y) = (0.0, 0.0);
            for (w, p) in t {
                x += w * p.x;
                y += w * p.y;
            }
            Point::new(x / 9.0, y / 9.0)
        };
        let q = g;
        g[1][1] = comb([
            (-4.0, q[0][0]),
            (6.0, q[0][1]),
            (6.0, q[1][0]),
            (-2.0, q[0][3]),
            (-2.0, q[3][0]),
            (3.0, q[3][1]),
            (3.0, q[1][3]),
            (-1.0, q[3][3]),
            (0.0, q[0][0]),
        ]);
        g[1][2] = comb([
            (-4.0, q[0][3]),
            (6.0, q[0][2]),
            (6.0, q[1][3]),
            (-2.0, q[0][0]),
            (-2.0, q[3][3]),
            (3.0, q[3][2]),
            (3.0, q[1][0]),
            (-1.0, q[3][0]),
            (0.0, q[0][0]),
        ]);
        g[2][1] = comb([
            (-4.0, q[3][0]),
            (6.0, q[3][1]),
            (6.0, q[2][0]),
            (-2.0, q[3][3]),
            (-2.0, q[0][0]),
            (3.0, q[0][1]),
            (3.0, q[2][3]),
            (-1.0, q[0][3]),
            (0.0, q[0][0]),
        ]);
        g[2][2] = comb([
            (-4.0, q[3][3]),
            (6.0, q[3][2]),
            (6.0, q[2][3]),
            (-2.0, q[3][0]),
            (-2.0, q[0][3]),
            (3.0, q[0][2]),
            (3.0, q[2][0]),
            (-1.0, q[0][0]),
            (0.0, q[0][0]),
        ]);
    }
    g
}

/// Splits a patch into a grid of Gouraud triangles; colours are
/// bilinear between the corners (c0 at p00, c1 at p03, c2 at p33, c3 at p30).
fn patch_triangles(g: &[[Point; 4]; 4], cols: &[Vec<f64>], out: &mut Vec<[(Point, Vec<f64>); 3]>) {
    const N: usize = 12;
    let bern = |t: f64| {
        let mt = 1.0 - t;
        [mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t]
    };
    let at = |u: f64, v: f64| -> (Point, Vec<f64>) {
        let (bu, bv) = (bern(u), bern(v));
        let mut p = Point::default();
        for (r, row) in g.iter().enumerate() {
            for (c, q) in row.iter().enumerate() {
                let w = bv[r] * bu[c];
                p.x += w * q.x;
                p.y += w * q.y;
            }
        }
        let n = cols[0].len();
        let col: Vec<f64> = (0..n)
            .map(|k| {
                let top = cols[0][k] * (1.0 - u) + cols[1][k] * u;
                let bottom = cols[3][k] * (1.0 - u) + cols[2][k] * u;
                top * (1.0 - v) + bottom * v
            })
            .collect();
        (p, col)
    };
    let grid: Vec<Vec<(Point, Vec<f64>)>> =
        (0..=N).map(|j| (0..=N).map(|i| at(i as f64 / N as f64, j as f64 / N as f64)).collect()).collect();
    for j in 0..N {
        for i in 0..N {
            if out.len() >= MAX_TRIANGLES {
                return;
            }
            out.push([grid[j][i].clone(), grid[j][i + 1].clone(), grid[j + 1][i].clone()]);
            out.push([grid[j][i + 1].clone(), grid[j + 1][i + 1].clone(), grid[j + 1][i].clone()]);
        }
    }
}

struct FunctionShader {
    sh: Shading,
    inv: Matrix,
    dom: Vec<f64>,
}

impl Shader for FunctionShader {
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8) {
        let p = self.inv.apply(Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
        let inside =
            self.dom.len() >= 4 && p.x >= self.dom[0] && p.x <= self.dom[1] && p.y >= self.dom[2] && p.y <= self.dom[3];
        if !inside {
            return ([0, 0, 0], 0);
        }
        (self.sh.color(&[p.x, p.y]), 255)
    }
}

struct GradientShader {
    radial: bool,
    c: Vec<f64>,
    extend: [bool; 2],
    lut: Vec<[u8; 3]>,
    inv: Matrix,
    background: Option<[u8; 3]>,
}

impl GradientShader {
    /// The parameter `s` in 0..1 at a shading-space point, or `None`
    /// outside the gradient (no extend there).
    fn param(&self, p: Point) -> Option<f64> {
        let c = &self.c;
        if !self.radial {
            let (dx, dy) = (c[2] - c[0], c[3] - c[1]);
            let den = dx * dx + dy * dy;
            let s = if den == 0.0 { 0.0 } else { ((p.x - c[0]) * dx + (p.y - c[1]) * dy) / den };
            return self.clamp(s);
        }
        // Radial (§8.7.4.5.4): the largest s with a circle through p.
        let (x0, y0, r0, x1, y1, r1) = (c[0], c[1], c[2], c[3], c[4], c[5]);
        let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
        let (pdx, pdy) = (p.x - x0, p.y - y0);
        let a = cdx * cdx + cdy * cdy - dr * dr;
        let b = pdx * cdx + pdy * cdy + r0 * dr;
        let cc = pdx * pdx + pdy * pdy - r0 * r0;
        let mut roots: Vec<f64> = Vec::with_capacity(2);
        if a.abs() < 1e-12 {
            if b.abs() > 1e-12 {
                roots.push(cc / (2.0 * b));
            }
        } else {
            let disc = b * b - a * cc;
            if disc < 0.0 {
                return None;
            }
            let sq = disc.sqrt();
            roots.push((b + sq) / a);
            roots.push((b - sq) / a);
        }
        roots.sort_by(|x, y| y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal));
        for s in roots {
            if r0 + s * dr < 0.0 {
                continue;
            }
            if let Some(v) = self.clamp(s) {
                return Some(v);
            }
        }
        None
    }

    fn clamp(&self, s: f64) -> Option<f64> {
        if s < 0.0 {
            return self.extend[0].then_some(0.0);
        }
        if s > 1.0 {
            return self.extend[1].then_some(1.0);
        }
        Some(s)
    }
}

impl Shader for GradientShader {
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8) {
        let p = self.inv.apply(Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
        match self.param(p) {
            Some(s) => (self.lut[((s * (LUT - 1) as f64).round() as usize).min(LUT - 1)], 255),
            None => match self.background {
                Some(bg) => (bg, 255),
                None => ([0, 0, 0], 0),
            },
        }
    }
}

/// A mesh drawn once into a buffer over the area it may show in.
struct MeshShader {
    area: IRect,
    rgb: Vec<u8>,
    alpha: Vec<u8>,
}

impl MeshShader {
    fn draw(sh: &Shading, tris: &[[(Point, Vec<f64>); 3]], ctm: &Matrix, area: IRect) -> MeshShader {
        let (w, h) = (area.width(), area.height());
        let mut out = MeshShader { area, rgb: vec![0; w * h * 3], alpha: vec![0; w * h] };
        if w == 0 || h == 0 {
            return out;
        }
        if let Some(bg) = sh.background {
            for i in 0..w * h {
                out.rgb[i * 3..i * 3 + 3].copy_from_slice(&bg);
                out.alpha[i] = 255;
            }
        }
        for t in tris {
            let p: Vec<Point> = t.iter().map(|(q, _)| ctm.apply(*q)).collect();
            let minx = p.iter().map(|q| q.x).fold(f64::MAX, f64::min).floor().max(f64::from(area.x0)) as i32;
            let maxx = p.iter().map(|q| q.x).fold(f64::MIN, f64::max).ceil().min(f64::from(area.x1)) as i32;
            let miny = p.iter().map(|q| q.y).fold(f64::MAX, f64::min).floor().max(f64::from(area.y0)) as i32;
            let maxy = p.iter().map(|q| q.y).fold(f64::MIN, f64::max).ceil().min(f64::from(area.y1)) as i32;
            let den = (p[1].y - p[2].y) * (p[0].x - p[2].x) + (p[2].x - p[1].x) * (p[0].y - p[2].y);
            if den.abs() < 1e-12 || minx >= maxx || miny >= maxy {
                continue;
            }
            for y in miny..maxy {
                for x in minx..maxx {
                    let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let l0 = ((p[1].y - p[2].y) * (px - p[2].x) + (p[2].x - p[1].x) * (py - p[2].y)) / den;
                    let l1 = ((p[2].y - p[0].y) * (px - p[2].x) + (p[0].x - p[2].x) * (py - p[2].y)) / den;
                    let l2 = 1.0 - l0 - l1;
                    let eps = -1e-3;
                    if l0 < eps || l1 < eps || l2 < eps {
                        continue;
                    }
                    let n = t[0].1.len();
                    let comps: Vec<f64> = (0..n).map(|k| l0 * t[0].1[k] + l1 * t[1].1[k] + l2 * t[2].1[k]).collect();
                    let c = sh.color(&comps);
                    let i = (y - area.y0) as usize * w + (x - area.x0) as usize;
                    out.rgb[i * 3..i * 3 + 3].copy_from_slice(&c);
                    out.alpha[i] = 255;
                }
            }
        }
        out
    }
}

impl Shader for MeshShader {
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8) {
        let a = self.area;
        if x < a.x0 || y < a.y0 || x >= a.x1 || y >= a.y1 {
            return ([0, 0, 0], 0);
        }
        let i = (y - a.y0) as usize * a.width() + (x - a.x0) as usize;
        ([self.rgb[i * 3], self.rgb[i * 3 + 1], self.rgb[i * 3 + 2]], self.alpha[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axial_and_radial_parameters() {
        let g = GradientShader {
            radial: false,
            c: vec![0.0, 0.0, 10.0, 0.0],
            extend: [false, true],
            lut: (0..LUT).map(|i| [(i * 255 / (LUT - 1)) as u8, 0, 0]).collect(),
            inv: Matrix::IDENTITY,
            background: None,
        };
        assert_eq!(g.param(Point::new(5.0, 3.0)), Some(0.5));
        assert_eq!(g.param(Point::new(-1.0, 0.0)), None);
        assert_eq!(g.param(Point::new(20.0, 0.0)), Some(1.0));
        let r = GradientShader { radial: true, c: vec![0.0, 0.0, 0.0, 0.0, 0.0, 10.0], ..g };
        let s = r.param(Point::new(5.0, 0.0)).unwrap();
        assert!((s - 0.5).abs() < 1e-9);
    }

    #[test]
    fn coons_grid_corners() {
        let pts: Vec<Point> = [
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (3.0, 2.0),
            (3.0, 3.0),
            (2.0, 3.0),
            (1.0, 3.0),
            (0.0, 3.0),
            (0.0, 2.0),
            (0.0, 1.0),
        ]
        .iter()
        .map(|&(x, y)| Point::new(x, y))
        .collect();
        let g = tensor_grid(&pts, false);
        // A flat square patch: the inner points are on the grid.
        assert!((g[1][1].x - 1.0).abs() < 1e-9 && (g[1][1].y - 1.0).abs() < 1e-9, "{:?}", g[1][1]);
        let mut tris = Vec::new();
        patch_triangles(&g, &[vec![0.0], vec![1.0], vec![1.0], vec![0.0]], &mut tris);
        assert_eq!(tris.len(), 12 * 12 * 2);
    }
}
