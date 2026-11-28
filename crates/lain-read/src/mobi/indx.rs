//! INDX indexes (NCX, KF8 skeletons, fragments): a header record with
//! the TAGX tag table, data records whose IDXT lists entry offsets, and
//! CNCX records holding the strings entries point at.
//!
//! An entry is a length-prefixed label, control bytes saying which
//! tags are present (and how many values each has), then the values as
//! forward varints (7 bits per byte, the last byte flagged with 0x80).

use super::palmdb::PalmDb;
use super::{MobiError, be16, be32, invalid};

/// Entries across all data records of one index.
const MAX_ENTRIES: usize = 1 << 20;

pub(crate) struct Entry<'a> {
    pub label: &'a [u8],
    tags: Vec<(u8, Vec<u64>)>,
}

impl Entry<'_> {
    pub fn values(&self, tag: u8) -> Option<&[u64]> {
        self.tags.iter().find(|t| t.0 == tag).map(|t| t.1.as_slice())
    }

    pub fn first(&self, tag: u8) -> Option<u64> {
        self.values(tag)?.first().copied()
    }
}

pub(crate) struct Index<'a> {
    pub entries: Vec<Entry<'a>>,
    cncx: Vec<&'a [u8]>,
}

/// One TAGX row: tag, values per entry, control-byte mask, end flag.
#[derive(Clone, Copy)]
struct TagDef {
    tag: u8,
    per_entry: u8,
    mask: u8,
    end: bool,
}

impl<'a> Index<'a> {
    /// The CNCX string at `offset` (record in the high 16 bits).
    pub fn string(&self, offset: u64) -> Option<&'a [u8]> {
        let rec = self.cncx.get(usize::try_from(offset >> 16).ok()?)?;
        let at = (offset & 0xffff) as usize;
        let (len, used) = varint(rec, at, rec.len())?;
        let start = at + used;
        rec.get(start..start.checked_add(usize::try_from(len).ok()?)?)
    }
}

/// Reads the index whose header record is `first`.
pub(crate) fn read<'a>(db: &PalmDb<'a>, first: usize) -> Result<Index<'a>, MobiError> {
    let head = db.need(first, "INDX")?;
    if !head.starts_with(b"INDX") {
        return invalid("bad INDX header");
    }
    let hlen = be32(head, 4).unwrap_or(0) as usize;
    let records = be32(head, 24).unwrap_or(0) as usize;
    let cncx_count = be32(head, 52).unwrap_or(0) as usize;
    if records > db.len() || cncx_count > db.len() {
        return invalid("INDX record count out of range");
    }
    let (control_bytes, tagx) = tag_table(head, hlen)?;
    let mut cncx = Vec::with_capacity(cncx_count);
    for k in 0..cncx_count {
        cncx.push(db.need(first + records + 1 + k, "CNCX")?);
    }
    let mut entries = Vec::new();
    for r in 1..=records {
        let data = db.need(first + r, "INDX data")?;
        if !data.starts_with(b"INDX") {
            return invalid("bad INDX data record");
        }
        let idxt = be32(data, 20).unwrap_or(0) as usize;
        let count = be32(data, 24).unwrap_or(0) as usize;
        if data.get(idxt..idxt.saturating_add(4)) != Some(b"IDXT") {
            return invalid("bad IDXT");
        }
        if count > data.len() / 2 || entries.len() + count > MAX_ENTRIES {
            return invalid("too many INDX entries");
        }
        let mut offsets = Vec::with_capacity(count + 1);
        for j in 0..count {
            offsets.push(usize::from(be16(data, idxt + 4 + 2 * j).ok_or_else(|| bad("IDXT out of bounds"))?));
        }
        offsets.push(idxt);
        for j in 0..count {
            let (start, end) = (offsets[j], offsets[j + 1]);
            if end < start || end > data.len() {
                return invalid("IDXT offsets out of order");
            }
            let label_len = usize::from(*data.get(start).ok_or_else(|| bad("INDX entry out of bounds"))?);
            let tags_at = start + 1 + label_len;
            let label = data.get(start + 1..tags_at).filter(|_| tags_at <= end).ok_or_else(|| bad("bad INDX label"))?;
            let tags = tag_values(data, tags_at, end, control_bytes, &tagx)?;
            entries.push(Entry { label, tags });
        }
    }
    Ok(Index { entries, cncx })
}

fn bad(msg: &str) -> MobiError {
    MobiError::Invalid(msg.into())
}

/// The TAGX section at `at`: (control byte count, rows).
fn tag_table(head: &[u8], at: usize) -> Result<(usize, Vec<TagDef>), MobiError> {
    if head.get(at..at.saturating_add(4)) != Some(b"TAGX") {
        return invalid("missing TAGX");
    }
    let len = be32(head, at + 4).unwrap_or(0) as usize;
    let control_bytes = be32(head, at + 8).unwrap_or(0) as usize;
    if !(1..=8).contains(&control_bytes) || !(12..=12 + 4 * 64).contains(&len) {
        return invalid("bad TAGX");
    }
    let rows = head.get(at + 12..at + len).ok_or_else(|| bad("TAGX out of bounds"))?;
    let tags = rows
        .as_chunks::<4>()
        .0
        .iter()
        .map(|r| TagDef { tag: r[0], per_entry: r[1], mask: r[2], end: r[3] & 1 != 0 })
        .collect();
    Ok((control_bytes, tags))
}

/// Decodes one entry's tag values from `data[at..end]`.
fn tag_values(
    data: &[u8],
    at: usize,
    end: usize,
    control_bytes: usize,
    tagx: &[TagDef],
) -> Result<Vec<(u8, Vec<u64>)>, MobiError> {
    let controls = data.get(at..at + control_bytes).filter(|_| at + control_bytes <= end);
    let controls = controls.ok_or_else(|| bad("INDX entry too short"))?;
    // (tag, value count or None, byte count or None, values per entry)
    let mut present: Vec<(u8, Option<usize>, Option<usize>, usize)> = Vec::new();
    let mut ci = 0usize;
    for t in tagx {
        if t.end {
            ci += 1;
            continue;
        }
        let Some(&cbyte) = controls.get(ci) else { break };
        if t.mask == 0 {
            continue;
        }
        let value = cbyte & t.mask;
        if value == 0 {
            continue;
        }
        if value == t.mask && t.mask.count_ones() > 1 {
            present.push((t.tag, None, Some(0), usize::from(t.per_entry)));
        } else {
            let count = usize::from(value >> t.mask.trailing_zeros());
            present.push((t.tag, Some(count), None, usize::from(t.per_entry)));
        }
    }
    let mut pos = at + control_bytes;
    // Byte counts follow the control bytes, in tag order, before the values.
    for p in present.iter_mut() {
        if p.2.is_some() {
            let (n, used) = varint(data, pos, end).ok_or_else(|| bad("bad INDX value"))?;
            pos += used;
            p.2 = Some(usize::try_from(n).map_err(|_| bad("bad INDX value"))?);
        }
    }
    let mut out = Vec::with_capacity(present.len());
    for (tag, count, bytes, per_entry) in present {
        let mut values = Vec::new();
        match (count, bytes) {
            (Some(count), _) => {
                for _ in 0..count.saturating_mul(per_entry) {
                    let (v, used) = varint(data, pos, end).ok_or_else(|| bad("bad INDX value"))?;
                    pos += used;
                    values.push(v);
                }
            }
            (None, Some(bytes)) => {
                let stop = pos.checked_add(bytes).filter(|&s| s <= end).ok_or_else(|| bad("bad INDX value"))?;
                while pos < stop {
                    let (v, used) = varint(data, pos, stop).ok_or_else(|| bad("bad INDX value"))?;
                    pos += used;
                    values.push(v);
                }
            }
            (None, None) => {}
        }
        out.push((tag, values));
    }
    Ok(out)
}

/// A forward varint at `data[at..end]`: (value, bytes used).
fn varint(data: &[u8], at: usize, end: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for k in 0..9 {
        let i = at.checked_add(k)?;
        if i >= end {
            return None;
        }
        let b = *data.get(i)?;
        v = (v << 7) | u64::from(b & 0x7f);
        if b & 0x80 != 0 {
            return Some((v, k + 1));
        }
    }
    None
}
