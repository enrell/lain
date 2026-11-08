//! The component wire protocol, version 1. Newline-delimited JSON
//! frames over a Unix socket; the host listens and the component dials.
//!
//! ```text
//! component -> host: {"op":"hello","provider":"<id>","protocol":1,"capabilities":[...]}
//! host -> component: {"seq":N,"cap":"lain.x@1","op":"get","input":{...}}
//!                    {"seq":N,"op":"health","input":null}
//! component -> host: {"seq":N,"output":{...}}
//!                    {"seq":N,"error":{"code":"not-found","msg":"..."}}
//! ```
//!
//! `op` selects the input shape inside a multi-input capability and may
//! be omitted for single-input ones. Error codes are the stable
//! `lain_core::codes`; a timeout is `outcome-unknown` because the call
//! may or may not have run.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only wire version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The bound on one frame. Payloads are small contract shapes; more is
/// a peer bug or an attack.
pub const MAX_FRAME: usize = 8 << 20;

/// The transport-level liveness op (never a capability).
pub const HEALTH_OP: &str = "health";

/// The component's first frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub op: String,
    pub provider: String,
    pub protocol: u32,
    pub capabilities: Vec<String>,
}

/// One invocation or health request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub seq: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cap: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub op: String,
    #[serde(default)]
    pub input: Value,
}

/// A typed wire error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub msg: String,
}

/// The answer to one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

/// Writes one frame. JSON never contains a literal newline, so `\n`
/// is a safe delimiter.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, v: &T) -> io::Result<()> {
    let mut raw = serde_json::to_vec(v).map_err(io::Error::other)?;
    raw.push(b'\n');
    w.write_all(&raw)?;
    w.flush()
}

/// Reads one bounded frame (without the newline). EOF before any byte
/// is `UnexpectedEof`.
pub fn read_frame<R: BufRead>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, if buf.is_empty() { "eof" } else { "eof inside a frame" }));
        }
        if let Some(pos) = chunk.iter().position(|b| *b == b'\n') {
            buf.extend_from_slice(&chunk[..pos]);
            r.consume(pos + 1);
            break;
        }
        let n = chunk.len();
        buf.extend_from_slice(chunk);
        r.consume(n);
        if buf.len() > MAX_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame exceeds {MAX_FRAME} bytes")));
        }
    }
    if buf.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame exceeds {MAX_FRAME} bytes")));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_match_the_go_encoding() {
        let mut out = Vec::new();
        write_frame(&mut out, &Request { seq: 1, cap: String::new(), op: HEALTH_OP.into(), input: Value::Null }).unwrap();
        assert_eq!(out, b"{\"seq\":1,\"op\":\"health\",\"input\":null}\n");
        let r: Response = serde_json::from_slice(br#"{"seq":2,"error":{"code":"not-found","msg":"x"}}"#).unwrap();
        assert_eq!(r.error.unwrap().code, "not-found");
    }

    #[test]
    fn read_frame_is_bounded_and_splits_lines() {
        let mut r = io::BufReader::new(&b"{\"a\":1}\n{\"b\":2}\n"[..]);
        assert_eq!(read_frame(&mut r).unwrap(), b"{\"a\":1}");
        assert_eq!(read_frame(&mut r).unwrap(), b"{\"b\":2}");
        assert_eq!(read_frame(&mut r).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        let big = vec![b'x'; MAX_FRAME + 10];
        let mut r = io::BufReader::new(&big[..]);
        assert_eq!(read_frame(&mut r).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
