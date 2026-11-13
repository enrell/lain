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
        let mut st = state.lock().unwrap();
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
            let saved = save(&mut st, &v["variables"]);
            return (200, serde_json::json!({"data": {"SaveMediaListEntry": saved}}).to_string());
        }
        let lists = if v["variables"]["type"] == "MANGA" { st.manga.clone() } else { st.anime.clone() };
        (200, serde_json::json!({"data": {"MediaListCollection": {"lists": lists}}}).to_string())
    }
}

/// `SaveMediaListEntry` as AniList runs it: the given fields change on
/// the stored entry (one is created in the first list for an unknown
/// media), COMPLETED fills the progress, and the whole entry comes back.
fn save(st: &mut AniListState, vars: &serde_json::Value) -> serde_json::Value {
    let id = vars["mediaId"].as_i64().unwrap_or(0);
    let mut at = None;
    for (set, lists) in [&st.anime, &st.manga].into_iter().enumerate() {
        for (li, l) in lists.as_array().into_iter().flatten().enumerate() {
            for (ei, e) in l["entries"].as_array().into_iter().flatten().enumerate() {
                if e["mediaId"] == id {
                    at = Some((set, li, ei));
                }
            }
        }
    }
    let (set, li, ei) = match at {
        Some(at) => at,
        None => {
            if st.anime.as_array().is_none_or(|a| a.is_empty()) {
                st.anime = serde_json::json!([{"name": "Planning", "isCustomList": false, "entries": []}]);
            }
            let list = st.anime[0]["entries"].as_array_mut().unwrap();
            list.push(al_entry(id, "ANIME", &format!("Media {id}"), "PLANNING", 0, 12));
            (0, 0, list.len() - 1)
        }
    };
    let entry = &mut (if set == 0 { &mut st.anime } else { &mut st.manga })[li]["entries"][ei];
    for (var, field) in [("progress", "progress"), ("progressVolumes", "progressVolumes"), ("status", "status")] {
        if !vars[var].is_null() {
            entry[field] = vars[var].clone();
        }
    }
    if let Some(raw) = vars["scoreRaw"].as_i64() {
        entry["score"] = serde_json::json!(raw as f64 / 10.0);
    }
    if vars["status"] == "COMPLETED" && vars["progress"].is_null() {
        let total = entry["media"]["episodes"].as_i64().or(entry["media"]["chapters"].as_i64()).unwrap_or(0);
        entry["progress"] = serde_json::json!(total);
    }
    entry.clone()
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
