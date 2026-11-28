//! EXTH: the extended metadata block after the MOBI header, a list of
//! typed records (author, publisher, cover offset, KF8 boundary, ...).

use super::be32;

pub(crate) const AUTHOR: u32 = 100;
pub(crate) const PUBLISHER: u32 = 101;
pub(crate) const DESCRIPTION: u32 = 103;
pub(crate) const ISBN: u32 = 104;
pub(crate) const SUBJECT: u32 = 105;
pub(crate) const PUBLISHED: u32 = 106;
pub(crate) const KF8_BOUNDARY: u32 = 121;
pub(crate) const RESOURCE_COUNT: u32 = 125;
pub(crate) const KF8_COVER_URI: u32 = 129;
pub(crate) const COVER_OFFSET: u32 = 201;
pub(crate) const UPDATED_TITLE: u32 = 503;
pub(crate) const LANGUAGE: u32 = 524;
pub(crate) const WRITING_MODE: u32 = 525;
pub(crate) const PAGE_PROGRESSION: u32 = 527;

/// The EXTH records in file order.
#[derive(Default)]
pub(crate) struct Exth<'a> {
    records: Vec<(u32, &'a [u8])>,
}

impl<'a> Exth<'a> {
    /// Parses the block at the start of `data`; a malformed block yields
    /// the records read before the damage.
    pub fn parse(data: &'a [u8]) -> Exth<'a> {
        let mut records = Vec::new();
        if !data.starts_with(b"EXTH") {
            return Exth { records };
        }
        let count = be32(data, 8).unwrap_or(0);
        let mut at = 12usize;
        for _ in 0..count {
            let (Some(kind), Some(len)) = (be32(data, at), be32(data, at + 4)) else { break };
            let len = len as usize;
            let Some(end) = at.checked_add(len).filter(|_| len >= 8) else { break };
            let Some(body) = data.get(at + 8..end) else { break };
            records.push((kind, body));
            at = end;
        }
        Exth { records }
    }

    /// The first record of `kind`.
    pub fn get(&self, kind: u32) -> Option<&'a [u8]> {
        self.records.iter().find(|r| r.0 == kind).map(|r| r.1)
    }

    /// Every record of `kind`, in order.
    pub fn all(&self, kind: u32) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.records.iter().filter(move |r| r.0 == kind).map(|r| r.1)
    }

    /// A record holding a big-endian integer (1 to 4 bytes).
    pub fn number(&self, kind: u32) -> Option<u32> {
        let b = self.get(kind)?;
        if b.is_empty() || b.len() > 4 {
            return None;
        }
        Some(b.iter().fold(0u32, |v, &x| (v << 8) | u32::from(x)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_damage() {
        let mut d = b"EXTH\0\0\0\0\0\0\0\x03".to_vec();
        d.extend_from_slice(&[0, 0, 0, 100, 0, 0, 0, 11, b'A', b'n', b'n']);
        d.extend_from_slice(&[0, 0, 0, 201, 0, 0, 0, 12, 0, 0, 0, 7]);
        d.extend_from_slice(&[0, 0, 0, 100, 0, 0, 0xff, 0xff, b'x']);
        let e = Exth::parse(&d);
        assert_eq!(e.get(AUTHOR), Some(&b"Ann"[..]));
        assert_eq!(e.number(COVER_OFFSET), Some(7));
        assert_eq!(e.all(AUTHOR).count(), 1);
    }
}
