//! The content stream interpreter (§8, §9): graphics state, paths,
//! clipping, colour, text, images, form and Type 3 glyph procedures,
//! patterns, shadings, transparency groups and soft masks, optional
//! content and annotation appearances, drawn onto a [`Canvas`].
//! Work is bounded by an operator budget and a deadline.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::color::{self, ColorSpace};
use crate::document::{Document, Page};
use crate::error::{PdfError, Result};
use crate::font::pdf::{Kind, PdfFont};
use crate::geom::{Matrix, Path, Point};
use crate::image::{self, ImageShader, RasterImage};
use crate::lexer::{Lexer, Token};
use crate::object::{Dict, ObjRef, Object, Stream};
use crate::raster::{self, Blend, Canvas, Coverage, FillRule, IRect, Mask, Shader, Solid};
use crate::shading::Shading;
use crate::stroke::{self, Cap, Join, StrokeStyle};

/// Pixels in one rendered page, at most.
pub const MAX_PAGE_PIXELS: usize = 40 << 20;
const MAX_DEPTH: usize = 24;
/// Soft masks and transparency groups drawn off screen, nested, at most;
/// deeper groups are drawn straight onto the page.
const MAX_OFFSCREEN: usize = 3;
const MAX_TILE: usize = 1024;

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Target width in pixels; the height follows the page's shape.
    pub width: usize,
    /// Operators executed, at most (a hostile page cannot loop forever).
    pub max_ops: usize,
    pub deadline: Option<Instant>,
    pub annotations: bool,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions { width: 1200, max_ops: 4_000_000, deadline: None, annotations: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub ops: usize,
    pub images: usize,
    /// Device pixels painted by images, as a share of the page (0..1+).
    pub image_area: f64,
    /// Some text used a font that is neither embedded nor installed.
    pub missing_fonts: bool,
    /// Features met and not drawn (`JPEG 2000 images`, …), deduplicated.
    pub unsupported: Vec<String>,
    /// The budget or deadline cut the page short.
    pub truncated: bool,
}

pub struct Rendered {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub stats: Stats,
}

/// Device matrix of a page at `scale` pixels per point, rotation applied.
pub fn page_matrix(page: &Page, scale: f64) -> Matrix {
    let [x0, y0, x1, y1] = page.crop_box;
    let s = scale;
    match page.rotate {
        90 => Matrix::new(0.0, s, s, 0.0, -y0 * s, -x0 * s),
        180 => Matrix::new(-s, 0.0, 0.0, s, x1 * s, -y0 * s),
        270 => Matrix::new(0.0, -s, -s, 0.0, y1 * s, x1 * s),
        _ => Matrix::new(s, 0.0, 0.0, -s, -x0 * s, y1 * s),
    }
}

pub fn render_page(doc: &Document, index: usize, opts: &RenderOptions) -> Result<Rendered> {
    let page = doc.page(index)?;
    let (pw, ph) = page.size();
    let width = opts.width.clamp(16, 12_000);
    let scale = width as f64 / pw.max(1.0);
    let height = ((ph * scale).round() as usize).max(1);
    if width.saturating_mul(height) > MAX_PAGE_PIXELS {
        return Err(PdfError::too_large("page size"));
    }
    let base = page_matrix(page, scale);
    let mut it = Interp::new(doc, Canvas::white(width, height), opts);
    it.gs.ctm = base;
    it.page_base = base;
    let content = doc.page_content(page);
    it.run(&content, page.resources.clone(), base, 0);
    if opts.annotations {
        it.annotations(page, base);
    }
    let Interp { canvas, mut stats, .. } = it;
    stats.unsupported.sort();
    stats.unsupported.dedup();
    Ok(Rendered { width, height, rgb: canvas.rgb, stats })
}

#[derive(Clone)]
struct Paint {
    cs: Arc<ColorSpace>,
    comps: Vec<f64>,
    rgb: [u8; 3],
    pattern: Option<Object>,
}

impl Paint {
    fn black() -> Paint {
        Paint { cs: Arc::new(ColorSpace::Gray), comps: vec![0.0], rgb: [0, 0, 0], pattern: None }
    }
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    clip: Option<Arc<Mask>>,
    fill: Paint,
    stroke: Paint,
    style: StrokeStyle,
    fill_alpha: f64,
    stroke_alpha: f64,
    blend: Blend,
    soft: Option<Arc<Mask>>,
    font: Option<Arc<PdfFont>>,
    font_size: f64,
    char_spacing: f64,
    word_spacing: f64,
    h_scale: f64,
    leading: f64,
    rise: f64,
    render_mode: i64,
}

impl GState {
    fn new() -> GState {
        GState {
            ctm: Matrix::IDENTITY,
            clip: None,
            fill: Paint::black(),
            stroke: Paint::black(),
            style: StrokeStyle::default(),
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            blend: Blend::Normal,
            soft: None,
            font: None,
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: usize,
    code: u32,
    m: [i32; 4],
    sub: [u8; 2],
}

struct Interp<'a> {
    doc: &'a Document,
    canvas: Canvas,
    gs: GState,
    stack: Vec<GState>,
    path: Path,
    clip_rule: Option<FillRule>,
    tm: Matrix,
    tlm: Matrix,
    text_clip: Path,
    fonts: HashMap<usize, Arc<PdfFont>>,
    font_refs: HashMap<ObjRef, Arc<PdfFont>>,
    images: HashMap<(ObjRef, usize, usize), Arc<RasterImage>>,
    glyphs: HashMap<GlyphKey, Option<Arc<Coverage>>>,
    opts: &'a RenderOptions,
    stats: Stats,
    stopped: bool,
    /// Optional content groups that are off.
    hidden: HashSet<ObjRef>,
    /// Marked-content nesting, and how deep the hidden part starts.
    marked: usize,
    hidden_from: Option<usize>,
    /// Uncoloured tiling patterns paint in this colour.
    forced: Option<[u8; 3]>,
    /// A Type 3 glyph declared `d1`: colour operators are ignored.
    in_d1: bool,
    /// The page's default space: pattern matrices are relative to it.
    page_base: Matrix,
    /// Soft masks and off-screen groups being drawn, nested: each holds a
    /// page-sized canvas, and a mask may name itself.
    offscreen: usize,
}

impl<'a> Interp<'a> {
    fn new(doc: &'a Document, canvas: Canvas, opts: &'a RenderOptions) -> Interp<'a> {
        let hidden = hidden_groups(doc);
        Interp {
            doc,
            canvas,
            gs: GState::new(),
            stack: Vec::new(),
            path: Path::new(),
            clip_rule: None,
            tm: Matrix::IDENTITY,
            tlm: Matrix::IDENTITY,
            text_clip: Path::new(),
            fonts: HashMap::new(),
            font_refs: HashMap::new(),
            images: HashMap::new(),
            glyphs: HashMap::new(),
            opts,
            stats: Stats::default(),
            stopped: false,
            hidden,
            marked: 0,
            hidden_from: None,
            forced: None,
            in_d1: false,
            page_base: Matrix::IDENTITY,
            offscreen: 0,
        }
    }

    fn unsupported(&mut self, what: &str) {
        if !self.stats.unsupported.iter().any(|u| u == what) && self.stats.unsupported.len() < 32 {
            self.stats.unsupported.push(what.to_string());
        }
    }

    fn limit(&self) -> IRect {
        let r = self.canvas.rect();
        match &self.gs.clip {
            Some(m) => r.intersect(&m.bounds),
            None => r,
        }
    }

    fn budget(&mut self) -> bool {
        self.stats.ops += 1;
        if self.stats.ops > self.opts.max_ops {
            self.stopped = true;
        } else if self.stats.ops.is_multiple_of(16)
            && let Some(d) = self.opts.deadline
            && Instant::now() > d
        {
            self.stopped = true;
        }
        if self.stopped {
            self.stats.truncated = true;
        }
        !self.stopped
    }

    /// Past the deadline: big fills check it themselves, since one of
    /// them can cost more than many cheap operators.
    fn late(&mut self) -> bool {
        if !self.stopped
            && let Some(d) = self.opts.deadline
            && Instant::now() > d
        {
            self.stopped = true;
            self.stats.truncated = true;
        }
        self.stopped
    }

    fn hidden_now(&self) -> bool {
        self.hidden_from.is_some()
    }

    // ---- the operator loop ----------------------------------------------

    /// Runs a content stream with `res` as its resources; `base` is the
    /// CTM patterns in this stream are relative to.
    fn run(&mut self, content: &[u8], res: Option<Arc<Dict>>, base: Matrix, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut l = Lexer::new(content);
        let mut ops: Vec<Object> = Vec::with_capacity(16);
        let saved_stack = self.stack.len();
        let saved_marked = self.marked;
        loop {
            if self.stopped {
                break;
            }
            let Some(tok) = l.next_token() else { break };
            let op = match tok {
                Token::Keyword(k) => k,
                other => {
                    if let Some(o) = l.object_from(other, 0)
                        && ops.len() < 64
                    {
                        ops.push(o);
                    }
                    continue;
                }
            };
            if !self.budget() {
                break;
            }
            if op == b"BI" {
                self.inline_image(&mut l, res.as_deref());
                ops.clear();
                continue;
            }
            self.op(op, &ops, res.as_ref(), base, depth);
            ops.clear();
        }
        // Unbalanced `q` and marked content end with the stream.
        while self.stack.len() > saved_stack {
            if let Some(g) = self.stack.pop() {
                self.gs = g;
            }
        }
        self.marked = saved_marked;
        if self.hidden_from.is_some_and(|h| h > saved_marked) {
            self.hidden_from = None;
        }
    }

    fn num(ops: &[Object], i: usize) -> f64 {
        ops.get(i).and_then(Object::as_f64).unwrap_or(0.0)
    }

    fn nums(ops: &[Object]) -> Vec<f64> {
        ops.iter().filter_map(Object::as_f64).collect()
    }

    fn op(&mut self, op: &[u8], ops: &[Object], res: Option<&Arc<Dict>>, base: Matrix, depth: usize) {
        let n = |i| Interp::num(ops, i);
        match op {
            // -- graphics state
            b"q" => {
                if self.stack.len() < 256 {
                    self.stack.push(self.gs.clone());
                }
            }
            b"Q" => {
                if let Some(g) = self.stack.pop() {
                    self.gs = g;
                }
            }
            b"cm" if ops.len() >= 6 => {
                let m = Matrix::new(n(0), n(1), n(2), n(3), n(4), n(5));
                if m.determinant().is_finite() {
                    self.gs.ctm = m.then(&self.gs.ctm);
                }
            }
            b"w" => self.gs.style.width = n(0),
            b"J" => {
                self.gs.style.cap = match n(0) as i64 {
                    1 => Cap::Round,
                    2 => Cap::Square,
                    _ => Cap::Butt,
                }
            }
            b"j" => {
                self.gs.style.join = match n(0) as i64 {
                    1 => Join::Round,
                    2 => Join::Bevel,
                    _ => Join::Miter,
                }
            }
            b"M" => self.gs.style.miter_limit = n(0),
            b"d" => {
                let arr = ops.first().and_then(Object::as_array).map(Interp::nums).unwrap_or_default();
                self.gs.style.dash = arr;
                self.gs.style.dash_phase = n(1);
            }
            b"ri" | b"i" => {}
            b"gs" => {
                if let Some(name) = ops.first().and_then(Object::as_name) {
                    self.ext_gstate(name, res);
                }
            }
            // -- path construction
            b"m" if ops.len() >= 2 => self.path.move_to(n(0), n(1)),
            b"l" if ops.len() >= 2 => {
                if self.path.is_empty() {
                    self.path.move_to(n(0), n(1));
                } else {
                    self.path.line_to(n(0), n(1));
                }
            }
            b"c" if ops.len() >= 6 => self.path.cubic_to(n(0), n(1), n(2), n(3), n(4), n(5)),
            b"v" if ops.len() >= 4 => {
                let cur = current_point(&self.path);
                self.path.cubic_to(cur.x, cur.y, n(0), n(1), n(2), n(3));
            }
            b"y" if ops.len() >= 4 => self.path.cubic_to(n(0), n(1), n(2), n(3), n(2), n(3)),
            b"h" => self.path.close(),
            b"re" if ops.len() >= 4 => {
                let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                self.path.move_to(x, y);
                self.path.line_to(x + w, y);
                self.path.line_to(x + w, y + h);
                self.path.line_to(x, y + h);
                self.path.close();
            }
            // -- painting
            b"S" => self.paint_path(false, true, FillRule::NonZero),
            b"s" => {
                self.path.close();
                self.paint_path(false, true, FillRule::NonZero);
            }
            b"f" | b"F" => self.paint_path(true, false, FillRule::NonZero),
            b"f*" => self.paint_path(true, false, FillRule::EvenOdd),
            b"B" => self.paint_path(true, true, FillRule::NonZero),
            b"B*" => self.paint_path(true, true, FillRule::EvenOdd),
            b"b" => {
                self.path.close();
                self.paint_path(true, true, FillRule::NonZero);
            }
            b"b*" => {
                self.path.close();
                self.paint_path(true, true, FillRule::EvenOdd);
            }
            b"n" => self.paint_path(false, false, FillRule::NonZero),
            b"W" => self.clip_rule = Some(FillRule::NonZero),
            b"W*" => self.clip_rule = Some(FillRule::EvenOdd),
            // -- colour
            b"CS" | b"cs" => {
                if self.in_d1 {
                    return;
                }
                let Some(o) = ops.first() else { return };
                let cs = Arc::new(ColorSpace::parse(self.doc, o, res.map(|r| r.as_ref())));
                let comps = cs.initial();
                let rgb = cs.to_rgb(&comps);
                let p = Paint { cs, comps, rgb, pattern: None };
                if op == b"CS" {
                    self.gs.stroke = p;
                } else {
                    self.gs.fill = p;
                }
            }
            b"SC" | b"SCN" | b"sc" | b"scn" => {
                if self.in_d1 {
                    return;
                }
                let stroke = op[0] == b'S';
                let paint = if stroke { &mut self.gs.stroke } else { &mut self.gs.fill };
                let comps = Interp::nums(ops);
                if let ColorSpace::Pattern { .. } = paint.cs.as_ref() {
                    let name = ops.last().and_then(Object::as_name);
                    paint.pattern = name.and_then(|nm| {
                        let pats = res.and_then(|r| self.doc.dict_in(r, "Pattern"))?;
                        let p = self.doc.get_in(&pats, nm);
                        (!p.is_null()).then_some(p)
                    });
                    paint.comps = comps.clone();
                    paint.rgb = paint.cs.to_rgb(&comps);
                } else {
                    paint.pattern = None;
                    paint.rgb = paint.cs.to_rgb(&comps);
                    paint.comps = comps;
                }
            }
            b"G" | b"g" | b"RG" | b"rg" | b"K" | b"k" => {
                if self.in_d1 {
                    return;
                }
                let cs = match op {
                    b"G" | b"g" => ColorSpace::Gray,
                    b"RG" | b"rg" => ColorSpace::Rgb,
                    _ => ColorSpace::Cmyk,
                };
                let comps = Interp::nums(ops);
                let rgb = cs.to_rgb(&comps);
                let p = Paint { cs: Arc::new(cs), comps, rgb, pattern: None };
                if op[0].is_ascii_uppercase() {
                    self.gs.stroke = p;
                } else {
                    self.gs.fill = p;
                }
            }
            // -- shading, XObjects
            b"sh" => {
                if let Some(name) = ops.first().and_then(Object::as_name) {
                    self.shade_op(name, res);
                }
            }
            b"Do" => {
                if let Some(name) = ops.first().and_then(Object::as_name) {
                    self.xobject(name, res, depth);
                }
            }
            // -- text
            b"BT" => {
                self.tm = Matrix::IDENTITY;
                self.tlm = Matrix::IDENTITY;
                self.text_clip = Path::new();
            }
            b"ET" => {
                if self.gs.render_mode >= 4 {
                    let clip = std::mem::take(&mut self.text_clip);
                    self.intersect_clip_device(&clip, FillRule::NonZero);
                }
            }
            b"Tc" => self.gs.char_spacing = n(0),
            b"Tw" => self.gs.word_spacing = n(0),
            b"Tz" => self.gs.h_scale = n(0) / 100.0,
            b"TL" => self.gs.leading = n(0),
            b"Ts" => self.gs.rise = n(0),
            b"Tr" => self.gs.render_mode = n(0) as i64,
            b"Tf" if ops.len() >= 2 => {
                self.gs.font_size = n(1);
                if let Some(name) = ops[0].as_name() {
                    self.gs.font = self.font(name, res);
                }
            }
            b"Td" if ops.len() >= 2 => {
                self.tlm = Matrix::translate(n(0), n(1)).then(&self.tlm);
                self.tm = self.tlm;
            }
            b"TD" if ops.len() >= 2 => {
                self.gs.leading = -n(1);
                self.tlm = Matrix::translate(n(0), n(1)).then(&self.tlm);
                self.tm = self.tlm;
            }
            b"Tm" if ops.len() >= 6 => {
                self.tlm = Matrix::new(n(0), n(1), n(2), n(3), n(4), n(5));
                self.tm = self.tlm;
            }
            b"T*" => self.next_line(),
            b"Tj" => {
                if let Some(s) = ops.first().and_then(Object::as_bytes) {
                    self.show(s, res, depth);
                }
            }
            b"'" => {
                self.next_line();
                if let Some(s) = ops.first().and_then(Object::as_bytes) {
                    self.show(s, res, depth);
                }
            }
            b"\"" if ops.len() >= 3 => {
                self.gs.word_spacing = n(0);
                self.gs.char_spacing = n(1);
                self.next_line();
                if let Some(s) = ops[2].as_bytes() {
                    self.show(s, res, depth);
                }
            }
            b"TJ" => {
                if let Some(a) = ops.first().and_then(Object::as_array) {
                    for item in a.iter() {
                        match item {
                            Object::String(s) => self.show(s, res, depth),
                            o => {
                                if let Some(v) = o.as_f64() {
                                    let fs = self.gs.font_size;
                                    let vertical = self.gs.font.as_ref().is_some_and(|f| f.vertical);
                                    let d = -v / 1000.0 * fs;
                                    let t = if vertical {
                                        Matrix::translate(0.0, d)
                                    } else {
                                        Matrix::translate(d * self.gs.h_scale, 0.0)
                                    };
                                    self.tm = t.then(&self.tm);
                                }
                            }
                        }
                    }
                }
            }
            b"d0" => {}
            b"d1" => self.in_d1 = true,
            // -- marked and optional content
            b"BMC" => self.marked += 1,
            b"BDC" => {
                self.marked += 1;
                if self.hidden_from.is_none()
                    && ops.first().and_then(Object::as_name) == Some("OC")
                    && let Some(prop) = ops.get(1)
                {
                    let oc = match prop {
                        Object::Name(nm) => res
                            .and_then(|r| self.doc.dict_in(r, "Properties"))
                            .map(|p| p.get(nm.as_str()).cloned().unwrap_or(Object::Null)),
                        other => Some(other.clone()),
                    };
                    if let Some(oc) = oc
                        && !self.visible(&oc)
                    {
                        self.hidden_from = Some(self.marked);
                    }
                }
            }
            b"EMC" => {
                if self.hidden_from == Some(self.marked) {
                    self.hidden_from = None;
                }
                self.marked = self.marked.saturating_sub(1);
            }
            _ => {}
        }
        let _ = base;
    }

    fn next_line(&mut self) {
        self.tlm = Matrix::translate(0.0, -self.gs.leading).then(&self.tlm);
        self.tm = self.tlm;
    }

    // ---- optional content --------------------------------------------------

    fn visible(&self, oc: &Object) -> bool {
        let r = oc.as_ref();
        let o = self.doc.resolve(oc);
        let Some(d) = o.as_dict() else { return true };
        match d.name("Type") {
            Some("OCMD") => {
                let ocgs: Vec<Object> = match self.doc.get_in(d, "OCGs") {
                    Object::Array(a) => a.iter().cloned().collect(),
                    Object::Null => Vec::new(),
                    single => vec![d.get("OCGs").cloned().unwrap_or(single)],
                };
                if ocgs.is_empty() {
                    return true;
                }
                let on: Vec<bool> = ocgs.iter().map(|g| g.as_ref().is_none_or(|r| !self.hidden.contains(&r))).collect();
                match d.name("P").unwrap_or("AnyOn") {
                    "AllOn" => on.iter().all(|&v| v),
                    "AnyOff" => on.iter().any(|&v| !v),
                    "AllOff" => on.iter().all(|&v| !v),
                    _ => on.iter().any(|&v| v),
                }
            }
            _ => r.is_none_or(|r| !self.hidden.contains(&r)),
        }
    }

    // ---- paths ----------------------------------------------------------------

    fn paint_path(&mut self, fill: bool, stroke: bool, rule: FillRule) {
        let path = std::mem::take(&mut self.path);
        if !self.hidden_now() {
            if fill {
                let dev = path.transformed(&self.gs.ctm);
                let paint = self.gs.fill.clone();
                self.fill_device(&dev, rule, &paint, self.gs.fill_alpha);
            }
            if stroke {
                self.stroke_user(&path, &self.gs.ctm.clone());
            }
        }
        if let Some(r) = self.clip_rule.take() {
            let dev = path.transformed(&self.gs.ctm);
            self.intersect_clip_device(&dev, r);
        }
    }

    fn stroke_user(&mut self, path: &Path, ctm: &Matrix) {
        // Thin rules (table lines, underlines) snap to the pixel grid, so
        // a one-pixel line is one dark row, not two grey ones (the
        // stroke adjustment viewers apply, §10.7.5).
        let dev_width = self.gs.style.width.abs() * ctm.expansion();
        let lines = match snapped(path, ctm, dev_width) {
            Some(dev) => {
                let style = StrokeStyle { width: dev_width.round().max(1.0), ..self.gs.style.clone() };
                stroke::stroke(&dev, &style, &Matrix::IDENTITY)
            }
            None => stroke::stroke(path, &self.gs.style, ctm),
        };
        if lines.is_empty() {
            return;
        }
        let limit = self.limit();
        if let Some(cov) = raster::coverage_lines(&lines, FillRule::NonZero, limit) {
            let paint = self.gs.stroke.clone();
            self.paint_coverage(&cov, &paint, self.gs.stroke_alpha);
        }
    }

    fn fill_device(&mut self, dev: &Path, rule: FillRule, paint: &Paint, alpha: f64) {
        let limit = self.limit();
        if limit.is_empty() {
            return;
        }
        if let Some(cov) = raster::coverage(dev, rule, limit) {
            self.paint_coverage(&cov, paint, alpha);
        }
    }

    fn intersect_clip_device(&mut self, dev: &Path, rule: FillRule) {
        let (w, h) = (self.canvas.width, self.canvas.height);
        let cov = raster::coverage(dev, rule, self.limit())
            .unwrap_or(Coverage { rect: IRect { x0: 0, y0: 0, x1: 0, y1: 0 }, data: Vec::new() });
        let m = Mask::intersect(self.gs.clip.as_deref(), &cov, w, h);
        self.gs.clip = Some(Arc::new(m));
    }

    fn paint_coverage(&mut self, cov: &Coverage, paint: &Paint, alpha: f64) {
        if paint.cs.invisible() || self.late() {
            return;
        }
        let a = alpha8(alpha);
        let rgb = self.forced.unwrap_or(paint.rgb);
        let clip = self.gs.clip.clone();
        let soft = self.gs.soft.clone();
        let blend = self.gs.blend;
        if let (Some(pat), None) = (&paint.pattern, self.forced) {
            let pat = pat.clone();
            let rect = cov.rect;
            if let Some(shader) = self.pattern_shader(&pat, rect, &paint.comps, &paint.cs) {
                raster::paint(&mut self.canvas, cov, shader.as_ref(), a, clip.as_deref(), soft.as_deref(), blend);
            }
            return;
        }
        raster::paint(&mut self.canvas, cov, &Solid(rgb), a, clip.as_deref(), soft.as_deref(), blend);
    }

    // ---- graphics state parameters ----------------------------------------------

    fn ext_gstate(&mut self, name: &str, res: Option<&Arc<Dict>>) {
        let doc = self.doc;
        let Some(all) = res.and_then(|r| doc.dict_in(r, "ExtGState")) else { return };
        let Some(d) = doc.dict_in(&all, name) else { return };
        for (k, v) in &d.entries {
            let v = doc.resolve(v);
            match k.as_str() {
                "LW" => self.gs.style.width = v.as_f64().unwrap_or(1.0),
                "LC" => {
                    self.gs.style.cap = match v.as_i64() {
                        Some(1) => Cap::Round,
                        Some(2) => Cap::Square,
                        _ => Cap::Butt,
                    }
                }
                "LJ" => {
                    self.gs.style.join = match v.as_i64() {
                        Some(1) => Join::Round,
                        Some(2) => Join::Bevel,
                        _ => Join::Miter,
                    }
                }
                "ML" => self.gs.style.miter_limit = v.as_f64().unwrap_or(10.0),
                "D" => {
                    if let Some(a) = v.as_array() {
                        self.gs.style.dash = a.first().and_then(Object::as_array).map(Interp::nums).unwrap_or_default();
                        self.gs.style.dash_phase = a.get(1).and_then(Object::as_f64).unwrap_or(0.0);
                    }
                }
                "CA" => self.gs.stroke_alpha = v.as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
                "ca" => self.gs.fill_alpha = v.as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
                "BM" => {
                    let n = match &v {
                        Object::Name(n) => n.as_str().to_string(),
                        Object::Array(a) => a.first().and_then(Object::as_name).unwrap_or("Normal").to_string(),
                        _ => "Normal".into(),
                    };
                    self.gs.blend = Blend::from_name(&n);
                }
                "Font" => {
                    if let Some(a) = v.as_array()
                        && let Some(r) = a.first().and_then(Object::as_ref)
                    {
                        self.gs.font = Some(self.font_by_ref(r));
                        self.gs.font_size = a.get(1).and_then(Object::as_f64).unwrap_or(self.gs.font_size);
                    }
                }
                "SMask" => match &v {
                    Object::Dict(sm) => self.gs.soft = self.soft_mask(sm),
                    _ => self.gs.soft = None,
                },
                _ => {}
            }
        }
    }

    /// Renders a soft mask group (§11.6.5.2) at the current CTM.
    fn soft_mask(&mut self, sm: &Dict) -> Option<Arc<Mask>> {
        let doc = self.doc;
        let luminosity = sm.name("S") != Some("Alpha");
        let Object::Stream(g) = doc.get_in(sm, "G") else { return None };
        let (w, h) = (self.canvas.width, self.canvas.height);
        if self.stack.len() > MAX_DEPTH || self.offscreen >= MAX_OFFSCREEN {
            return None;
        }
        let backdrop = {
            let bc = doc.numbers_in(sm, "BC");
            let cs = doc
                .dict_in(&g.dict, "Group")
                .map(|gr| ColorSpace::parse(doc, gr.get("CS").unwrap_or(&Object::Null), None));
            match (cs, bc.is_empty()) {
                (Some(cs), false) => cs.to_rgb(&bc),
                _ => [0, 0, 0],
            }
        };
        let canvas = if luminosity {
            let mut c = Canvas::white(w, h);
            for px in c.rgb.as_chunks_mut::<3>().0.iter_mut() {
                px.copy_from_slice(&backdrop);
            }
            c
        } else {
            Canvas::transparent(w, h)
        };
        let saved_canvas = std::mem::replace(&mut self.canvas, canvas);
        let saved_gs = self.gs.clone();
        self.gs.soft = None;
        self.gs.clip = None;
        self.gs.fill_alpha = 1.0;
        self.gs.stroke_alpha = 1.0;
        self.gs.blend = Blend::Normal;
        self.offscreen += 1;
        self.form(&g, None, self.stack.len() + 1, true);
        self.offscreen -= 1;
        let drawn = std::mem::replace(&mut self.canvas, saved_canvas);
        self.gs = saved_gs;
        let data: Vec<u8> = if luminosity {
            drawn.rgb.as_chunks::<3>().0.iter().map(|p| color::luminosity([p[0], p[1], p[2]])).collect()
        } else {
            drawn.alpha.unwrap_or_else(|| vec![255; w * h])
        };
        Some(Arc::new(Mask { width: w, height: h, data, bounds: IRect { x0: 0, y0: 0, x1: w as i32, y1: h as i32 } }))
    }

    // ---- fonts and text ---------------------------------------------------------------

    fn font(&mut self, name: &str, res: Option<&Arc<Dict>>) -> Option<Arc<PdfFont>> {
        let doc = self.doc;
        let fonts = res.and_then(|r| doc.dict_in(r, "Font"))?;
        let o = fonts.get(name)?.clone();
        if let Some(r) = o.as_ref() {
            return Some(self.font_by_ref(r));
        }
        let d = doc.resolve(&o);
        let d = d.as_dict()?;
        let key = d as *const Dict as usize;
        if let Some(f) = self.fonts.get(&key) {
            return Some(f.clone());
        }
        let f = Arc::new(PdfFont::load(doc, d));
        self.fonts.insert(key, f.clone());
        Some(f)
    }

    fn font_by_ref(&mut self, r: ObjRef) -> Arc<PdfFont> {
        if let Some(f) = self.font_refs.get(&r) {
            return f.clone();
        }
        let o = self.doc.get(r.num).unwrap_or(Object::Null);
        let d = o.as_dict().cloned().unwrap_or_default();
        let f = Arc::new(PdfFont::load(self.doc, &d));
        self.font_refs.insert(r, f.clone());
        f
    }

    fn show(&mut self, s: &[u8], res: Option<&Arc<Dict>>, depth: usize) {
        let Some(font) = self.gs.font.clone() else { return };
        let fs = self.gs.font_size;
        let th = self.gs.h_scale;
        let mode = self.gs.render_mode;
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        let clip = mode >= 4;
        let hidden = self.hidden_now();
        for (code, len) in font.codes(s) {
            if self.stopped {
                return;
            }
            let w0 = font.advance(code);
            let trm_text = Matrix::new(fs * th, 0.0, 0.0, fs, 0.0, self.gs.rise).then(&self.tm);
            if !hidden && (fill || stroke || clip) {
                if font.kind == Kind::Type3 {
                    if fill && let Some(proc_) = font.type3_proc(code) {
                        self.type3_glyph(&font, &proc_, &trm_text, res, depth);
                    }
                } else {
                    let vertical_shift =
                        if font.vertical { Matrix::translate(-w0 / 2.0 * fs, -0.88 * fs) } else { Matrix::IDENTITY };
                    let trm = vertical_shift.then(&trm_text);
                    match font.glyph(code) {
                        Some(g) => {
                            if fill {
                                self.fill_glyph(&font, code, &g, &trm);
                            }
                            if stroke {
                                let user = g.transformed(&trm);
                                let ctm = self.gs.ctm;
                                self.stroke_user(&user, &ctm);
                            }
                            if clip {
                                let dev = g.transformed(&trm.then(&self.gs.ctm));
                                self.text_clip.segs.extend(dev.segs);
                            }
                        }
                        None => {
                            if font.substituted && mode != 3 && font.text(code).is_some_and(|t| !t.trim().is_empty()) {
                                self.stats.missing_fonts = true;
                            }
                        }
                    }
                }
            }
            // Advance (§9.4.4).
            let space = if font.is_space(code, len) { self.gs.word_spacing } else { 0.0 };
            if font.vertical {
                let ty = -fs + self.gs.char_spacing + space;
                self.tm = Matrix::translate(0.0, ty).then(&self.tm);
            } else {
                let tx = (w0 * fs + self.gs.char_spacing + space) * th;
                self.tm = Matrix::translate(tx, 0.0).then(&self.tm);
            }
            if !self.budget() {
                return;
            }
        }
    }

    /// Fills a glyph through the coverage cache: the same glyph at the
    /// same size and sub-pixel phase is rasterized once per page.
    fn fill_glyph(&mut self, font: &Arc<PdfFont>, code: u32, g: &Arc<Path>, trm: &Matrix) {
        let m = trm.then(&self.gs.ctm);
        let (ox, oy) = (m.e, m.f);
        if !ox.is_finite() || !oy.is_finite() {
            return;
        }
        let q = |v: f64| (v * 64.0).round().clamp(-1e9, 1e9) as i32;
        let (fx, fy) = (ox - ox.floor(), oy - oy.floor());
        let sub = [(fx * 4.0).floor() as u8 % 4, (fy * 4.0).floor() as u8 % 4];
        let key = GlyphKey { font: Arc::as_ptr(font) as usize, code, m: [q(m.a), q(m.b), q(m.c), q(m.d)], sub };
        let (ix, iy) = (ox.floor() as i64, oy.floor() as i64);
        let cov = match self.glyphs.get(&key) {
            Some(c) => c.clone(),
            None => {
                let local = Matrix::new(m.a, m.b, m.c, m.d, f64::from(sub[0]) / 4.0, f64::from(sub[1]) / 4.0);
                let dev = g.transformed(&local);
                // Big glyphs (drop caps, titles) are not cached.
                let c = dev.bounds().and_then(|(x0, y0, x1, y1)| {
                    if x1 - x0 > 600.0 || y1 - y0 > 600.0 {
                        return None;
                    }
                    let lim = IRect {
                        x0: x0.floor() as i32 - 1,
                        y0: y0.floor() as i32 - 1,
                        x1: x1.ceil() as i32 + 2,
                        y1: y1.ceil() as i32 + 2,
                    };
                    raster::coverage(&dev, FillRule::NonZero, lim).map(Arc::new)
                });
                if self.glyphs.len() > 20_000 {
                    self.glyphs.clear();
                }
                if c.is_some() || dev.is_empty() {
                    self.glyphs.insert(key, c.clone());
                }
                if c.is_none() && !dev.is_empty() {
                    // Uncached path: draw directly.
                    let full = g.transformed(&m);
                    let paint = self.gs.fill.clone();
                    self.fill_device(&full, FillRule::NonZero, &paint, self.gs.fill_alpha);
                    return;
                }
                c
            }
        };
        let Some(cov) = cov else { return };
        if ix.abs() > 1 << 24 || iy.abs() > 1 << 24 {
            return;
        }
        let r = cov.rect;
        let placed = Coverage {
            rect: IRect { x0: r.x0 + ix as i32, y0: r.y0 + iy as i32, x1: r.x1 + ix as i32, y1: r.y1 + iy as i32 },
            data: Vec::new(),
        };
        let lim = self.limit();
        if placed.rect.intersect(&lim).is_empty() {
            return;
        }
        let placed = Coverage { rect: placed.rect, data: cov.data.clone() };
        let paint = self.gs.fill.clone();
        self.paint_coverage(&placed, &paint, self.gs.fill_alpha);
    }

    fn type3_glyph(
        &mut self,
        font: &Arc<PdfFont>,
        proc_: &Arc<Stream>,
        trm: &Matrix,
        res: Option<&Arc<Dict>>,
        depth: usize,
    ) {
        if depth + 1 > MAX_DEPTH {
            return;
        }
        let Some(t3) = font.type3.as_ref() else { return };
        let Ok(content) = self.doc.decode_stream(proc_) else { return };
        let saved = self.gs.clone();
        let saved_stack = std::mem::take(&mut self.stack);
        let (tm, tlm, path, d1) = (self.tm, self.tlm, std::mem::take(&mut self.path), self.in_d1);
        self.gs.ctm = t3.matrix.then(trm).then(&self.gs.ctm);
        self.in_d1 = false;
        let r = t3.resources.clone().or_else(|| res.cloned());
        let base = self.gs.ctm;
        self.run(&content.data, r, base, depth + 1);
        self.gs = saved;
        self.stack = saved_stack;
        self.tm = tm;
        self.tlm = tlm;
        self.path = path;
        self.in_d1 = d1;
    }

    // ---- XObjects ------------------------------------------------------------------------

    fn xobject(&mut self, name: &str, res: Option<&Arc<Dict>>, depth: usize) {
        let doc = self.doc;
        let Some(xs) = res.and_then(|r| doc.dict_in(r, "XObject")) else { return };
        let Some(entry) = xs.get(name).cloned() else { return };
        let r = entry.as_ref();
        let Object::Stream(s) = doc.resolve(&entry) else { return };
        if let Some(oc) = s.dict.get("OC")
            && !self.visible(oc)
        {
            return;
        }
        if self.hidden_now() {
            return;
        }
        match s.dict.name("Subtype") {
            Some("Image") => self.image_xobject(&s, r, res),
            Some("Form") => self.form(&s, res.cloned(), depth + 1, false),
            _ => {}
        }
    }

    /// Runs a form XObject; `force_inline` draws a soft mask's group
    /// straight onto the (already off-screen) canvas.
    fn form(&mut self, s: &Arc<Stream>, parent_res: Option<Arc<Dict>>, depth: usize, force_inline: bool) {
        if depth > MAX_DEPTH {
            return;
        }
        let doc = self.doc;
        let Ok(content) = doc.decode_stream(s) else { return };
        let res = doc.dict_in(&s.dict, "Resources").or(parent_res);
        let m = doc.numbers_in(&s.dict, "Matrix");
        let matrix = if m.len() == 6 { Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5]) } else { Matrix::IDENTITY };
        let saved = self.gs.clone();
        let saved_path = std::mem::take(&mut self.path);
        self.gs.ctm = matrix.then(&self.gs.ctm);
        let bbox = doc.numbers_in(&s.dict, "BBox");
        if bbox.len() == 4 {
            let mut p = Path::new();
            let (x0, y0, x1, y1) = (bbox[0], bbox[1], bbox[2], bbox[3]);
            p.move_to(x0, y0);
            p.line_to(x1, y0);
            p.line_to(x1, y1);
            p.line_to(x0, y1);
            p.close();
            let dev = p.transformed(&self.gs.ctm);
            self.intersect_clip_device(&dev, FillRule::NonZero);
        }
        let group = doc.dict_in(&s.dict, "Group").is_some_and(|g| g.name("S") == Some("Transparency"));
        let needs_offscreen = !force_inline
            && group
            && (self.gs.fill_alpha < 1.0 || self.gs.blend != Blend::Normal || self.gs.soft.is_some());
        if needs_offscreen
            && self.offscreen < MAX_OFFSCREEN
            && self.canvas.width * self.canvas.height <= MAX_PAGE_PIXELS
        {
            let (w, h) = (self.canvas.width, self.canvas.height);
            let alpha = alpha8(self.gs.fill_alpha);
            let (blend, soft, clip) = (self.gs.blend, self.gs.soft.clone(), self.gs.clip.clone());
            let saved_canvas = std::mem::replace(&mut self.canvas, Canvas::transparent(w, h));
            self.gs.fill_alpha = 1.0;
            self.gs.stroke_alpha = 1.0;
            self.gs.blend = Blend::Normal;
            self.gs.soft = None;
            let base = self.gs.ctm;
            self.offscreen += 1;
            self.run(&content.data, res, base, depth);
            self.offscreen -= 1;
            let group_canvas = std::mem::replace(&mut self.canvas, saved_canvas);
            self.canvas.composite(&group_canvas, alpha, clip.as_deref(), soft.as_deref(), blend);
        } else {
            let base = self.gs.ctm;
            self.run(&content.data, res, base, depth);
        }
        self.gs = saved;
        self.path = saved_path;
    }

    fn image_xobject(&mut self, s: &Arc<Stream>, r: Option<ObjRef>, res: Option<&Arc<Dict>>) {
        let ctm = self.gs.ctm;
        let target = draw_size(&ctm);
        let bucket = (target.0.next_power_of_two(), target.1.next_power_of_two());
        let img = match r.and_then(|r| self.images.get(&(r, bucket.0, bucket.1)).cloned()) {
            Some(i) => i,
            None => {
                let decoded = match self.doc.decode_stream(s) {
                    Ok(d) => d,
                    Err(e) => {
                        let what = if e.kind == crate::error::ErrorKind::Unsupported {
                            e.message
                        } else {
                            "damaged images".into()
                        };
                        self.unsupported(&what);
                        return;
                    }
                };
                let img = match image::decode(self.doc, &s.dict, decoded, res.map(|r| r.as_ref()), target) {
                    Ok(i) => Arc::new(i),
                    Err(_) => {
                        self.unsupported("damaged images");
                        return;
                    }
                };
                if let Some(r) = r {
                    if self.images.len() > 64 {
                        self.images.clear();
                    }
                    self.images.insert((r, bucket.0, bucket.1), img.clone());
                }
                img
            }
        };
        self.draw_image(&img, &ctm);
    }

    fn inline_image(&mut self, l: &mut Lexer, res: Option<&Dict>) {
        let mut d = Dict::new();
        loop {
            let Some(t) = l.next_token() else { return };
            match t {
                Token::Keyword(b"ID") => break,
                Token::Name(k) => {
                    let Some(v) = l.object() else { return };
                    d.entries.push((k, v));
                }
                _ => {}
            }
        }
        // One whitespace byte after `ID`, then the data.
        let start = (l.pos + 1).min(l.data.len());
        let data = l.data;
        let filters = self.doc.filters(&d);
        let expected = if filters.is_empty() {
            let w = d.get_any(&["W", "Width"]).and_then(Object::as_i64).unwrap_or(0).max(0) as usize;
            let h = d.get_any(&["H", "Height"]).and_then(Object::as_i64).unwrap_or(0).max(0) as usize;
            let bpc = d.get_any(&["BPC", "BitsPerComponent"]).and_then(Object::as_i64).unwrap_or(1).max(1) as usize;
            let mask = d.get_any(&["IM", "ImageMask"]).and_then(Object::as_bool).unwrap_or(false);
            let n = if mask {
                1
            } else {
                let cs = d.get_any(&["CS", "ColorSpace"]).cloned().unwrap_or(Object::name("DeviceGray"));
                ColorSpace::parse(self.doc, &cs, res).components().max(1)
            };
            Some((w * n * bpc).div_ceil(8) * h)
        } else {
            None
        };
        let end = match expected {
            Some(n) if start + n <= data.len() => start + n,
            _ => find_ei(data, start).unwrap_or(data.len()),
        };
        let raw = &data[start..end.max(start)];
        // Skip past `EI`.
        let after = find_ei(data, end).map_or(data.len(), |e| e + 2);
        l.pos = after.min(data.len());
        if self.hidden_now() {
            return;
        }
        let decoded = match crate::filter::decode(raw, &filters, None) {
            Ok(x) => x,
            Err(e) => {
                self.unsupported(&e.message);
                return;
            }
        };
        let ctm = self.gs.ctm;
        if let Ok(img) = image::decode(self.doc, &d, decoded, res, draw_size(&ctm)) {
            self.draw_image(&img, &ctm);
        }
    }

    fn draw_image(&mut self, img: &RasterImage, ctm: &Matrix) {
        let mut unit = Path::new();
        unit.move_to(0.0, 0.0);
        unit.line_to(1.0, 0.0);
        unit.line_to(1.0, 1.0);
        unit.line_to(0.0, 1.0);
        unit.close();
        let dev = unit.transformed(ctm);
        let limit = self.limit();
        let Some(cov) = raster::coverage(&dev, FillRule::NonZero, limit) else { return };
        let fill = self.forced.unwrap_or(self.gs.fill.rgb);
        let Some(shader) = ImageShader::new(img, ctm, fill) else { return };
        self.stats.images += 1;
        let page = (self.canvas.width * self.canvas.height).max(1) as f64;
        if !img.stencil {
            self.stats.image_area += (cov.rect.width() * cov.rect.height()) as f64 / page;
        }
        let a = alpha8(self.gs.fill_alpha);
        let clip = self.gs.clip.clone();
        let soft = self.gs.soft.clone();
        raster::paint(&mut self.canvas, &cov, &shader, a, clip.as_deref(), soft.as_deref(), self.gs.blend);
    }

    // ---- shadings and patterns ----------------------------------------------------------------

    fn shade_op(&mut self, name: &str, res: Option<&Arc<Dict>>) {
        if self.hidden_now() {
            return;
        }
        let doc = self.doc;
        let Some(shs) = res.and_then(|r| doc.dict_in(r, "Shading")) else { return };
        let Some(o) = shs.get(name).cloned() else { return };
        let Some(sh) = Shading::parse(doc, &o, res.map(|r| r.as_ref())) else { return };
        let limit = self.limit();
        if limit.is_empty() {
            return;
        }
        // `sh` paints the clip (or the shading's bbox).
        let cov = match sh.bbox {
            Some(b) => {
                let mut p = Path::new();
                p.move_to(b[0], b[1]);
                p.line_to(b[2], b[1]);
                p.line_to(b[2], b[3]);
                p.line_to(b[0], b[3]);
                p.close();
                raster::coverage(&p.transformed(&self.gs.ctm), FillRule::NonZero, limit)
            }
            None => Some(Coverage { rect: limit, data: vec![255; limit.width() * limit.height()] }),
        };
        let Some(cov) = cov else { return };
        let ctm = self.gs.ctm;
        let Some(shader) = sh.shader(doc, &ctm, cov.rect) else {
            self.unsupported("some shadings");
            return;
        };
        let a = alpha8(self.gs.fill_alpha);
        let clip = self.gs.clip.clone();
        let soft = self.gs.soft.clone();
        raster::paint(&mut self.canvas, &cov, shader.as_ref(), a, clip.as_deref(), soft.as_deref(), self.gs.blend);
    }

    fn pattern_shader(&mut self, pat: &Object, area: IRect, comps: &[f64], cs: &ColorSpace) -> Option<Box<dyn Shader>> {
        let doc = self.doc;
        let d = pat.as_dict()?.clone();
        let m = doc.numbers_in(&d, "Matrix");
        let pm = if m.len() == 6 { Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5]) } else { Matrix::IDENTITY };
        let to_dev = pm.then(&self.page_base);
        match doc.get_in(&d, "PatternType").as_i64() {
            Some(2) => {
                let sh = Shading::parse(doc, d.get("Shading")?, None)?;
                sh.shader(doc, &to_dev, area)
            }
            Some(1) => {
                let s = pat.as_stream()?.clone();
                let tile = self.render_tile(&s, &to_dev, comps, cs)?;
                Some(Box::new(tile))
            }
            _ => None,
        }
    }

    /// Renders one cell of a tiling pattern (§8.7.3) at device resolution.
    fn render_tile(&mut self, s: &Arc<Stream>, to_dev: &Matrix, comps: &[f64], cs: &ColorSpace) -> Option<TileShader> {
        let doc = self.doc;
        let d = &s.dict;
        let xstep = doc.f64_in(d, "XStep")?.abs();
        let ystep = doc.f64_in(d, "YStep")?.abs();
        let bbox = doc.numbers_in(d, "BBox");
        if bbox.len() < 4 || xstep < 1e-6 || ystep < 1e-6 || self.stack.len() > MAX_DEPTH {
            return None;
        }
        let scale = to_dev.expansion().max(1e-6);
        let tw = ((xstep * scale).ceil() as usize).clamp(1, MAX_TILE);
        let th = ((ystep * scale).ceil() as usize).clamp(1, MAX_TILE);
        let (sx, sy) = (tw as f64 / xstep, th as f64 / ystep);
        // Pattern cell space to tile pixels (y down).
        let cell = Matrix::new(sx, 0.0, 0.0, -sy, -bbox[0].min(bbox[2]) * sx, (bbox[1].min(bbox[3]) + ystep) * sy);
        let uncolored = doc.get_in(d, "PaintType").as_i64() == Some(2);
        let forced = if uncolored {
            let under = match cs {
                ColorSpace::Pattern { under: Some(u) } => u.to_rgb(comps),
                _ => [0, 0, 0],
            };
            Some(under)
        } else {
            None
        };
        let content = doc.decode_stream(s).ok()?;
        let res = doc.dict_in(d, "Resources");
        let saved_canvas = std::mem::replace(&mut self.canvas, Canvas::transparent(tw, th));
        let saved_gs = self.gs.clone();
        let saved_stack = std::mem::take(&mut self.stack);
        let saved_forced = self.forced;
        let saved_path = std::mem::take(&mut self.path);
        let saved_base = self.page_base;
        self.gs = GState::new();
        self.gs.ctm = cell;
        self.page_base = cell;
        self.forced = forced;
        self.run(&content.data, res, cell, saved_stack.len() + 1);
        let tile = std::mem::replace(&mut self.canvas, saved_canvas);
        self.gs = saved_gs;
        self.stack = saved_stack;
        self.forced = saved_forced;
        self.path = saved_path;
        self.page_base = saved_base;
        // Device pixel to tile pixel.
        let inv = to_dev.invert()?.then(&cell);
        Some(TileShader { tile, inv })
    }

    // ---- annotations ----------------------------------------------------------------------------

    fn annotations(&mut self, page: &Page, base: Matrix) {
        let doc = self.doc;
        let annots = doc.array_in(&page.dict, "Annots");
        for a in annots.iter().take(2000) {
            let Some(ad) = a.as_dict() else { continue };
            let flags = doc.get_in(ad, "F").as_i64().unwrap_or(0);
            if flags & (2 | 32) != 0 || ad.name("Subtype") == Some("Popup") {
                continue;
            }
            let Some(ap) = doc.dict_in(ad, "AP") else { continue };
            let n = doc.get_in(&ap, "N");
            let s = match n {
                Object::Stream(s) => s,
                Object::Dict(states) => {
                    let Some(state) = ad.name("AS") else { continue };
                    match doc.get_in(&states, state) {
                        Object::Stream(s) => s,
                        _ => continue,
                    }
                }
                _ => continue,
            };
            let rect = doc.numbers_in(ad, "Rect");
            let bbox = doc.numbers_in(&s.dict, "BBox");
            if rect.len() < 4 || bbox.len() < 4 {
                continue;
            }
            let m = doc.numbers_in(&s.dict, "Matrix");
            let fm = if m.len() == 6 { Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5]) } else { Matrix::IDENTITY };
            // §12.5.5: the transformed BBox fitted onto Rect.
            let corners = [(bbox[0], bbox[1]), (bbox[2], bbox[1]), (bbox[2], bbox[3]), (bbox[0], bbox[3])]
                .map(|(x, y)| fm.apply(Point::new(x, y)));
            let (bx0, by0) = corners.iter().fold((f64::MAX, f64::MAX), |(a, b), p| (a.min(p.x), b.min(p.y)));
            let (bx1, by1) = corners.iter().fold((f64::MIN, f64::MIN), |(a, b), p| (a.max(p.x), b.max(p.y)));
            let (rx0, ry0, rx1, ry1) =
                (rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3]));
            if bx1 - bx0 < 1e-9 || by1 - by0 < 1e-9 {
                continue;
            }
            let sx = (rx1 - rx0) / (bx1 - bx0);
            let sy = (ry1 - ry0) / (by1 - by0);
            let fit = Matrix::new(sx, 0.0, 0.0, sy, rx0 - bx0 * sx, ry0 - by0 * sy);
            let saved = self.gs.clone();
            self.gs = GState::new();
            self.gs.ctm = fit.then(&base);
            self.form(&s, page.resources.clone(), 1, false);
            self.gs = saved;
            if self.stopped {
                break;
            }
        }
    }
}

struct TileShader {
    tile: Canvas,
    inv: Matrix,
}

impl Shader for TileShader {
    fn shade(&self, x: i32, y: i32) -> ([u8; 3], u8) {
        let p = self.inv.apply(Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
        let (w, h) = (self.tile.width as i64, self.tile.height as i64);
        let tx = (p.x.floor() as i64).rem_euclid(w) as usize;
        let ty = (p.y.floor() as i64).rem_euclid(h) as usize;
        let i = ty * self.tile.width + tx;
        let a = self.tile.alpha.as_ref().map_or(255, |a| a[i]);
        ([self.tile.rgb[i * 3], self.tile.rgb[i * 3 + 1], self.tile.rgb[i * 3 + 2]], a)
    }
}

/// A thin path of only horizontal and vertical lines, in device space
/// with each line centred on a pixel row or column; `None` otherwise.
fn snapped(path: &Path, ctm: &Matrix, dev_width: f64) -> Option<Path> {
    use crate::geom::Seg;
    if dev_width >= 2.5 || path.segs.len() > 4096 {
        return None;
    }
    let dev = path.transformed(ctm);
    let mut prev = Point::default();
    let mut start = Point::default();
    for s in &dev.segs {
        match *s {
            Seg::Move(p) => {
                prev = p;
                start = p;
            }
            Seg::Line(p) => {
                if (p.x - prev.x).abs() > 0.01 && (p.y - prev.y).abs() > 0.01 {
                    return None;
                }
                prev = p;
            }
            Seg::Close => {
                if (start.x - prev.x).abs() > 0.01 && (start.y - prev.y).abs() > 0.01 {
                    return None;
                }
                prev = start;
            }
            _ => return None,
        }
    }
    // Odd widths centre on pixel centres, even ones on pixel edges.
    let w = dev_width.round().max(1.0);
    let snap = |v: f64| if w as i64 % 2 == 1 { v.floor() + 0.5 } else { v.round() };
    let at = |p: Point| Point::new(snap(p.x), snap(p.y));
    let segs = dev
        .segs
        .iter()
        .map(|s| match *s {
            Seg::Move(p) => Seg::Move(at(p)),
            Seg::Line(p) => Seg::Line(at(p)),
            other => other,
        })
        .collect();
    Some(Path { segs })
}

fn alpha8(a: f64) -> u8 {
    (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn current_point(p: &Path) -> Point {
    use crate::geom::Seg;
    let mut start = Point::default();
    let mut cur = Point::default();
    for s in &p.segs {
        match *s {
            Seg::Move(q) => {
                start = q;
                cur = q;
            }
            Seg::Line(q) | Seg::Quad(_, q) | Seg::Cubic(_, _, q) => cur = q,
            Seg::Close => cur = start,
        }
    }
    cur
}

/// The size in device pixels an image drawn by `ctm` covers.
fn draw_size(ctm: &Matrix) -> (usize, usize) {
    let w = (ctm.a * ctm.a + ctm.b * ctm.b).sqrt();
    let h = (ctm.c * ctm.c + ctm.d * ctm.d).sqrt();
    (w.clamp(1.0, 1e5) as usize, h.clamp(1.0, 1e5) as usize)
}

/// The `EI` ending inline image data at or after `from`: whitespace on
/// both sides, followed by something that is not binary data.
fn find_ei(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < data.len() {
        if data[i] == b'E'
            && data[i + 1] == b'I'
            && (i == 0 || crate::lexer::is_white(data[i - 1]))
            && data.get(i + 2).is_none_or(|&c| crate::lexer::is_white(c))
        {
            // The next bytes should look like an operator stream.
            let tail = &data[(i + 2).min(data.len())..(i + 40).min(data.len())];
            if tail.iter().all(|&c| c.is_ascii() && (c >= 0x20 || crate::lexer::is_white(c))) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Optional content groups off in the default configuration.
fn hidden_groups(doc: &Document) -> HashSet<ObjRef> {
    let mut out = HashSet::new();
    let Some(cat) = doc.catalog() else { return out };
    let Some(ocp) = doc.dict_in(&cat, "OCProperties") else { return out };
    let Some(d) = doc.dict_in(&ocp, "D") else { return out };
    let refs = |key: &str, d: &Dict| -> Vec<ObjRef> {
        match d.get(key).map(|o| doc.resolve(o)) {
            Some(Object::Array(a)) => a.iter().filter_map(Object::as_ref).collect(),
            _ => Vec::new(),
        }
    };
    if d.name("BaseState") == Some("OFF") {
        out.extend(refs("OCGs", &ocp));
        for r in refs("ON", &d) {
            out.remove(&r);
        }
    } else {
        out.extend(refs("OFF", &d));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{build, stream};

    fn render(content: &[u8], w: usize) -> Rendered {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 5 0 R >>",
            &stream("", content),
        ]);
        let doc = Document::open(pdf).unwrap();
        render_page(&doc, 0, &RenderOptions { width: w, ..RenderOptions::default() }).unwrap()
    }

    fn px(r: &Rendered, x: usize, y: usize) -> [u8; 3] {
        let i = (y * r.width + x) * 3;
        [r.rgb[i], r.rgb[i + 1], r.rgb[i + 2]]
    }

    #[test]
    fn fills_strokes_and_the_y_flip() {
        // A red square at the bottom left in PDF space is at the bottom
        // left of the picture.
        let r = render(b"1 0 0 rg 0 0 50 50 re f 0 0 1 RG 4 w 0 90 m 100 90 l S", 100);
        assert_eq!((r.width, r.height), (100, 100));
        assert_eq!(px(&r, 10, 90), [255, 0, 0]);
        assert_eq!(px(&r, 10, 40), [255, 255, 255]);
        assert_eq!(px(&r, 60, 10), [0, 0, 255]);
        assert_eq!(px(&r, 60, 14), [255, 255, 255]);
        // Scaled to 200 pixels wide.
        let r = render(b"0 g 0 0 50 50 re f", 200);
        assert_eq!(px(&r, 20, 180), [0, 0, 0]);
        assert_eq!(px(&r, 120, 180), [255, 255, 255]);
    }

    #[test]
    fn clipping_and_graphics_state() {
        let r = render(b"q 0 0 50 100 re W n 0 g 0 0 100 100 re f Q 0 0 1 rg 90 90 10 10 re f", 100);
        assert_eq!(px(&r, 25, 50), [0, 0, 0]);
        assert_eq!(px(&r, 75, 50), [255, 255, 255]);
        assert_eq!(px(&r, 95, 5), [0, 0, 255], "Q restored the clip");
        // cm scales the next path.
        let r = render(b"2 0 0 2 0 0 cm 0 g 0 0 10 10 re f", 100);
        assert_eq!(px(&r, 15, 85), [0, 0, 0]);
        assert_eq!(px(&r, 25, 85), [255, 255, 255]);
    }

    #[test]
    fn inline_images() {
        // A 2×2 RGB inline image filling the page: red, green / blue, white.
        let mut content = b"q 100 0 0 100 0 0 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID ".to_vec();
        content.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        content.extend_from_slice(b" EI Q");
        let r = render(&content, 100);
        assert_eq!(px(&r, 5, 5), [255, 0, 0]);
        assert_eq!(px(&r, 95, 5), [0, 255, 0]);
        assert_eq!(px(&r, 5, 95), [0, 0, 255]);
        assert_eq!(r.stats.images, 1);
        assert!(r.stats.image_area > 0.9);
        // A stencil mask paints the fill colour where bits are 0.
        let mut content = b"0 0 1 rg q 100 0 0 100 0 0 cm BI /W 2 /H 1 /IM true ID ".to_vec();
        content.push(0b0100_0000);
        content.extend_from_slice(b" EI Q");
        let r = render(&content, 100);
        assert_eq!(px(&r, 10, 50), [0, 0, 255]);
        assert_eq!(px(&r, 90, 50), [255, 255, 255]);
    }

    #[test]
    fn transparency() {
        let r = render(b"/GS0 gs 0 g 0 0 100 100 re f", 100);
        assert_eq!(px(&r, 50, 50), [0, 0, 0], "an unknown gs is ignored");
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 5 0 R /Resources << /ExtGState << /H << /ca 0.5 >> >> >> >>",
            &stream("", b"/H gs 0 g 0 0 100 100 re f"),
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 100, ..RenderOptions::default() }).unwrap();
        assert_eq!(px(&r, 50, 50), [128, 128, 128]);
    }

    #[test]
    fn rotation_and_crop() {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Rotate 90 /Contents 5 0 R >>",
            &stream("", b"1 0 0 rg 0 0 20 20 re f"),
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 100, ..RenderOptions::default() }).unwrap();
        assert_eq!((r.width, r.height), (100, 200));
        // The page's bottom-left corner is the picture's top-left.
        assert_eq!(px(&r, 5, 5), [255, 0, 0]);
        assert_eq!(px(&r, 95, 195), [255, 255, 255]);
    }

    #[test]
    fn budget_and_hostile_content() {
        let mut content = Vec::new();
        for _ in 0..5000 {
            content.extend_from_slice(b"q 0 0 1 1 re f ");
        }
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 5 0 R >>",
            &stream("", &content),
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 50, max_ops: 1000, ..RenderOptions::default() }).unwrap();
        assert!(r.stats.truncated);
        for junk in [
            &b"BI /W 99999 /H 99999 ID xyz"[..],
            b"1 2 3 4 5 6 cm cm cm Tj TJ ' \" BT Tf ET",
            b"q q q Q Q Q Q Q W n",
            b"/X Do /Y sh /Z gs",
        ] {
            let _ = render(junk, 50);
        }
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;
    use crate::document::tests::{build, stream};

    #[test]
    fn thin_curve_is_thin() {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 5 0 R >>",
            &stream("", b"q 0.0163 0 0 0.0163 0 0 cm 1.58728 w 500 500 m 1500 5500 3500 -500 5500 5000 c S Q"),
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 100, ..RenderOptions::default() }).unwrap();
        // A 0.26-pixel line is drawn one pixel wide, no wider.
        let dark: usize = r.rgb.chunks(3).filter(|p| p[0] < 128).count();
        assert!((90..170).contains(&dark), "{dark}");
    }
}

/// A rendered page as an image file: JPEG when pictures cover most of
/// it (a scan, a photo), PNG otherwise (text stays crisp).
pub fn encode(r: &Rendered) -> Result<(Vec<u8>, &'static str)> {
    if r.stats.image_area > 0.45 {
        let b = crate::codec::jpeg::encode(&r.rgb, r.width as u32, r.height as u32, 3, 85)
            .map_err(|e| PdfError::invalid(format!("jpeg: {e}")))?;
        return Ok((b, "image/jpeg"));
    }
    let b = crate::codec::png::encode(&r.rgb, r.width as u32, r.height as u32, crate::codec::png::PngColor::Rgb, 4)
        .map_err(|e| PdfError::invalid(format!("png: {e}")))?;
    Ok((b, "image/png"))
}

#[cfg(test)]
mod text_tests {
    use super::*;
    use crate::document::tests::{build, stream};

    /// A standard 14 font named without being embedded is drawn with an
    /// installed look-alike; with none installed the page says so.
    #[test]
    fn non_embedded_fonts_use_a_substitute() {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 60] /Contents 5 0 R /Resources << /Font << /F1 6 0 R >> >> >>",
            &stream("", b"BT /F1 32 Tf 10 15 Td (Hello) Tj ET"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 200, ..RenderOptions::default() }).unwrap();
        let dark = r.rgb.chunks(3).filter(|p| p[0] < 100).count();
        if crate::font::system::available() {
            assert!(dark > 100, "text drawn with a substitute: {dark}");
            assert!(!r.stats.missing_fonts);
        } else {
            assert!(r.stats.missing_fonts, "no substitute installed: said");
        }
    }
}

#[cfg(test)]
mod hostile_tests {
    use super::*;
    use crate::document::tests::{build, stream};

    /// A soft mask whose group sets the same soft mask again would recurse
    /// for ever; it stops at the off-screen limit.
    #[test]
    fn a_self_referencing_soft_mask_ends() {
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 50] /Contents 5 0 R /Resources 7 0 R >>",
            &stream("", b"/G0 gs 0 g 0 0 50 50 re f"),
            &stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Group << /S /Transparency >> /Resources 7 0 R",
                b"/G0 gs 1 g 0 0 50 50 re f",
            ),
            b"<< /ExtGState << /G0 << /SMask << /S /Luminosity /G 6 0 R >> >> >> >>",
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 50, ..RenderOptions::default() }).unwrap();
        assert_eq!((r.width, r.height), (50, 50));
    }
}

#[cfg(test)]
mod jpx_tests {
    use super::*;
    use crate::document::tests::{build, stream};

    /// A JPEG 2000 image (made by OpenJPEG or ImageMagick when one is
    /// installed) is drawn.
    #[test]
    fn jpeg_2000_images_are_drawn() {
        let dir = std::env::temp_dir().join(format!("lain-pdf-jpx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A 4×2 PPM: left half red, right half blue.
        let mut ppm = b"P6\n4 2\n255\n".to_vec();
        for _ in 0..2 {
            ppm.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255]);
        }
        std::fs::write(dir.join("in.ppm"), &ppm).unwrap();
        let (inp, out) =
            (dir.join("in.ppm").to_string_lossy().into_owned(), dir.join("out.jp2").to_string_lossy().into_owned());
        let run = |cmd: &str, args: &[&str]| {
            std::process::Command::new(cmd).args(args).output().is_ok_and(|o| o.status.success())
        };
        let ok = run("opj_compress", &["-i", &inp, "-o", &out]) || run("magick", &[&inp, &out]);
        if !ok {
            eprintln!("no JPEG 2000 encoder installed; skipped");
            std::fs::remove_dir_all(dir).ok();
            return;
        }
        let jp2 = std::fs::read(dir.join("out.jp2")).unwrap();
        let pdf = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
            b"<< >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 5 0 R /Resources << /XObject << /I 6 0 R >> >> >>",
            &stream("", b"q 40 0 0 20 0 0 cm /I Do Q"),
            &stream("/Type /XObject /Subtype /Image /Width 4 /Height 2 /BitsPerComponent 8 /Filter /JPXDecode", &jp2),
        ]);
        let doc = Document::open(pdf).unwrap();
        let r = render_page(&doc, 0, &RenderOptions { width: 40, ..RenderOptions::default() }).unwrap();
        let px = |x: usize, y: usize| r.rgb[(y * r.width + x) * 3..(y * r.width + x) * 3 + 3].to_vec();
        assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
        let (left, right) = (px(4, 10), px(35, 10));
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
        std::fs::remove_dir_all(dir).ok();
    }
}
