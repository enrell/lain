//! The byte transport under the protocol: a Unix socket on Unix and a
//! named pipe on Windows (D-174, through tokio's
//! `tokio::net::windows::named_pipe`). The NDJSON frames are identical
//! on both; `{sock}` expands to the socket path or the pipe name.
//!
//! The host listens ([`Listener`]) and the component dials
//! ([`connect`]). Everything here is blocking with explicit timeouts,
//! like the rest of the host.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use lain_core::Error;

/// A connected peer, as the host sees it.
pub(crate) trait Conn: Send {
    /// One frame, waiting at most `timeout` (`None`: forever).
    fn read_frame(&mut self, timeout: Option<Duration>) -> io::Result<Vec<u8>>;
    /// One frame (bytes without the newline).
    fn write_frame(&mut self, frame: &[u8], timeout: Option<Duration>) -> io::Result<()>;
}

/// Whether an I/O error is a timeout.
pub(crate) fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The component side: dials the host at `address` (what `{sock}`
/// expanded to) and returns a buffered reader and a writer.
pub fn connect(address: &str) -> io::Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
    imp::connect(address)
}

pub(crate) use imp::Listener;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    /// The longest socket path every Unix accepts (`sun_path` is 104
    /// bytes on macOS and the BSDs, 108 on Linux, with the terminator).
    pub(crate) const MAX_SOCK_PATH: usize = 100;

    /// `<sock_dir>/<id>.sock`, or, when that is too long for `sun_path`
    /// (deep data dirs, macOS's `$TMPDIR`), a private 0700 directory
    /// under `/tmp` named after it.
    pub(crate) fn socket_path(sock_dir: &Path, id: &str) -> Result<PathBuf, Error> {
        let mkdir = |d: &Path| -> Result<(), Error> {
            std::fs::create_dir_all(d).map_err(|e| Error::internal(format!("component socket dir: {e}")))?;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
            Ok(())
        };
        let preferred = sock_dir.join(format!("{id}.sock"));
        if preferred.as_os_str().len() < MAX_SOCK_PATH {
            mkdir(sock_dir)?;
            return Ok(preferred);
        }
        let short = PathBuf::from("/tmp").join(format!("lain-{:016x}", fnv(&sock_dir.to_string_lossy())));
        mkdir(&short)?;
        let path = short.join(format!("{:016x}.sock", fnv(id)));
        if path.as_os_str().len() >= MAX_SOCK_PATH {
            return Err(Error::internal("component socket path too long"));
        }
        Ok(path)
    }

    pub(crate) struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        pub(crate) fn bind(sock_dir: &Path, id: &str) -> Result<Listener, Error> {
            let path = socket_path(sock_dir, id)?;
            // A stale socket from a crashed host would block the bind.
            let _ = std::fs::remove_file(&path);
            let inner = UnixListener::bind(&path).map_err(|e| Error::internal(format!("component socket: {e}")))?;
            inner.set_nonblocking(true).map_err(|e| Error::internal(e.to_string()))?;
            Ok(Listener { inner, path })
        }

        pub(crate) fn address(&self) -> String {
            self.path.to_string_lossy().into_owned()
        }

        pub(crate) fn accept(&self, deadline: Instant, give_up: &dyn Fn() -> bool) -> Result<Box<dyn Conn>, String> {
            loop {
                match self.inner.accept() {
                    Ok((s, _)) => {
                        s.set_nonblocking(false).map_err(|e| e.to_string())?;
                        let writer = s.try_clone().map_err(|e| e.to_string())?;
                        return Ok(Box::new(UnixConn { reader: BufReader::new(s), writer }));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if give_up() || Instant::now() >= deadline {
                            return Err("child never connected".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
        }

        pub(crate) fn cleanup(&self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    struct UnixConn {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Conn for UnixConn {
        fn read_frame(&mut self, timeout: Option<Duration>) -> io::Result<Vec<u8>> {
            self.reader.get_ref().set_read_timeout(timeout)?;
            crate::protocol::read_frame(&mut self.reader)
        }
        fn write_frame(&mut self, frame: &[u8], timeout: Option<Duration>) -> io::Result<()> {
            self.writer.set_write_timeout(timeout)?;
            self.writer.write_all(frame)?;
            self.writer.write_all(b"\n")?;
            self.writer.flush()
        }
    }

    pub(super) fn connect(address: &str) -> io::Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
        let s = UnixStream::connect(address)?;
        let w = s.try_clone()?;
        Ok((Box::new(BufReader::new(s)), Box::new(w)))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::io::BufReader;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader, ReadHalf, WriteHalf};
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use tokio::runtime::Runtime;

    use crate::protocol::MAX_FRAME;

    /// A pipe name unique to this host process and component.
    fn pipe_name(sock_dir: &Path, id: &str) -> String {
        format!(r"\\.\pipe\lain-{}-{:016x}-{:016x}", std::process::id(), fnv(&sock_dir.to_string_lossy()), fnv(id))
    }

    pub(crate) struct Listener {
        name: String,
        rt: Arc<Runtime>,
        /// The instance the next child connects to. One instance is
        /// always open, so the name stays ours between respawns.
        next: Mutex<Option<NamedPipeServer>>,
    }

    impl Listener {
        pub(crate) fn bind(sock_dir: &Path, id: &str) -> Result<Listener, Error> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .map_err(|e| Error::internal(format!("pipe runtime: {e}")))?;
            let name = pipe_name(sock_dir, id);
            // first_pipe_instance: fail instead of joining a pipe someone
            // else created under our name.
            let first = {
                let _g = rt.enter();
                ServerOptions::new()
                    .first_pipe_instance(true)
                    .reject_remote_clients(true)
                    .create(&name)
                    .map_err(|e| Error::internal(format!("component pipe: {e}")))?
            };
            Ok(Listener { name, rt: Arc::new(rt), next: Mutex::new(Some(first)) })
        }

        pub(crate) fn address(&self) -> String {
            self.name.clone()
        }

        fn instance(&self) -> Result<NamedPipeServer, String> {
            if let Some(s) = self.next.lock().unwrap_or_else(|e| e.into_inner()).take() {
                return Ok(s);
            }
            let _g = self.rt.enter();
            ServerOptions::new().reject_remote_clients(true).create(&self.name).map_err(|e| e.to_string())
        }

        pub(crate) fn accept(&self, deadline: Instant, give_up: &dyn Fn() -> bool) -> Result<Box<dyn Conn>, String> {
            let server = self.instance()?;
            loop {
                let r =
                    self.rt.block_on(async { tokio::time::timeout(Duration::from_millis(50), server.connect()).await });
                match r {
                    Ok(Ok(())) => break,
                    Ok(Err(e)) => return Err(e.to_string()),
                    Err(_) => {
                        if give_up() || Instant::now() >= deadline {
                            // Keep the unconnected instance for the next try.
                            *self.next.lock().unwrap_or_else(|e| e.into_inner()) = Some(server);
                            return Err("child never connected".into());
                        }
                    }
                }
            }
            // Open the next instance now so the name never lapses.
            if let Ok(s) = {
                let _g = self.rt.enter();
                ServerOptions::new().reject_remote_clients(true).create(&self.name)
            } {
                *self.next.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
            }
            let (r, w) = tokio::io::split(server);
            Ok(Box::new(PipeConn { rt: self.rt.clone(), reader: AsyncBufReader::new(r), writer: w }))
        }

        pub(crate) fn cleanup(&self) {
            self.next.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }

    struct PipeConn {
        rt: Arc<Runtime>,
        reader: AsyncBufReader<ReadHalf<NamedPipeServer>>,
        writer: WriteHalf<NamedPipeServer>,
    }

    fn timed_out() -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, "component pipe timed out")
    }

    impl Conn for PipeConn {
        fn read_frame(&mut self, timeout: Option<Duration>) -> io::Result<Vec<u8>> {
            let reader = &mut self.reader;
            let read = async move {
                let mut buf = Vec::new();
                loop {
                    let chunk = reader.fill_buf().await?;
                    if chunk.is_empty() {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof"));
                    }
                    if let Some(pos) = chunk.iter().position(|b| *b == b'\n') {
                        buf.extend_from_slice(&chunk[..pos]);
                        reader.consume(pos + 1);
                        return Ok(buf);
                    }
                    let n = chunk.len();
                    buf.extend_from_slice(chunk);
                    reader.consume(n);
                    if buf.len() > MAX_FRAME {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("frame exceeds {MAX_FRAME} bytes"),
                        ));
                    }
                }
            };
            match timeout {
                Some(t) => self.rt.block_on(async { tokio::time::timeout(t, read).await.map_err(|_| timed_out())? }),
                None => self.rt.block_on(read),
            }
        }

        fn write_frame(&mut self, frame: &[u8], timeout: Option<Duration>) -> io::Result<()> {
            let writer = &mut self.writer;
            let write = async move {
                writer.write_all(frame).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await
            };
            match timeout {
                Some(t) => self.rt.block_on(async { tokio::time::timeout(t, write).await.map_err(|_| timed_out())? }),
                None => self.rt.block_on(write),
            }
        }
    }

    pub(super) fn connect(address: &str) -> io::Result<(Box<dyn BufRead + Send>, Box<dyn Write + Send>)> {
        // A pipe client is an ordinary file handle; a busy pipe (all
        // instances taken) is retried briefly.
        const ERROR_PIPE_BUSY: i32 = 231;
        let deadline = Instant::now() + Duration::from_secs(5);
        let f = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(address) {
                Ok(f) => break f,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        };
        let w = f.try_clone()?;
        Ok((Box::new(BufReader::new(f)), Box::new(w)))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::imp::*;
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn long_socket_dirs_fall_back_to_a_private_tmp_dir() {
        let short = socket_path(Path::new("/tmp/lain-short-test"), "a").unwrap();
        assert_eq!(short, PathBuf::from("/tmp/lain-short-test/a.sock"));
        let deep = PathBuf::from("/tmp").join("x".repeat(120));
        let p = socket_path(&deep, "community.anime-parser").unwrap();
        assert!(p.starts_with("/tmp") && p.as_os_str().len() < MAX_SOCK_PATH, "{}", p.display());
        assert_eq!(p, socket_path(&deep, "community.anime-parser").unwrap(), "stable");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let _ = std::fs::remove_dir_all("/tmp/lain-short-test");
    }
}
