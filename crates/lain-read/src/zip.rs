//! A ZIP reader for comic archives (D-195): the central directory
//! (ZIP64 included), stored and deflated members, CRC-32 checked.
//! Encrypted members and other compression methods are refused. The
//! inflater is `miniz_oxide`; everything else is here.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// A ZIP failure, with a message fit for an `invalid-archive` error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipError(pub String);

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "zip: {}", self.0)
    }
}

impl std::error::Error for ZipError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ZipError> {
    Err(ZipError(msg.into()))
}

fn io(e: std::io::Error) -> ZipError {
    ZipError(e.to_string())
}

/// One member, as the central directory describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    header_offset: u64,
}

/// The central directory may not exceed this (a hostile file cannot
/// make the reader allocate gigabytes for names).
const MAX_DIRECTORY: u64 = 64 << 20;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[i..i + 8]);
    u64::from_le_bytes(a)
}

/// An opened archive: its path and member list. Reads reopen the file,
/// so an `Archive` is cheap to keep and safe to share.
#[derive(Debug, Clone)]
pub struct Archive {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Archive, ZipError> {
        let mut f = File::open(path).map_err(io)?;
        let len = f.metadata().map_err(io)?.len();
        let (cd_offset, cd_size, count) = directory(&mut f, len)?;
        if cd_size > MAX_DIRECTORY || cd_offset.checked_add(cd_size).is_none_or(|end| end > len) {
            return err("central directory out of bounds");
        }
        f.seek(SeekFrom::Start(cd_offset)).map_err(io)?;
        let mut cd = vec![0u8; cd_size as usize];
        f.read_exact(&mut cd).map_err(io)?;
        let mut entries = Vec::with_capacity(count.min(1 << 16) as usize);
        let mut i = 0usize;
        while i + 46 <= cd.len() && u32_at(&cd, i) == CENTRAL_SIG {
            let flags = u16_at(&cd, i + 8);
            let method = u16_at(&cd, i + 10);
            let crc32 = u32_at(&cd, i + 16);
            let mut compressed_size = u64::from(u32_at(&cd, i + 20));
            let mut size = u64::from(u32_at(&cd, i + 24));
            let (n, e, c) = (u16_at(&cd, i + 28) as usize, u16_at(&cd, i + 30) as usize, u16_at(&cd, i + 32) as usize);
            let mut header_offset = u64::from(u32_at(&cd, i + 42));
            let end = i + 46 + n + e + c;
            if end > cd.len() {
                return err("truncated central directory");
            }
            let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + n]).into_owned();
            // ZIP64 extra field: the 0xffffffff values, in order.
            let mut extra = &cd[i + 46 + n..i + 46 + n + e];
            while extra.len() >= 4 {
                let (id, sz) = (u16_at(extra, 0), u16_at(extra, 2) as usize);
                if 4 + sz > extra.len() {
                    break;
                }
                if id == 0x0001 {
                    let mut field = &extra[4..4 + sz];
                    for v in [&mut size, &mut compressed_size, &mut header_offset] {
                        if *v == 0xffff_ffff && field.len() >= 8 {
                            *v = u64_at(field, 0);
                            field = &field[8..];
                        }
                    }
                }
                extra = &extra[4 + sz..];
            }
            entries.push(Entry { name, method, flags, crc32, compressed_size, size, header_offset });
            i = end;
        }
        // Some writers wrap the 16-bit count past 65535 entries, so only
        // a directory that ends before its first declared entry fails.
        if entries.is_empty() && count > 0 {
            return err("truncated central directory");
        }
        Ok(Archive { path: path.to_path_buf(), entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The member named `name` (exact match).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    fn data(&self, e: &Entry) -> Result<File, ZipError> {
        if e.flags & 1 != 0 {
            return err("encrypted member");
        }
        if e.method != 0 && e.method != 8 {
            return err(format!("unsupported compression method {}", e.method));
        }
        let mut f = File::open(&self.path).map_err(io)?;
        f.seek(SeekFrom::Start(e.header_offset)).map_err(io)?;
        let mut h = [0u8; 30];
        f.read_exact(&mut h).map_err(|_| ZipError("truncated local header".into()))?;
        if u32_at(&h, 0) != LOCAL_SIG {
            return err("bad local header");
        }
        let skip = u64::from(u16_at(&h, 26)) + u64::from(u16_at(&h, 28));
        f.seek(SeekFrom::Current(skip as i64)).map_err(io)?;
        Ok(f)
    }

    /// The whole member, at most `limit` bytes; its CRC-32 is checked.
    pub fn read(&self, e: &Entry, limit: u64) -> Result<Vec<u8>, ZipError> {
        if e.size > limit {
            return err(format!("member exceeds {limit} bytes"));
        }
        let f = self.data(e)?;
        let mut packed = Vec::with_capacity(e.compressed_size.min(limit) as usize);
        f.take(e.compressed_size).read_to_end(&mut packed).map_err(io)?;
        if (packed.len() as u64) < e.compressed_size {
            return err("truncated member");
        }
        let out = match e.method {
            0 => packed,
            _ => miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, limit as usize)
                .map_err(|x| ZipError(format!("inflate: {:?}", x.status)))?,
        };
        if out.len() as u64 != e.size || crc32(&out) != e.crc32 {
            return err("checksum mismatch");
        }
        Ok(out)
    }

    /// The first `n` bytes of a member (fewer if it is shorter), for
    /// header probes; nothing past them is inflated.
    pub fn head(&self, e: &Entry, n: usize) -> Result<Vec<u8>, ZipError> {
        let f = self.data(e)?;
        let mut packed = f.take(e.compressed_size);
        if e.method == 0 {
            let mut out = Vec::new();
            packed.take(n as u64).read_to_end(&mut out).map_err(io)?;
            return Ok(out);
        }
        use miniz_oxide::inflate::stream::{InflateState, inflate};
        use miniz_oxide::{DataFormat, MZFlush, MZStatus};
        let mut state = InflateState::new_boxed(DataFormat::Raw);
        let mut out = vec![0u8; n];
        let (mut written, mut buf, mut have) = (0usize, vec![0u8; 64 << 10], 0usize);
        let mut eof = false;
        while written < n {
            if have == 0 && !eof {
                have = packed.read(&mut buf).map_err(io)?;
                eof = have == 0;
            }
            let flush = if eof { MZFlush::Finish } else { MZFlush::None };
            let r = inflate(&mut state, &buf[..have], &mut out[written..], flush);
            buf.copy_within(r.bytes_consumed..have, 0);
            have -= r.bytes_consumed;
            written += r.bytes_written;
            match r.status {
                Ok(MZStatus::StreamEnd) => break,
                Ok(_) if eof && r.bytes_written == 0 && r.bytes_consumed == 0 => break,
                Ok(_) => {}
                Err(miniz_oxide::MZError::Buf) if eof => break,
                Err(miniz_oxide::MZError::Buf) => {}
                Err(x) => return err(format!("inflate: {x:?}")),
            }
        }
        out.truncate(written);
        Ok(out)
    }
}

/// Finds the end-of-central-directory record: (offset, size, count).
fn directory(f: &mut File, len: u64) -> Result<(u64, u64, u64), ZipError> {
    if len < 22 {
        return err("not a zip file");
    }
    let tail_len = len.min(22 + 65_535 + 20);
    f.seek(SeekFrom::Start(len - tail_len)).map_err(io)?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail).map_err(io)?;
    let Some(at) = (0..=tail.len() - 22).rev().find(|&i| u32_at(&tail, i) == EOCD_SIG) else {
        return err("not a zip file");
    };
    let count = u64::from(u16_at(&tail, at + 10));
    let size = u64::from(u32_at(&tail, at + 12));
    let offset = u64::from(u32_at(&tail, at + 16));
    if (count == 0xffff || size == 0xffff_ffff || offset == 0xffff_ffff)
        && at >= 20
        && u32_at(&tail, at - 20) == ZIP64_LOCATOR_SIG
    {
        let z64 = u64_at(&tail, at - 20 + 8);
        if z64.checked_add(56).is_none_or(|end| end > len) {
            return err("zip64 record out of bounds");
        }
        let mut r = [0u8; 56];
        f.seek(SeekFrom::Start(z64)).map_err(io)?;
        f.read_exact(&mut r).map_err(io)?;
        if u32_at(&r, 0) != ZIP64_EOCD_SIG {
            return err("bad zip64 record");
        }
        return Ok((u64_at(&r, 48), u64_at(&r, 40), u64_at(&r, 32)));
    }
    Ok((offset, size, count))
}

static CRC_TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();

/// CRC-32 (IEEE), as ZIP and PNG use it.
pub fn crc32(data: &[u8]) -> u32 {
    let t = CRC_TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| t[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8))
}

/// A minimal ZIP writer for tests and fixtures: stored or deflated
/// members, no ZIP64.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    /// Adds a member; `deflate` compresses it.
    pub fn add(&mut self, name: &str, data: &[u8], deflate: bool) -> &mut Writer {
        let crc = crc32(data);
        let (method, packed) =
            if deflate { (8u16, miniz_oxide::deflate::compress_to_vec(data, 6)) } else { (0u16, data.to_vec()) };
        let offset = self.out.len() as u32;
        let mut h = Vec::new();
        h.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes());
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&[0, 0, 0x21, 0]); // time, date
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&h);
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(&packed);

        let c = &mut self.central;
        c.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&h[4..30]);
        c.extend_from_slice(&0u16.to_le_bytes()); // comment
        c.extend_from_slice(&0u16.to_le_bytes()); // disk
        c.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        self.count += 1;
        self
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut out = self.out.clone();
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&self.central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&(self.central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lain-zip-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.zip");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn crc_matches_the_reference() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn stored_and_deflated_members_round_trip() {
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let raw = Writer::new().add("a.txt", b"hello", false).add("dir/b.bin", &big, true).finish();
        let p = tmp("rt", &raw);
        let z = Archive::open(&p).unwrap();
        let names: Vec<&str> = z.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "dir/b.bin"]);
        assert_eq!(z.read(z.find("a.txt").unwrap(), 1 << 20).unwrap(), b"hello");
        let b = z.find("dir/b.bin").unwrap();
        assert_eq!(z.read(b, 1 << 20).unwrap(), big);
        assert_eq!(z.head(b, 1000).unwrap(), big[..1000]);
        assert_eq!(z.head(z.find("a.txt").unwrap(), 3).unwrap(), b"hel");
        assert_eq!(z.head(z.find("a.txt").unwrap(), 30).unwrap(), b"hello");
        assert!(z.read(b, 1000).unwrap_err().0.contains("exceeds"));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn corruption_is_an_error_never_a_panic() {
        let raw = Writer::new().add("a.txt", b"hello world", true).finish();
        // A flipped data byte fails the checksum or the inflate.
        let mut bad = raw.clone();
        bad[30 + 5 + 2] ^= 0xff;
        let p = tmp("crc", &bad);
        let z = Archive::open(&p).unwrap();
        assert!(z.read(&z.entries()[0], 1 << 20).is_err());
        // Garbage and truncations.
        for junk in [b"nope".to_vec(), vec![0u8; 100], raw[..raw.len() - 3].to_vec(), raw[10..].to_vec()] {
            std::fs::write(&p, &junk).unwrap();
            if let Ok(z) = Archive::open(&p) {
                for e in z.entries() {
                    let _ = z.read(e, 1 << 20);
                    let _ = z.head(e, 10);
                }
            }
        }
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}
