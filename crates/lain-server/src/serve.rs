//! Serving a file from disk the way Go's `http.ServeContent` does
//! (D-191): `Last-Modified`, conditional requests (`If-Match`,
//! `If-Unmodified-Since`, `If-None-Match`, `If-Modified-Since`,
//! `If-Range` by date), single and multiple byte ranges, 416 with
//! `Content-Range: bytes */size`, and HEAD. The bytes stream in bounded
//! chunks, each read on the blocking pool, so a slow client holds no
//! thread and a large file never sits in memory.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use hyper::body::{Bytes, Frame, SizeHint};

const CHUNK: u64 = 256 << 10;

enum Part {
    Bytes(Bytes),
    Range { start: u64, len: u64 },
}

type PendingRead = tokio::task::JoinHandle<std::io::Result<(File, Vec<u8>)>>;

/// The streamed body: literal parts (multipart framing) and file ranges.
pub struct FileBody {
    file: Option<File>,
    parts: VecDeque<Part>,
    pending: Option<PendingRead>,
}

impl FileBody {
    fn remaining(&self) -> u64 {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Bytes(b) => b.len() as u64,
                Part::Range { len, .. } => *len,
            })
            .sum()
    }
}

impl hyper::body::Body for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        loop {
            if let Some(h) = self.pending.as_mut() {
                let done = match Pin::new(h).poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(r) => r,
                };
                self.pending = None;
                let (f, buf) = match done {
                    Ok(Ok(x)) => x,
                    Ok(Err(e)) => return Poll::Ready(Some(Err(e))),
                    Err(e) => return Poll::Ready(Some(Err(std::io::Error::other(e)))),
                };
                self.file = Some(f);
                if buf.is_empty() {
                    return Poll::Ready(Some(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))));
                }
                if let Some(Part::Range { start, len }) = self.parts.front_mut() {
                    *start += buf.len() as u64;
                    *len = len.saturating_sub(buf.len() as u64);
                }
                return Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))));
            }
            match self.parts.front_mut() {
                None => return Poll::Ready(None),
                Some(Part::Bytes(b)) => {
                    let b = std::mem::take(b);
                    self.parts.pop_front();
                    return Poll::Ready(Some(Ok(Frame::data(b))));
                }
                Some(Part::Range { len: 0, .. }) => {
                    self.parts.pop_front();
                }
                Some(Part::Range { start, len }) => {
                    let (start, n) = (*start, (*len).min(CHUNK));
                    let Some(mut f) = self.file.take() else {
                        return Poll::Ready(Some(Err(std::io::Error::other("file handle lost"))));
                    };
                    self.pending = Some(tokio::task::spawn_blocking(move || {
                        f.seek(SeekFrom::Start(start))?;
                        let mut buf = vec![0u8; n as usize];
                        let mut got = 0;
                        while got < buf.len() {
                            match f.read(&mut buf[got..]) {
                                Ok(0) => break,
                                Ok(k) => got += k,
                                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                                Err(e) => return Err(e),
                            }
                        }
                        buf.truncate(got);
                        Ok((f, buf))
                    }));
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.pending.is_none() && self.remaining() == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining())
    }
}

// ---- HTTP dates -------------------------------------------------------

const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Mon, 02 Jan 2006 15:04:05 GMT` for Unix seconds.
pub fn http_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Parses the IMF-fixdate form (what Go writes and clients echo).
pub fn parse_http_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let rest = s.split_once(", ")?.1;
    let f: Vec<&str> = rest.split(' ').collect();
    if f.len() != 5 || f[4] != "GMT" {
        return None;
    }
    let d: i64 = f[0].parse().ok()?;
    let m = MONTHS.iter().position(|x| *x == f[1])? as i64 + 1;
    let y: i64 = f[2].parse().ok()?;
    let t: Vec<i64> = f[3].split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if t.len() != 3 || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400 + t[0] * 3600 + t[1] * 60 + t[2])
}

// ---- ranges -------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum RangeError {
    Invalid,
    NoOverlap,
}

/// Go's `parseRange`: (start, length) pairs within `size`.
pub fn parse_range(s: &str, size: u64) -> Result<Vec<(u64, u64)>, RangeError> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let Some(spec) = s.strip_prefix("bytes=") else { return Err(RangeError::Invalid) };
    let mut out = Vec::new();
    let mut no_overlap = false;
    for ra in spec.split(',') {
        let ra = ra.trim_matches([' ', '\t']);
        if ra.is_empty() {
            continue;
        }
        let Some((start, end)) = ra.split_once('-') else { return Err(RangeError::Invalid) };
        let (start, end) = (start.trim_matches([' ', '\t']), end.trim_matches([' ', '\t']));
        if start.is_empty() {
            if end.is_empty() || end.starts_with('-') {
                return Err(RangeError::Invalid);
            }
            let n: u64 = end.parse().map_err(|_| RangeError::Invalid)?;
            let n = n.min(size);
            out.push((size - n, n));
            continue;
        }
        let i: u64 = start.parse().map_err(|_| RangeError::Invalid)?;
        if i >= size {
            no_overlap = true;
            continue;
        }
        let len = if end.is_empty() {
            size - i
        } else {
            let e: u64 = end.parse().map_err(|_| RangeError::Invalid)?;
            if i > e {
                return Err(RangeError::Invalid);
            }
            e.min(size - 1) - i + 1
        };
        out.push((i, len));
    }
    if no_overlap && out.is_empty() {
        return Err(RangeError::NoOverlap);
    }
    Ok(out)
}

fn hv(s: &str) -> HeaderValue {
    HeaderValue::from_str(s).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn get<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn etag_list_has_star(v: &str) -> bool {
    v.split(',').any(|t| t.trim() == "*")
}

fn simple(status: StatusCode, mut headers: HeaderMap, text: &str) -> Response {
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.remove(header::LAST_MODIFIED);
    let mut r = Response::new(Body::from(format!("{text}\n")));
    *r.status_mut() = status;
    *r.headers_mut() = headers;
    r
}

fn boundary() -> String {
    let mut b = [0u8; 15];
    if getrandom::fill(&mut b).is_err() {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b.copy_from_slice(&t.to_le_bytes()[..15]);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The response for one file. `headers` already carries what the route
/// sets (`Content-Type`, `Cache-Control`, ...).
pub fn content(
    file: File,
    size: u64,
    modified: Option<SystemTime>,
    req: &HeaderMap,
    method: &Method,
    mut headers: HeaderMap,
) -> Response {
    let mtime = modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).filter(|s| *s > 0);
    let get_or_head = method == Method::GET || method == Method::HEAD;
    // Preconditions (RFC 7232 order, as Go).
    if let Some(im) = get(req, "if-match") {
        if !etag_list_has_star(im) {
            return precondition_failed(headers);
        }
    } else if let (Some(ius), Some(m)) = (get(req, "if-unmodified-since").and_then(parse_http_date), mtime)
        && m > ius
    {
        return precondition_failed(headers);
    }
    let not_modified = if let Some(inm) = get(req, "if-none-match") {
        if etag_list_has_star(inm) {
            if !get_or_head {
                return precondition_failed(headers);
            }
            true
        } else {
            false
        }
    } else {
        get_or_head
            && matches!((get(req, "if-modified-since").and_then(parse_http_date), mtime), (Some(ims), Some(m)) if m <= ims)
    };
    if let Some(m) = mtime {
        headers.insert(header::LAST_MODIFIED, hv(&http_date(m)));
    }
    if not_modified {
        headers.remove(header::CONTENT_TYPE);
        headers.remove(header::CONTENT_LENGTH);
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        *r.headers_mut() = headers;
        return r;
    }
    let mut range = get(req, "range").unwrap_or("").to_string();
    if !range.is_empty()
        && let Some(ir) = get(req, "if-range")
    {
        // No ETag is ever sent, so an entity-tag If-Range never matches.
        let keep =
            !ir.starts_with('"') && !ir.starts_with("W/") && parse_http_date(ir).is_some_and(|d| Some(d) == mtime);
        if !keep {
            range.clear();
        }
    }
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let mut ranges = match parse_range(&range, size) {
        Ok(r) => r,
        Err(RangeError::NoOverlap) if size == 0 => Vec::new(),
        Err(e) => {
            if e == RangeError::NoOverlap {
                headers.insert(header::CONTENT_RANGE, hv(&format!("bytes */{size}")));
            }
            let text = if e == RangeError::NoOverlap { "invalid range: failed to overlap" } else { "invalid range" };
            return simple(StatusCode::RANGE_NOT_SATISFIABLE, headers, text);
        }
    };
    if ranges.iter().map(|r| r.1).sum::<u64>() > size {
        ranges.clear();
    }
    let ctype = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut parts = VecDeque::new();
    let status = match ranges.len() {
        0 => {
            parts.push_back(Part::Range { start: 0, len: size });
            StatusCode::OK
        }
        1 => {
            let (s, n) = ranges[0];
            headers.insert(header::CONTENT_RANGE, hv(&format!("bytes {s}-{}/{size}", s + n - 1)));
            parts.push_back(Part::Range { start: s, len: n });
            StatusCode::PARTIAL_CONTENT
        }
        _ => {
            let b = boundary();
            for (i, (s, n)) in ranges.iter().enumerate() {
                let lead = if i == 0 { format!("--{b}\r\n") } else { format!("\r\n--{b}\r\n") };
                let head =
                    format!("{lead}Content-Range: bytes {s}-{}/{size}\r\nContent-Type: {ctype}\r\n\r\n", s + n - 1);
                parts.push_back(Part::Bytes(Bytes::from(head)));
                parts.push_back(Part::Range { start: *s, len: *n });
            }
            parts.push_back(Part::Bytes(Bytes::from(format!("\r\n--{b}--\r\n"))));
            headers.insert(header::CONTENT_TYPE, hv(&format!("multipart/byteranges; boundary={b}")));
            StatusCode::PARTIAL_CONTENT
        }
    };
    let body = FileBody { file: Some(file), parts, pending: None };
    headers.insert(header::CONTENT_LENGTH, hv(&body.remaining().to_string()));
    let mut r = if method == Method::HEAD { Response::new(Body::empty()) } else { Response::new(Body::new(body)) };
    *r.status_mut() = status;
    *r.headers_mut() = headers;
    r
}

fn precondition_failed(headers: HeaderMap) -> Response {
    let mut r = Response::new(Body::empty());
    *r.status_mut() = StatusCode::PRECONDITION_FAILED;
    *r.headers_mut() = headers;
    r.headers_mut().remove(header::CONTENT_TYPE);
    r
}

/// `attachment; filename=...` the way Go's `mime.FormatMediaType`
/// quotes it.
pub fn attachment(name: &str) -> String {
    let token = |c: char| c.is_ascii_graphic() && !"()<>@,;:\\\"/[]?=".contains(c);
    if !name.is_empty() && name.chars().all(token) {
        return format!("attachment; filename={name}");
    }
    if name.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        let esc: String =
            name.chars().flat_map(|c| if c == '"' || c == '\\' { vec!['\\', c] } else { vec![c] }).collect();
        return format!("attachment; filename=\"{esc}\"");
    }
    let enc: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename*=utf-8''{enc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_136_214_245), "Mon, 02 Jan 2006 15:04:05 GMT");
        for t in [0, 951_782_400, 1_794_173_709, 4_102_444_800] {
            assert_eq!(parse_http_date(&http_date(t)), Some(t));
        }
        assert_eq!(parse_http_date("yesterday"), None);
    }

    #[test]
    fn ranges_follow_go() {
        assert_eq!(parse_range("bytes=0-7", 100), Ok(vec![(0, 8)]));
        assert_eq!(parse_range("bytes=90-", 100), Ok(vec![(90, 10)]));
        assert_eq!(parse_range("bytes=-10", 100), Ok(vec![(90, 10)]));
        assert_eq!(parse_range("bytes=-500", 100), Ok(vec![(0, 100)]));
        assert_eq!(parse_range("bytes=50-500", 100), Ok(vec![(50, 50)]));
        assert_eq!(parse_range("bytes=0-1, 4-5", 100), Ok(vec![(0, 2), (4, 2)]));
        assert_eq!(parse_range("bytes=100-", 100), Err(RangeError::NoOverlap));
        assert_eq!(parse_range("bytes=100-,0-1", 100), Ok(vec![(0, 2)]));
        assert_eq!(parse_range("bytes=5-1", 100), Err(RangeError::Invalid));
        assert_eq!(parse_range("items=0-1", 100), Err(RangeError::Invalid));
        assert_eq!(parse_range("bytes=x", 100), Err(RangeError::Invalid));
        assert_eq!(parse_range("", 100), Ok(vec![]));
    }

    #[test]
    fn attachment_names() {
        assert_eq!(attachment("a.mkv"), "attachment; filename=a.mkv");
        assert_eq!(attachment("[Fansub-A] Show 01.mkv"), "attachment; filename=\"[Fansub-A] Show 01.mkv\"");
        assert_eq!(attachment("ação.mkv"), "attachment; filename*=utf-8''a%C3%A7%C3%A3o.mkv");
    }
}
