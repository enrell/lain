//! PDF objects (ISO 32000-1 §7.3). Composite values sit behind `Arc`,
//! so resolving and passing objects around is cheap and a parsed
//! document can be shared between threads.

use std::sync::Arc;

/// An indirect reference: object number and generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjRef {
    pub num: u32,
    pub generation: u16,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Object {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// Raw string bytes, escapes and hex already decoded (and the bytes
    /// decrypted once the object is loaded through the document).
    String(Arc<[u8]>),
    Name(Name),
    Array(Arc<Vec<Object>>),
    Dict(Arc<Dict>),
    Stream(Arc<Stream>),
    Ref(ObjRef),
}

/// A name without its slash, `#xx` escapes decoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(pub Arc<str>);

impl Name {
    pub fn new(s: &str) -> Name {
        Name(Arc::from(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A dictionary: few keys, so a vector searched linearly beats a map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict {
    pub entries: Vec<(Name, Object)>,
}

impl Dict {
    pub fn new() -> Dict {
        Dict::default()
    }

    pub fn get(&self, key: &str) -> Option<&Object> {
        self.entries.iter().find(|(k, _)| k.as_str() == key).map(|(_, v)| v)
    }

    /// The first key present of `keys` (abbreviated inline-image keys).
    pub fn get_any(&self, keys: &[&str]) -> Option<&Object> {
        keys.iter().find_map(|k| self.get(k))
    }

    pub fn insert(&mut self, key: &str, value: Object) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| k.as_str() == key) {
            slot.1 = value;
        } else {
            self.entries.push((Name::new(key), value));
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `/Type`'s or another key's name value, without resolving.
    pub fn name(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Object::Name(n)) => Some(n.as_str()),
            _ => None,
        }
    }
}

/// A stream: its dictionary and where its bytes are in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    /// Byte range of the encoded data in the document's buffer.
    pub start: usize,
    pub len: usize,
    /// The indirect object the stream is, for decryption keys.
    pub id: Option<ObjRef>,
}

impl Object {
    pub fn name(s: &str) -> Object {
        Object::Name(Name::new(s))
    }

    pub fn string(b: &[u8]) -> Object {
        Object::String(Arc::from(b))
    }

    pub fn array(v: Vec<Object>) -> Object {
        Object::Array(Arc::new(v))
    }

    pub fn dict(d: Dict) -> Object {
        Object::Dict(Arc::new(d))
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Object::Int(i) => Some(i),
            // A real where an integer belongs is truncated, as viewers do.
            Object::Real(r) if r.is_finite() && r.abs() < 9.0e15 => Some(r as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Object::Int(i) => Some(i as f64),
            Object::Real(r) if r.is_finite() => Some(r),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Object::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Object::Name(n) => Some(n.as_str()),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The dictionary of a dictionary or of a stream.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Arc<Stream>> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Option<ObjRef> {
        match *self {
            Object::Ref(r) => Some(r),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_lookup_and_replace() {
        let mut d = Dict::new();
        d.insert("Type", Object::name("Page"));
        d.insert("Count", Object::Int(3));
        d.insert("Count", Object::Int(4));
        assert_eq!(d.len(), 2);
        assert_eq!(d.name("Type"), Some("Page"));
        assert_eq!(d.get("Count").and_then(Object::as_i64), Some(4));
        assert_eq!(d.get_any(&["W", "Count"]).and_then(Object::as_i64), Some(4));
        assert!(Object::Real(2.9).as_i64() == Some(2) && Object::Real(f64::NAN).as_f64().is_none());
    }
}
