//! Points, affine matrices and paths: the shapes every other module
//! speaks in. Coordinates are `f64`; a matrix is PDF's `[a b c d e f]`,
//! mapping `(x, y)` to `(a·x + c·y + e, b·x + d·y + f)`.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Matrix {
    fn default() -> Matrix {
        Matrix::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub const fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
        Matrix { a, b, c, d, e, f }
    }

    pub fn from_array(m: [f64; 6]) -> Matrix {
        Matrix::new(m[0], m[1], m[2], m[3], m[4], m[5])
    }

    pub fn translate(x: f64, y: f64) -> Matrix {
        Matrix::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    pub fn scale(x: f64, y: f64) -> Matrix {
        Matrix::new(x, 0.0, 0.0, y, 0.0, 0.0)
    }

    /// `self` applied first, then `then` (PDF's `self × then`).
    pub fn then(&self, then: &Matrix) -> Matrix {
        let (m, n) = (self, then);
        Matrix {
            a: m.a * n.a + m.b * n.c,
            b: m.a * n.b + m.b * n.d,
            c: m.c * n.a + m.d * n.c,
            d: m.c * n.b + m.d * n.d,
            e: m.e * n.a + m.f * n.c + n.e,
            f: m.e * n.b + m.f * n.d + n.f,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point { x: self.a * p.x + self.c * p.y + self.e, y: self.b * p.x + self.d * p.y + self.f }
    }

    /// A vector: the matrix without its translation.
    pub fn apply_vector(&self, x: f64, y: f64) -> Point {
        Point { x: self.a * x + self.c * y, y: self.b * x + self.d * y }
    }

    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Matrix { a, b, c, d, e: -(self.e * a + self.f * c), f: -(self.e * b + self.f * d) })
    }

    /// The factor a length grows by, on average over directions.
    pub fn expansion(&self) -> f64 {
        self.determinant().abs().sqrt()
    }
}

/// One path segment; a contour starts with `Move`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(Point),
    Line(Point),
    /// Control point, end.
    Quad(Point, Point),
    /// Two control points, end.
    Cubic(Point, Point, Point),
    Close,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.segs.push(Seg::Move(Point::new(x, y)));
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        self.segs.push(Seg::Line(Point::new(x, y)));
    }

    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        self.segs.push(Seg::Quad(Point::new(cx, cy), Point::new(x, y)));
    }

    pub fn cubic_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        self.segs.push(Seg::Cubic(Point::new(c1x, c1y), Point::new(c2x, c2y), Point::new(x, y)));
    }

    pub fn close(&mut self) {
        self.segs.push(Seg::Close);
    }

    /// Appends `other` with every point mapped through `m`.
    pub fn extend_transformed(&mut self, other: &Path, m: &Matrix) {
        self.segs.reserve(other.segs.len());
        for s in &other.segs {
            self.segs.push(match *s {
                Seg::Move(p) => Seg::Move(m.apply(p)),
                Seg::Line(p) => Seg::Line(m.apply(p)),
                Seg::Quad(c, p) => Seg::Quad(m.apply(c), m.apply(p)),
                Seg::Cubic(c1, c2, p) => Seg::Cubic(m.apply(c1), m.apply(c2), m.apply(p)),
                Seg::Close => Seg::Close,
            });
        }
    }

    pub fn transformed(&self, m: &Matrix) -> Path {
        let mut out = Path { segs: Vec::with_capacity(self.segs.len()) };
        out.extend_transformed(self, m);
        out
    }

    /// The bounding box of every point, control points included:
    /// `(min_x, min_y, max_x, max_y)`, `None` when the path has no point.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        let mut add = |p: Point| {
            let r = b.get_or_insert((p.x, p.y, p.x, p.y));
            r.0 = r.0.min(p.x);
            r.1 = r.1.min(p.y);
            r.2 = r.2.max(p.x);
            r.3 = r.3.max(p.y);
        };
        for s in &self.segs {
            match *s {
                Seg::Move(p) | Seg::Line(p) => add(p),
                Seg::Quad(c, p) => {
                    add(c);
                    add(p);
                }
                Seg::Cubic(c1, c2, p) => {
                    add(c1);
                    add(c2);
                    add(p);
                }
                Seg::Close => {}
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_in_pdf_order() {
        let m = Matrix::scale(2.0, 3.0).then(&Matrix::translate(10.0, 20.0));
        assert_eq!(m.apply(Point::new(1.0, 1.0)), Point::new(12.0, 23.0));
        let inv = m.invert().unwrap();
        let p = inv.apply(Point::new(12.0, 23.0));
        assert!((p.x - 1.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9);
        assert!(Matrix::scale(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn paths_transform_and_bound() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.cubic_to(1.0, 2.0, 3.0, -1.0, 4.0, 0.0);
        p.close();
        assert_eq!(p.bounds(), Some((0.0, -1.0, 4.0, 2.0)));
        let t = p.transformed(&Matrix::translate(1.0, 1.0));
        assert_eq!(t.bounds(), Some((1.0, 0.0, 5.0, 3.0)));
        assert_eq!(Path::new().bounds(), None);
    }
}
