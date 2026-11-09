//! One supervised ffmpeg: stdout carries `-progress` blocks, stderr is
//! kept (bounded) for the failure text, and the child can be killed or
//! paused from other threads while its runner waits.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One reading of ffmpeg's `-progress` stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    pub fraction: Option<f64>,
    pub fps: f64,
    pub speed: f64,
    pub bitrate_kbps: i64,
}

/// `1234.5kbits/s` into kbps.
pub fn parse_bitrate_kbps(v: &str) -> Option<i64> {
    let t = v.trim().trim_end_matches("kbits/s").trim_end_matches("kbit/s").trim();
    if t.is_empty() || t == "N/A" {
        return None;
    }
    t.parse::<f64>().ok().filter(|k| k.is_finite()).map(|k| (k + 0.5) as i64)
}

/// Drains a progress stream block by block (each ends with
/// `progress=`); a block is reported only when it carries a fact. An
/// unknown duration never invents a fraction (D-039).
pub fn consume_progress(r: impl Read, total_sec: f64, mut on: impl FnMut(Sample)) {
    let mut block = Sample::default();
    let mut pos_us: i64 = 0;
    for line in BufReader::new(r).lines() {
        let Ok(line) = line else { break };
        let Some((k, v)) = line.trim().split_once('=') else { continue };
        match k {
            "out_time_us" | "out_time_ms" => {
                if let Ok(us) = v.trim().parse::<i64>()
                    && us >= 0
                {
                    pos_us = us;
                }
            }
            "fps" => {
                if let Ok(f) = v.trim().parse::<f64>()
                    && f > 0.0
                {
                    block.fps = f;
                }
            }
            "speed" => {
                if let Ok(s) = v.trim().trim_end_matches('x').parse::<f64>()
                    && s > 0.0
                {
                    block.speed = s;
                }
            }
            "bitrate" => {
                if let Some(k) = parse_bitrate_kbps(v) {
                    block.bitrate_kbps = k;
                }
            }
            "progress" => {
                let mut s = std::mem::take(&mut block);
                if total_sec > 0.0 {
                    s.fraction = Some((pos_us as f64 / (total_sec * 1e6)).clamp(0.0, 1.0));
                }
                if v.trim() == "end" {
                    s.fraction = Some(1.0);
                }
                if s.fraction.is_some() || s.fps > 0.0 || s.bitrate_kbps > 0 || s.speed > 0.0 {
                    on(s);
                }
            }
            _ => {}
        }
    }
}

/// A running child others can stop or pause.
#[derive(Clone)]
pub struct Handle {
    child: Arc<Mutex<Child>>,
    pub pid: u32,
}

impl Handle {
    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }

    /// SIGSTOP (throttling); unsupported off Unix, where the process
    /// simply never pauses (like Go).
    pub fn pause(&self) -> bool {
        signal(self.pid, true)
    }

    pub fn resume(&self) -> bool {
        signal(self.pid, false)
    }
}

#[cfg(unix)]
fn signal(pid: u32, stop: bool) -> bool {
    let sig = if stop { libc::SIGSTOP } else { libc::SIGCONT };
    // SAFETY: kill(2) with a pid we spawned and still own; no memory is
    // shared with the call.
    unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
}

#[cfg(not(unix))]
fn signal(_: u32, _: bool) -> bool {
    false
}

/// Why a supervised run failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Spawn(String),
    /// Killed by the deadline or a stop request.
    Stopped,
    Failed(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Spawn(e) => write!(f, "{e}"),
            RunError::Stopped => write!(f, "stopped"),
            RunError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// Runs `cmd` to completion. `on_start` receives the handle right after
/// spawn (it may kill at once); `on_sample` gets progress when given;
/// `stop` is polled and kills the child when it turns true; `deadline`
/// bounds the run.
pub fn run(
    mut cmd: Command,
    total_sec: f64,
    deadline: Option<Duration>,
    on_start: impl FnOnce(&Handle),
    on_sample: Option<Box<dyn FnMut(Sample) + Send>>,
    stop: impl Fn() -> bool,
) -> Result<(), RunError> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| RunError::Spawn(e.to_string()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pid = child.id();
    let handle = Handle { child: Arc::new(Mutex::new(child)), pid };
    on_start(&handle);
    let out = stdout.map(|s| {
        std::thread::spawn(move || match on_sample {
            Some(mut f) => consume_progress(s, total_sec, &mut *f),
            None => {
                let _ = std::io::copy(&mut BufReader::new(s), &mut std::io::sink());
            }
        })
    });
    let err = stderr.map(|mut s| {
        std::thread::spawn(move || {
            let mut keep = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let room = (16usize << 10).saturating_sub(keep.len());
                keep.extend_from_slice(&buf[..n.min(room)]);
            }
            keep
        })
    });
    let start = Instant::now();
    let mut stopped = false;
    let status = loop {
        let polled = handle.child.lock().map(|mut c| c.try_wait());
        match polled {
            Ok(Ok(Some(st))) => break Some(st),
            Ok(Ok(None)) => {}
            _ => break None,
        }
        if !stopped && (stop() || deadline.is_some_and(|d| start.elapsed() >= d)) {
            stopped = true;
            // A paused child must run again to die cleanly on platforms
            // that queue the kill.
            handle.resume();
            handle.kill();
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    if let Some(h) = out {
        let _ = h.join();
    }
    let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    if stopped {
        return Err(RunError::Stopped);
    }
    match status {
        Some(st) if st.success() => Ok(()),
        Some(st) => Err(RunError::Failed(format!("{st}: {}", String::from_utf8_lossy(&stderr).trim()))),
        None => Err(RunError::Failed("ffmpeg could not be waited on".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_blocks() {
        let text = "frame=1\nfps=24.5\nbitrate=1234.6kbits/s\nout_time_us=5000000\nspeed=2.0x\nprogress=continue\nout_time_us=10000000\nprogress=end\n";
        let mut got = Vec::new();
        consume_progress(text.as_bytes(), 10.0, |s| got.push(s));
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].fraction, got[0].fps, got[0].bitrate_kbps, got[0].speed), (Some(0.5), 24.5, 1235, 2.0));
        assert_eq!(got[1].fraction, Some(1.0));
        let mut none = Vec::new();
        consume_progress("out_time_us=5\nprogress=continue\n".as_bytes(), 0.0, |s| none.push(s));
        assert!(none.is_empty(), "no duration and no metric reports nothing");
        assert_eq!(parse_bitrate_kbps("N/A"), None);
    }

    #[cfg(unix)]
    #[test]
    fn stop_kills_and_failure_keeps_stderr() {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "sleep 30"]);
        let flag = std::sync::atomic::AtomicBool::new(false);
        let t = Instant::now();
        let e = run(
            c,
            0.0,
            None,
            |_| flag.store(true, std::sync::atomic::Ordering::SeqCst),
            None,
            || flag.load(std::sync::atomic::Ordering::SeqCst),
        );
        assert_eq!(e, Err(RunError::Stopped));
        assert!(t.elapsed() < Duration::from_secs(5));
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "echo boom >&2; exit 2"]);
        match run(c, 0.0, None, |_| {}, None, || false) {
            Err(RunError::Failed(m)) => assert!(m.contains("boom"), "{m}"),
            other => panic!("{other:?}"),
        }
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "sleep 30"]);
        assert_eq!(run(c, 0.0, Some(Duration::from_millis(100)), |_| {}, None, || false), Err(RunError::Stopped));
    }
}
