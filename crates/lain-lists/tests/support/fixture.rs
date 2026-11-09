//! A loopback HTTP/1.1 fixture server (std only) standing in for a
//! remote platform: every request is recorded and answered by a
//! swappable handler. Shared with the server's tests by `#[path]`.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One request as the fixture saw it.
#[derive(Debug, Clone, Default)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

pub type Handler = Box<dyn Fn(&Recorded) -> (u16, String) + Send>;

pub struct Fixture {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    handler: Arc<Mutex<Handler>>,
    stop: Arc<AtomicBool>,
}

impl Fixture {
    pub fn start(handler: impl Fn(&Recorded) -> (u16, String) + Send + 'static) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Mutex<Handler>> = Arc::new(Mutex::new(Box::new(handler)));
        let stop = Arc::new(AtomicBool::new(false));
        let (reqs, h, st) = (requests.clone(), handler.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if st.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut conn) = conn else { continue };
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if r.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let mut rec = Recorded {
                    method: parts.next().unwrap_or("").into(),
                    path: parts.next().unwrap_or("").into(),
                    ..Recorded::default()
                };
                let mut len = 0;
                loop {
                    line.clear();
                    if r.read_line(&mut line).is_err() {
                        break;
                    }
                    let l = line.trim_end();
                    if l.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = l.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        rec.headers.push((k.trim().into(), v.trim().into()));
                    }
                }
                let mut body = vec![0; len];
                let _ = r.read_exact(&mut body);
                rec.body = String::from_utf8_lossy(&body).into_owned();
                let (status, out) = (h.lock().unwrap())(&rec);
                reqs.lock().unwrap().push(rec);
                let _ = write!(
                    conn,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
            }
        });
        Fixture { url, requests, handler, stop }
    }

    pub fn set(&self, handler: impl Fn(&Recorded) -> (u16, String) + Send + 'static) {
        *self.handler.lock().unwrap() = Box::new(handler);
    }

    pub fn taken(&self) -> Vec<Recorded> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

/// A fake AniList: `/oauth/token` trades code `good` for a JWT-shaped
/// token, GraphQL answers `Viewer`, the two collections and
/// `SaveMediaListEntry`; a token other than the issued one is a 401.
pub struct AniListState {
    pub anime: serde_json::Value,
    pub manga: serde_json::Value,
    pub token_ok: bool,
}

pub fn token() -> String {
    // exp far in the future, as AniList's year-lived tokens are.
    let claims = "eyJleHAiOjQxMDI0NDQ4MDB9"; // {"exp":4102444800}
    format!("eyJhbGciOiJub25lIn0.{claims}.sig")
}

pub fn anilist(state: Arc<Mutex<AniListState>>) -> impl Fn(&Recorded) -> (u16, String) + Send + 'static {
    move |r: &Recorded| {
        let st = state.lock().unwrap();
        if r.path.ends_with("/oauth/token") {
            let v = r.json();
            if v["code"] != "good" {
                return (
                    400,
                    r#"{"error":"invalid_grant","error_description":"The authorization code is invalid"}"#.into(),
                );
            }
            return (
                200,
                serde_json::json!({"access_token": token(), "token_type": "Bearer", "expires_in": 31536000})
                    .to_string(),
            );
        }
        if !st.token_ok || r.header("authorization") != format!("Bearer {}", token()) {
            return (401, r#"{"errors":[{"message":"Invalid token"}]}"#.into());
        }
        let v = r.json();
        let q = v["query"].as_str().unwrap_or("");
        if q.contains("Viewer") {
            return (200, r#"{"data":{"Viewer":{"id":42,"name":"watcher"}}}"#.into());
        }
        if q.contains("SaveMediaListEntry") {
            let vars = &v["variables"];
            return (
                200,
                serde_json::json!({"data": {"SaveMediaListEntry": {"progress": vars["progress"], "status": vars["status"].as_str().unwrap_or("CURRENT")}}})
                    .to_string(),
            );
        }
        let lists = if v["variables"]["type"] == "MANGA" { st.manga.clone() } else { st.anime.clone() };
        (200, serde_json::json!({"data": {"MediaListCollection": {"lists": lists}}}).to_string())
    }
}

/// One AniList list entry.
pub fn al_entry(media_id: i64, kind: &str, title: &str, status: &str, progress: i64, total: i64) -> serde_json::Value {
    serde_json::json!({
        "mediaId": media_id, "status": status, "progress": progress, "progressVolumes": null,
        "score": 8.5, "repeat": 0, "notes": null,
        "startedAt": {"year": 2024, "month": 1, "day": 5}, "completedAt": {"year": null, "month": null, "day": null},
        "media": {"id": media_id, "type": kind, "format": "TV", "episodes": if kind == "ANIME" { serde_json::json!(total) } else { serde_json::Value::Null },
                  "chapters": if kind == "MANGA" { serde_json::json!(total) } else { serde_json::Value::Null },
                  "title": {"romaji": format!("{title} (romaji)"), "english": title, "native": null},
                  "coverImage": {"extraLarge": format!("https://img.example/{media_id}.jpg"), "large": null}}
    })
}
