//! The PDF tokenizer and object parser (§7.2, §7.3). One lexer serves
//! the file body, object streams and content streams; it never panics
//! and never reads past its buffer, and nesting is bounded.

use std::sync::Arc;

use crate::object::{Dict, Name, ObjRef, Object};

/// Arrays and dictionaries nested deeper than this are refused.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    Int(i64),
    Real(f64),
    String(Vec<u8>),
    Name(Name),
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
    /// A bare word: `obj`, `R`, `true`, an operator in a content stream.
    Keyword(&'a [u8]),
}

pub fn is_white(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delim(b)
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Lexer<'a> {
        Lexer { data, pos: 0 }
    }

    pub fn at(data: &'a [u8], pos: usize) -> Lexer<'a> {
        Lexer { data, pos: pos.min(data.len()) }
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Skips whitespace and comments.
    pub fn skip_ws(&mut self) {
        while let Some(&b) = self.data.get(self.pos) {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(&c) = self.data.get(self.pos) {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Token<'a>> {
        self.skip_ws();
        let b = *self.data.get(self.pos)?;
        match b {
            b'[' => {
                self.pos += 1;
                Some(Token::ArrayStart)
            }
            b']' => {
                self.pos += 1;
                Some(Token::ArrayEnd)
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(Token::DictStart)
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Some(Token::DictEnd)
            }
            b'<' => Some(Token::String(self.hex_string())),
            b'(' => Some(Token::String(self.literal_string())),
            b'/' => Some(Token::Name(self.name())),
            // Stray closers and braces (PostScript calculator functions
            // parse braces themselves) are words of their own.
            b')' | b'>' | b'{' | b'}' => {
                self.pos += 1;
                Some(Token::Keyword(&self.data[self.pos - 1..self.pos]))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => Some(self.number()),
            _ => {
                let start = self.pos;
                while self.data.get(self.pos).is_some_and(|&c| is_regular(c)) {
                    self.pos += 1;
                }
                Some(Token::Keyword(&self.data[start..self.pos]))
            }
        }
    }

    fn number(&mut self) -> Token<'a> {
        let start = self.pos;
        while self.data.get(self.pos).is_some_and(|&c| is_regular(c)) {
            self.pos += 1;
        }
        let word = &self.data[start..self.pos];
        parse_number(word).unwrap_or(Token::Keyword(word))
    }

    fn name(&mut self) -> Name {
        self.pos += 1; // the slash
        let mut out = Vec::new();
        while let Some(&c) = self.data.get(self.pos) {
            if !is_regular(c) {
                break;
            }
            if c == b'#'
                && let (Some(h), Some(l)) = (
                    self.data.get(self.pos + 1).copied().and_then(hex),
                    self.data.get(self.pos + 2).copied().and_then(hex),
                )
            {
                out.push(h << 4 | l);
                self.pos += 3;
                continue;
            }
            out.push(c);
            self.pos += 1;
        }
        Name(Arc::from(String::from_utf8_lossy(&out).as_ref()))
    }

    fn hex_string(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while let Some(&c) = self.data.get(self.pos) {
            self.pos += 1;
            if c == b'>' {
                break;
            }
            let Some(v) = hex(c) else { continue };
            match hi.take() {
                Some(h) => out.push(h << 4 | v),
                None => hi = Some(v),
            }
        }
        // An odd final digit is followed by an implicit 0.
        if let Some(h) = hi {
            out.push(h << 4);
        }
        out
    }

    fn literal_string(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(&c) = self.data.get(self.pos) {
            self.pos += 1;
            match c {
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    // An end of line inside a string reads as one \n.
                    if self.data.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// One object, with `n g R` references recognised. `None` at the end
    /// of the data or on a token that cannot start an object (the token
    /// is consumed).
    pub fn object(&mut self) -> Option<Object> {
        self.object_depth(0)
    }

    fn object_depth(&mut self, depth: usize) -> Option<Object> {
        let tok = self.next_token()?;
        self.object_from(tok, depth)
    }

    /// The object that starts with `tok`.
    pub fn object_from(&mut self, tok: Token<'a>, depth: usize) -> Option<Object> {
        if depth > MAX_DEPTH {
            return None;
        }
        match tok {
            Token::Int(n) => {
                // `n g R`: look ahead two tokens, rewind when it is not one.
                let save = self.pos;
                if n >= 0
                    && n <= i64::from(u32::MAX)
                    && let Some(Token::Int(g)) = self.next_token()
                    && (0..=65535).contains(&g)
                    && let Some(Token::Keyword(b"R")) = self.next_token()
                {
                    return Some(Object::Ref(ObjRef { num: n as u32, generation: g as u16 }));
                }
                self.pos = save;
                Some(Object::Int(n))
            }
            Token::Real(r) => Some(Object::Real(r)),
            Token::String(s) => Some(Object::String(Arc::from(s))),
            Token::Name(n) => Some(Object::Name(n)),
            Token::ArrayStart => {
                let mut v = Vec::new();
                while let Some(t) = self.next_token() {
                    if t == Token::ArrayEnd {
                        break;
                    }
                    // A dictionary end inside an array closes nothing: skip it.
                    if t == Token::DictEnd {
                        continue;
                    }
                    if let Some(o) = self.object_from(t, depth + 1) {
                        v.push(o);
                    }
                }
                Some(Object::array(v))
            }
            Token::DictStart => Some(Object::dict(self.dict_body(depth))),
            Token::Keyword(b"true") => Some(Object::Bool(true)),
            Token::Keyword(b"false") => Some(Object::Bool(false)),
            Token::Keyword(b"null") => Some(Object::Null),
            _ => None,
        }
    }

    /// The entries after `<<` up to `>>`.
    pub fn dict_body(&mut self, depth: usize) -> Dict {
        let mut d = Dict::new();
        loop {
            let save = self.pos;
            let Some(t) = self.next_token() else { break };
            match t {
                Token::DictEnd => break,
                Token::Name(k) => {
                    let vsave = self.pos;
                    match self.next_token() {
                        // `/Key /Next` loses no key when a value is missing.
                        Some(Token::DictEnd) => {
                            self.pos = vsave;
                            continue;
                        }
                        Some(vt) => {
                            if let Some(v) = self.object_from(vt, depth + 1) {
                                d.entries.push((k, v));
                            }
                        }
                        None => break,
                    }
                }
                // A keyword such as `stream` or `endobj` means the closing
                // `>>` is missing: stop before it.
                Token::Keyword(w) if w == b"stream" || w == b"endobj" || w == b"obj" => {
                    self.pos = save;
                    break;
                }
                _ => {}
            }
        }
        d
    }
}

/// An integer or real token, PDF style (`.5`, `-3.`, `+1`), with the
/// malformed forms some writers emit (`--5`, `1.2.3`) read leniently.
pub fn parse_number(word: &[u8]) -> Option<Token<'static>> {
    let mut neg = false;
    let mut i = 0;
    while i < word.len() && (word[i] == b'-' || word[i] == b'+') {
        if word[i] == b'-' {
            neg = !neg;
        }
        i += 1;
    }
    let digits = &word[i..];
    if digits.is_empty() || !digits.iter().all(|&c| c.is_ascii_digit() || c == b'.') {
        return None;
    }
    if !digits.contains(&b'.') {
        let mut v: i64 = 0;
        for &c in digits {
            v = v.checked_mul(10)?.checked_add(i64::from(c - b'0'))?;
            if v > 1 << 53 {
                // Out of integer range: keep it as a real.
                let r = std::str::from_utf8(digits).ok()?.parse::<f64>().ok()?;
                return Some(Token::Real(if neg { -r } else { r }));
            }
        }
        return Some(Token::Int(if neg { -v } else { v }));
    }
    // Only the first dot counts.
    let mut s = String::with_capacity(digits.len() + 1);
    let mut dot = false;
    for &c in digits {
        if c == b'.' {
            if dot {
                break;
            }
            dot = true;
        }
        s.push(c as char);
    }
    if s == "." {
        return Some(Token::Real(0.0));
    }
    let r = s.parse::<f64>().ok()?;
    Some(Token::Real(if neg { -r } else { r }))
}

/// Finds `needle` at or after `from`.
pub fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Finds the last `needle` that starts before `before`.
pub fn rfind(hay: &[u8], needle: &[u8], before: usize) -> Option<usize> {
    let end = before.min(hay.len());
    if needle.len() > end {
        return None;
    }
    hay[..end].windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Object {
        Lexer::new(s.as_bytes()).object().unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(parse("42"), Object::Int(42));
        assert_eq!(parse("-3.5"), Object::Real(-3.5));
        assert_eq!(parse(".5"), Object::Real(0.5));
        assert_eq!(parse("--4"), Object::Int(4));
        assert_eq!(parse("1.2.3"), Object::Real(1.2));
        assert_eq!(parse("true"), Object::Bool(true));
        assert_eq!(parse("null"), Object::Null);
        assert_eq!(parse("/A#20B"), Object::name("A B"));
        assert_eq!(parse("12 0 R"), Object::Ref(ObjRef { num: 12, generation: 0 }));
    }

    #[test]
    fn strings() {
        assert_eq!(parse("(a\\(b\\)c (nested) \\101\\n)").as_bytes().unwrap(), b"a(b)c (nested) A\n");
        assert_eq!(parse("(line\\\ncont)").as_bytes().unwrap(), b"linecont");
        assert_eq!(parse("<48 65 6c6C 6>").as_bytes().unwrap(), b"Hell`");
        assert_eq!(parse("(\\777)").as_bytes().unwrap(), &[0xff]);
    }

    #[test]
    fn composites() {
        let o = parse("<< /Type /Page /Kids [1 0 R 2 0 R] /N 3 /Sub << /X [1 2.5 (s)] >> >>");
        let d = o.as_dict().unwrap();
        assert_eq!(d.name("Type"), Some("Page"));
        assert_eq!(d.get("Kids").unwrap().as_array().unwrap().len(), 2);
        assert_eq!(d.get("Sub").unwrap().as_dict().unwrap().get("X").unwrap().as_array().unwrap().len(), 3);
        // A missing value and a missing `>>` still give the other keys.
        let o = parse("<< /A 1 /C >>");
        assert_eq!(o.as_dict().unwrap().get("A"), Some(&Object::Int(1)));
        assert_eq!(o.as_dict().unwrap().len(), 1);
        let mut l = Lexer::new(b"<< /Length 5 stream");
        let d = l.object().unwrap();
        assert_eq!(d.as_dict().unwrap().get("Length"), Some(&Object::Int(5)));
        assert_eq!(l.next_token(), Some(Token::Keyword(b"stream")));
    }

    #[test]
    fn hostile_input_never_panics() {
        let deep = "[".repeat(10_000);
        assert!(Lexer::new(deep.as_bytes()).object().is_some());
        for s in ["(", "<", "<<", "/", "\\", "(\\", "<<<<>>", "]", "99999999999999999999999", "1 0", "%"] {
            let mut l = Lexer::new(s.as_bytes());
            while l.next_token().is_some() {}
            let _ = Lexer::new(s.as_bytes()).object();
        }
    }
}
