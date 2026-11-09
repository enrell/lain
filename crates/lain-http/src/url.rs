//! The small URL subset outbound calls need: `http(s)://host[:port]`
//! plus a path and query, and Go-compatible escaping (`url.QueryEscape`,
//! `url.PathEscape`). No userinfo: credentials travel in headers.

use crate::Error;

/// A parsed absolute `http` or `https` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    /// Lower-case host; IPv6 without brackets.
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub target: String,
}

impl Url {
    pub fn parse(raw: &str) -> Result<Url, Error> {
        let raw = raw.trim();
        let bad = |m: &str| Error::invalid_url(m);
        let (tls, rest) = if let Some(r) = strip_prefix_ci(raw, "https://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(raw, "http://") {
            (false, r)
        } else {
            return Err(bad("only http and https urls"));
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') {
            return Err(bad("credentials do not belong in the url"));
        }
        let default_port = if tls { 443 } else { 80 };
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(|| bad("unclosed ["))?;
            match after.strip_prefix(':') {
                Some(p) => (h, p.parse().map_err(|_| bad("bad port"))?),
                None if after.is_empty() => (h, default_port),
                None => return Err(bad("bad host")),
            }
        } else {
            match authority.rsplit_once(':') {
                Some((h, "")) => (h, default_port),
                Some((h, p)) => (h, p.parse().map_err(|_| bad("bad port"))?),
                None => (authority, default_port),
            }
        };
        if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7f || b"<>\"\\^`{|}%".contains(&b)) {
            return Err(bad("bad host"));
        }
        let tail = tail.split('#').next().unwrap_or("");
        let target = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        if target.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(bad("spaces or control characters in the path"));
        }
        Ok(Url { tls, host: host.to_ascii_lowercase(), port, target })
    }

    /// `host[:port]`, the port only when it is not the scheme's default
    /// (the `Host` header form).
    pub fn authority(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == if self.tls { 443 } else { 80 } { host } else { format!("{host}:{}", self.port) }
    }

    /// The pacing and pooling key: `host:port`.
    pub fn host_key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Resolves a redirect `Location` against this URL: absolute
    /// `http(s)` URLs, scheme-relative `//host/...` and absolute paths.
    pub fn join(&self, location: &str) -> Result<Url, Error> {
        let scheme = if self.tls { "https:" } else { "http:" };
        if location.starts_with("//") {
            return Url::parse(&format!("{scheme}{location}"));
        }
        if location.starts_with('/') {
            return Url::parse(&format!("{scheme}//{}{location}", self.authority()));
        }
        if strip_prefix_ci(location, "http://").is_some() || strip_prefix_ci(location, "https://").is_some() {
            return Url::parse(location);
        }
        // A relative path: resolve against the current directory.
        let path = self.target.split('?').next().unwrap_or("/");
        let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(1)];
        Url::parse(&format!("{scheme}//{}{dir}{location}", self.authority()))
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}{}", if self.tls { "https" } else { "http" }, self.authority(), self.target)
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

/// Go's `url.QueryEscape`: unreserved bytes stay, a space becomes `+`,
/// everything else is `%XX`.
pub fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            _ if unreserved(b) => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Go's `url.PathEscape`: one path segment, so `/`, `;`, `,` and `?`
/// are escaped while `$&+:=@` stay.
pub fn path_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if unreserved(b) || matches!(b, b'$' | b'&' | b'+' | b':' | b'=' | b'@') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `application/x-www-form-urlencoded` text (`+` is a space);
/// a malformed escape stays as written.
pub fn query_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Splits a query string into decoded `(name, value)` pairs, in order.
pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (query_unescape(k), query_unescape(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_joins() {
        let u = Url::parse("https://API.example.org/v4/anime?q=a+b#frag").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.target.as_str()),
            (true, "api.example.org", 443, "/v4/anime?q=a+b")
        );
        assert_eq!(u.authority(), "api.example.org");
        assert_eq!(Url::parse("http://h:8080").unwrap().target, "/");
        assert_eq!(Url::parse("http://h?x=1").unwrap().target, "/?x=1");
        assert_eq!(Url::parse("http://h:8080/a").unwrap().authority(), "h:8080");
        let v6 = Url::parse("http://[::1]:9/x").unwrap();
        assert_eq!((v6.host.as_str(), v6.port, v6.authority()), ("::1", 9, "[::1]:9".to_string()));
        for bad in
            ["ftp://h/", "http://", "http://u:p@h/", "http://h:x/", "http://[::1/", "file:///etc", "http://h/a b"]
        {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(u.join("/other?x").unwrap().to_string(), "https://api.example.org/other?x");
        assert_eq!(u.join("//cdn.example.org/f").unwrap().to_string(), "https://cdn.example.org/f");
        assert_eq!(u.join("http://h2/x").unwrap().to_string(), "http://h2/x");
        assert_eq!(u.join("manga").unwrap().to_string(), "https://api.example.org/v4/manga");
    }

    #[test]
    fn escapes_like_go() {
        assert_eq!(query_escape("Sousou no Frieren: 2/3?&="), "Sousou+no+Frieren%3A+2%2F3%3F%26%3D");
        assert_eq!(query_escape("ü"), "%C3%BC");
        assert_eq!(path_escape("a b/c;d,e?f$&+:=@"), "a%20b%2Fc%3Bd%2Ce%3Ff$&+:=@");
        assert_eq!(query_unescape("a+b%2Fc%zz%"), "a b/c%zz%");
        assert_eq!(
            parse_query("filter%5Btext%5D=a+b&page%5Blimit%5D=5&flag"),
            vec![
                ("filter[text]".to_string(), "a b".to_string()),
                ("page[limit]".to_string(), "5".to_string()),
                ("flag".to_string(), String::new())
            ]
        );
    }
}
