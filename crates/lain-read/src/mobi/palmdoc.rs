//! PalmDOC compression: an LZ77 variant with byte-pair space coding.
//! Each text record is compressed on its own, so back references never
//! reach into an earlier record.

use super::{MobiError, invalid};

/// Appends the decompressed `src` to `out`. Fails once `out` would grow
/// past `limit` bytes. Back references that point before the start of
/// this record are corrupt and skipped, as other readers do.
pub(crate) fn decompress(src: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), MobiError> {
    let base = out.len();
    let mut i = 0usize;
    while i < src.len() {
        let c = src[i];
        i += 1;
        match c {
            1..=8 => {
                let end = i.saturating_add(usize::from(c)).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i = end;
            }
            0x00 | 0x09..=0x7f => out.push(c),
            0x80..=0xbf => {
                let Some(&low) = src.get(i) else { break };
                i += 1;
                let m = ((usize::from(c) << 8) | usize::from(low)) & 0x3fff;
                let (dist, len) = (m >> 3, (m & 7) + 3);
                let produced = out.len() - base;
                if dist == 0 || dist > produced {
                    continue;
                }
                // The copy may overlap itself (dist < len), so go byte by byte.
                let from = out.len() - dist;
                for k in 0..len {
                    let b = out[from + k];
                    out.push(b);
                }
            }
            _ => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
        if out.len() > limit {
            return invalid("decompressed text exceeds its cap");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_pairs_and_back_references() {
        // "ab" literal, a space pair " c", a 2-byte literal run, then a
        // back reference of distance 5, length 3.
        let src = [b'a', b'b', 0xe3, 2, b'x', b'y', 0x80, (5 << 3)];
        let mut out = Vec::new();
        decompress(&src, &mut out, 1 << 20).unwrap();
        assert_eq!(out, b"ab cxyb c");
    }

    #[test]
    fn corrupt_input_never_panics() {
        for src in [&[0x80u8][..], &[0x80, 0xff], &[8, 1], &[0xbf, 0xff, 0xbf, 0xff]] {
            let mut out = Vec::new();
            let _ = decompress(src, &mut out, 1 << 20);
        }
        let mut out = Vec::new();
        assert!(decompress(&[b'a'; 100], &mut out, 10).is_err());
    }
}
