//! HUFF/CDIC compression: a canonical Huffman code over a phrase
//! dictionary whose phrases may themselves be compressed. One HUFF
//! record holds the code tables, the CDIC records the phrases.

use super::palmdb::PalmDb;
use super::{MobiError, be16, be32, invalid};

/// Phrases nest; real books stay a few levels deep.
const MAX_DEPTH: u32 = 32;
/// One expanded phrase may not exceed this.
const MAX_PHRASE: usize = 4 << 20;
/// Expanded phrases are kept; their total may not exceed this.
const MAX_MEMO: usize = 256 << 20;

struct Phrase<'a> {
    data: &'a [u8],
    /// Stored uncompressed in the CDIC record.
    literal: bool,
    expanded: Option<Vec<u8>>,
    busy: bool,
}

pub(crate) struct HuffCdic<'a> {
    /// Indexed by the top 8 bits of the code: (length, terminal, max code).
    dict1: [(u32, bool, u64); 256],
    mincode: [u64; 33],
    maxcode: [u64; 33],
    phrases: Vec<Phrase<'a>>,
    memo: usize,
}

impl<'a> HuffCdic<'a> {
    /// Loads the HUFF record at `first` and the `count - 1` CDIC records
    /// after it.
    pub fn load(db: &PalmDb<'a>, first: usize, count: usize) -> Result<HuffCdic<'a>, MobiError> {
        let huff = db.need(first, "HUFF")?;
        if !huff.starts_with(b"HUFF") {
            return invalid("bad HUFF record");
        }
        let (Some(off1), Some(off2)) = (be32(huff, 8), be32(huff, 12)) else { return invalid("bad HUFF record") };
        let (off1, off2) = (off1 as usize, off2 as usize);
        if off1.checked_add(1024).is_none_or(|e| e > huff.len()) || off2.checked_add(256).is_none_or(|e| e > huff.len())
        {
            return invalid("HUFF tables out of bounds");
        }
        let mut dict1 = [(0u32, false, 0u64); 256];
        for (i, slot) in dict1.iter_mut().enumerate() {
            let v = be32(huff, off1 + 4 * i).unwrap_or(0);
            let (len, term, max) = (v & 0x1f, v & 0x80 != 0, u64::from(v >> 8));
            let max = if term && len > 0 { ((max + 1) << (32 - len)) - 1 } else { 0 };
            *slot = (len, term, max);
        }
        let (mut mincode, mut maxcode) = ([0u64; 33], [0u64; 33]);
        maxcode[0] = 0xffff_ffff;
        for len in 1..=32usize {
            let lo = u64::from(be32(huff, off2 + 8 * (len - 1)).unwrap_or(0));
            let hi = u64::from(be32(huff, off2 + 8 * (len - 1) + 4).unwrap_or(0));
            mincode[len] = lo << (32 - len);
            maxcode[len] = ((hi + 1) << (32 - len)) - 1;
        }
        let mut phrases = Vec::new();
        for k in 1..count {
            let cdic = db.need(first + k, "CDIC")?;
            if !cdic.starts_with(b"CDIC") {
                return invalid("bad CDIC record");
            }
            let hlen = be32(cdic, 4).unwrap_or(0) as usize;
            let (Some(total), Some(bits)) = (be32(cdic, 8), be32(cdic, 12)) else { return invalid("bad CDIC record") };
            if bits > 16 || hlen < 16 || hlen > cdic.len() {
                return invalid("bad CDIC record");
            }
            let n = (1usize << bits).min((total as usize).saturating_sub(phrases.len()));
            for j in 0..n {
                let off = usize::from(be16(cdic, hlen + 2 * j).ok_or_else(|| bad("CDIC offsets out of bounds"))?);
                let at = hlen + off;
                let blen = be16(cdic, at).ok_or_else(|| bad("CDIC phrase out of bounds"))?;
                let len = usize::from(blen & 0x7fff);
                let data = cdic.get(at + 2..at + 2 + len).ok_or_else(|| bad("CDIC phrase out of bounds"))?;
                phrases.push(Phrase { data, literal: blen & 0x8000 != 0, expanded: None, busy: false });
            }
        }
        Ok(HuffCdic { dict1, mincode, maxcode, phrases, memo: 0 })
    }

    /// Appends the decompressed record to `out`, failing past `limit`.
    pub fn decompress(&mut self, data: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), MobiError> {
        self.unpack(data, out, limit, 0)
    }

    fn unpack(&mut self, data: &[u8], out: &mut Vec<u8>, limit: usize, depth: u32) -> Result<(), MobiError> {
        if depth > MAX_DEPTH {
            return invalid("HUFF/CDIC phrases nest too deep");
        }
        let total = data.len().saturating_mul(8);
        let mut pos = 0usize;
        loop {
            let code = peek32(data, pos);
            let (mut len, term, mut max) = self.dict1[(code >> 24) as usize];
            if len == 0 {
                return invalid("bad HUFF code");
            }
            if !term {
                while (len as usize) < 33 && code < self.mincode[len as usize] {
                    len += 1;
                }
                if len > 32 {
                    return invalid("bad HUFF code");
                }
                max = self.maxcode[len as usize];
            }
            pos += len as usize;
            if pos > total {
                break;
            }
            let index = max.checked_sub(code).ok_or_else(|| bad("bad HUFF code"))? >> (32 - len);
            let index = usize::try_from(index).map_err(|_| bad("bad HUFF code"))?;
            let Some(p) = self.phrases.get_mut(index) else { return invalid("HUFF code past the dictionary") };
            if p.literal {
                out.extend_from_slice(p.data);
            } else if let Some(done) = &p.expanded {
                out.extend_from_slice(done);
            } else if p.busy {
                return invalid("HUFF/CDIC phrase refers to itself");
            } else {
                p.busy = true;
                let src = p.data;
                let mut buf = Vec::new();
                self.unpack(src, &mut buf, MAX_PHRASE, depth + 1)?;
                self.memo += buf.len();
                if self.memo > MAX_MEMO {
                    return invalid("HUFF/CDIC dictionary expands too far");
                }
                out.extend_from_slice(&buf);
                let p = &mut self.phrases[index];
                p.busy = false;
                p.expanded = Some(buf);
            }
            if out.len() > limit {
                return invalid("decompressed text exceeds its cap");
            }
        }
        Ok(())
    }
}

fn bad(msg: &str) -> MobiError {
    MobiError::Invalid(msg.into())
}

/// The 32 bits starting at bit `pos` (most significant first), zero
/// padded past the end.
fn peek32(data: &[u8], pos: usize) -> u64 {
    let byte = pos >> 3;
    let mut v = 0u64;
    for k in 0..5 {
        v = (v << 8) | u64::from(data.get(byte + k).copied().unwrap_or(0));
    }
    (v >> (8 - (pos & 7))) & 0xffff_ffff
}
