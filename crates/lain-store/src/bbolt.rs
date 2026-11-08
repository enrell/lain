//! A small read-only reader for Go bbolt files (`lain.db`), used by
//! `lain-rs import`. It never writes: the file is opened read-only, a
//! shared advisory lock is taken the way bbolt's own read-only mode
//! does (so a running Go server, which holds the exclusive lock, is
//! detected instead of read mid-write), and the bytes are parsed from
//! memory.
//!
//! Format (bbolt v1.3+, little-endian): pages of `page_size` bytes,
//! each with a 16-byte header `{id u64, flags u16, count u16, overflow
//! u32}`. Pages 0 and 1 are meta pages; the valid one (magic, version,
//! FNV-1a checksum) with the higher txid wins. Branch elements are
//! `{pos u32, ksize u32, pgid u64}`, leaf elements `{flags u32, pos
//! u32, ksize u32, vsize u32}` with `pos` relative to the element.
//! A leaf value with flag 0x01 is a bucket: `{root u64, sequence u64}`,
//! followed by an inline page when `root == 0`.
//!
//! Every offset is bounds-checked: a damaged file is an error, never a
//! panic.

use std::collections::BTreeSet;
use std::path::Path;

const MAGIC: u32 = 0xED0C_DAED;
const VERSION: u32 = 2;
const PAGE_HEADER: usize = 16;
const ELEMENT: usize = 16;
const BUCKET_HEADER: usize = 16;
const FLAG_BRANCH: u16 = 0x01;
const FLAG_LEAF: u16 = 0x02;
const LEAF_BUCKET: u32 = 0x01;
const MAX_DEPTH: usize = 64;

/// One key/value pair (or a nested bucket) of a bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Value(Vec<u8>, Vec<u8>),
    Bucket(Vec<u8>),
}

/// An opened bbolt file.
pub struct BoltFile {
    data: Vec<u8>,
    page_size: usize,
    root: u64,
    txid: u64,
}

fn u16_at(b: &[u8], off: usize) -> Result<u16, String> {
    b.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| format!("truncated at {off}"))
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| format!("truncated at {off}"))
}

fn u64_at(b: &[u8], off: usize) -> Result<u64, String> {
    b.get(off..off + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| format!("truncated at {off}"))
}

fn fnv1a64(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A parsed meta page: (page_size, root pgid, txid) when valid.
fn meta_at(data: &[u8], off: usize) -> Option<(usize, u64, u64)> {
    let m = off + PAGE_HEADER;
    let raw = data.get(m..m + 64)?;
    if u32_at(raw, 0).ok()? != MAGIC || u32_at(raw, 4).ok()? != VERSION {
        return None;
    }
    if fnv1a64(&raw[..56]) != u64_at(raw, 56).ok()? {
        return None;
    }
    let page_size = u32_at(raw, 8).ok()? as usize;
    Some((page_size, u64_at(raw, 16).ok()?, u64_at(raw, 48).ok()?))
}

impl BoltFile {
    /// Opens `path` read-only. Fails when another process holds the
    /// exclusive lock (a running Go server): stop it or import from a
    /// `lain backup` snapshot.
    pub fn open(path: &Path) -> Result<BoltFile, String> {
        let mut f =
            std::fs::OpenOptions::new().read(true).open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        match f.try_lock_shared() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "{} is locked by a running server; stop the Go server or import from a `lain backup` snapshot",
                    path.display()
                ));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("lock {}: {e}", path.display())),
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut data).map_err(|e| format!("read {}: {e}", path.display()))?;
        let _ = f.unlock();
        BoltFile::from_bytes(data)
    }

    /// Parses an in-memory copy of a bbolt file.
    pub fn from_bytes(data: Vec<u8>) -> Result<BoltFile, String> {
        let first = meta_at(&data, 0);
        let page_size = match first {
            Some((ps, _, _)) => ps,
            // Page 0 damaged: probe the usual page sizes for page 1.
            None => [4096usize, 8192, 16384, 65536]
                .into_iter()
                .find(|ps| meta_at(&data, *ps).is_some_and(|(p, _, _)| p == *ps))
                .ok_or("not a bbolt file (no valid meta page)")?,
        };
        if !(512..=1 << 20).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(format!("implausible page size {page_size}"));
        }
        let second = meta_at(&data, page_size);
        let (_, root, txid) = match (first, second) {
            (Some(a), Some(b)) => {
                if b.2 > a.2 {
                    b
                } else {
                    a
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return Err("not a bbolt file (no valid meta page)".into()),
        };
        Ok(BoltFile { data, page_size, root, txid })
    }

    /// The committed transaction id that was read.
    pub fn txid(&self) -> u64 {
        self.txid
    }

    fn page(&self, pgid: u64) -> Result<&[u8], String> {
        let start = (pgid as usize).checked_mul(self.page_size).ok_or("page id overflow")?;
        let head = self.data.get(start..start + PAGE_HEADER).ok_or_else(|| format!("page {pgid} out of range"))?;
        let overflow = u32_at(head, 12)? as usize;
        let end = start + (overflow + 1) * self.page_size;
        let end = end.min(self.data.len());
        self.data.get(start..end).ok_or_else(|| format!("page {pgid} out of range"))
    }

    /// Collects the entries of the tree rooted at a page slice.
    fn walk(&self, page: &[u8], out: &mut Vec<Entry>, depth: usize, seen: &mut BTreeSet<u64>) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("tree too deep (cycle?)".into());
        }
        let flags = u16_at(page, 8)?;
        let count = u16_at(page, 10)? as usize;
        for i in 0..count {
            let el = PAGE_HEADER + i * ELEMENT;
            if flags & FLAG_BRANCH != 0 {
                let child = u64_at(page, el + 8)?;
                if !seen.insert(child) {
                    return Err(format!("page {child} referenced twice"));
                }
                let p = self.page(child)?;
                self.walk(p, out, depth + 1, seen)?;
            } else if flags & FLAG_LEAF != 0 {
                let eflags = u32_at(page, el)?;
                let pos = u32_at(page, el + 4)? as usize;
                let ksize = u32_at(page, el + 8)? as usize;
                let vsize = u32_at(page, el + 12)? as usize;
                let kstart = el + pos;
                let key = page.get(kstart..kstart + ksize).ok_or("leaf key out of range")?;
                let val = page.get(kstart + ksize..kstart + ksize + vsize).ok_or("leaf value out of range")?;
                if eflags & LEAF_BUCKET != 0 {
                    out.push(Entry::Bucket(key.to_vec()));
                } else {
                    out.push(Entry::Value(key.to_vec(), val.to_vec()));
                }
            } else {
                return Err(format!("unexpected page flags {flags:#x}"));
            }
        }
        Ok(())
    }

    /// The entries of the bucket whose header is `value`.
    fn bucket_entries(&self, value: &[u8]) -> Result<Vec<Entry>, String> {
        let root = u64_at(value, 0)?;
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        if root == 0 {
            let inline = value.get(BUCKET_HEADER..).ok_or("inline bucket truncated")?;
            if inline.len() < PAGE_HEADER {
                return Err("inline bucket truncated".into());
            }
            self.walk(inline, &mut out, 0, &mut seen)?;
        } else {
            seen.insert(root);
            self.walk(self.page(root)?, &mut out, 0, &mut seen)?;
        }
        Ok(out)
    }

    fn root_entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>, bool)>, String> {
        // The root bucket's leaves hold every top-level bucket header.
        let mut out = Vec::new();
        let mut stack = vec![(self.root, 0usize)];
        let mut seen = BTreeSet::new();
        while let Some((pgid, depth)) = stack.pop() {
            if depth > MAX_DEPTH || !seen.insert(pgid) {
                return Err("root tree is cyclic".into());
            }
            let page = self.page(pgid)?;
            let flags = u16_at(page, 8)?;
            let count = u16_at(page, 10)? as usize;
            for i in (0..count).rev() {
                let el = PAGE_HEADER + i * ELEMENT;
                if flags & FLAG_BRANCH != 0 {
                    stack.push((u64_at(page, el + 8)?, depth + 1));
                } else if flags & FLAG_LEAF != 0 {
                    let eflags = u32_at(page, el)?;
                    let pos = u32_at(page, el + 4)? as usize;
                    let ksize = u32_at(page, el + 8)? as usize;
                    let vsize = u32_at(page, el + 12)? as usize;
                    let kstart = el + pos;
                    let key = page.get(kstart..kstart + ksize).ok_or("root key out of range")?;
                    let val = page.get(kstart + ksize..kstart + ksize + vsize).ok_or("root value out of range")?;
                    out.push((key.to_vec(), val.to_vec(), eflags & LEAF_BUCKET != 0));
                } else {
                    return Err(format!("unexpected root page flags {flags:#x}"));
                }
            }
        }
        Ok(out)
    }

    /// Top-level bucket names, sorted.
    pub fn bucket_names(&self) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = self
            .root_entries()?
            .into_iter()
            .filter(|(_, _, is_bucket)| *is_bucket)
            .map(|(k, _, _)| String::from_utf8_lossy(&k).into_owned())
            .collect();
        names.sort();
        Ok(names)
    }

    /// Every entry of a top-level bucket in key order, or `None` when
    /// the bucket does not exist.
    pub fn bucket(&self, name: &str) -> Result<Option<Vec<Entry>>, String> {
        for (k, v, is_bucket) in self.root_entries()? {
            if is_bucket && k == name.as_bytes() {
                return self.bucket_entries(&v).map(Some);
            }
        }
        Ok(None)
    }

    /// The key/value pairs of a top-level bucket (nested buckets are
    /// skipped; lain's buckets have none).
    pub fn pairs(&self, name: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        Ok(self
            .bucket(name)?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| match e {
                Entry::Value(k, v) => Some((k, v)),
                Entry::Bucket(_) => None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn reads_a_go_server_database() {
        let f = BoltFile::from_bytes(fixture("go-server.bbolt")).unwrap();
        let names = f.bucket_names().unwrap();
        for b in ["users", "users_by_name", "libraries", "items", "items_by_library", "progress", "meta"] {
            assert!(names.contains(&b.to_string()), "missing bucket {b}: {names:?}");
        }
        let users = f.pairs("users").unwrap();
        assert!(users.iter().any(|(k, _)| k == b"user-admin"));
        let admin: serde_json::Value =
            serde_json::from_slice(&users.iter().find(|(k, _)| k == b"user-admin").unwrap().1).unwrap();
        assert_eq!(admin["role"], "admin");
        assert!(f.pairs("meta").unwrap().iter().any(|(k, _)| k == b"secret"));
        assert!(!f.pairs("items").unwrap().is_empty());
        assert!(f.bucket("no-such-bucket").unwrap().is_none());
    }

    #[test]
    fn reads_branches_overflow_and_inline_buckets() {
        // Written by testdata/gen (Go, real bbolt): "big" spans branch
        // pages, "huge" holds a value larger than a page, "small" is
        // an inline bucket, "nested" holds a sub-bucket.
        let f = BoltFile::from_bytes(fixture("generated.bbolt")).unwrap();
        let big = f.pairs("big").unwrap();
        assert_eq!(big.len(), 5000);
        for (i, (k, v)) in big.iter().enumerate() {
            assert_eq!(k, format!("k{i:05}").as_bytes());
            assert_eq!(v, format!("v{i:05}-{}", "x".repeat(i % 50)).as_bytes());
        }
        let huge = f.pairs("huge").unwrap();
        assert_eq!(huge.len(), 1);
        assert_eq!(huge[0].1.len(), 20_000);
        assert!(huge[0].1.iter().enumerate().all(|(i, b)| *b == b'a' + (i % 26) as u8));
        assert_eq!(f.pairs("small").unwrap(), vec![(b"a".to_vec(), b"1".to_vec()), (b"b".to_vec(), b"2".to_vec())]);
        let nested = f.bucket("nested").unwrap().unwrap();
        assert!(nested.contains(&Entry::Bucket(b"child".to_vec())));
        assert!(nested.contains(&Entry::Value(b"top".to_vec(), b"t".to_vec())));
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        assert!(BoltFile::from_bytes(vec![]).is_err());
        assert!(BoltFile::from_bytes(vec![0u8; 8192]).is_err());
        let good = fixture("go-server.bbolt");
        // Flip bytes across the whole file: every outcome is Ok or Err.
        for step in [7usize, 97, 1021] {
            let mut bad = good.clone();
            let mut i = 4096 * 2;
            while i < bad.len() {
                bad[i] ^= 0x5a;
                i += step;
            }
            if let Ok(f) = BoltFile::from_bytes(bad) {
                for name in f.bucket_names().unwrap_or_default() {
                    let _ = f.pairs(&name);
                }
            }
        }
        // Truncation.
        let f = BoltFile::from_bytes(good[..good.len() / 2].to_vec());
        if let Ok(f) = f {
            let _ = f.pairs("items");
        }
    }
}
