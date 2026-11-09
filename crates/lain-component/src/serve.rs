//! The component side, for providers written in Rust: dial the host
//! socket, announce with `hello`, answer frames until the host hangs up.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

use lain_core::{Error, Provider, codes};

use crate::protocol::{HEALTH_OP, Hello, PROTOCOL_VERSION, Request, Response, WireError, read_frame, write_frame};

/// Serves `p` on the host address (`{sock}`: a socket path, or a pipe
/// name on Windows) until EOF.
pub fn serve(address: &str, p: &dyn Provider) -> io::Result<()> {
    let (mut reader, mut writer) = crate::transport::connect(address)?;
    write_frame(
        &mut writer,
        &Hello {
            op: "hello".into(),
            provider: p.id().to_string(),
            protocol: PROTOCOL_VERSION,
            capabilities: p.capabilities(),
        },
    )?;
    loop {
        let raw = match read_frame(&mut reader) {
            Ok(raw) => raw,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = match serde_json::from_slice::<Request>(&raw) {
            Ok(req) => answer(p, req),
            Err(e) => {
                // Without a sequence id the stream cannot be trusted.
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad request frame: {e}")));
            }
        };
        write_frame(&mut writer, &resp)?;
    }
}

fn wire(e: Error) -> WireError {
    WireError { code: e.code, msg: e.msg }
}

/// Maps one request to its response; a panicking provider answers
/// `internal` and the connection survives.
pub(crate) fn answer(p: &dyn Provider, req: Request) -> Response {
    let seq = req.seq;
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Value, Error> {
        if req.cap.is_empty() && req.op == HEALTH_OP {
            return p.health().map(|_| json!({"ok": true}));
        }
        if req.cap.is_empty() {
            return Err(Error::invalid("cap required"));
        }
        if !p.capabilities().contains(&req.cap) {
            return Err(Error::invalid(format!("unknown capability {}", req.cap)));
        }
        p.invoke(&req.cap, &req.op, req.input)
    }));
    match result {
        Ok(Ok(output)) => Response { seq, output: Some(output), error: None },
        Ok(Err(e)) => Response { seq, output: None, error: Some(wire(e)) },
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".into());
            Response {
                seq,
                output: None,
                error: Some(WireError { code: codes::INTERNAL.into(), msg: format!("panic: {msg}") }),
            }
        }
    }
}
