//! PDF functions (§7.10): sampled (0), exponential (2), stitching (3)
//! and PostScript calculator (4) functions, evaluated with every input
//! and output clipped to its domain and range.

use crate::document::Document;
use crate::object::{Dict, Object};

const MAX_SAMPLES: usize = 1 << 24;
const MAX_PS_OPS: usize = 4096;

#[derive(Debug, Clone)]
pub enum Function {
    Sampled {
        domain: Vec<f64>,
        range: Vec<f64>,
        size: Vec<usize>,
        bps: u32,
        encode: Vec<f64>,
        decode: Vec<f64>,
        samples: Vec<f64>,
    },
    Exponential {
        domain: Vec<f64>,
        range: Vec<f64>,
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    Stitching {
        domain: Vec<f64>,
        range: Vec<f64>,
        functions: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    PostScript {
        domain: Vec<f64>,
        range: Vec<f64>,
        code: Vec<Ps>,
    },
    /// Several 1-in-1-out functions forming an n-out one (shadings).
    Array(Vec<Function>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ps {
    Num(f64),
    Op(&'static str),
    /// `{ then } { else }` blocks as instruction ranges: `If(len)`,
    /// `IfElse(then_len, else_len)` skip over their bodies.
    If(usize),
    IfElse(usize, usize),
}

impl Function {
    /// Parses a function object (a dictionary or a stream).
    pub fn parse(doc: &Document, o: &Object) -> Option<Function> {
        Function::parse_depth(doc, &doc.resolve(o), 0)
    }

    fn parse_depth(doc: &Document, o: &Object, depth: usize) -> Option<Function> {
        if depth > 8 {
            return None;
        }
        if let Object::Array(a) = o {
            let fs: Vec<Function> =
                a.iter().filter_map(|f| Function::parse_depth(doc, &doc.resolve(f), depth + 1)).collect();
            return (!fs.is_empty()).then_some(Function::Array(fs));
        }
        let d: &Dict = o.as_dict()?;
        let domain = doc.numbers_in(d, "Domain");
        let range = doc.numbers_in(d, "Range");
        match doc.get_in(d, "FunctionType").as_i64()? {
            0 => {
                let s = o.as_stream()?;
                let data = doc.decode_stream(s).ok()?.data;
                let size: Vec<usize> =
                    doc.numbers_in(d, "Size").iter().map(|v| v.clamp(1.0, 65536.0) as usize).collect();
                let bps = doc.get_in(d, "BitsPerSample").as_i64().unwrap_or(8) as u32;
                if !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) || size.is_empty() || range.len() < 2 {
                    return None;
                }
                let outs = range.len() / 2;
                let count = size.iter().try_fold(outs, |acc, &s| acc.checked_mul(s))?;
                if count > MAX_SAMPLES {
                    return None;
                }
                let mut encode = doc.numbers_in(d, "Encode");
                if encode.len() < size.len() * 2 {
                    encode = size.iter().flat_map(|&s| [0.0, (s - 1) as f64]).collect();
                }
                let mut decode = doc.numbers_in(d, "Decode");
                if decode.len() < range.len() {
                    decode = range.clone();
                }
                let max = ((1u64 << bps) - 1) as f64;
                let mut samples = Vec::with_capacity(count);
                let mut bits = BitReader { data: &data, pos: 0 };
                for _ in 0..count {
                    samples.push(bits.read(bps).map_or(0.0, |v| v as f64 / max));
                }
                Some(Function::Sampled { domain, range, size, bps, encode, decode, samples })
            }
            2 => {
                let c0 = non_empty(doc.numbers_in(d, "C0"), vec![0.0]);
                let c1 = non_empty(doc.numbers_in(d, "C1"), vec![1.0]);
                let n = doc.f64_in(d, "N").unwrap_or(1.0);
                Some(Function::Exponential { domain, range, c0, c1, n })
            }
            3 => {
                let functions: Vec<Function> = doc
                    .array_in(d, "Functions")
                    .iter()
                    .filter_map(|f| Function::parse_depth(doc, f, depth + 1))
                    .collect();
                if functions.is_empty() {
                    return None;
                }
                let bounds = doc.numbers_in(d, "Bounds");
                let mut encode = doc.numbers_in(d, "Encode");
                if encode.len() < functions.len() * 2 {
                    encode = functions.iter().flat_map(|_| [0.0, 1.0]).collect();
                }
                Some(Function::Stitching { domain, range, functions, bounds, encode })
            }
            4 => {
                let s = o.as_stream()?;
                let data = doc.decode_stream(s).ok()?.data;
                let code = parse_ps(&data)?;
                Some(Function::PostScript { domain, range, code })
            }
            _ => None,
        }
    }

    /// Output count, when known.
    pub fn outputs(&self) -> usize {
        match self {
            Function::Sampled { range, .. } | Function::PostScript { range, .. } => range.len() / 2,
            Function::Exponential { c0, .. } => c0.len(),
            Function::Stitching { functions, range, .. } => {
                if range.len() >= 2 {
                    range.len() / 2
                } else {
                    functions.first().map_or(1, Function::outputs)
                }
            }
            Function::Array(fs) => fs.len(),
        }
    }

    pub fn eval(&self, input: &[f64]) -> Vec<f64> {
        let mut out = Vec::with_capacity(4);
        self.eval_into(input, &mut out, 0);
        out
    }

    fn eval_into(&self, input: &[f64], out: &mut Vec<f64>, depth: usize) {
        if depth > 8 {
            return;
        }
        match self {
            Function::Array(fs) => {
                for f in fs {
                    let mut o = Vec::new();
                    f.eval_into(input, &mut o, depth + 1);
                    out.push(o.first().copied().unwrap_or(0.0));
                }
            }
            Function::Exponential { domain, range, c0, c1, n } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain, 0);
                let p = if *n == 1.0 { x } else { x.max(0.0).powf(*n) };
                for i in 0..c0.len().max(c1.len()) {
                    let a = c0.get(i).copied().unwrap_or(0.0);
                    let b = c1.get(i).copied().unwrap_or(1.0);
                    out.push(clip(a + p * (b - a), range, i));
                }
            }
            Function::Stitching { domain, range, functions, bounds, encode } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain, 0);
                let k = bounds.iter().take_while(|&&b| x >= b).count().min(functions.len() - 1);
                let lo = if k == 0 { domain.first().copied().unwrap_or(0.0) } else { bounds[k - 1] };
                let hi = bounds.get(k).copied().unwrap_or_else(|| domain.get(1).copied().unwrap_or(1.0));
                let (e0, e1) = (encode[2 * k], encode[2 * k + 1]);
                let t = if hi > lo { e0 + (x - lo) * (e1 - e0) / (hi - lo) } else { e0 };
                let start = out.len();
                functions[k].eval_into(&[t], out, depth + 1);
                for (i, v) in out[start..].iter_mut().enumerate() {
                    *v = clip(*v, range, i);
                }
            }
            Function::Sampled { domain, range, size, encode, decode, samples, .. } => {
                let m = size.len();
                let outs = range.len() / 2;
                // Multilinear interpolation over the 2^m corner samples
                // (m is 1 or 2 in practice; capped at 4).
                let mut e = vec![0.0; m];
                for (j, ej) in e.iter_mut().enumerate() {
                    let x = clip(input.get(j).copied().unwrap_or(0.0), domain, j);
                    let (d0, d1) =
                        (domain.get(2 * j).copied().unwrap_or(0.0), domain.get(2 * j + 1).copied().unwrap_or(1.0));
                    let v = if d1 > d0 {
                        encode[2 * j] + (x - d0) * (encode[2 * j + 1] - encode[2 * j]) / (d1 - d0)
                    } else {
                        encode[2 * j]
                    };
                    *ej = v.clamp(0.0, (size[j] - 1) as f64);
                }
                let corners = 1usize << m.min(4);
                let mut acc = vec![0.0; outs];
                for c in 0..corners {
                    let mut weight = 1.0;
                    let mut index = 0usize;
                    let mut stride = outs;
                    for j in 0..m {
                        let base = e[j].floor();
                        let frac = e[j] - base;
                        let hi = c >> j & 1 == 1;
                        let pos = (base as usize + usize::from(hi)).min(size[j] - 1);
                        weight *= if hi { frac } else { 1.0 - frac };
                        index += pos * stride;
                        stride *= size[j];
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += weight * samples.get(index + k).copied().unwrap_or(0.0);
                    }
                }
                for (k, a) in acc.iter().enumerate() {
                    let v = decode[2 * k] + a * (decode[2 * k + 1] - decode[2 * k]);
                    out.push(clip(v, range, k));
                }
            }
            Function::PostScript { domain, range, code } => {
                let mut stack: Vec<f64> = input.iter().enumerate().map(|(i, &v)| clip(v, domain, i)).collect();
                run_ps(code, &mut stack, 0);
                let outs = range.len() / 2;
                let start = stack.len().saturating_sub(outs);
                for (i, v) in stack[start..].iter().enumerate() {
                    out.push(clip(*v, range, i));
                }
                while out.len() < outs {
                    out.push(0.0);
                }
            }
        }
    }
}

fn non_empty(v: Vec<f64>, d: Vec<f64>) -> Vec<f64> {
    if v.is_empty() { d } else { v }
}

fn clip(v: f64, r: &[f64], i: usize) -> f64 {
    let v = if v.is_finite() { v } else { 0.0 };
    match (r.get(2 * i), r.get(2 * i + 1)) {
        (Some(&lo), Some(&hi)) if lo <= hi => v.clamp(lo, hi),
        _ => v,
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = v << 1 | u64::from(byte >> (7 - self.pos % 8) & 1);
            self.pos += 1;
        }
        Some(v)
    }
}

const PS_OPS: &[&str] = &[
    "abs", "add", "atan", "ceiling", "cos", "cvi", "cvr", "div", "exp", "floor", "idiv", "ln", "log", "mod", "mul",
    "neg", "round", "sin", "sqrt", "sub", "truncate", "and", "bitshift", "eq", "false", "ge", "gt", "le", "lt", "ne",
    "not", "or", "true", "xor", "copy", "dup", "exch", "index", "pop", "roll",
];

/// Parses `{ ... }` into a flat instruction list with skip lengths.
fn parse_ps(src: &[u8]) -> Option<Vec<Ps>> {
    let mut l = crate::lexer::Lexer::new(src);
    let mut out = Vec::new();
    // The outer braces.
    loop {
        match l.next_token()? {
            crate::lexer::Token::Keyword(b"{") => break,
            _ => continue,
        }
    }
    parse_block(&mut l, &mut out, 0)?;
    Some(out)
}

fn parse_block(l: &mut crate::lexer::Lexer, out: &mut Vec<Ps>, depth: usize) -> Option<()> {
    use crate::lexer::Token;
    if depth > 32 || out.len() > MAX_PS_OPS {
        return None;
    }
    // Blocks parsed but not yet claimed by `if`/`ifelse`.
    let mut pending: Vec<Vec<Ps>> = Vec::new();
    loop {
        let t = l.next_token()?;
        match t {
            Token::Keyword(b"}") => {
                for b in pending {
                    out.extend(b);
                }
                return Some(());
            }
            Token::Keyword(b"{") => {
                let mut b = Vec::new();
                parse_block(l, &mut b, depth + 1)?;
                pending.push(b);
            }
            Token::Int(i) => out.push(Ps::Num(i as f64)),
            Token::Real(r) => out.push(Ps::Num(r)),
            Token::Keyword(b"if") => {
                let body = pending.pop()?;
                out.push(Ps::If(body.len()));
                out.extend(body);
            }
            Token::Keyword(b"ifelse") => {
                let e = pending.pop()?;
                let th = pending.pop()?;
                out.push(Ps::IfElse(th.len(), e.len()));
                out.extend(th);
                out.extend(e);
            }
            Token::Keyword(w) => {
                let name = std::str::from_utf8(w).ok()?;
                let op = PS_OPS.iter().find(|o| **o == name)?;
                out.push(Ps::Op(op));
            }
            _ => return None,
        }
        if out.len() > MAX_PS_OPS {
            return None;
        }
    }
}

fn run_ps(code: &[Ps], st: &mut Vec<f64>, depth: usize) {
    if depth > 32 {
        return;
    }
    let mut i = 0;
    while i < code.len() {
        if st.len() > 100 {
            return;
        }
        match &code[i] {
            Ps::Num(v) => st.push(*v),
            Ps::If(n) => {
                let c = st.pop().unwrap_or(0.0);
                if c != 0.0 {
                    run_ps(&code[i + 1..(i + 1 + n).min(code.len())], st, depth + 1);
                }
                i += n;
            }
            Ps::IfElse(a, b) => {
                let c = st.pop().unwrap_or(0.0);
                let th = (i + 1).min(code.len())..(i + 1 + a).min(code.len());
                let el = th.end..(th.end + b).min(code.len());
                if c != 0.0 {
                    run_ps(&code[th], st, depth + 1);
                } else {
                    run_ps(&code[el], st, depth + 1);
                }
                i += a + b;
            }
            Ps::Op(op) => ps_op(op, st),
        }
        i += 1;
    }
}

fn ps_op(op: &str, st: &mut Vec<f64>) {
    let pop = |st: &mut Vec<f64>| st.pop().unwrap_or(0.0);
    let b2 = |v: bool| if v { 1.0 } else { 0.0 };
    match op {
        "abs" | "ceiling" | "cos" | "cvi" | "cvr" | "floor" | "ln" | "log" | "neg" | "round" | "sin" | "sqrt"
        | "truncate" | "not" => {
            let a = pop(st);
            st.push(match op {
                "abs" => a.abs(),
                "ceiling" => a.ceil(),
                "cos" => a.to_radians().cos(),
                "sin" => a.to_radians().sin(),
                "cvi" | "truncate" => a.trunc(),
                "cvr" => a,
                "floor" => a.floor(),
                "ln" => a.ln(),
                "log" => a.log10(),
                "neg" => -a,
                "round" => (a + 0.5).floor(),
                "sqrt" => a.max(0.0).sqrt(),
                // `not` on booleans (1/0) and integers alike.
                _ => {
                    if a == 0.0 || a == 1.0 {
                        1.0 - a
                    } else {
                        !(a as i64) as f64
                    }
                }
            });
        }
        "add" | "exp" | "sub" | "mul" | "div" | "idiv" | "mod" | "atan" | "and" | "or" | "xor" | "bitshift" | "eq"
        | "ne" | "ge" | "gt" | "le" | "lt" => {
            let b = pop(st);
            let a = pop(st);
            st.push(match op {
                "add" => a + b,
                "exp" => a.powf(b),
                "sub" => a - b,
                "mul" => a * b,
                "div" => {
                    if b == 0.0 {
                        0.0
                    } else {
                        a / b
                    }
                }
                "idiv" => {
                    if b as i64 == 0 {
                        0.0
                    } else {
                        ((a as i64) / (b as i64)) as f64
                    }
                }
                "mod" => {
                    if b as i64 == 0 {
                        0.0
                    } else {
                        ((a as i64) % (b as i64)) as f64
                    }
                }
                "atan" => {
                    let d = a.atan2(b).to_degrees();
                    if d < 0.0 { d + 360.0 } else { d }
                }
                "and" => ((a as i64) & (b as i64)) as f64,
                "or" => ((a as i64) | (b as i64)) as f64,
                "xor" => ((a as i64) ^ (b as i64)) as f64,
                "bitshift" => {
                    let s = (b as i64).clamp(-63, 63);
                    if s >= 0 { ((a as i64) << s) as f64 } else { ((a as i64) >> -s) as f64 }
                }
                "eq" => b2(a == b),
                "ne" => b2(a != b),
                "ge" => b2(a >= b),
                "gt" => b2(a > b),
                "le" => b2(a <= b),
                _ => b2(a < b),
            });
        }
        "true" => st.push(1.0),
        "false" => st.push(0.0),
        "dup" => {
            let a = st.last().copied().unwrap_or(0.0);
            st.push(a);
        }
        "pop" => {
            st.pop();
        }
        "exch" => {
            let n = st.len();
            if n >= 2 {
                st.swap(n - 1, n - 2);
            }
        }
        "copy" => {
            let n = (pop(st).max(0.0) as usize).min(st.len());
            let start = st.len() - n;
            let copy: Vec<f64> = st[start..].to_vec();
            st.extend(copy);
        }
        "index" => {
            let n = pop(st).max(0.0) as usize;
            let v = st.len().checked_sub(n + 1).and_then(|i| st.get(i).copied()).unwrap_or(0.0);
            st.push(v);
        }
        "roll" => {
            let j = pop(st) as i64;
            let n = (pop(st).max(0.0) as usize).min(st.len());
            if n > 0 {
                let start = st.len() - n;
                let s = &mut st[start..];
                let k = j.rem_euclid(n as i64) as usize;
                s.rotate_right(k);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ps(src: &str, input: &[f64], range: usize) -> Vec<f64> {
        let code = parse_ps(src.as_bytes()).unwrap();
        let f = Function::PostScript { domain: vec![0.0, 1.0, 0.0, 1.0], range: [0.0, 100.0].repeat(range), code };
        f.eval(input)
    }

    #[test]
    fn exponential_and_stitching() {
        let e = Function::Exponential {
            domain: vec![0.0, 1.0],
            range: vec![],
            c0: vec![0.0, 1.0],
            c1: vec![1.0, 0.0],
            n: 1.0,
        };
        assert_eq!(e.eval(&[0.25]), vec![0.25, 0.75]);
        let s = Function::Stitching {
            domain: vec![0.0, 1.0],
            range: vec![],
            functions: vec![e.clone(), e],
            bounds: vec![0.5],
            encode: vec![0.0, 1.0, 1.0, 0.0],
        };
        assert_eq!(s.eval(&[0.25]), vec![0.5, 0.5]);
        assert_eq!(s.eval(&[1.0]), vec![0.0, 1.0]);
    }

    #[test]
    fn sampled_interpolates() {
        let f = Function::Sampled {
            domain: vec![0.0, 1.0],
            range: vec![0.0, 1.0],
            size: vec![2],
            bps: 8,
            encode: vec![0.0, 1.0],
            decode: vec![0.0, 1.0],
            samples: vec![0.0, 1.0],
        };
        assert_eq!(f.eval(&[0.5]), vec![0.5]);
        assert_eq!(f.eval(&[7.0]), vec![1.0]);
    }

    #[test]
    fn postscript_calculator() {
        assert_eq!(ps("{ 2 mul exch 3 add }", &[0.5, 0.25], 2), vec![0.5, 3.5]);
        assert_eq!(ps("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.7], 1), vec![1.0]);
        assert_eq!(ps("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.2], 1), vec![0.0]);
        assert_eq!(ps("{ 1 2 3 3 1 roll }", &[0.0], 3), vec![3.0, 1.0, 2.0]);
        assert!(parse_ps(b"{ bogus }").is_none());
        assert!(parse_ps(b"{ { { ").is_none());
    }
}
