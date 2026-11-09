//! Maps a torrent's byte space onto its files under the download dir.
//! Files are created sparse on first write; reads past what exists
//! return zeros, so a partial piece simply fails verification.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::Info;
use crate::metainfo::sha1;

pub(crate) struct Storage {
    dir: PathBuf,
    info: Arc<Info>,
    files: Mutex<HashMap<usize, Arc<File>>>,
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, off)
}
#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::write_at(f, buf, off)
}
#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(f, buf, off)
}
#[cfg(windows)]
fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_write(f, buf, off)
}

impl Storage {
    pub fn new(dir: PathBuf, info: Arc<Info>) -> Storage {
        Storage { dir, info, files: Mutex::new(HashMap::new()) }
    }

    /// The on-disk path of file i.
    pub fn path(&self, i: usize) -> PathBuf {
        let mut p = self.dir.clone();
        for c in &self.info.files[i].path {
            p.push(c);
        }
        p
    }

    fn open(&self, i: usize, create: bool) -> io::Result<Arc<File>> {
        let mut files = self.files.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = files.get(&i) {
            return Ok(f.clone());
        }
        let p = self.path(i);
        if let Ok(md) = fs::symlink_metadata(&p)
            && !md.is_file()
        {
            return Err(io::Error::other(format!("{} is not a regular file", p.display())));
        }
        if !create {
            // Read-only handles are not cached.
            return File::open(&p).map(Arc::new);
        }
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&p)?;
        let want = self.info.files[i].length as u64;
        if f.metadata().map(|m| m.len() > want).unwrap_or(false) {
            let _ = f.set_len(want);
        }
        let f = Arc::new(f);
        files.insert(i, f.clone());
        Ok(f)
    }

    /// Calls `f(file, file_off, buf_off, len)` for every file segment
    /// covering `[off, off+n)`.
    fn span(&self, off: i64, n: i64, mut f: impl FnMut(usize, u64, usize, usize) -> io::Result<()>) -> io::Result<()> {
        let mut done = 0i64;
        for (i, file) in self.info.files.iter().enumerate() {
            if n - done == 0 {
                break;
            }
            let end = file.offset + file.length;
            if off + done >= end || file.length == 0 || off + done < file.offset {
                continue;
            }
            let start = off + done - file.offset;
            let len = (end - (off + done)).min(n - done);
            f(i, start as u64, done as usize, len as usize)?;
            done += len;
        }
        if done != n {
            return Err(io::Error::other("range outside torrent"));
        }
        Ok(())
    }

    pub fn write_at(&self, b: &[u8], off: i64) -> io::Result<()> {
        self.span(off, b.len() as i64, |i, foff, boff, len| {
            let f = self.open(i, true)?;
            let mut src = &b[boff..boff + len];
            let mut at = foff;
            while !src.is_empty() {
                let n = write_at(&f, src, at)?;
                if n == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                src = &src[n..];
                at += n as u64;
            }
            Ok(())
        })
    }

    /// Fills b from torrent offset off; missing data reads as zeros.
    pub fn read_at(&self, b: &mut [u8], off: i64) -> io::Result<()> {
        let len = b.len() as i64;
        self.span(off, len, |i, foff, boff, len| {
            let dst = &mut b[boff..boff + len];
            let f = match self.open(i, false) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    dst.fill(0);
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            let mut got = 0;
            while got < dst.len() {
                let n = read_at(&f, &mut dst[got..], foff + got as u64)?;
                if n == 0 {
                    break;
                }
                got += n;
            }
            dst[got..].fill(0);
            Ok(())
        })
    }

    /// Hashes piece n against the metainfo.
    pub fn verify(&self, n: usize) -> io::Result<bool> {
        let mut buf = vec![0u8; self.info.piece_size(n) as usize];
        self.read_at(&mut buf, n as i64 * self.info.piece_length)?;
        Ok(sha1(&buf) == self.info.pieces[n])
    }

    /// Whether piece n touches only files that exist (a cheap pre-check).
    pub fn piece_on_disk(&self, n: usize) -> bool {
        let mut ok = true;
        let _ = self.span(n as i64 * self.info.piece_length, self.info.piece_size(n), |i, _, _, _| {
            if !self.path(i).exists() {
                ok = false;
            }
            Ok(())
        });
        ok
    }

    /// Whether every file exists at its full size: the condition under
    /// which a saved resume bitfield is trusted.
    pub fn complete(&self) -> bool {
        self.info
            .files
            .iter()
            .enumerate()
            .all(|(i, f)| fs::metadata(self.path(i)).map(|m| m.len() == f.length as u64).unwrap_or(false))
    }

    /// Releases cached handles.
    pub fn close(&self) {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Deletes the torrent's files and any directories left empty.
    pub fn remove(&self) -> io::Result<()> {
        self.close();
        let mut first = None;
        for i in 0..self.info.files.len() {
            if let Err(e) = fs::remove_file(self.path(i))
                && e.kind() != io::ErrorKind::NotFound
                && first.is_none()
            {
                first = Some(e);
            }
        }
        if self.info.multi {
            let mut dirs = Vec::new();
            let mut stack = vec![self.dir.join(&self.info.name)];
            while let Some(d) = stack.pop() {
                if let Ok(rd) = fs::read_dir(&d) {
                    for e in rd.flatten() {
                        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            stack.push(e.path());
                        }
                    }
                    dirs.push(d);
                }
            }
            // Deepest first.
            dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
            for d in dirs {
                let _ = fs::remove_dir(d);
            }
        }
        first.map_or(Ok(()), Err)
    }
}
