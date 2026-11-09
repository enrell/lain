//! BitTorrent's bencoding (BEP 3). Byte strings stay bytes. Input is
//! untrusted: nesting depth and element counts are bounded, and every
//! failure is an error, never a panic.

use std::collections::BTreeMap;

use crate::Error;

/// Limits that keep hostile input from exhausting memory or the stack.
pub const MAX_DEPTH: usize = 64;
pub const MAX_ITEMS: usize = 1 << 20;

/// A decoded value. `Raw` is encode-only: pre-encoded bytes inserted
/// verbatim (Go's `RawMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(BTreeMap<Vec<u8>, Value>),
    Raw(Vec<u8>),
}

impl Value {
    pub fn str(s: impl AsRef<[u8]>) -> Value {
        Value::Bytes(s.as_ref().to_vec())
    }
    /// A dictionary from `(key, value)` pairs.
    pub fn dict<'a>(pairs: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
        Value::Dict(pairs.into_iter().map(|(k, v)| (k.as_bytes().to_vec(), v)).collect())
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    /// The byte string as UTF-8 text.
    pub fn as_str(&self) -> Option<&str> {
        self.as_bytes().and_then(|b| std::str::from_utf8(b).ok())
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_dict(&self) -> Option<&BTreeMap<Vec<u8>, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    /// A dictionary member.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_dict().and_then(|d| d.get(key.as_bytes()))
    }
}

fn syntax(at: usize, msg: impl std::fmt::Display) -> Error {
    Error::Syntax(format!("at {at}: {msg}"))
}

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
    items: usize,
    raw: Option<BTreeMap<Vec<u8>, Vec<u8>>>,
}

/// Parses exactly one value; trailing bytes are an error.
pub fn decode(data: &[u8]) -> Result<Value, Error> {
    let (v, rest) = decode_prefix(data)?;
    if !rest.is_empty() {
        return Err(syntax(data.len() - rest.len(), "trailing data"));
    }
    Ok(v)
}

/// Parses one value and returns the bytes after it (ut_metadata sends a
/// dict followed by raw piece bytes).
pub fn decode_prefix(data: &[u8]) -> Result<(Value, &[u8]), Error> {
    let mut d = Decoder { data, pos: 0, items: 0, raw: None };
    let v = d.value(0)?;
    Ok((v, &data[d.pos..]))
}

/// Parses a top-level dictionary and also returns the exact encoded
/// bytes of each of its values (the info hash is over the original
/// encoding, not a re-encoding).
#[allow(clippy::type_complexity)]
pub fn decode_dict(data: &[u8]) -> Result<(BTreeMap<Vec<u8>, Value>, BTreeMap<Vec<u8>, Vec<u8>>), Error> {
    let mut d = Decoder { data, pos: 0, items: 0, raw: Some(BTreeMap::new()) };
    let v = d.value(0)?;
    if d.pos != data.len() {
        return Err(syntax(d.pos, "trailing data"));
    }
    match v {
        Value::Dict(m) => Ok((m, d.raw.unwrap_or_default())),
        _ => Err(syntax(0, "not a dictionary")),
    }
}

impl Decoder<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(syntax(self.pos, "nesting too deep"));
        }
        self.items += 1;
        if self.items > MAX_ITEMS {
            return Err(syntax(self.pos, "too many items"));
        }
        let Some(&c) = self.data.get(self.pos) else { return Err(syntax(self.pos, "unexpected end")) };
        match c {
            b'i' => self.integer().map(Value::Int),
            b'l' => {
                self.pos += 1;
                let mut list = Vec::new();
                loop {
                    match self.data.get(self.pos) {
                        None => return Err(syntax(self.pos, "unterminated list")),
                        Some(b'e') => {
                            self.pos += 1;
                            return Ok(Value::List(list));
                        }
                        Some(_) => list.push(self.value(depth + 1)?),
                    }
                }
            }
            b'd' => {
                self.pos += 1;
                let mut m = BTreeMap::new();
                loop {
                    match self.data.get(self.pos) {
                        None => return Err(syntax(self.pos, "unterminated dictionary")),
                        Some(b'e') => {
                            self.pos += 1;
                            return Ok(Value::Dict(m));
                        }
                        Some(c) if !c.is_ascii_digit() => {
                            return Err(syntax(self.pos, "dictionary key must be a string"));
                        }
                        Some(_) => {}
                    }
                    let k = self.string()?;
                    let start = self.pos;
                    let v = self.value(depth + 1)?;
                    if depth == 0
                        && let Some(raw) = self.raw.as_mut()
                    {
                        raw.insert(k.clone(), self.data[start..self.pos].to_vec());
                    }
                    m.insert(k, v);
                }
            }
            b'0'..=b'9' => self.string().map(Value::Bytes),
            c => Err(syntax(self.pos, format!("unexpected byte {:?}", c as char))),
        }
    }

    fn integer(&mut self) -> Result<i64, Error> {
        let start = self.pos;
        let Some(end) = self.data[self.pos..].iter().position(|&b| b == b'e') else {
            return Err(syntax(start, "unterminated integer"));
        };
        let s = &self.data[self.pos + 1..self.pos + end];
        let text = String::from_utf8_lossy(s);
        let bad = s.is_empty()
            || s == b"-"
            || s == b"-0"
            || (s.len() > 1 && s[0] == b'0')
            || (s.len() > 2 && s[0] == b'-' && s[1] == b'0');
        if bad {
            return Err(syntax(start, format!("non-canonical integer {text:?}")));
        }
        let valid = s.iter().enumerate().all(|(i, &b)| b.is_ascii_digit() || (i == 0 && b == b'-'));
        let n = if valid { text.parse::<i64>().ok() } else { None };
        let Some(n) = n else { return Err(syntax(start, format!("bad integer {text:?}"))) };
        self.pos += end + 1;
        Ok(n)
    }

    fn string(&mut self) -> Result<Vec<u8>, Error> {
        let start = self.pos;
        let colon = self.data[self.pos..].iter().position(|&b| b == b':');
        let Some(colon) = colon.filter(|&c| c > 0 && c <= 10) else {
            return Err(syntax(start, "bad string length"));
        };
        let digits = &self.data[self.pos..self.pos + colon];
        if !digits.iter().all(u8::is_ascii_digit) || (colon > 1 && digits[0] == b'0') {
            return Err(syntax(start, "bad string length"));
        }
        let n: usize = std::str::from_utf8(digits).ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
        self.pos += colon + 1;
        if n > self.data.len() - self.pos {
            return Err(syntax(start, "string longer than input"));
        }
        let s = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(s)
    }
}

/// Writes v canonically (dictionary keys sorted).
pub fn encode(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(&mut out, v);
    out
}

fn encode_into(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Int(n) => out.extend_from_slice(format!("i{n}e").as_bytes()),
        Value::Bytes(b) => {
            out.extend_from_slice(format!("{}:", b.len()).as_bytes());
            out.extend_from_slice(b);
        }
        Value::Raw(b) => out.extend_from_slice(b),
        Value::List(l) => {
            out.push(b'l');
            for e in l {
                encode_into(out, e);
            }
            out.push(b'e');
        }
        Value::Dict(m) => {
            out.push(b'd');
            for (k, v) in m {
                out.extend_from_slice(format!("{}:", k.len()).as_bytes());
                out.extend_from_slice(k);
                encode_into(out, v);
            }
            out.push(b'e');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let v = Value::dict([
            ("announce", Value::str("http://tracker-exemplo.invalid/announce")),
            (
                "info",
                Value::dict([
                    ("name", Value::str(b"a\x00b")),
                    ("piece length", Value::Int(16384)),
                    (
                        "files",
                        Value::List(vec![Value::dict([
                            ("length", Value::Int(3)),
                            ("path", Value::List(vec![Value::str("x"), Value::str("y")])),
                        ])]),
                    ),
                ]),
            ),
            ("neg", Value::Int(-42)),
        ]);
        let raw = encode(&v);
        let got = decode(&raw).unwrap();
        assert_eq!(got, v);
        assert_eq!(encode(&got), raw, "encoding is not canonical");
    }

    #[test]
    fn decode_dict_keeps_raw_values() {
        let (m, raw) = decode_dict(b"d4:infod4:name3:abc6:lengthi5ee3:zzzi1ee").unwrap();
        assert_eq!(raw[&b"info"[..]], b"d4:name3:abc6:lengthi5ee");
        assert_eq!(m[&b"zzz"[..]].as_int(), Some(1));
    }

    #[test]
    fn decode_prefix_leaves_trailer() {
        let (_, rest) = decode_prefix(b"d8:msg_typei1e5:piecei0eeRAWBYTES").unwrap();
        assert_eq!(rest, b"RAWBYTES");
    }

    #[test]
    fn rejects_malformed() {
        let deep = format!("{}{}", "l".repeat(MAX_DEPTH + 5), "e".repeat(MAX_DEPTH + 5));
        for input in [
            "",
            "i",
            "ie",
            "i-e",
            "i-0e",
            "i03e",
            "i1",
            "l",
            "d",
            "di1ei2ee",
            "3:ab",
            "-1:a",
            "01:a",
            "x",
            "i1ei2e",
            "99999999999:a",
            "i99999999999999999999e",
            "i+1e",
            "i1-e",
            ":a",
            &deep,
        ] {
            assert!(matches!(decode(input.as_bytes()), Err(Error::Syntax(_))), "{input:?} accepted");
        }
    }

    #[test]
    fn garbage_never_panics_and_round_trips() {
        let seeds: [&[u8]; 5] = [b"d4:infod4:name3:abcee", b"li1ei-2e3:abce", b"i0e", b"0:", b"d1:ad1:bleee"];
        let mut rng = crate::sync::Rng::new(7);
        for i in 0..20_000 {
            let mut data = seeds[i % seeds.len()].to_vec();
            rng.mutate(&mut data);
            if let Ok(v) = decode(&data) {
                let raw = encode(&v);
                assert_eq!(decode(&raw).as_ref(), Ok(&v), "re-decode mismatch for {data:?}");
            }
        }
    }
}
