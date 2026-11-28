//! Record 0 of a book (or of the KF8 half of a joint file): the PalmDOC
//! header, the MOBI header and its EXTH block.

use super::exth::Exth;
use super::{MobiError, be16, be32, invalid};

const NULL: u32 = 0xffff_ffff;

pub(crate) const COMPRESSION_NONE: u16 = 1;
pub(crate) const COMPRESSION_PALMDOC: u16 = 2;
pub(crate) const COMPRESSION_HUFF: u16 = 17480;

/// The fields the converter uses. Record numbers are absolute (the
/// header's own record index has been added).
pub(crate) struct Header<'a> {
    /// The record index of this record 0.
    pub start: usize,
    pub compression: u16,
    pub text_length: usize,
    pub text_records: usize,
    pub encryption: u16,
    pub encoding: u32,
    pub version: u32,
    pub full_name: &'a [u8],
    pub locale: u32,
    pub first_resource: Option<usize>,
    pub huff: Option<(usize, usize)>,
    pub extra_flags: u16,
    pub fdst: Option<usize>,
    pub ncx: Option<usize>,
    pub fragments: Option<usize>,
    pub skeletons: Option<usize>,
    pub exth: Exth<'a>,
}

impl<'a> Header<'a> {
    pub fn parse(rec0: &'a [u8], start: usize) -> Result<Header<'a>, MobiError> {
        if rec0.len() < 16 {
            return invalid("record 0 too short");
        }
        let compression = be16(rec0, 0).unwrap_or(0);
        let text_length = be32(rec0, 4).unwrap_or(0) as usize;
        let text_records = usize::from(be16(rec0, 8).unwrap_or(0));
        let encryption = be16(rec0, 12).unwrap_or(0);
        if rec0.get(16..20) != Some(b"MOBI") {
            if encryption != 0 {
                return Err(MobiError::Drm);
            }
            return Err(MobiError::Unsupported("no MOBI header".into()));
        }
        let header_len = be32(rec0, 20).unwrap_or(0) as usize;
        // The MOBI header proper; fields past its declared length are absent.
        let mobi = &rec0[..rec0.len().min(16usize.saturating_add(header_len))];
        let field = |at: usize| be32(mobi, at);
        // A record number relative to this header's record 0.
        let record = |at: usize| match field(at) {
            None | Some(NULL) | Some(0) => None,
            Some(v) => start.checked_add(v as usize),
        };
        let version = field(0x24).unwrap_or(0);
        let name_off = field(0x54).unwrap_or(0) as usize;
        let name_len = field(0x58).unwrap_or(0) as usize;
        let full_name = name_off
            .checked_add(name_len)
            .and_then(|end| rec0.get(name_off..end))
            .map(|n| n.split(|&b| b == 0).next().unwrap_or(n))
            .unwrap_or(&[]);
        let huff = match (record(0x70), field(0x74)) {
            (Some(first), Some(n)) if n > 0 && n != NULL => Some((first, n as usize)),
            _ => None,
        };
        let extra_flags = if header_len >= 0xe4 { be16(mobi, 0xf2).unwrap_or(0) } else { 0 };
        let kf8 = version >= 8;
        let exth = if field(0x80).unwrap_or(0) & 0x40 != 0 {
            rec0.get(16 + header_len..).map(Exth::parse).unwrap_or_default()
        } else {
            Exth::default()
        };
        Ok(Header {
            start,
            compression,
            text_length,
            text_records,
            encryption,
            encoding: field(0x1c).unwrap_or(1252),
            version,
            full_name,
            locale: field(0x5c).unwrap_or(0),
            first_resource: record(0x6c),
            huff,
            extra_flags,
            fdst: if kf8 { record(0xc0) } else { None },
            ncx: record(0xf4),
            fragments: if kf8 { record(0xf8) } else { None },
            skeletons: if kf8 { record(0xfc) } else { None },
            exth,
        })
    }

    pub fn utf8(&self) -> bool {
        self.encoding == 65001
    }

    /// Checks the fields every text path needs.
    pub fn check_text(&self) -> Result<(), MobiError> {
        if self.encryption != 0 {
            return Err(MobiError::Drm);
        }
        match self.compression {
            COMPRESSION_NONE | COMPRESSION_PALMDOC | COMPRESSION_HUFF => Ok(()),
            c => Err(MobiError::Unsupported(format!("compression type {c}"))),
        }
    }
}
