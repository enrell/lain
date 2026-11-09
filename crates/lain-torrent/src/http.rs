//! The HTTP seam for trackers: a blocking GET the engine runs on the
//! blocking pool. [`PlainHttp`] speaks plain `http://` only; a server
//! with TLS injects its own client.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::metainfo::split_url;

/// A blocking HTTP GET: `(status, body)`.
pub trait HttpGet: Send + Sync {
    fn get(&self, url: &str, timeout: Duration) -> Result<(u16, Vec<u8>), String>;
}

/// A minimal HTTP/1.1 client over `std::net` for `http://` URLs:
/// Content-Length, chunked and close-delimited bodies, a body cap and up
/// to five redirects.
#[derive(Debug, Clone)]
pub struct PlainHttp {
    pub max_body: usize,
}

impl Default for PlainHttp {
    fn default() -> Self {
        PlainHttp { max_body: 4 << 20 }
    }
}

const MAX_HEADER: usize = 64 << 10;

impl HttpGet for PlainHttp {
    fn get(&self, url: &str, timeout: Duration) -> Result<(u16, Vec<u8>), String> {
        let deadline = Instant::now() + timeout;
        let mut url = url.to_string();
        for _ in 0..=5 {
            let (status, headers, body) = self.once(&url, deadline)?;
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = header(&headers, "location")
            {
                url = resolve(&url, &loc).ok_or("bad redirect")?;
                continue;
            }
            return Ok((status, body));
        }
        Err("too many redirects".into())
    }
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
}

fn resolve(base: &str, loc: &str) -> Option<String> {
    if loc.contains("://") {
        return Some(loc.to_string());
    }
    let (scheme, host, _) = split_url(base)?;
    loc.starts_with('/').then(|| format!("{scheme}://{host}{loc}"))
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| "timeout".into())
}

impl PlainHttp {
    #[allow(clippy::type_complexity)]
    fn once(&self, url: &str, deadline: Instant) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
        let (scheme, authority, rest) = split_url(url).ok_or("bad url")?;
        if scheme != "http" {
            return Err(format!("{scheme} needs a TLS-capable client"));
        }
        let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(&authority).to_string();
        if host.is_empty() {
            return Err("bad url".into());
        }
        let target = if rest.is_empty() || rest.starts_with('?') { format!("/{rest}") } else { rest };
        let target = target.split('#').next().unwrap_or("/").to_string();
        let hostport = if host.starts_with('[') {
            if host.contains("]:") { host.clone() } else { format!("{host}:80") }
        } else if host.contains(':') {
            host.clone()
        } else {
            format!("{host}:80")
        };
        let addrs: Vec<_> = hostport.to_socket_addrs().map_err(|e| e.to_string())?.collect();
        let mut last = String::from("no address");
        let mut conn = None;
        for a in addrs {
            match TcpStream::connect_timeout(&a, remaining(deadline)?) {
                Ok(c) => {
                    conn = Some(c);
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        let mut conn = conn.ok_or(last)?;
        conn.set_write_timeout(Some(remaining(deadline)?)).map_err(|e| e.to_string())?;
        let req = format!(
            "GET {target} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Lain\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
        );
        conn.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(conn);
        let mut line = String::new();
        let mut header_bytes = 0;
        let mut read_line = |r: &mut BufReader<TcpStream>, line: &mut String| -> Result<(), String> {
            r.get_ref().set_read_timeout(Some(remaining(deadline)?)).map_err(|e| e.to_string())?;
            line.clear();
            let n = r.by_ref().take(MAX_HEADER as u64).read_line(line).map_err(|e| e.to_string())?;
            header_bytes += n;
            if n == 0 || header_bytes > MAX_HEADER {
                return Err("bad response head".into());
            }
            Ok(())
        };
        read_line(&mut r, &mut line)?;
        let status: u16 = line
            .split_whitespace()
            .nth(1)
            .filter(|_| line.starts_with("HTTP/1."))
            .and_then(|s| s.parse().ok())
            .ok_or("bad status line")?;
        let mut headers = Vec::new();
        loop {
            read_line(&mut r, &mut line)?;
            let l = line.trim_end_matches(['\r', '\n']);
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        let cap = self.max_body;
        let mut body = Vec::new();
        let chunked = header(&headers, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
        let read_n = |r: &mut BufReader<TcpStream>, n: usize, body: &mut Vec<u8>| -> Result<(), String> {
            if body.len() + n > cap {
                return Err("body too large".into());
            }
            r.get_ref().set_read_timeout(Some(remaining(deadline)?)).map_err(|e| e.to_string())?;
            let start = body.len();
            body.resize(start + n, 0);
            r.read_exact(&mut body[start..]).map_err(|e| e.to_string())
        };
        if chunked {
            loop {
                read_line(&mut r, &mut line)?;
                let size = line.trim().split(';').next().unwrap_or("");
                let n = usize::from_str_radix(size.trim(), 16).map_err(|_| "bad chunk size")?;
                if n == 0 {
                    break;
                }
                read_n(&mut r, n, &mut body)?;
                read_line(&mut r, &mut line)?;
            }
        } else if let Some(len) = header(&headers, "content-length") {
            let n: usize = len.parse().map_err(|_| "bad content-length")?;
            read_n(&mut r, n, &mut body)?;
        } else {
            r.get_ref().set_read_timeout(Some(remaining(deadline)?)).map_err(|e| e.to_string())?;
            r.by_ref().take(cap as u64 + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
            if body.len() > cap {
                return Err("body too large".into());
            }
        }
        Ok((status, headers, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(responses: Vec<&'static str>) -> String {
        let ln = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = ln.local_addr().unwrap();
        std::thread::spawn(move || {
            for resp in responses {
                let (mut c, _) = ln.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf);
                let _ = c.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn bodies_and_redirects() {
        let base = serve(vec![
            "HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
            "HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\nno",
            "HTTP/1.0 200 OK\r\n\r\nuntil close",
        ]);
        let h = PlainHttp::default();
        let t = Duration::from_secs(5);
        assert_eq!(h.get(&format!("{base}/a"), t).unwrap(), (200, b"abcde".to_vec()));
        assert_eq!(h.get(&base, t).unwrap(), (404, b"no".to_vec()));
        assert_eq!(h.get(&format!("{base}/x?y=1"), t).unwrap(), (200, b"until close".to_vec()));
        assert!(h.get("https://tracker-exemplo.invalid/announce", t).unwrap_err().contains("TLS"));
    }
}
