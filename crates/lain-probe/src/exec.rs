//! Running an external tool (ffprobe, ffmpeg) with a deadline. The
//! child's pipes are drained on their own threads so a chatty tool can
//! never block on a full pipe, and a tool that outlives its deadline is
//! killed, never waited on forever.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// What a finished tool left behind.
#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    /// The exit status and the trimmed head of stderr, for error texts.
    pub fn describe(&self) -> String {
        let err = String::from_utf8_lossy(&self.stderr).trim().to_string();
        if err.is_empty() { self.status.to_string() } else { format!("{}: {err}", self.status) }
    }
}

/// Why a tool did not produce an [`Output`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The binary could not be started (absent, not executable).
    Spawn(String),
    /// The deadline passed; the child was killed.
    Timeout,
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Spawn(e) => write!(f, "{e}"),
            ExecError::Timeout => write!(f, "timed out"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Bytes kept of a tool's stderr: enough for a diagnosis, bounded.
const STDERR_CAP: usize = 64 << 10;
/// Bytes kept of a tool's stdout (ffprobe JSON, encoder lists).
const STDOUT_CAP: usize = 16 << 20;

/// How long a finished tool's pipes may stay open: a grandchild that
/// inherited them (a shell's `sleep`, a daemon) must not hold the
/// caller hostage.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Reads a pipe to its end on a thread; the result arrives on the
/// channel, so the caller can stop waiting.
pub fn drain(mut r: impl Read + Send + 'static, cap: usize) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(out.len());
                    out.extend_from_slice(&buf[..n.min(room)]);
                }
            }
        }
        let _ = tx.send(out);
    });
    rx
}

/// What a drain read, waiting at most [`PIPE_GRACE`].
pub fn collect(rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>) -> Vec<u8> {
    rx.and_then(|r| r.recv_timeout(PIPE_GRACE).ok()).unwrap_or_default()
}

/// Runs `cmd` to completion or until `timeout`, feeding `stdin` when
/// given (else the child reads nothing).
pub fn run(mut cmd: Command, stdin: Option<Box<dyn Read + Send>>, timeout: Duration) -> Result<Output, ExecError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn().map_err(|e| ExecError::Spawn(e.to_string()))?;
    let feeder = match (stdin, child.stdin.take()) {
        (Some(mut src), Some(mut dst)) => Some(std::thread::spawn(move || {
            let _ = std::io::copy(&mut src, &mut dst);
            let _ = dst.flush();
        })),
        _ => None,
    };
    let out = child.stdout.take().map(|s| drain(s, STDOUT_CAP));
    let err = child.stderr.take().map(|s| drain(s, STDERR_CAP));
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };
    // A timed-out child is gone; nothing it left behind is waited on.
    let Some(status) = status else { return Err(ExecError::Timeout) };
    let stdout = collect(out);
    let stderr = collect(err);
    if let Some(f) = feeder
        && f.is_finished()
    {
        let _ = f.join();
    }
    Ok(Output { status, stdout, stderr })
}

/// Resolves a tool like the shell does: a name with a path separator is
/// taken as given, a bare name is searched on `PATH` (with `.exe` on
/// Windows). `None` when nothing executable is found.
pub fn look_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let p = Path::new(name);
    if p.components().count() > 1 || p.is_absolute() {
        return executable(p).then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if executable(&candidate) {
            return Some(candidate);
        }
        if cfg!(windows) && candidate.extension().is_none() {
            let exe = candidate.with_extension("exe");
            if executable(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

fn executable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_is_a_spawn_error() {
        let e = run(Command::new("lain-no-such-tool-xyz"), None, Duration::from_secs(1)).unwrap_err();
        assert!(matches!(e, ExecError::Spawn(_)), "{e:?}");
        assert!(look_path("lain-no-such-tool-xyz").is_none());
        assert!(look_path("").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn deadline_kills_the_child() {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "echo started; sleep 30; true"]);
        let start = Instant::now();
        assert_eq!(run(c, None, Duration::from_millis(200)).unwrap_err(), ExecError::Timeout);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn output_and_stdin_round_trip() {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "cat; echo oops >&2; exit 3"]);
        let out = run(c, Some(Box::new(std::io::Cursor::new(b"hello".to_vec()))), Duration::from_secs(5)).unwrap();
        assert_eq!(out.stdout, b"hello");
        assert_eq!(out.status.code(), Some(3));
        assert!(out.describe().contains("oops"));
        assert!(look_path("sh").is_some());
    }
}
