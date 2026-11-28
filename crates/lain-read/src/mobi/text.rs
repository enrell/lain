//! The book text: text records with their trailing entries removed,
//! decompressed and concatenated.

use super::header::{COMPRESSION_HUFF, COMPRESSION_NONE, COMPRESSION_PALMDOC, Header};
use super::huffcdic::HuffCdic;
use super::palmdb::PalmDb;
use super::{MAX_RECORD, MAX_TEXT, MobiError, invalid, palmdoc};

/// The decompressed text of the book `h` heads (all flows, for KF8).
pub(crate) fn extract(db: &PalmDb<'_>, h: &Header<'_>) -> Result<Vec<u8>, MobiError> {
    h.check_text()?;
    if h.text_records == 0 {
        return invalid("no text records");
    }
    let mut huff = match (h.compression, h.huff) {
        (COMPRESSION_HUFF, Some((first, count))) => Some(HuffCdic::load(db, first, count)?),
        (COMPRESSION_HUFF, None) => return invalid("HUFF/CDIC book without a HUFF record"),
        _ => None,
    };
    let mut out = Vec::with_capacity(h.text_length.min(MAX_TEXT));
    for i in 1..=h.text_records {
        let rec = db.need(h.start + i, "text")?;
        let rec = &rec[..rec.len() - trailing_size(rec, h.extra_flags)];
        let limit = out.len().saturating_add(MAX_RECORD).min(MAX_TEXT);
        match h.compression {
            COMPRESSION_NONE => out.extend_from_slice(rec),
            COMPRESSION_PALMDOC => palmdoc::decompress(rec, &mut out, limit)?,
            _ => {
                if let Some(hc) = huff.as_mut() {
                    hc.decompress(rec, &mut out, limit)?;
                }
            }
        }
        if out.len() > MAX_TEXT {
            return invalid("text exceeds 256 MB");
        }
    }
    if h.text_length > 0 && h.text_length < out.len() {
        out.truncate(h.text_length);
    }
    Ok(out)
}

/// The size of the trailing entries `flags` announces at the end of a
/// text record, at most the record's length. Bit 0 is the multibyte
/// overlap; each higher bit is an entry whose size is a backward
/// varint at its end.
pub(crate) fn trailing_size(rec: &[u8], flags: u16) -> usize {
    let len = rec.len();
    let mut num = 0usize;
    let mut bits = flags >> 1;
    while bits != 0 {
        if bits & 1 != 0 {
            num = num.saturating_add(backward_varint(&rec[..len - num.min(len)]));
            if num >= len {
                return len;
            }
        }
        bits >>= 1;
    }
    if flags & 1 != 0 && num < len {
        num += usize::from(rec[len - num - 1] & 3) + 1;
    }
    num.min(len)
}

/// A varint read backward from the end of `data`: 7 bits per byte, the
/// last byte least significant, the first byte flagged with 0x80.
fn backward_varint(data: &[u8]) -> usize {
    let mut value = 0usize;
    let mut shift = 0;
    for &b in data.iter().rev() {
        value |= usize::from(b & 0x7f) << shift;
        shift += 7;
        if b & 0x80 != 0 || shift >= 28 {
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_entries_are_measured() {
        // text, a multibyte overlap of 1 byte (+ its count byte), and a
        // 3-byte entry with its size byte.
        let rec = [b'h', b'i', 0xa9, 0x01, 0xaa, 0xbb, 0x83];
        assert_eq!(trailing_size(&rec, 0b11), 5);
        assert_eq!(trailing_size(&rec, 0b10), 3);
        assert_eq!(trailing_size(&rec, 0), 0);
        // Corrupt sizes never exceed the record.
        assert_eq!(trailing_size(&[0xff], 0b1111), 1);
        assert_eq!(trailing_size(&[], 0b11), 0);
    }
}
