//! Replays conformance/component/*.ndjson against the Rust host and the
//! Rust component side.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use lain_component::protocol::{read_frame, write_frame};
use lain_component::transport;
use lain_component::{Manifest, Process, Timeouts};
use lain_conformance::fixtures::{self, From, Line};
use lain_conformance::matcher::matches;
use lain_core::{Error, Provider, codes};

/// Replaces matcher tokens with concrete values so a scripted peer can
/// send a fixture frame.
fn concrete(v: &Value) -> Value {
    match v {
        Value::String(s) if s == "$string" => Value::String("fixture".into()),
        Value::String(s) if s == "$any" => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(concrete).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), concrete(v))).collect()),
        other => other.clone(),
    }
}

fn manifest_for(lines: &[Line], entry: &str, args: Vec<String>) -> Manifest {
    let hello = &lines[0].frame;
    let mut m = Manifest { id: hello["provider"].as_str().unwrap().to_string(), ..Manifest::default() };
    m.capabilities =
        hello["capabilities"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    m.execution.kind = "process".into();
    m.execution.entrypoint = entry.into();
    m.execution.args = args;
    m
}

/// Re-execs this test binary as a scripted component (like Go's
/// TestHelperProcess). The last two argv entries are the script mode
/// and the socket; libtest treats them as filters that match nothing.
fn self_exec_manifest(lines: &[Line], mode: &str) -> Manifest {
    let exe = std::env::current_exe().unwrap();
    let args = ["--exact", "scripted_component", "--include-ignored", "--quiet", "--test-threads=1", mode, "{sock}"];
    manifest_for(lines, exe.to_str().unwrap(), args.iter().map(|s| s.to_string()).collect())
}

/// The scripted component. It runs only when re-executed with a socket.
#[test]
#[ignore = "helper: runs only as a re-executed component"]
fn scripted_component() {
    let args: Vec<String> = std::env::args().collect();
    let Some(sock) = args.last().filter(|s| s.ends_with(".sock") || s.starts_with(r"\\.\pipe\")).cloned() else {
        return;
    };
    let mode = args[args.len() - 2].clone();
    if mode == "serve-echo" {
        lain_component::serve(&sock, &Echo).unwrap();
        return;
    }
    let (mut r, mut w) = transport::connect(&sock).unwrap();
    if let Some(name) = mode.strip_prefix("hang:") {
        let lines = fixtures::load(&fixtures::dir().join(name)).unwrap();
        write_frame(&mut w, &lines[0].frame).unwrap();
        // Read one request and never answer.
        let _ = read_frame(&mut r);
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if mode == "bad-hello" {
        write_frame(&mut w, &json!({"op": "hello", "provider": "someone-else", "protocol": 1, "capabilities": []}))
            .unwrap();
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    let lines = fixtures::load(&fixtures::dir().join(&mode)).unwrap();
    write_frame(&mut w, &lines[0].frame).unwrap();
    let mut i = 1;
    while i < lines.len() {
        let raw = match read_frame(&mut r) {
            Ok(raw) => raw,
            Err(_) => std::process::exit(0),
        };
        let got: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(lines[i].from, From::Host);
        if let Err(e) = matches(&lines[i].frame, &got) {
            // Exit non-zero: the host test sees the component die.
            eprintln!("host frame {i} differs: {e}");
            std::process::exit(3);
        }
        write_frame(&mut w, &concrete(&lines[i + 1].frame)).unwrap();
        i += 2;
    }
    // Keep serving health until the host hangs up.
    while let Ok(raw) = read_frame(&mut r) {
        let req: Value = serde_json::from_slice(&raw).unwrap();
        write_frame(&mut w, &json!({"seq": req["seq"], "output": {"ok": true}})).unwrap();
    }
}

fn scratch(tag: &str) -> PathBuf {
    lain_store::test_dir(tag)
}

#[test]
fn rust_host_sends_the_recorded_frames() {
    for file in fixtures::files() {
        let lines = fixtures::load(&file).unwrap();
        let dir = scratch("host-fixture");
        let mode = file.file_name().unwrap().to_str().unwrap().to_string();
        let p = Process::spawn(self_exec_manifest(&lines, &mode), &dir).unwrap();
        assert!(p.wait_ready(Duration::from_secs(20)), "{mode}: never ready: {:?}", p.health());
        let mut i = 1;
        while i < lines.len() {
            let host = &lines[i].frame;
            let want = &lines[i + 1].frame;
            let note = &lines[i].note;
            if host["op"] == "health" && host.get("cap").is_none() {
                assert!(p.health().is_ok(), "{mode} line {}: health failed ({note})", i + 1);
            } else {
                let got = p.invoke(
                    host["cap"].as_str().unwrap_or(""),
                    host["op"].as_str().unwrap_or(""),
                    host["input"].clone(),
                );
                match (want.get("output"), want.get("error")) {
                    (Some(out), _) => {
                        let out_got = got.unwrap_or_else(|e| panic!("{mode} line {}: {e} ({note})", i + 1));
                        matches(out, &out_got).unwrap_or_else(|e| panic!("{mode} line {}: {e}", i + 1));
                    }
                    (_, Some(err)) => {
                        let e = got.expect_err("want an error");
                        assert_eq!(e.code, err["code"].as_str().unwrap(), "{mode} line {}", i + 1);
                    }
                    _ => panic!("fixture answer has neither output nor error"),
                }
            }
            i += 2;
        }
        p.close();
        std::fs::remove_dir_all(dir).ok();
    }
}

/// A provider answering from a fixture's recorded outputs, to check
/// the Rust component side frames them exactly as Go does.
#[cfg(unix)]
struct Recorded {
    id: String,
    caps: Vec<String>,
    answers: std::collections::BTreeMap<String, Value>,
}

#[cfg(unix)]
impl Provider for Recorded {
    fn id(&self) -> &str {
        &self.id
    }
    fn capabilities(&self) -> Vec<String> {
        self.caps.clone()
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, _cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if !op.is_empty() && op != "identify" {
            return Err(Error::invalid(format!("no op {op}")));
        }
        if !input["path"].is_string() {
            return Err(Error::invalid("bad input"));
        }
        self.answers.get(&input.to_string()).cloned().ok_or_else(|| Error::new(codes::NOT_FOUND, "unrecorded"))
    }
}

#[cfg(unix)]
#[test]
fn rust_component_answers_like_go() {
    use std::io::BufReader;
    use std::os::unix::net::UnixListener;
    for file in fixtures::files() {
        let lines = fixtures::load(&file).unwrap();
        let hello = &lines[0].frame;
        let mut answers = std::collections::BTreeMap::new();
        for pair in lines[1..].chunks(2) {
            if let Some(out) = pair[1].frame.get("output") {
                answers.insert(pair[0].frame["input"].to_string(), out.clone());
            }
        }
        let provider = Recorded {
            id: hello["provider"].as_str().unwrap().into(),
            caps: hello["capabilities"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().into()).collect(),
            answers,
        };
        let dir = scratch("serve-fixture");
        // macOS's $TMPDIR is too deep for sun_path; /tmp is short everywhere.
        let sock = PathBuf::from(format!("/tmp/lain-fx-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let lis = UnixListener::bind(&sock).unwrap();
        let sock2 = sock.clone();
        let t = std::thread::spawn(move || lain_component::serve(&sock2.to_string_lossy(), &provider));
        let (conn, _) = lis.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut w = conn.try_clone().unwrap();
        let mut r = BufReader::new(conn);
        for (i, l) in lines.iter().enumerate() {
            match l.from {
                From::Host => write_frame(&mut w, &l.frame).unwrap(),
                From::Component => {
                    let got: Value = serde_json::from_slice(&read_frame(&mut r).unwrap()).unwrap();
                    matches(&l.frame, &got)
                        .unwrap_or_else(|e| panic!("{} line {}: {e} ({})", file.display(), i + 1, l.note));
                }
            }
        }
        drop(w);
        drop(r);
        t.join().unwrap().unwrap();
        std::fs::remove_file(&sock).ok();
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn timeouts_are_outcome_unknown_and_respawn() {
    let file = fixtures::files().into_iter().next().unwrap();
    let lines = fixtures::load(&file).unwrap();
    let name = file.file_name().unwrap().to_str().unwrap();
    let dir = scratch("hang");
    let t = Timeouts {
        invoke: Duration::from_millis(300),
        backoff_start: Duration::from_millis(50),
        ..Timeouts::default()
    };
    let p = Process::spawn_with(self_exec_manifest(&lines, &format!("hang:{name}")), &dir, t).unwrap();
    assert!(p.wait_ready(Duration::from_secs(20)));
    let first = p.pid();
    let e = p.invoke("lain.media.identify@1", "identify", json!({"path": "/x.mkv"})).unwrap_err();
    assert_eq!(e.code, codes::OUTCOME_UNKNOWN);
    // The broken peer is killed and a fresh one handshakes.
    assert!(p.health().is_err());
    std::thread::sleep(Duration::from_millis(200));
    assert!(p.wait_ready(Duration::from_secs(20)), "no respawn");
    assert_ne!(p.pid(), first);
    p.close();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_bad_hello_never_serves() {
    let file = fixtures::files().into_iter().next().unwrap();
    let lines = fixtures::load(&file).unwrap();
    let dir = scratch("bad-hello");
    let t =
        Timeouts { handshake: Duration::from_secs(2), backoff_start: Duration::from_millis(50), ..Timeouts::default() };
    let p = Process::spawn_with(self_exec_manifest(&lines, "bad-hello"), &dir, t).unwrap();
    assert!(!p.wait_ready(Duration::from_secs(2)));
    let e = p.health().unwrap_err();
    assert_eq!(e.code, codes::DEPENDENCY_UNAVAILABLE);
    assert_eq!(
        p.invoke("lain.media.identify@1", "identify", Value::Null).unwrap_err().code,
        codes::DEPENDENCY_UNAVAILABLE
    );
    p.close();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_unspawnable_entrypoint_degrades() {
    let dir = scratch("nospawn");
    let mut m = Manifest { id: "ghost".into(), capabilities: vec!["lain.x@1".into()], ..Manifest::default() };
    m.execution.kind = "process".into();
    m.execution.entrypoint = "/nonexistent/lain-ghost".into();
    let p = Process::spawn(m.clone(), &dir).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let e = p.health().unwrap_err();
    assert_eq!(e.code, codes::DEPENDENCY_UNAVAILABLE);
    p.close();
    m.execution.kind = "wasm".into();
    assert_eq!(Process::spawn(m, &dir).err().unwrap().code, codes::INVALID_MESSAGE);
    std::fs::remove_dir_all(dir).ok();
}

/// A Rust provider for the Rust-to-Rust test: echoes its input.
struct Echo;

impl Provider for Echo {
    fn id(&self) -> &str {
        "test-echo"
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["lain.test.echo@1".into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, _cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        match op {
            "echo" => Ok(json!({"echo": input})),
            "panic" => panic!("asked to"),
            _ => Err(Error::not_found(format!("no op {op}"))),
        }
    }
}

/// A Rust component (`serve`) under the Rust host, through the
/// platform transport: a Unix socket, or a named pipe on Windows.
#[test]
fn rust_component_under_the_rust_host() {
    let exe = std::env::current_exe().unwrap();
    let mut m =
        Manifest { id: "test-echo".into(), capabilities: vec!["lain.test.echo@1".into()], ..Manifest::default() };
    m.execution.kind = "process".into();
    m.execution.entrypoint = exe.to_string_lossy().into_owned();
    m.execution.args =
        ["--exact", "scripted_component", "--include-ignored", "--quiet", "--test-threads=1", "serve-echo", "{sock}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let dir = scratch("echo");
    let t = Timeouts { backoff_start: Duration::from_millis(50), ..Timeouts::default() };
    let p = Process::spawn_with(m, &dir, t).unwrap();
    assert!(p.wait_ready(Duration::from_secs(20)), "never ready: {:?}", p.health());
    p.health().unwrap();
    let big = "x".repeat(200_000);
    let out = p.invoke("lain.test.echo@1", "echo", json!({"n": 1, "big": big})).unwrap();
    assert_eq!(out["echo"]["n"], 1);
    assert_eq!(out["echo"]["big"].as_str().unwrap().len(), 200_000);
    assert_eq!(p.invoke("lain.test.echo@1", "nope", Value::Null).unwrap_err().code, codes::NOT_FOUND);
    assert_eq!(p.invoke("lain.test.echo@1", "panic", Value::Null).unwrap_err().code, codes::INTERNAL);
    assert_eq!(p.invoke("lain.other@1", "echo", Value::Null).unwrap_err().code, codes::INVALID_MESSAGE);
    // Survives errors and panics; respawns after the child dies.
    p.health().unwrap();
    let pid = p.pid();
    p.kill_child_for_test();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while (p.pid() == pid || p.health().is_err()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(p.pid(), pid, "no respawn");
    assert_eq!(p.invoke("lain.test.echo@1", "echo", json!(2)).unwrap()["echo"], 2);
    p.close();
    std::fs::remove_dir_all(dir).ok();
}
