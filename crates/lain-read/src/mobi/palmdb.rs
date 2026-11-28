//! The PalmDB container: a 78-byte header and a table of record
//! offsets. Records are the byte ranges between consecutive offsets.

use super::{MAX_RECORD, MobiError, be16, be32, invalid};

/// A parsed PalmDB file. Records borrow from the file bytes.
pub(crate) struct PalmDb<'a> {
    data: &'a [u8],
    pub name: &'a [u8],
    pub created: u32,
    pub modified: u32,
    offsets: Vec<usize>,
}

impl<'a> PalmDb<'a> {
    pub fn parse(data: &'a [u8]) -> Result<PalmDb<'a>, MobiError> {
        if data.len() < 78 {
            return invalid("file too short for a PalmDB header");
        }
        match &data[60..68] {
            b"BOOKMOBI" => {}
            b"TEXtREAd" => return Err(MobiError::Unsupported("plain PalmDOC text".into())),
            _ if data.starts_with(b"TPZ") => return Err(MobiError::Unsupported("Topaz book".into())),
            _ => return invalid("not a MOBI file"),
        }
        let name_end = data[..32].iter().position(|&b| b == 0).unwrap_or(32);
        let count = usize::from(be16(data, 76).unwrap_or(0));
        if count == 0 {
            return invalid("no records");
        }
        let table_end = 78 + 8 * count;
        if table_end > data.len() {
            return invalid("record table out of bounds");
        }
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let off = be32(data, 78 + 8 * i).unwrap_or(0) as usize;
            if off < table_end || off > data.len() {
                return invalid(format!("record {i} out of bounds"));
            }
            if let Some(&prev) = offsets.last()
                && off < prev
            {
                return invalid(format!("record {i} out of order"));
            }
            offsets.push(off);
        }
        for i in 0..count {
            let end = offsets.get(i + 1).copied().unwrap_or(data.len());
            if end - offsets[i] > MAX_RECORD {
                return invalid(format!("record {i} exceeds 64 MB"));
            }
        }
        Ok(PalmDb {
            data,
            name: &data[..name_end],
            created: be32(data, 36).unwrap_or(0),
            modified: be32(data, 40).unwrap_or(0),
            offsets,
        })
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Record `i`, or `None` past the last record.
    pub fn record(&self, i: usize) -> Option<&'a [u8]> {
        let start = *self.offsets.get(i)?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        self.data.get(start..end)
    }

    /// Record `i`, or an `Invalid` error naming `what`.
    pub fn need(&self, i: usize, what: &str) -> Result<&'a [u8], MobiError> {
        self.record(i).ok_or_else(|| MobiError::Invalid(format!("missing {what} record")))
    }
}
