//! Text encodings: MOBI text is either UTF-8 (65001) or Windows-1252.

/// Windows-1252 code points for 0x80..=0x9f; the five undefined bytes
/// map to the C1 control of the same value, as browsers do.
const CP1252: [u16; 32] = [
    0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008d,
    0x017d, 0x008f, 0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a,
    0x0153, 0x009d, 0x017e, 0x0178,
];

/// One Windows-1252 byte as a character.
pub(crate) fn cp1252(b: u8) -> char {
    match b {
        0x80..=0x9f => char::from_u32(u32::from(CP1252[usize::from(b - 0x80)])).unwrap_or('\u{fffd}'),
        _ => char::from(b),
    }
}

/// Appends `bytes` decoded as UTF-8 (invalid sequences become U+FFFD) or
/// as Windows-1252.
pub(crate) fn push_decoded(out: &mut String, bytes: &[u8], utf8: bool) {
    if utf8 {
        out.push_str(&String::from_utf8_lossy(bytes));
    } else if bytes.is_ascii() {
        // ASCII is valid UTF-8 as is.
        out.push_str(&String::from_utf8_lossy(bytes));
    } else {
        out.reserve(bytes.len());
        out.extend(bytes.iter().map(|&b| cp1252(b)));
    }
}

/// `bytes` decoded, see [`push_decoded`].
pub(crate) fn decode(bytes: &[u8], utf8: bool) -> String {
    let mut s = String::new();
    push_decoded(&mut s, bytes, utf8);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_maps_the_high_controls() {
        assert_eq!(decode(b"\x93hi\x94 \x80 \xe9", false), "\u{201c}hi\u{201d} \u{20ac} \u{e9}");
        assert_eq!(decode("caf\u{e9}".as_bytes(), true), "caf\u{e9}");
        assert_eq!(decode(b"a\xffb", true), "a\u{fffd}b");
    }
}
