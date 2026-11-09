//! The resumable fetcher (Go `fetch.go`).
//!
//! `part` is the partial file the bytes accumulate in; it survives a
//! pause or crash so the next fetch continues from its size. `validator`
//! is the ETag or Last-Modified the previous attempt saw: a resume is
//! trusted when the origin still serves the same representation
//! (If-Range). An origin with no validator can still resume when it
//! advertised byte ranges and the total it reports for the remainder
//! equals the total recorded before; any other mismatch restarts from 0
//! rather than splicing two files together.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::http::{Cancel, Http, HttpErrorKind, Request};
use crate::{CODE_HTTP, CODE_INVALID, CODE_IO, Error};

/// The write granularity; budget checks happen per chunk.
pub const CHUNK: usize = 256 << 10;

/// The longest an origin's Retry-After may park a job.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// One transfer.
pub struct FetchRequest<'a> {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub part: PathBuf,
    pub validator: String,
    /// The full size seen by the previous attempt, 0 unknown.
    pub total: i64,
    /// The previous attempt saw `Accept-Ranges: bytes`.
    pub ranges: bool,
    /// Consulted before bytes are written; `None` is unlimited.
    pub reserve: Option<&'a (dyn Fn(i64) -> Result<(), Error> + Sync)>,
    /// `(done, total)` as bytes land; total is -1 while unknown.
    pub progress: Option<&'a (dyn Fn(i64, i64) + Sync)>,
    /// Connect timeout and the longest a read may stall.
    pub timeout: Duration,
}

impl<'a> FetchRequest<'a> {
    pub fn new(url: &str, part: impl Into<PathBuf>) -> FetchRequest<'a> {
        FetchRequest {
            url: url.into(),
            headers: Vec::new(),
            part: part.into(),
            validator: String::new(),
            total: 0,
            ranges: false,
            reserve: None,
            progress: None,
            timeout: Duration::from_secs(60),
        }
    }
}

/// A finished or interrupted transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchResult {
    /// Bytes now in the part.
    pub bytes: i64,
    /// The expected total, -1 if the origin never said.
    pub total: i64,
    /// To pass back on the next resume.
    pub validator: String,
    /// The origin-suggested name (Content-Disposition), may be empty.
    pub filename: String,
    /// A partial file was discarded.
    pub restarted: bool,
    /// The origin serves byte ranges.
    pub ranges: bool,
}

/// Downloads `req.url` into `req.part`, resuming from the part's size.
/// It returns when the body is exhausted, the cancel flag is set (the
/// part is kept; the error is [`Error::canceled`]) or a write is refused
/// by the budget.
pub fn fetch(http: &dyn Http, req: &FetchRequest<'_>, cancel: &Cancel) -> (FetchResult, Option<Error>) {
    let mut res = FetchResult { total: -1, validator: req.validator.clone(), ..Default::default() };
    let mut have = match std::fs::metadata(&req.part) {
        Ok(m) => m.len() as i64,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return (res, Some(Error::new(CODE_IO, e.to_string()))),
    };
    if crate::http::parse_url(&req.url).is_none() {
        return (res, Some(Error::new(CODE_INVALID, "the URL is not an absolute http(s) URL")));
    }
    let mut hreq = Request::get(&req.url);
    hreq.timeout = req.timeout;
    hreq.cancel = Some(cancel.clone());
    hreq.headers = req.headers.clone();
    // Ask for the remainder only when the previous representation can be
    // named (validator) or at least sized (ranges + total); otherwise a
    // 206 could belong to another file.
    let sized = req.validator.is_empty() && req.ranges && req.total > 0;
    if have > 0 && (!req.validator.is_empty() || sized) {
        hreq.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("range") && !k.eq_ignore_ascii_case("if-range"));
        hreq.headers.push(("Range".into(), format!("bytes={have}-")));
        if !req.validator.is_empty() {
            hreq.headers.push(("If-Range".into(), req.validator.clone()));
        }
    }
    let resp = match http.send(&hreq) {
        Ok(r) => r,
        Err(e) => {
            if cancel.is_canceled() || e.kind == HttpErrorKind::Canceled {
                res.bytes = have;
                return (res, Some(Error::canceled()));
            }
            if e.kind == HttpErrorKind::Invalid {
                return (res, Some(Error::new(CODE_INVALID, e.msg)));
            }
            return (res, Some(Error::transient(e.msg)));
        }
    };
    res.ranges = resp.header("accept-ranges") == Some("bytes") || resp.status == 206;

    let mut append = false;
    match resp.status {
        206 => {
            let ok = parse_content_range(resp.header("content-range").unwrap_or(""));
            match ok {
                Some((start, total)) if start == have && !(sized && total != req.total) => {
                    res.total = total;
                    append = true;
                }
                _ => {
                    // Not the remainder of what we hold: drop the part so
                    // the retry starts clean instead of splicing.
                    let _ = std::fs::remove_file(&req.part);
                    res.restarted = true;
                    return (
                        res,
                        Some(Error::transient("origin answered a different range; restarting from the beginning")),
                    );
                }
            }
        }
        200 => {
            res.restarted = have > 0;
            have = 0;
            res.total = resp.content_length();
        }
        416 => {
            // The part already holds every byte (a crash after the last
            // write but before the job was marked done).
            if let Some((_, total)) = parse_content_range(resp.header("content-range").unwrap_or(""))
                && total == have
            {
                res.bytes = have;
                res.total = total;
                return (res, None);
            }
            let _ = std::fs::remove_file(&req.part);
            res.restarted = true;
            return (res, Some(Error::transient("origin refused the resume range; restarting from the beginning")));
        }
        c => {
            let mut e = Error::new(CODE_HTTP, format!("origin answered {c}"));
            if c >= 500 || c == 408 || c == 429 {
                e.retry = true;
                e.retry_after = retry_after(resp.header("retry-after").unwrap_or(""), SystemTime::now());
            }
            return (res, Some(e));
        }
    }
    res.validator = match (resp.header("etag"), resp.header("last-modified")) {
        (Some(v), _) if !v.is_empty() && !v.starts_with("W/") => v.to_string(),
        (_, Some(v)) if !v.is_empty() => v.to_string(),
        _ => String::new(),
    };
    res.filename = disposition_name(resp.header("content-disposition").unwrap_or(""));

    if let Some(reserve) = req.reserve
        && res.total > 0
    {
        // Refuse up front when the whole remainder cannot fit; a size the
        // origin never stated is checked chunk by chunk instead.
        if let Err(e) = reserve(res.total - have) {
            res.bytes = have;
            return (res, Some(e));
        }
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    let mut f = match opts.open(&req.part) {
        Ok(f) => f,
        Err(e) => return (res, Some(Error::new(CODE_IO, e.to_string()))),
    };
    let mut done = have;
    let total = res.total;
    let report = |done: i64| {
        if let Some(p) = req.progress {
            p(done, total);
        }
    };
    report(done);
    let mut body = resp.body;
    let mut buf = vec![0u8; CHUNK];
    loop {
        match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Some(reserve) = req.reserve
                    && res.total <= 0
                    && let Err(e) = reserve(n as i64)
                {
                    res.bytes = done;
                    return (res, Some(e));
                }
                if let Err(e) = f.write_all(&buf[..n]) {
                    res.bytes = done;
                    return (res, Some(Error::new(CODE_IO, e.to_string())));
                }
                done += n as i64;
                report(done);
            }
            Err(e) => {
                res.bytes = done;
                if cancel.is_canceled() {
                    return (res, Some(Error::canceled()));
                }
                return (res, Some(Error::transient(format!("transfer interrupted: {e}"))));
            }
        }
    }
    if let Err(e) = f.flush().and_then(|_| f.sync_data()) {
        res.bytes = done;
        return (res, Some(Error::new(CODE_IO, e.to_string())));
    }
    res.bytes = done;
    if res.total > 0 && done != res.total {
        let msg = format!("short body: {done} of {} bytes", res.total);
        return (res, Some(Error::transient(msg)));
    }
    if res.total < 0 {
        res.total = done;
    }
    (res, None)
}

/// Reads `bytes start-end/total`; total is -1 for `*`.
pub fn parse_content_range(v: &str) -> Option<(i64, i64)> {
    let v = v.trim().strip_prefix("bytes ")?;
    let (rng, tot) = v.split_once('/')?;
    let total = if tot == "*" { -1 } else { tot.parse::<i64>().ok().filter(|t| *t >= 0)? };
    if rng == "*" {
        return Some((0, total));
    }
    let (s, _) = rng.split_once('-')?;
    let start = s.parse::<i64>().ok().filter(|s| *s >= 0)?;
    Some((start, total))
}

/// Delay-seconds or an HTTP date, capped at an hour; zero when absent.
pub fn retry_after(v: &str, now: SystemTime) -> Duration {
    let v = v.trim();
    if v.is_empty() {
        return Duration::ZERO;
    }
    let d = if let Ok(n) = v.parse::<i64>() {
        if n >= 0 { Duration::from_secs(n as u64) } else { Duration::ZERO }
    } else if let Some(t) = parse_http_date(v) {
        let at = UNIX_EPOCH + Duration::from_secs(t.max(0) as u64);
        at.duration_since(now).unwrap_or(Duration::ZERO)
    } else {
        Duration::ZERO
    };
    d.min(MAX_RETRY_AFTER)
}

fn month(m: &str) -> Option<i64> {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|x| *x == m)
        .map(|i| i as i64 + 1)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn hms(s: &str) -> Option<i64> {
    let mut it = s.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, sec) = (it.next()??, it.next()??, it.next()??);
    (it.next().is_none() && h < 24 && m < 60 && sec < 61).then_some(h * 3600 + m * 60 + sec)
}

/// Unix seconds of an IMF-fixdate, RFC 850 or asctime date (Go's
/// `http.ParseTime`).
pub fn parse_http_date(v: &str) -> Option<i64> {
    let f: Vec<&str> = v.split_whitespace().collect();
    let (y, mo, d, t) = match f.as_slice() {
        // Sun, 06 Nov 1994 08:49:37 GMT
        [_, d, m, y, t, "GMT"] => (y.parse().ok()?, month(m)?, d.parse().ok()?, hms(t)?),
        // Sunday, 06-Nov-94 08:49:37 GMT
        [_, dmy, t, "GMT"] => {
            let mut p = dmy.split('-');
            let (d, m, y) = (p.next()?, p.next()?, p.next()?);
            let y: i64 = y.parse().ok()?;
            (
                if y < 70 {
                    2000 + y
                } else if y < 100 {
                    1900 + y
                } else {
                    y
                },
                month(m)?,
                d.parse().ok()?,
                hms(t)?,
            )
        }
        // Sun Nov  6 08:49:37 1994
        [_, m, d, t, y] => (y.parse().ok()?, month(m)?, d.parse().ok()?, hms(t)?),
        _ => return None,
    };
    if !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + t)
}

fn pct_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c > 0x20 && c < 0x7f && !b"()<>@,;:\\\"/[]?={}".contains(&c))
}

/// The filename a `Content-Disposition` suggests (`filename*` RFC 5987
/// first, then `filename`); empty when it does not parse.
pub fn disposition_name(v: &str) -> String {
    let v = v.trim();
    if v.is_empty() {
        return String::new();
    }
    let (kind, mut rest) = v.split_once(';').unwrap_or((v, ""));
    if !is_token(kind.trim()) {
        return String::new();
    }
    let (mut plain, mut ext) = (None, None);
    loop {
        rest = rest.trim_start_matches([' ', '\t', ';']);
        if rest.is_empty() {
            break;
        }
        let Some((k, after)) = rest.split_once('=') else { return String::new() };
        let k = k.trim().to_ascii_lowercase();
        let after = after.trim_start();
        let val;
        if let Some(q) = after.strip_prefix('"') {
            let mut out = String::new();
            let mut it = q.char_indices();
            let mut end = None;
            while let Some((i, c)) = it.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = it.next() {
                            out.push(n);
                        }
                    }
                    '"' => {
                        end = Some(i + 1);
                        break;
                    }
                    c => out.push(c),
                }
            }
            let Some(end) = end else { return String::new() };
            val = out;
            rest = &q[end..];
        } else {
            let end = after.find(';').unwrap_or(after.len());
            val = after[..end].trim().to_string();
            rest = &after[end..];
        }
        match k.as_str() {
            "filename" => plain = Some(val),
            "filename*" => {
                let mut p = val.splitn(3, '\'');
                if let (Some(cs), Some(_), Some(enc)) = (p.next(), p.next(), p.next())
                    && (cs.eq_ignore_ascii_case("utf-8") || cs.eq_ignore_ascii_case("us-ascii"))
                {
                    ext = pct_decode(enc);
                }
            }
            _ => {}
        }
    }
    ext.or(plain).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::PlainHttp;
    use crate::limits::check_with;
    use crate::testsrv::{Gate, Srv, respond, serve_content};
    use crate::{CODE_QUOTA, Limits};
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, Ordering};

    pub fn payload() -> Vec<u8> {
        b"lain-fixture-0123456789\n".repeat(4096)
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = lain_store::test_dir(&format!("dl-fetch-{tag}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn origin(etag: &'static str) -> std::sync::Arc<Srv> {
        let body = payload();
        Srv::start(move |req, s| serve_content(req, s, &body, etag, None))
    }

    fn run<'a>(url: &str, part: &Path, f: impl FnOnce(&mut FetchRequest<'a>)) -> (FetchResult, Option<Error>) {
        let mut r = FetchRequest::new(url, part);
        f(&mut r);
        fetch(&PlainHttp::new(), &r, &Cancel::new())
    }

    #[test]
    fn whole_file() {
        let srv = origin(r#""v1""#);
        let part = tmp("whole").join("a.part");
        let last = AtomicI64::new(0);
        let p = |d: i64, _: i64| last.store(d, Ordering::SeqCst);
        let (res, err) = run(&srv.url, &part, |r| r.progress = Some(&p));
        assert!(err.is_none(), "{err:?}");
        let n = payload().len() as i64;
        assert_eq!(std::fs::read(&part).unwrap(), payload());
        assert_eq!((res.bytes, res.total, last.load(Ordering::SeqCst)), (n, n, n));
        assert_eq!((res.validator.as_str(), res.filename.as_str()), (r#""v1""#, "Frieren - 01.cbz"));
    }

    #[test]
    fn resumes_with_matching_validator() {
        let srv = origin(r#""v1""#);
        let part = tmp("resume").join("a.part");
        std::fs::write(&part, &payload()[..1000]).unwrap();
        let (res, err) = run(&srv.url, &part, |r| r.validator = r#""v1""#.into());
        assert!(err.is_none() && !res.restarted, "{err:?}");
        assert_eq!(std::fs::read(&part).unwrap(), payload());
        assert_eq!(srv.ranges(), ["bytes=1000-"]);
    }

    #[test]
    fn restarts_when_origin_changed() {
        let srv = origin(r#""v2""#);
        let part = tmp("changed").join("a.part");
        std::fs::write(&part, b"bytes of an older representation").unwrap();
        let (res, err) = run(&srv.url, &part, |r| r.validator = r#""v1""#.into());
        assert!(err.is_none() && res.restarted);
        assert_eq!(std::fs::read(&part).unwrap(), payload());
    }

    #[test]
    fn without_validator_never_asks_for_a_range() {
        let srv = origin("");
        let part = tmp("novalidator").join("a.part");
        std::fs::write(&part, b"junk").unwrap();
        let (_, err) = run(&srv.url, &part, |_| {});
        assert!(err.is_none());
        assert_eq!(std::fs::read(&part).unwrap(), payload());
        assert_eq!(srv.ranges(), [""]);
    }

    #[test]
    fn origin_ignoring_ranges_restarts() {
        let body = payload();
        let srv = Srv::start(move |_, s| respond(s, 200, &[("ETag", r#""v1""#)], &body));
        let part = tmp("ignore").join("a.part");
        std::fs::write(&part, &payload()[..500]).unwrap();
        let (res, err) = run(&srv.url, &part, |r| r.validator = r#""v1""#.into());
        assert!(err.is_none() && res.restarted);
        assert_eq!(std::fs::read(&part).unwrap(), payload());
    }

    #[test]
    fn complete_part_answered_416() {
        let srv = origin(r#""v1""#);
        let part = tmp("416").join("a.part");
        std::fs::write(&part, payload()).unwrap();
        let (res, err) = run(&srv.url, &part, |r| r.validator = r#""v1""#.into());
        assert!(err.is_none() && res.bytes == payload().len() as i64, "{err:?}");
    }

    #[test]
    fn cancel_keeps_part() {
        let gate = Gate::default();
        let g2 = gate.clone();
        let srv = Srv::start(move |_, s| {
            let _ = std::io::Write::write_all(s, &crate::testsrv::head(200, &[], Some(2000)));
            let _ = std::io::Write::write_all(s, &payload()[..1000]);
            g2.wait();
        });
        let part = tmp("cancel").join("a.part");
        let c = Cancel::new();
        let c2 = c.clone();
        let p = move |d: i64, _: i64| {
            if d >= 1000 {
                c2.cancel();
            }
        };
        let mut r = FetchRequest::new(&srv.url, &part);
        r.progress = Some(&p);
        let (res, err) = fetch(&PlainHttp::new(), &r, &c);
        gate.open();
        assert!(err.as_ref().is_some_and(Error::is_canceled), "{err:?}");
        assert_eq!((res.bytes, std::fs::metadata(&part).unwrap().len()), (1000, 1000));
    }

    fn no_free(_: &Path) -> Option<i64> {
        None
    }

    #[test]
    fn budget_refuses_known_size_up_front() {
        let srv = origin(r#""v1""#);
        let part = tmp("budget1").join("a.part");
        let lim = Limits { max_bytes: 10, min_free_bytes: 0 };
        let reserve = |n: i64| check_with(&lim, Path::new("/"), 0, n, no_free);
        let (_, err) = run(&srv.url, &part, |r| r.reserve = Some(&reserve));
        assert_eq!(crate::code_of(&err), CODE_QUOTA);
        assert!(!part.exists(), "a refused download must not create its part");
    }

    #[test]
    fn budget_stops_unknown_size_midway() {
        let srv = Srv::start(|_, s| {
            let _ = std::io::Write::write_all(s, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            for _ in 0..8 {
                let _ = std::io::Write::write_all(s, format!("{:x}\r\n", 8000).as_bytes());
                let _ = std::io::Write::write_all(s, &payload()[..8000]);
                let _ = std::io::Write::write_all(s, b"\r\n");
                let _ = std::io::Write::flush(s);
            }
            let _ = std::io::Write::write_all(s, b"0\r\n\r\n");
        });
        let part = tmp("budget2").join("a.part");
        let used = Mutex::new(0i64);
        let lim = Limits { max_bytes: 20000, min_free_bytes: 0 };
        let reserve = |n: i64| {
            let mut u = used.lock().unwrap();
            check_with(&lim, Path::new("/"), *u, n, no_free)?;
            *u += n;
            Ok(())
        };
        let (res, err) = run(&srv.url, &part, |r| r.reserve = Some(&reserve));
        assert_eq!(crate::code_of(&err), CODE_QUOTA);
        assert!(res.bytes <= 20000, "wrote {} bytes past the budget", res.bytes);
    }

    #[test]
    fn http_errors() {
        let srv = Srv::start(|_, s| respond(s, 404, &[], b"no"));
        let d = tmp("errors");
        let (_, err) = run(&srv.url, &d.join("p"), |_| {});
        let e = err.unwrap();
        assert!(e.code == CODE_HTTP && e.to_string().contains("404") && !e.retry, "{e}");
        let (_, err) = run("::bad", &d.join("p"), |_| {});
        assert_eq!(crate::code_of(&err), CODE_INVALID);
        let srv = Srv::start(|_, s| respond(s, 503, &[("Retry-After", "7")], b""));
        let (_, err) = run(&srv.url, &d.join("p"), |_| {});
        let e = err.unwrap();
        assert!(e.retry && e.retry_after == Duration::from_secs(7), "{e:?}");
    }

    #[test]
    fn content_ranges() {
        for (inp, want) in [
            ("bytes 10-99/100", Some((10, 100))),
            ("bytes 0-0/*", Some((0, -1))),
            ("bytes */100", Some((0, 100))),
            ("bytes x-1/2", None),
            ("items 1-2/3", None),
            ("bytes 1-2", None),
        ] {
            assert_eq!(parse_content_range(inp), want, "{inp}");
        }
    }

    #[test]
    fn retry_after_values() {
        let now = UNIX_EPOCH + Duration::from_secs(784_111_687); // 1994-11-06 08:48:07
        assert_eq!(retry_after("7", now), Duration::from_secs(7));
        assert_eq!(retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now), Duration::from_secs(90));
        assert_eq!(retry_after("Sunday, 06-Nov-94 08:49:37 GMT", now), Duration::from_secs(90));
        assert_eq!(retry_after("Sun Nov  6 08:49:37 1994", now), Duration::from_secs(90));
        assert_eq!(retry_after("", now), Duration::ZERO);
        assert_eq!(retry_after("soon", now), Duration::ZERO);
        assert_eq!(retry_after("99999", now), MAX_RETRY_AFTER);
    }

    #[test]
    fn dispositions() {
        for (inp, want) in [
            (r#"attachment; filename="Frieren - 01.cbz""#, "Frieren - 01.cbz"),
            ("attachment; filename=plain.mkv", "plain.mkv"),
            (r#"attachment; filename="a.mkv"; filename*=UTF-8''%C3%BC.mkv"#, "ü.mkv"),
            (r#"attachment; filename="q\"x.mkv""#, "q\"x.mkv"),
            ("inline", ""),
            ("", ""),
            (r#"attachment; filename="open"#, ""),
            ("bad type; filename=x", ""),
        ] {
            assert_eq!(disposition_name(inp), want, "{inp}");
        }
    }
}
