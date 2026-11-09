//! A minimal zip reader for theme archives: the central directory,
//! stored and deflated entries (flate2, already in the tree), no zip64,
//! no encryption. Sizes are capped so a hostile archive cannot fill
//! memory.

use std::io::Read;

const MAX_TOTAL: u64 = 64 << 20;
const MAX_ENTRIES: usize = 4096;

fn u16le(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated zip".into())
}

fn u32le(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated zip".into())
}

/// Every file entry as (name, bytes). Directory entries are skipped.
pub fn read_all(raw: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // End of central directory: the last 0x06054b50 within 64 KiB + 22.
    let min = raw.len().saturating_sub(22 + 65_535);
    let eocd = (min..raw.len().saturating_sub(21))
        .rev()
        .find(|&i| raw[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("not a zip archive")?;
    let count = u16le(raw, eocd + 10)? as usize;
    let mut at = u32le(raw, eocd + 16)? as usize;
    if count > MAX_ENTRIES {
        return Err("too many entries".into());
    }
    let mut out = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        if u32le(raw, at)? != 0x0201_4b50 {
            return Err("bad central directory".into());
        }
        let method = u16le(raw, at + 10)?;
        let csize = u32le(raw, at + 20)? as usize;
        let usize_ = u32le(raw, at + 24)? as u64;
        let (nlen, elen, clen) =
            (u16le(raw, at + 28)? as usize, u16le(raw, at + 30)? as usize, u16le(raw, at + 32)? as usize);
        let local = u32le(raw, at + 42)? as usize;
        let name = String::from_utf8_lossy(raw.get(at + 46..at + 46 + nlen).ok_or("truncated zip")?).replace('\\', "/");
        at += 46 + nlen + elen + clen;
        if name.ends_with('/') {
            continue;
        }
        total += usize_;
        if total > MAX_TOTAL {
            return Err("archive too large".into());
        }
        if u32le(raw, local)? != 0x0403_4b50 {
            return Err(format!("{name}: bad local header"));
        }
        let data_at = local + 30 + u16le(raw, local + 26)? as usize + u16le(raw, local + 28)? as usize;
        let data = raw.get(data_at..data_at + csize).ok_or_else(|| format!("{name}: truncated"))?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_ as usize);
                flate2::read::DeflateDecoder::new(data)
                    .take(usize_ + 1)
                    .read_to_end(&mut v)
                    .map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: compression method {m} not supported")),
        };
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored-only archive with one file, built by hand.
    fn stored(name: &str, body: &[u8]) -> Vec<u8> {
        let mut z = Vec::new();
        let crc = 0u32; // not checked by this reader
        z.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(name.as_bytes());
        z.extend_from_slice(body);
        let cd = z.len();
        z.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(name.as_bytes());
        let cdlen = z.len() - cd;
        z.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend_from_slice(&(cdlen as u32).to_le_bytes());
        z.extend_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z
    }

    #[test]
    fn reads_a_stored_entry_and_rejects_garbage() {
        let z = stored("t/theme.json", b"{}");
        assert_eq!(read_all(&z).unwrap(), vec![("t/theme.json".to_string(), b"{}".to_vec())]);
        assert!(read_all(b"nope").is_err());
        assert!(read_all(&z[..z.len() - 3]).is_err());
    }
}
