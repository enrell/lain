//! Stroking (§8.5.3): a path's outline turned into closed polygons, one
//! per segment, join and cap, all wound the same way so a non-zero fill
//! of their union is the stroke. Geometry is built in user space (the
//! pen is round there) and mapped to device space at the end.

use std::f64::consts::PI;

use crate::geom::{Matrix, Path, Point};
use crate::raster;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeStyle {
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub dash_phase: f64,
}

impl Default for StrokeStyle {
    fn default() -> StrokeStyle {
        StrokeStyle {
            width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
        }
    }
}

/// Polygons are capped: a hostile dash pattern cannot make millions.
const MAX_PIECES: usize = 400_000;

/// The stroke of user-space `path` under `ctm`, as device-space line
/// contours ready for [`raster::coverage_lines`]. Lines thinner than one
/// device pixel are drawn one pixel wide, as viewers do.
pub fn stroke(path: &Path, style: &StrokeStyle, ctm: &Matrix) -> Vec<(Point, Point)> {
    let expansion = ctm.expansion().max(1e-9);
    let min_user = 1.0 / expansion;
    let width = style.width.abs().max(min_user);
    let half = width / 2.0;
    let tol = 0.2 / expansion;
    let polylines = polylines(path, tol);
    let polylines = if dashing(style) { dash(&polylines, style) } else { polylines };
    let mut out: Vec<(Point, Point)> = Vec::new();
    let arc_steps = ((half * expansion).sqrt() * 4.0).ceil().clamp(6.0, 64.0) as usize;
    let mut pieces = 0usize;
    let mut emit = |poly: &[Point], out: &mut Vec<(Point, Point)>| {
        pieces += 1;
        if pieces > MAX_PIECES || poly.len() < 3 {
            return;
        }
        let dev: Vec<Point> = poly.iter().map(|p| ctm.apply(*p)).collect();
        // Every polygon counter-clockwise in device space.
        let area: f64 = (0..dev.len())
            .map(|i| {
                let (a, b) = (dev[i], dev[(i + 1) % dev.len()]);
                a.x * b.y - b.x * a.y
            })
            .sum();
        let n = dev.len();
        if area < 0.0 {
            for i in (0..n).rev() {
                out.push((dev[(i + 1) % n], dev[i]));
            }
        } else {
            for i in 0..n {
                out.push((dev[i], dev[(i + 1) % n]));
            }
        }
    };
    for pl in &polylines {
        let pts = &pl.points;
        if pts.is_empty() {
            continue;
        }
        if pts.len() == 1 || pts.iter().all(|p| *p == pts[0]) {
            // A zero-length subpath shows only with round or square caps.
            match style.cap {
                Cap::Round => emit(&circle(pts[0], half, arc_steps), &mut out),
                Cap::Square => {
                    let p = pts[0];
                    emit(
                        &[
                            Point::new(p.x - half, p.y - half),
                            Point::new(p.x + half, p.y - half),
                            Point::new(p.x + half, p.y + half),
                            Point::new(p.x - half, p.y + half),
                        ],
                        &mut out,
                    );
                }
                Cap::Butt => {}
            }
            continue;
        }
        let segs: Vec<(Point, Point)> = pts.windows(2).filter(|w| w[0] != w[1]).map(|w| (w[0], w[1])).collect();
        for &(a, b) in &segs {
            let (nx, ny) = normal(a, b, half);
            emit(
                &[
                    Point::new(a.x + nx, a.y + ny),
                    Point::new(b.x + nx, b.y + ny),
                    Point::new(b.x - nx, b.y - ny),
                    Point::new(a.x - nx, a.y - ny),
                ],
                &mut out,
            );
        }
        // Joins between consecutive segments (and around a closed one).
        let mut joins: Vec<((Point, Point), (Point, Point))> = segs.windows(2).map(|w| (w[0], w[1])).collect();
        if pl.closed && segs.len() > 1 {
            joins.push((segs[segs.len() - 1], segs[0]));
        }
        for (s1, s2) in joins {
            join(s1, s2, half, style, arc_steps, &mut |poly| emit(poly, &mut out));
        }
        if !pl.closed && !segs.is_empty() {
            let (a, b) = segs[0];
            cap(a, b, half, style.cap, arc_steps, &mut |poly| emit(poly, &mut out));
            let (a, b) = segs[segs.len() - 1];
            cap(b, a, half, style.cap, arc_steps, &mut |poly| emit(poly, &mut out));
        }
    }
    out
}

fn dashing(style: &StrokeStyle) -> bool {
    !style.dash.is_empty()
        && style.dash.iter().all(|d| d.is_finite() && *d >= 0.0)
        && style.dash.iter().sum::<f64>() > 1e-9
}

fn normal(a: Point, b: Point, half: f64) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-12);
    (-dy / len * half, dx / len * half)
}

fn circle(c: Point, r: f64, steps: usize) -> Vec<Point> {
    (0..steps * 2)
        .map(|i| {
            let t = i as f64 / (steps * 2) as f64 * 2.0 * PI;
            Point::new(c.x + r * t.cos(), c.y + r * t.sin())
        })
        .collect()
}

/// The cap at `end`, the segment coming from `from`.
fn cap(end: Point, from: Point, half: f64, cap: Cap, steps: usize, emit: &mut dyn FnMut(&[Point])) {
    match cap {
        Cap::Butt => {}
        Cap::Round => emit(&circle(end, half, steps)),
        Cap::Square => {
            let (dx, dy) = (end.x - from.x, end.y - from.y);
            let len = (dx * dx + dy * dy).sqrt().max(1e-12);
            let (ux, uy) = (dx / len * half, dy / len * half);
            let (nx, ny) = (-uy, ux);
            emit(&[
                Point::new(end.x + nx, end.y + ny),
                Point::new(end.x + nx + ux, end.y + ny + uy),
                Point::new(end.x - nx + ux, end.y - ny + uy),
                Point::new(end.x - nx, end.y - ny),
            ]);
        }
    }
}

fn join(
    s1: (Point, Point),
    s2: (Point, Point),
    half: f64,
    style: &StrokeStyle,
    steps: usize,
    emit: &mut dyn FnMut(&[Point]),
) {
    let v = s1.1;
    let (d1x, d1y) = (s1.1.x - s1.0.x, s1.1.y - s1.0.y);
    let (d2x, d2y) = (s2.1.x - s2.0.x, s2.1.y - s2.0.y);
    let cross = d1x * d2y - d1y * d2x;
    let dot = d1x * d2x + d1y * d2y;
    if cross.abs() < 1e-12 && dot > 0.0 {
        return; // straight on
    }
    match style.join {
        Join::Round => emit(&circle(v, half, steps)),
        Join::Bevel | Join::Miter => {
            let (n1x, n1y) = normal(s1.0, s1.1, half);
            let (n2x, n2y) = normal(s2.0, s2.1, half);
            // The outer side is opposite the turn.
            let sign = if cross > 0.0 { -1.0 } else { 1.0 };
            let a = Point::new(v.x + sign * n1x, v.y + sign * n1y);
            let b = Point::new(v.x + sign * n2x, v.y + sign * n2y);
            if style.join == Join::Miter {
                let l1 = (d1x * d1x + d1y * d1y).sqrt().max(1e-12);
                let l2 = (d2x * d2x + d2y * d2y).sqrt().max(1e-12);
                let cos = (-(dot) / (l1 * l2)).clamp(-1.0, 1.0);
                let theta = cos.acos();
                let ratio = 1.0 / (theta / 2.0).sin().max(1e-9);
                if ratio <= style.miter_limit.max(1.0) {
                    // The miter tip: along the bisector of the outer normals.
                    let (mx, my) = (a.x + b.x - 2.0 * v.x, a.y + b.y - 2.0 * v.y);
                    let ml = (mx * mx + my * my).sqrt();
                    if ml > 1e-12 {
                        let dist = half * ratio;
                        let tip = Point::new(v.x + mx / ml * dist, v.y + my / ml * dist);
                        emit(&[v, a, tip, b]);
                        return;
                    }
                }
            }
            emit(&[v, a, b]);
        }
    }
}

pub struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// User-space polylines of a path, curves flattened within `tol`. A
/// subpath that is only a move paints nothing (§8.5.3.1).
pub fn polylines(path: &Path, tol: f64) -> Vec<Polyline> {
    use crate::geom::Seg;
    let mut out: Vec<Polyline> = Vec::new();
    let mut sub = Path::new();
    let mut drawn = false;
    let flush = |sub: &mut Path, out: &mut Vec<Polyline>, closed: bool, drawn: bool| {
        if drawn {
            let mut pts: Vec<Point> = Vec::new();
            raster::flatten_into(
                sub,
                tol,
                &mut |a, b| {
                    if pts.is_empty() {
                        pts.push(a);
                    }
                    pts.push(b);
                },
                false,
            );
            out.push(Polyline { points: pts, closed });
        }
        sub.segs.clear();
    };
    let mut start = Point::default();
    for s in &path.segs {
        match *s {
            Seg::Move(p) => {
                flush(&mut sub, &mut out, false, drawn);
                drawn = false;
                start = p;
                sub.segs.push(*s);
            }
            Seg::Close => {
                if drawn {
                    sub.segs.push(Seg::Line(start));
                    flush(&mut sub, &mut out, true, true);
                } else {
                    sub.segs.clear();
                }
                drawn = false;
                // Drawing may go on from the start point.
                sub.segs.push(Seg::Move(start));
            }
            other => {
                if sub.is_empty() {
                    sub.segs.push(Seg::Move(start));
                }
                sub.segs.push(other);
                drawn = true;
            }
        }
    }
    flush(&mut sub, &mut out, false, drawn);
    out
}

/// Splits polylines into dashes (§8.4.3.6).
fn dash(lines: &[Polyline], style: &StrokeStyle) -> Vec<Polyline> {
    let pattern: Vec<f64> =
        if style.dash.len() % 2 == 1 { [style.dash.clone(), style.dash.clone()].concat() } else { style.dash.clone() };
    let total: f64 = pattern.iter().sum();
    let mut out = Vec::new();
    for pl in lines {
        // The phase restarts on every subpath.
        let mut phase = style.dash_phase.rem_euclid(total);
        let mut idx = 0usize;
        while phase >= pattern[idx] {
            phase -= pattern[idx];
            idx = (idx + 1) % pattern.len();
        }
        let mut left = pattern[idx] - phase;
        let mut on = idx.is_multiple_of(2);
        let mut current: Vec<Point> = Vec::new();
        if on && let Some(p) = pl.points.first() {
            current.push(*p);
        }
        for w in pl.points.windows(2) {
            let (mut a, b) = (w[0], w[1]);
            let mut seg = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
            while seg > 0.0 {
                if out.len() > MAX_PIECES {
                    return out;
                }
                let step = left.min(seg);
                let t = step / seg;
                let p = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                if on {
                    current.push(p);
                }
                seg -= step;
                left -= step;
                a = p;
                if left <= 1e-12 {
                    if on {
                        out.push(Polyline { points: std::mem::take(&mut current), closed: false });
                    } else {
                        current.clear();
                        current.push(a);
                    }
                    on = !on;
                    idx = (idx + 1) % pattern.len();
                    left = pattern[idx];
                    if left <= 1e-12 {
                        // A zero-length dash still caps.
                        if on {
                            out.push(Polyline { points: vec![a], closed: false });
                        }
                        on = !on;
                        idx = (idx + 1) % pattern.len();
                        left = pattern[idx];
                        if !on {
                            current.clear();
                        } else {
                            current = vec![a];
                        }
                    }
                }
            }
        }
        if on && current.len() > 1 {
            out.push(Polyline { points: current, closed: false });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{FillRule, IRect, coverage_lines};

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
        let mut p = Path::new();
        p.move_to(x0, y0);
        p.line_to(x1, y1);
        p
    }

    fn area(lines: &[(Point, Point)]) -> f64 {
        let c = coverage_lines(lines, FillRule::NonZero, IRect { x0: 0, y0: 0, x1: 200, y1: 200 }).unwrap();
        c.data.iter().map(|&v| f64::from(v) / 255.0).sum()
    }

    #[test]
    fn a_horizontal_line_has_width_times_length() {
        let st = StrokeStyle { width: 4.0, ..StrokeStyle::default() };
        let a = area(&stroke(&line(10.0, 50.0, 110.0, 50.0), &st, &Matrix::IDENTITY));
        assert!((a - 400.0).abs() < 2.0, "{a}");
        let sq = StrokeStyle { cap: Cap::Square, ..st.clone() };
        let a = area(&stroke(&line(10.0, 50.0, 110.0, 50.0), &sq, &Matrix::IDENTITY));
        assert!((a - 416.0).abs() < 2.0, "{a}");
        // Scaled by the CTM.
        let a = area(&stroke(&line(5.0, 25.0, 55.0, 25.0), &st, &Matrix::scale(2.0, 2.0)));
        assert!((a - 800.0).abs() < 4.0, "{a}");
    }

    #[test]
    fn closed_squares_and_joins() {
        let mut p = Path::new();
        p.move_to(20.0, 20.0);
        p.line_to(120.0, 20.0);
        p.line_to(120.0, 120.0);
        p.line_to(20.0, 120.0);
        p.close();
        for join in [Join::Miter, Join::Round, Join::Bevel] {
            let st = StrokeStyle { width: 10.0, join, ..StrokeStyle::default() };
            let a = area(&stroke(&p, &st, &Matrix::IDENTITY));
            // A frame 110 outside, 90 inside: 12100 − 8100 = 4000 with
            // miters; round and bevel corners cut a little.
            let expect = 4000.0;
            assert!(a <= expect + 5.0 && a > expect - 60.0, "{join:?} {a}");
        }
    }

    #[test]
    fn dashes_split_the_line() {
        let st = StrokeStyle { width: 2.0, dash: vec![10.0, 10.0], ..StrokeStyle::default() };
        let a = area(&stroke(&line(0.0, 50.0, 100.0, 50.0), &st, &Matrix::IDENTITY));
        assert!((a - 100.0).abs() < 2.0, "{a}");
        // A zero-length subpath with round caps is a dot.
        let mut p = Path::new();
        p.move_to(50.0, 50.0);
        p.line_to(50.0, 50.0);
        let st = StrokeStyle { width: 10.0, cap: Cap::Round, ..StrokeStyle::default() };
        let a = area(&stroke(&p, &st, &Matrix::IDENTITY));
        assert!((a - PI * 25.0).abs() < 3.0, "{a}");
        // Hostile dash arrays end.
        let st = StrokeStyle { dash: vec![1e-9, 1e-9], ..StrokeStyle::default() };
        let _ = stroke(&line(0.0, 0.0, 1e6, 0.0), &st, &Matrix::IDENTITY);
    }

    #[test]
    fn hairlines_are_one_pixel() {
        let st = StrokeStyle { width: 0.0, ..StrokeStyle::default() };
        let a = area(&stroke(&line(10.0, 50.0, 110.0, 50.0), &st, &Matrix::IDENTITY));
        assert!((a - 100.0).abs() < 2.0, "{a}");
    }
}
