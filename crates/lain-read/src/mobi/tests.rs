//! End-to-end tests: synthetic MOBI 7, HUFF/CDIC, KF8 and joint books
//! built here, converted, and checked as EPUBs (package, spine, links,
//! well-formed XML); hostile variants of them; and real books from
//! `MOBI_SAMPLES` (ignored by default).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::zip::Archive;

// ---------------------------------------------------------------------
// Builders.

const NULL: u32 = 0xffff_ffff;
const EOF_RECORD: &[u8] = b"\xe9\x8e\r\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\nIDATx\x9cc\0\x01\0\0\x05\0\x01\r\n-\xb4\0\0\0\0IEND\xaeB`\x82";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0\xff\xd9";

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

/// A PalmDB file holding `records`.
fn palmdb(name: &str, records: &[Vec<u8>]) -> Vec<u8> {
    let n = records.len();
    let mut out = vec![0u8; 78];
    let name = name.as_bytes();
    out[..name.len().min(31)].copy_from_slice(&name[..name.len().min(31)]);
    put32(&mut out, 36, 1_000_000_000);
    put32(&mut out, 40, 1_000_000_000);
    out[60..68].copy_from_slice(b"BOOKMOBI");
    put32(&mut out, 68, 2 * n as u32);
    put16(&mut out, 76, n as u16);
    let mut off = 78 + 8 * n + 2;
    for (i, r) in records.iter().enumerate() {
        out.extend_from_slice(&(off as u32).to_be_bytes());
        out.extend_from_slice(&((2 * i) as u32).to_be_bytes());
        off += r.len();
    }
    out.extend_from_slice(&[0, 0]);
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

/// The fields of a record 0 the tests vary.
struct Rec0 {
    compression: u16,
    text_len: usize,
    text_records: usize,
    encryption: u16,
    version: u32,
    utf8: bool,
    first_resource: u32,
    huff: (u32, u32),
    extra_flags: u16,
    ncx: u32,
    fdst: u32,
    fdst_count: u32,
    frag: u32,
    skel: u32,
    exth: Vec<(u32, Vec<u8>)>,
    title: &'static str,
    locale: u32,
}

impl Default for Rec0 {
    fn default() -> Rec0 {
        Rec0 {
            compression: 1,
            text_len: 0,
            text_records: 0,
            encryption: 0,
            version: 6,
            utf8: true,
            first_resource: NULL,
            huff: (0, 0),
            extra_flags: 0,
            ncx: NULL,
            fdst: NULL,
            fdst_count: 0,
            frag: NULL,
            skel: NULL,
            exth: Vec::new(),
            title: "Book",
            locale: 9,
        }
    }
}

fn rec0(r: &Rec0) -> Vec<u8> {
    const HLEN: usize = 264;
    let mut h = vec![0u8; 16 + HLEN];
    put16(&mut h, 0, r.compression);
    put32(&mut h, 4, r.text_len as u32);
    put16(&mut h, 8, r.text_records as u16);
    put16(&mut h, 10, 4096);
    put16(&mut h, 12, r.encryption);
    h[16..20].copy_from_slice(b"MOBI");
    put32(&mut h, 20, HLEN as u32);
    put32(&mut h, 24, 2);
    put32(&mut h, 28, if r.utf8 { 65001 } else { 1252 });
    put32(&mut h, 32, 0x1234_5678);
    put32(&mut h, 36, r.version);
    for at in (0x28..0x50).step_by(4) {
        put32(&mut h, at, NULL);
    }
    put32(&mut h, 0x50, r.text_records as u32 + 1);
    put32(&mut h, 0x5c, r.locale);
    put32(&mut h, 0x68, r.version);
    put32(&mut h, 0x6c, r.first_resource);
    put32(&mut h, 0x70, r.huff.0);
    put32(&mut h, 0x74, r.huff.1);
    put32(&mut h, 0x80, 0x50);
    put32(&mut h, 0xa4, NULL);
    put32(&mut h, 0xa8, NULL);
    if r.version >= 8 {
        put32(&mut h, 0xc0, r.fdst);
        put32(&mut h, 0xc4, r.fdst_count);
    } else {
        put16(&mut h, 0xc0, 1);
        put16(&mut h, 0xc2, r.text_records as u16);
        put32(&mut h, 0xc4, 1);
    }
    for at in [0xc8, 0xd0, 0xe0, 0xe8, 0xec] {
        put32(&mut h, at, NULL);
    }
    put16(&mut h, 0xf2, r.extra_flags);
    put32(&mut h, 0xf4, r.ncx);
    put32(&mut h, 0xf8, r.frag);
    put32(&mut h, 0xfc, r.skel);
    put32(&mut h, 0x100, NULL);
    put32(&mut h, 0x104, NULL);
    let mut exth = b"EXTH\0\0\0\0\0\0\0\0".to_vec();
    for (kind, data) in &r.exth {
        exth.extend_from_slice(&kind.to_be_bytes());
        exth.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        exth.extend_from_slice(data);
    }
    while !exth.len().is_multiple_of(4) {
        exth.push(0);
    }
    let elen = exth.len() as u32;
    put32(&mut exth, 4, elen);
    put32(&mut exth, 8, r.exth.len() as u32);
    h.extend_from_slice(&exth);
    let name_at = h.len() as u32;
    put32(&mut h, 0x54, name_at);
    put32(&mut h, 0x58, r.title.len() as u32);
    h.extend_from_slice(r.title.as_bytes());
    h.extend_from_slice(&[0, 0, 0, 0]);
    h
}

fn num32(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// A greedy PalmDOC compressor (back references optional).
fn palmdoc_compress(src: &[u8], backrefs: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let literal = |c: u8| c == 0 || (0x09..0x80).contains(&c);
    while i < src.len() {
        if backrefs {
            let mut best = (0, 0);
            for j in i.saturating_sub(2047)..i {
                let mut l = 0;
                while l < 10 && i + l < src.len() && src[j + l] == src[i + l] {
                    l += 1;
                }
                if l > best.1 {
                    best = (i - j, l);
                }
            }
            if best.1 >= 3 {
                let m = (best.0 << 3) | (best.1 - 3);
                out.push(0x80 | (m >> 8) as u8);
                out.push((m & 0xff) as u8);
                i += best.1;
                continue;
            }
        }
        let c = src[i];
        if c == b' ' && i + 1 < src.len() && (0x40..0x80).contains(&src[i + 1]) {
            out.push(src[i + 1] ^ 0x80);
            i += 2;
        } else if literal(c) {
            out.push(c);
            i += 1;
        } else {
            let mut n = 1;
            while n < 8 && i + n < src.len() && !literal(src[i + n]) {
                n += 1;
            }
            out.push(n as u8);
            out.extend_from_slice(&src[i..i + n]);
            i += n;
        }
    }
    out
}

/// Splits text into records of about `size` bytes, compresses them and
/// appends the trailing entries `flags` announces (a multibyte overlap
/// for bit 0, a 2-byte entry for bit 1).
fn text_records(text: &[u8], size: usize, compression: u16, flags: u16) -> Vec<Vec<u8>> {
    let mut recs = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let end = (at + size).min(text.len());
        let chunk = &text[at..end];
        let mut rec = match compression {
            1 => chunk.to_vec(),
            _ => palmdoc_compress(chunk, text.len() < 1 << 20),
        };
        if flags & 1 != 0 {
            // The continuation bytes of a character cut at the boundary.
            let overlap: Vec<u8> = text[end..].iter().take_while(|&&b| b & 0xc0 == 0x80).take(3).copied().collect();
            rec.extend_from_slice(&overlap);
            rec.push(overlap.len() as u8);
        }
        if flags & 2 != 0 {
            rec.extend_from_slice(&[0x5a, 0x82]);
        }
        recs.push(rec);
        at = end;
    }
    recs
}

/// A forward varint, as INDX values are written.
fn fvar(mut v: u64) -> Vec<u8> {
    let mut b = vec![(v & 0x7f) as u8 | 0x80];
    v >>= 7;
    while v > 0 {
        b.push((v & 0x7f) as u8);
        v >>= 7;
    }
    b.reverse();
    b
}

/// A CNCX record of `strings`, and each string's offset.
fn cncx(strings: &[&str]) -> (Vec<u8>, Vec<u64>) {
    let mut rec = Vec::new();
    let mut offs = Vec::new();
    for s in strings {
        offs.push(rec.len() as u64);
        rec.extend_from_slice(&fvar(s.len() as u64));
        rec.extend_from_slice(s.as_bytes());
    }
    (rec, offs)
}

type Tagx = [(u8, u8, u8, u8)];
type IndexEntry = (Vec<u8>, Vec<(u8, Vec<u64>)>);

/// An INDX index: header record, one data record, then `cncx_recs`.
fn indx(tagx: &Tagx, entries: &[IndexEntry], cncx_recs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let control_bytes = 1 + tagx.iter().filter(|t| t.3 == 1).count().saturating_sub(1);
    let mut head = vec![0u8; 192];
    head[..4].copy_from_slice(b"INDX");
    put32(&mut head, 4, 192);
    put32(&mut head, 24, 1);
    put32(&mut head, 28, 65001);
    put32(&mut head, 36, entries.len() as u32);
    put32(&mut head, 52, cncx_recs.len() as u32);
    head.extend_from_slice(b"TAGX");
    head.extend_from_slice(&num32(12 + 4 * tagx.len() as u32));
    head.extend_from_slice(&num32(control_bytes as u32));
    for t in tagx {
        head.extend_from_slice(&[t.0, t.1, t.2, t.3]);
    }
    let mut data = vec![0u8; 192];
    data[..4].copy_from_slice(b"INDX");
    put32(&mut data, 4, 192);
    let mut offsets = Vec::new();
    for (label, tags) in entries {
        offsets.push(data.len());
        data.push(label.len() as u8);
        data.extend_from_slice(label);
        let mut controls = vec![0u8; control_bytes];
        let (mut counts, mut values) = (Vec::new(), Vec::new());
        let mut ci = 0;
        for &(tag, per, mask, end) in tagx {
            if end == 1 {
                ci += 1;
                continue;
            }
            let Some((_, vals)) = tags.iter().find(|t| t.0 == tag) else { continue };
            let groups = (vals.len() / usize::from(per)) as u8;
            let shifted = groups << mask.trailing_zeros();
            let mut bytes = Vec::new();
            for &v in vals {
                bytes.extend_from_slice(&fvar(v));
            }
            if mask.count_ones() > 1 && shifted & mask == mask || shifted & !mask != 0 {
                // All mask bits set: a byte count comes first.
                controls[ci] |= mask;
                counts.extend_from_slice(&fvar(bytes.len() as u64));
            } else {
                controls[ci] |= shifted;
            }
            values.extend_from_slice(&bytes);
        }
        data.extend_from_slice(&controls);
        data.extend_from_slice(&counts);
        data.extend_from_slice(&values);
    }
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let idxt = data.len();
    data.extend_from_slice(b"IDXT");
    for o in &offsets {
        data.extend_from_slice(&(*o as u16).to_be_bytes());
    }
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    put32(&mut data, 20, idxt as u32);
    put32(&mut data, 24, entries.len() as u32);
    let mut out = vec![head, data];
    out.extend(cncx_recs);
    out
}

fn tail() -> Vec<Vec<u8>> {
    vec![b"FLIS\0\0\0\x08\0\x41\0\0".to_vec(), b"FCIS\0\0\0\x14\0\0\0\x10".to_vec(), EOF_RECORD.to_vec()]
}

/// Replaces each `marker` (10 bytes) with the 10-digit offset of the
/// `target` that follows it in `targets`.
fn fill_filepos(template: &str, targets: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (marker, target) in targets {
        assert_eq!(marker.len(), 10);
        let pos = template.find(target).unwrap_or_else(|| panic!("{target}"));
        s = s.replace(marker, &format!("{pos:010}"));
    }
    s
}

const MOBI7_HTML: &str = concat!(
    "<html><head><guide><reference type=\"toc\" title=\"Table of Contents\" filepos=TTTTTTTTTT />",
    "</guide></head><body>",
    "<p>Contents</p><p><a filepos=AAAAAAAAAA>Chapter One</a></p>",
    "<p><a filepos=BBBBBBBBBB>Chapter Two</a></p>",
    "<mbp:pagebreak/>",
    "<h2>Chapter One</h2><p height=\"1em\" width=\"0pt\">Caf\u{e9} na\u{ef}ve \u{2014} text with an image ",
    "<img recindex=\"00001\" /> and <font size=\"5\" color=\"red\">big red</font> and ",
    "<a filepos=CCCCCCCCCC>a link to the middle</a> of <b>two<i>.</b></i></p>",
    "<mbp:pagebreak/>",
    "<h2>Chapter Two</h2><p>Second chapter &amp; end.</p><p>Here is the middle target. ",
    "<a href=\"http://example.com/x?a=1&b=2\">web</a> <a href=\"javascript:alert(1)\">bad</a></p>",
    "<script>alert('x')</script><table><tr><td>cell<td>two</table>",
    "</body></html>",
);

/// The MOBI 7 test book: compression 1 or 2, with or without an NCX.
fn mobi7_book(compression: u16, with_ncx: bool) -> Vec<u8> {
    let html = fill_filepos(
        MOBI7_HTML,
        &[
            ("TTTTTTTTTT", "<p>Contents"),
            ("AAAAAAAAAA", "<h2>Chapter One"),
            ("BBBBBBBBBB", "<mbp:pagebreak/><h2>Chapter Two"),
            ("CCCCCCCCCC", "middle target"),
        ],
    );
    let text = html.as_bytes();
    let flags = 0b11;
    let texts = text_records(text, 97, compression, flags);
    let mut records: Vec<Vec<u8>> = Vec::new();
    let n_text = texts.len();
    let mut after = 1 + n_text;
    let mut ncx_index = NULL;
    let mut ncx_recs = Vec::new();
    if with_ncx {
        let pos = |t: &str| html.find(t).unwrap() as u64;
        let (c, offs) = cncx(&["Contents", "Chapter One", "Chapter Two", "Middle &amp; target"]);
        let tagx = [(1, 1, 1, 0), (2, 1, 2, 0), (3, 1, 4, 0), (4, 1, 8, 0), (21, 1, 16, 0), (0, 0, 0, 1)];
        let entries = vec![
            (b"0".to_vec(), vec![(1, vec![pos("<p>Contents")]), (2, vec![10]), (3, vec![offs[0]]), (4, vec![0])]),
            (b"1".to_vec(), vec![(1, vec![pos("<h2>Chapter One")]), (2, vec![10]), (3, vec![offs[1]]), (4, vec![0])]),
            (b"2".to_vec(), vec![(1, vec![pos("<h2>Chapter Two")]), (2, vec![10]), (3, vec![offs[2]]), (4, vec![0])]),
            (
                b"3".to_vec(),
                vec![(1, vec![pos("middle target")]), (2, vec![5]), (3, vec![offs[3]]), (4, vec![1]), (21, vec![2])],
            ),
        ];
        ncx_recs = indx(&tagx, &entries, vec![c]);
        ncx_index = after as u32;
        after += ncx_recs.len();
    }
    let first_image = after as u32;
    let exth = vec![
        (100, b"Ann Author".to_vec()),
        (100, b"Bob Writer".to_vec()),
        (101, b"Test Press".to_vec()),
        (103, b"<p>A <b>test</b> &amp; more</p>".to_vec()),
        (104, b"978-0-00-000000-2".to_vec()),
        (105, b"Fiction".to_vec()),
        (105, b"Tests".to_vec()),
        (106, b"2001-02-03T00:00:00+00:00".to_vec()),
        (201, 1u32.to_be_bytes().to_vec()),
        (524, b"en".to_vec()),
    ];
    records.push(rec0(&Rec0 {
        compression,
        text_len: text.len(),
        text_records: n_text,
        first_resource: first_image,
        extra_flags: flags,
        ncx: ncx_index,
        exth,
        title: "Synthetic Seven",
        ..Rec0::default()
    }));
    records.extend(texts);
    records.extend(ncx_recs);
    records.push(PNG.to_vec());
    records.push(JPEG.to_vec());
    records.extend(tail());
    palmdb("Synthetic_Seven", &records)
}

/// Canonical Huffman tables for code `lengths` (non-decreasing, one per
/// dictionary entry), coded the way HUFF records expect: shorter codes
/// take the larger values. Returns (HUFF record, codes).
fn huff_tables(lengths: &[u32]) -> (Vec<u8>, Vec<(u32, u32)>) {
    let max = *lengths.iter().max().unwrap();
    let count = |l: u32| lengths.iter().filter(|&&x| x == l).count() as u64;
    let mut first = vec![0u64; 34];
    let mut code = 0u64;
    for l in 1..=32u32 {
        first[l as usize] = code;
        code = (code + count(l)) << 1;
    }
    let mut codes = Vec::new();
    let mut seen = HashMap::new();
    let mut off = HashMap::new();
    for (i, &l) in lengths.iter().enumerate() {
        let k = seen.entry(l).or_insert(0u64);
        off.entry(l).or_insert(i as u64);
        let c = (1u64 << l) - 1 - (first[l as usize] + *k);
        *k += 1;
        codes.push((c as u32, l));
    }
    let c_of = |l: u32| ((1u64 << l) - 1).wrapping_sub(first[l as usize]);
    let maxcode = |l: u32| off.get(&l).copied().unwrap_or(0) + c_of(l);
    let mut dict1 = Vec::new();
    for p in 0..256u32 {
        let hit = codes.iter().find(|&&(c, l)| l <= 8 && (c << (8 - l)) <= p && p < ((c + 1) << (8 - l)));
        let v = match hit {
            Some(&(_, l)) => l | 0x80 | ((maxcode(l) as u32) << 8),
            None => 9,
        };
        dict1.extend_from_slice(&v.to_be_bytes());
    }
    let mut dict2 = Vec::new();
    for l in 1..=32u32 {
        let (lo, hi) = if l <= max { (c_of(l) + 1 - count(l), maxcode(l)) } else { (0, 0) };
        dict2.extend_from_slice(&(lo as u32).to_be_bytes());
        dict2.extend_from_slice(&(hi as u32).to_be_bytes());
    }
    let mut huff = b"HUFF".to_vec();
    huff.extend_from_slice(&num32(24));
    huff.extend_from_slice(&num32(24));
    huff.extend_from_slice(&num32(24 + 1024));
    huff.extend_from_slice(&[0; 8]);
    huff.extend_from_slice(&dict1);
    huff.extend_from_slice(&dict2);
    (huff, codes)
}

/// The symbols as Huffman-coded bytes (zero padded).
fn huff_encode(codes: &[(u32, u32)], symbols: &[usize]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    for &s in symbols {
        let (c, l) = codes[s];
        for k in (0..l).rev() {
            bits.push(c >> k & 1 == 1);
        }
    }
    bits.chunks(8).map(|ch| ch.iter().enumerate().fold(0u8, |b, (i, &x)| b | (u8::from(x) << (7 - i)))).collect()
}

/// A HUFF/CDIC-compressed MOBI 7 book with two chapters.
fn huff_book() -> Vec<u8> {
    let phrases: [&[u8]; 10] = [
        b"<html><head></head><body>",
        b"<p>",
        b"</p>",
        b"Hello",
        b" ",
        b"world",
        b"<mbp:pagebreak/>",
        b"Huff",
        b"!",
        b"</body></html>",
    ];
    let lengths = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 10];
    let (huff, codes) = huff_tables(&lengths);
    // Entry 10 is itself compressed: "Hello world".
    let nested = huff_encode(&codes, &[3, 4, 5]);
    let mut entries: Vec<(Vec<u8>, bool)> = phrases.iter().map(|p| (p.to_vec(), true)).collect();
    entries.push((nested, false));
    let mut cdics = Vec::new();
    for chunk in entries.chunks(8) {
        let mut rec = b"CDIC".to_vec();
        rec.extend_from_slice(&num32(16));
        rec.extend_from_slice(&num32(entries.len() as u32));
        rec.extend_from_slice(&num32(3));
        let mut body = Vec::new();
        let mut offs = Vec::new();
        for (data, literal) in chunk {
            offs.push((2 * chunk.len() + body.len()) as u16);
            let flag = if *literal { 0x8000 } else { 0 };
            body.extend_from_slice(&(data.len() as u16 | flag).to_be_bytes());
            body.extend_from_slice(data);
        }
        for o in offs {
            rec.extend_from_slice(&o.to_be_bytes());
        }
        rec.extend_from_slice(&body);
        cdics.push(rec);
    }
    let t1 = huff_encode(&codes, &[0, 1, 10, 2, 6]);
    let t2 = huff_encode(&codes, &[1, 7, 4, 5, 8, 2, 9]);
    let text_len = b"<html><head></head><body><p>Hello world</p><mbp:pagebreak/>".len()
        + b"<p>Huff world!</p></body></html>".len();
    let mut records = vec![rec0(&Rec0 {
        compression: 17480,
        text_len,
        text_records: 2,
        huff: (3, 3),
        extra_flags: 0b10,
        title: "Huffman",
        ..Rec0::default()
    })];
    for t in [t1, t2] {
        let mut r = t;
        r.extend_from_slice(&[0x11, 0x82]);
        records.push(r);
    }
    records.push(huff);
    records.extend(cdics);
    records.extend(tail());
    palmdb("Huffman", &records)
}

const SKEL0: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Part One</title><link href=\"kindle:flow:0001?mime=text/css\" rel=\"stylesheet\" type=\"text/css\"/></head><body aid=\"0\"></body></html>";
const FRAG_A: &str = "<div aid=\"1\"><p aid=\"2\">First part. <a href=\"kindle:pos:fid:0001:off:0000000000\">To the second fragment</a> and <a href=\"kindle:pos:fid:0002:off:OFFSETCCCC\">to part two</a>.</p></div>";
const FRAG_B: &str = "<h1 aid=\"3\">Second fragment</h1><p aid=\"4\">Target text <img src=\"kindle:embed:0001?mime=image/png\"/> here.</p><div aid=\"9\"><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 10 10\"><image width=\"10\" height=\"10\" xlink:href=\"kindle:embed:0001\"/></svg></div>";
const SKEL1: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Part Two</title><link href=\"kindle:flow:0001?mime=text/css\" rel=\"stylesheet\" type=\"text/css\"/><style>h1 { color: blue }</style></head><body aid=\"5\"></body></html>";
const FRAG_C: &str = "<h1 aid=\"6\">Part Two</h1><p aid=\"7\">Back to <a href=\"kindle:pos:fid:0000:off:0000000000\">the start</a>. Middle <span aid=\"8\">anchor</span> text \u{e9}t\u{e9}.</p>";
const CSS: &str = "body { margin: 0 }\n@font-face { font-family: \"Test\"; src: url(kindle:embed:0002?mime=application/x-font-ttf); }\np { background: url(\"kindle:embed:0001?mime=image/png\") }\n@import url(http://example.com/x.css);\n";

fn base32(mut v: usize, width: usize) -> String {
    let digits = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";
    let mut s = vec![b'0'; width];
    for i in (0..width).rev() {
        s[i] = digits[v % 32];
        v /= 32;
    }
    String::from_utf8(s).unwrap()
}

fn font_data() -> Vec<u8> {
    let mut f = b"\0\x01\0\0".to_vec();
    f.extend((0..1500u32).map(|i| (i * 7 % 251) as u8));
    f
}

/// A FONT record: zlib-compressed and XOR-obfuscated.
fn font_record() -> Vec<u8> {
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&font_data(), 6);
    let key = b"0123456789abcdef";
    let obf: Vec<u8> = z.iter().enumerate().map(|(i, &b)| if i < 1040 { b ^ key[i % 16] } else { b }).collect();
    let mut rec = b"FONT".to_vec();
    rec.extend_from_slice(&num32(font_data().len() as u32));
    rec.extend_from_slice(&num32(3));
    rec.extend_from_slice(&num32(24));
    rec.extend_from_slice(&num32(16));
    rec.extend_from_slice(&num32(24 + obf.len() as u32));
    rec.extend_from_slice(&obf);
    rec.extend_from_slice(key);
    rec
}

fn resc_record() -> Vec<u8> {
    let mut r = b"RESC\0\0\0\x10\0\0\0\x01\0\0\0\0".to_vec();
    r.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><package><spine page-progression-direction=\"ltr\"><itemref idref=\"x0\" skelid=\"0\"/><itemref idref=\"x1\" skelid=\"1\" properties=\"page-spread-right\"/></spine></package>\0");
    r
}

/// The records of a KF8 book from its record 0 on. Its resources are at
/// `first_resource` (relative) when `own_resources`; the tail records
/// (FDST...) follow.
fn kf8_records(own_resources: bool, extra_exth: Vec<(u32, Vec<u8>)>) -> Vec<Vec<u8>> {
    let off_c = FRAG_C.find("Middle").unwrap();
    let frag_a = FRAG_A.replace("OFFSETCCCC", &base32(off_c, 10));
    let (a, b, c) = (frag_a.as_str(), FRAG_B, FRAG_C);
    let flow0 = format!("{SKEL0}{a}{b}{SKEL1}{c}");
    let text = format!("{flow0}{CSS}");
    let ip0 = SKEL0.find("</body>").unwrap();
    let start1 = SKEL0.len() + a.len() + b.len();
    let ip1 = start1 + SKEL1.find("</body>").unwrap();
    let skel_tagx = [(1, 1, 3, 0), (6, 2, 12, 0), (0, 0, 0, 1)];
    let skel = indx(
        &skel_tagx,
        &[
            (b"SKEL0000000".to_vec(), vec![(1, vec![2]), (6, vec![0, SKEL0.len() as u64])]),
            (b"SKEL0000001".to_vec(), vec![(1, vec![1]), (6, vec![start1 as u64, SKEL1.len() as u64])]),
        ],
        vec![],
    );
    let (sel, sel_offs) = cncx(&["P-//*[@aid='0']", "P-//*[@aid='5']"]);
    let frag_tagx = [(2, 1, 1, 0), (3, 1, 2, 0), (4, 1, 4, 0), (6, 2, 8, 0), (0, 0, 0, 1)];
    let entry = |ip: usize, sel: u64, file: u64, seq: u64, start: usize, len: usize| {
        (
            format!("{ip:010}").into_bytes(),
            vec![(2, vec![sel]), (3, vec![file]), (4, vec![seq]), (6, vec![start as u64, len as u64])],
        )
    };
    let frag = indx(
        &frag_tagx,
        &[
            entry(ip0, sel_offs[0], 0, 0, 0, a.len()),
            entry(ip0 + a.len(), sel_offs[0], 0, 1, a.len(), b.len()),
            entry(ip1, sel_offs[1], 1, 2, 0, c.len()),
        ],
        vec![sel],
    );
    let (labels, l) = cncx(&["Part One", "Part Two", "Second fragment", "Anchor"]);
    let ncx_tagx =
        [(1, 1, 1, 0), (2, 1, 2, 0), (3, 1, 4, 0), (4, 1, 8, 0), (6, 2, 16, 0), (21, 1, 32, 0), (0, 0, 0, 1)];
    let span = c.find("<span").unwrap() as u64;
    let ncx = indx(
        &ncx_tagx,
        &[
            (b"0".to_vec(), vec![(1, vec![0]), (3, vec![l[0]]), (4, vec![0]), (6, vec![0, 0])]),
            (b"1".to_vec(), vec![(1, vec![0]), (3, vec![l[1]]), (4, vec![0]), (6, vec![2, 0])]),
            (b"2".to_vec(), vec![(1, vec![0]), (3, vec![l[2]]), (4, vec![1]), (6, vec![1, 0]), (21, vec![0])]),
            (b"3".to_vec(), vec![(1, vec![0]), (3, vec![l[3]]), (4, vec![1]), (6, vec![2, span]), (21, vec![1])]),
        ],
        vec![labels],
    );
    let flags = 0b11;
    let texts = text_records(text.as_bytes(), 211, 2, flags);
    let mut idx = 1 + texts.len();
    let skel_at = idx;
    idx += skel.len();
    let frag_at = idx;
    idx += frag.len();
    let ncx_at = idx;
    idx += ncx.len();
    let resources = vec![PNG.to_vec(), font_record(), resc_record(), JPEG.to_vec()];
    let res_at = idx;
    if own_resources {
        idx += resources.len();
    }
    let fdst_at = idx;
    let mut fdst = b"FDST".to_vec();
    fdst.extend_from_slice(&num32(12));
    fdst.extend_from_slice(&num32(2));
    for (s, e) in [(0, flow0.len()), (flow0.len(), text.len())] {
        fdst.extend_from_slice(&num32(s as u32));
        fdst.extend_from_slice(&num32(e as u32));
    }
    let mut exth = vec![
        (100, b"Cara Coder".to_vec()),
        (503, b"Synthetic Eight".to_vec()),
        (524, b"fr_CA".to_vec()),
        (201, 3u32.to_be_bytes().to_vec()),
        (125, 4u32.to_be_bytes().to_vec()),
    ];
    exth.extend(extra_exth);
    let mut records = vec![rec0(&Rec0 {
        compression: 2,
        text_len: text.len(),
        text_records: texts.len(),
        version: 8,
        first_resource: if own_resources { res_at as u32 } else { fdst_at as u32 },
        extra_flags: flags,
        ncx: ncx_at as u32,
        fdst: fdst_at as u32,
        fdst_count: 2,
        frag: frag_at as u32,
        skel: skel_at as u32,
        exth,
        title: "Synthetic Eight",
        ..Rec0::default()
    })];
    records.extend(texts);
    records.extend(skel);
    records.extend(frag);
    records.extend(ncx);
    if own_resources {
        records.extend(resources);
    }
    records.push(fdst);
    records.extend(tail());
    records
}

fn kf8_book() -> Vec<u8> {
    palmdb("Synthetic_Eight", &kf8_records(true, vec![]))
}

/// A joint file: a MOBI 7 book with the shared resources, BOUNDARY, and
/// the KF8 book. `break_kf8` damages the KF8 skeleton index.
fn joint_book(break_kf8: bool) -> Vec<u8> {
    let html = "<html><body><p>Old MOBI text <img recindex=\"00001\"/></p></body></html>";
    let texts = text_records(html.as_bytes(), 4096, 1, 0);
    let first_res = 1 + texts.len();
    let resources = vec![PNG.to_vec(), font_record(), resc_record(), JPEG.to_vec()];
    let boundary = first_res + resources.len() + 3;
    let mut kf8 = kf8_records(false, vec![]);
    if break_kf8 {
        // The first index header is the skeleton index.
        for r in kf8.iter_mut() {
            if r.starts_with(b"INDX") && r.windows(4).any(|w| w == b"TAGX") {
                r[0] = b'X';
                break;
            }
        }
    }
    let mut records = vec![rec0(&Rec0 {
        text_len: html.len(),
        text_records: texts.len(),
        first_resource: first_res as u32,
        exth: vec![
            (100, b"Cara Coder".to_vec()),
            (201, 3u32.to_be_bytes().to_vec()),
            (121, ((boundary + 1) as u32).to_be_bytes().to_vec()),
            (125, 4u32.to_be_bytes().to_vec()),
        ],
        title: "Joint",
        ..Rec0::default()
    })];
    records.extend(texts);
    records.extend(resources);
    records.extend(tail());
    assert_eq!(records.len(), boundary);
    records.push(b"BOUNDARY".to_vec());
    records.extend(kf8);
    palmdb("Joint", &records)
}

// ---------------------------------------------------------------------
// A small XML parser, to check well-formedness and read the package.

#[derive(Debug, Default, Clone)]
struct XNode {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<XNode>,
    text: String,
}

impl XNode {
    fn attr(&self, n: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == n).map(|a| a.1.as_str())
    }

    fn all<'n>(&'n self, name: &str, out: &mut Vec<&'n XNode>) {
        if self.name == name {
            out.push(self);
        }
        for c in &self.children {
            c.all(name, out);
        }
    }

    fn find_all(&self, name: &str) -> Vec<&XNode> {
        let mut v = Vec::new();
        self.all(name, &mut v);
        v
    }

    fn text_all(&self) -> String {
        let mut s = self.text.clone();
        for c in &self.children {
            s.push_str(&c.text_all());
        }
        s
    }

    fn ids(&self, out: &mut HashSet<String>) {
        if let Some(i) = self.attr("id") {
            out.insert(i.to_string());
        }
        for c in &self.children {
            c.ids(out);
        }
    }
}

struct XmlParser<'s> {
    s: &'s [u8],
    i: usize,
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn is_name_char(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

impl XmlParser<'_> {
    fn err<T>(&self, m: &str) -> Result<T, String> {
        let lo = self.i.saturating_sub(40);
        let hi = (self.i + 40).min(self.s.len());
        Err(format!("{m} at {}: {:?}", self.i, String::from_utf8_lossy(&self.s[lo..hi])))
    }

    fn starts(&self, p: &str) -> bool {
        self.s[self.i..].starts_with(p.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn until(&mut self, end: &str) -> Result<(), String> {
        match self.s[self.i..].windows(end.len()).position(|w| w == end.as_bytes()) {
            Some(p) => {
                self.i += p + end.len();
                Ok(())
            }
            None => self.err(&format!("unterminated, expected {end}")),
        }
    }

    fn misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.until("?>")?;
            } else if self.starts("<!--") {
                self.i += 4;
                self.until("-->")?;
            } else if self.starts("<!DOCTYPE") {
                self.until(">")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let st = self.i;
        if self.i >= self.s.len() || !is_name_start(self.s[self.i]) {
            return self.err("bad name");
        }
        while self.i < self.s.len() && is_name_char(self.s[self.i]) {
            self.i += 1;
        }
        Ok(String::from_utf8_lossy(&self.s[st..self.i]).into_owned())
    }

    fn reference(&mut self, out: &mut String) -> Result<(), String> {
        let st = self.i;
        let Some(semi) = self.s[st..].iter().position(|&b| b == b';') else { return self.err("bad reference") };
        let r = String::from_utf8_lossy(&self.s[st + 1..st + semi]).into_owned();
        let c = match r.as_str() {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ if r.starts_with("#x") => {
                char::from_u32(u32::from_str_radix(&r[2..], 16).map_err(|_| "bad char ref".to_string())?)
                    .ok_or("bad char")?
            }
            _ if r.starts_with('#') => {
                char::from_u32(r[1..].parse::<u32>().map_err(|_| "bad char ref".to_string())?).ok_or("bad char")?
            }
            _ => return self.err("undefined entity"),
        };
        out.push(c);
        self.i = st + semi + 1;
        Ok(())
    }

    fn element(&mut self, ns: &[HashSet<String>], depth: usize) -> Result<XNode, String> {
        if depth > 300 {
            return self.err("too deep");
        }
        self.i += 1;
        let name = self.name()?;
        let mut node = XNode { name: name.clone(), ..XNode::default() };
        let mut prefixes = HashSet::new();
        loop {
            let before = self.i;
            self.skip_ws();
            if self.starts("/>") || self.starts(">") {
                break;
            }
            if self.i == before {
                return self.err("missing space between attributes");
            }
            let an = self.name()?;
            self.skip_ws();
            if !self.starts("=") {
                return self.err("attribute without value");
            }
            self.i += 1;
            self.skip_ws();
            let q = self.s.get(self.i).copied().ok_or("eof")?;
            if q != b'"' && q != b'\'' {
                return self.err("unquoted attribute");
            }
            self.i += 1;
            let mut v = String::new();
            loop {
                let c = *self.s.get(self.i).ok_or("eof in attribute")?;
                if c == q {
                    self.i += 1;
                    break;
                }
                match c {
                    b'<' => return self.err("< in attribute"),
                    b'&' => self.reference(&mut v)?,
                    _ => {
                        let st = self.i;
                        self.i += 1;
                        while self.i < self.s.len() && !matches!(self.s[self.i], b'&' | b'<') && self.s[self.i] != q {
                            self.i += 1;
                        }
                        v.push_str(&String::from_utf8_lossy(&self.s[st..self.i]));
                    }
                }
            }
            if node.attrs.iter().any(|a| a.0 == an) {
                return self.err("duplicate attribute");
            }
            if let Some(p) = an.strip_prefix("xmlns:") {
                prefixes.insert(p.to_string());
            }
            node.attrs.push((an, v));
        }
        let mut scope: Vec<HashSet<String>> = ns.to_vec();
        scope.push(prefixes);
        let declared = |n: &str| match n.split_once(':') {
            None => true,
            Some(("xml", _)) | Some(("xmlns", _)) => true,
            Some((p, _)) => scope.iter().any(|s| s.contains(p)),
        };
        if !declared(&node.name) {
            return self.err("undeclared element prefix");
        }
        if let Some(a) = node.attrs.iter().find(|a| !declared(&a.0)) {
            return Err(format!("undeclared attribute prefix {}", a.0));
        }
        if self.starts("/>") {
            self.i += 2;
            return Ok(node);
        }
        self.i += 1;
        loop {
            if self.i >= self.s.len() {
                return self.err("unclosed element");
            }
            if self.starts("</") {
                self.i += 2;
                let end = self.name()?;
                self.skip_ws();
                if end != name || !self.starts(">") {
                    return self.err(&format!("mismatched end tag {end} for {name}"));
                }
                self.i += 1;
                return Ok(node);
            }
            if self.starts("<!--") {
                self.i += 4;
                self.until("-->")?;
            } else if self.starts("<![CDATA[") {
                let st = self.i + 9;
                self.until("]]>")?;
                node.text.push_str(&String::from_utf8_lossy(&self.s[st..self.i - 3]));
            } else if self.starts("<?") {
                self.until("?>")?;
            } else if self.starts("<") {
                let child = self.element(&scope, depth + 1)?;
                node.children.push(child);
            } else if self.starts("&") {
                let mut t = String::new();
                self.reference(&mut t)?;
                node.text.push_str(&t);
            } else {
                if self.starts("]]>") {
                    return self.err("]]> in text");
                }
                let c = self.s[self.i];
                if c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r') {
                    return self.err("control character");
                }
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len() && !matches!(self.s[self.i], b'<' | b'&' | b']') {
                    let c = self.s[self.i];
                    if c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r') {
                        return self.err("control character");
                    }
                    self.i += 1;
                }
                node.text.push_str(&String::from_utf8_lossy(&self.s[st..self.i]));
            }
        }
    }
}

/// Parses a whole XML document (prolog, one root, nothing after).
fn parse_xml(s: &[u8]) -> Result<XNode, String> {
    if std::str::from_utf8(s).is_err() {
        return Err("not UTF-8".into());
    }
    let mut p = XmlParser { s, i: 0 };
    p.misc()?;
    if !p.starts("<") {
        return p.err("no root element");
    }
    let root = p.element(&[], 0)?;
    p.misc()?;
    if p.i != s.len() {
        return p.err("content after the root");
    }
    Ok(root)
}

// ---------------------------------------------------------------------
// The EPUB checker.

struct Epub {
    files: BTreeMap<String, Vec<u8>>,
    opf: XNode,
    /// Spine documents, as OEBPS-relative paths.
    spine: Vec<String>,
    /// Navigation entries, depth first: (depth, label, href).
    nav: Vec<(usize, String, String)>,
}

impl Epub {
    fn text(&self, path: &str) -> String {
        String::from_utf8(self.files[&format!("OEBPS/{path}")].clone()).unwrap()
    }

    fn meta(&self, name: &str) -> Vec<String> {
        self.opf.find_all(name).iter().map(|n| n.text_all()).collect()
    }
}

fn temp_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lain-mobi-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{tag}.epub"))
}

/// Resolves `href` from a file in `dir` (OEBPS-relative); `None` when it
/// is an external URL.
fn resolve(dir: &str, href: &str) -> Option<(String, Option<String>)> {
    if href.contains("://") || href.starts_with("mailto:") {
        return None;
    }
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f.to_string())),
        None => (href, None),
    };
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
    if path.is_empty() {
        return Some((String::new(), frag));
    }
    for seg in path.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." => {}
            s => parts.push(s),
        }
    }
    Some((parts.join("/"), frag))
}

/// Opens the EPUB and checks every invariant; returns it for more checks.
fn check_epub(bytes: &[u8], tag: &str) -> Epub {
    let path = temp_path(tag);
    std::fs::write(&path, bytes).unwrap();
    let z = Archive::open(&path).unwrap_or_else(|e| panic!("{tag}: {e}"));
    let first = &z.entries()[0];
    assert_eq!(first.name, "mimetype", "{tag}");
    assert_eq!(first.method, 0, "{tag}: mimetype must be stored");
    let mut files = BTreeMap::new();
    for e in z.entries() {
        assert!(files.insert(e.name.clone(), z.read(e, 1 << 30).unwrap()).is_none(), "{tag}: duplicate {}", e.name);
    }
    std::fs::remove_file(&path).ok();
    assert_eq!(files["mimetype"], b"application/epub+zip");
    let container = parse_xml(&files["META-INF/container.xml"]).unwrap();
    assert_eq!(container.find_all("rootfile")[0].attr("full-path"), Some("OEBPS/content.opf"));
    let opf = parse_xml(&files["OEBPS/content.opf"]).unwrap_or_else(|e| panic!("{tag}: opf: {e}"));
    assert_eq!(opf.attr("version"), Some("3.0"));
    let ident = opf.find_all("dc:identifier");
    assert_eq!(ident.len(), 1);
    assert_eq!(ident[0].attr("id"), opf.attr("unique-identifier"));
    assert!(!opf.find_all("dc:title")[0].text_all().is_empty());
    assert_eq!(opf.find_all("dc:language").len(), 1);
    assert_eq!(
        opf.find_all("meta").iter().filter(|m| m.attr("property") == Some("dcterms:modified")).count(),
        1,
        "{tag}"
    );
    // The manifest lists exactly the package's files.
    let mut manifest = HashMap::new();
    for item in opf.find_all("item") {
        let href = item.attr("href").unwrap();
        let full = format!("OEBPS/{href}");
        assert!(files.contains_key(&full), "{tag}: manifest item {href} missing");
        assert!(manifest.insert(item.attr("id").unwrap().to_string(), item).is_none(), "{tag}: duplicate id");
    }
    for name in files.keys() {
        if name == "mimetype" || name.starts_with("META-INF/") || name == "OEBPS/content.opf" {
            continue;
        }
        let rel = &name["OEBPS/".len()..];
        assert!(manifest.values().any(|i| i.attr("href") == Some(rel)), "{tag}: {name} not in the manifest");
    }
    let navs: Vec<_> = manifest.values().filter(|i| i.attr("properties") == Some("nav")).collect();
    assert_eq!(navs.len(), 1, "{tag}");
    let mut spine = Vec::new();
    for r in opf.find_all("itemref") {
        let item = manifest[r.attr("idref").unwrap()];
        assert_eq!(item.attr("media-type"), Some("application/xhtml+xml"));
        spine.push(item.attr("href").unwrap().to_string());
    }
    assert!(!spine.is_empty(), "{tag}: empty spine");
    // Every XHTML file is well formed, and its links resolve.
    let mut docs = HashMap::new();
    for (id, item) in &manifest {
        let mt = item.attr("media-type").unwrap();
        let href = item.attr("href").unwrap();
        let data = &files[&format!("OEBPS/{href}")];
        if mt == "application/xhtml+xml" {
            let x = parse_xml(data).unwrap_or_else(|e| panic!("{tag}: {href}: {e}"));
            assert_eq!(x.name, "html");
            assert_eq!(x.attr("xmlns"), Some("http://www.w3.org/1999/xhtml"));
            assert_eq!(x.find_all("title").len(), 1, "{tag}: {href} title");
            let has_svg = !x.find_all("svg").is_empty();
            let props = item.attr("properties").unwrap_or("");
            assert_eq!(has_svg, props.split(' ').any(|p| p == "svg"), "{tag}: {href} svg property");
            docs.insert(href.to_string(), x);
        } else if mt == "application/x-dtbncx+xml" || mt == "image/svg+xml" {
            parse_xml(data).unwrap_or_else(|e| panic!("{tag}: {href}: {e}"));
        }
        assert!(item.attr("id").is_some_and(|i| i == id.as_str()));
    }
    let ids: HashMap<&String, HashSet<String>> = docs
        .iter()
        .map(|(k, v)| {
            let mut s = HashSet::new();
            v.ids(&mut s);
            (k, s)
        })
        .collect();
    let check_link = |from: &str, href: &str| {
        let dir = from.rsplit_once('/').map_or("", |p| p.0);
        let Some((path, frag)) = resolve(dir, href) else { return };
        let target = if path.is_empty() { from.to_string() } else { path };
        assert!(files.contains_key(&format!("OEBPS/{target}")), "{tag}: {from} links to missing {href}");
        if let Some(f) = frag {
            let f = f.replace("%20", " ").replace("%23", "#").replace("%25", "%");
            assert!(ids.get(&target).is_some_and(|s| s.contains(&f)), "{tag}: {from} links to missing id {href}");
        }
    };
    for (href, x) in &docs {
        for (el, attr) in [("a", "href"), ("img", "src"), ("link", "href"), ("image", "xlink:href")] {
            for n in x.find_all(el) {
                if let Some(v) = n.attr(attr) {
                    check_link(href, v);
                }
            }
        }
    }
    for (_, item) in manifest.iter().filter(|(_, i)| i.attr("media-type") == Some("text/css")) {
        let href = item.attr("href").unwrap();
        let css = String::from_utf8_lossy(&files[&format!("OEBPS/{href}")]).into_owned();
        let mut rest = css.as_str();
        while let Some(p) = rest.find("url(") {
            let tail = &rest[p + 4..];
            let end = tail.find(')').unwrap();
            let u = tail[..end].trim().trim_matches(['"', '\'']);
            if !u.is_empty() {
                check_link(href, u);
            }
            rest = &tail[end..];
        }
    }
    for item in manifest.values() {
        let mt = item.attr("media-type").unwrap();
        if mt.starts_with("image/") && mt != "image/svg+xml" {
            let e = z.find(&format!("OEBPS/{}", item.attr("href").unwrap())).unwrap();
            assert_eq!(e.method, 0, "{tag}: images are stored");
        }
    }
    // The navigation: nested lists of links that resolve.
    let nav_href = navs[0].attr("href").unwrap();
    let nav_doc = &docs[nav_href];
    let toc = nav_doc.find_all("nav").into_iter().find(|n| n.attr("epub:type") == Some("toc")).expect("toc nav");
    let mut nav = Vec::new();
    fn walk(ol: &XNode, depth: usize, out: &mut Vec<(usize, String, String)>) {
        for li in ol.children.iter().filter(|c| c.name == "li") {
            let a = li.children.iter().find(|c| c.name == "a").expect("li without a link");
            assert!(!a.text_all().trim().is_empty(), "empty nav label");
            out.push((depth, a.text_all(), a.attr("href").unwrap().to_string()));
            for sub in li.children.iter().filter(|c| c.name == "ol") {
                walk(sub, depth + 1, out);
            }
        }
    }
    let ol = toc.children.iter().find(|c| c.name == "ol").expect("nav ol");
    walk(ol, 0, &mut nav);
    assert!(!nav.is_empty());
    let ncx = parse_xml(&files["OEBPS/toc.ncx"]).unwrap();
    let points = ncx.find_all("navPoint");
    assert_eq!(points.len(), nav.len(), "{tag}: NCX and nav differ");
    for (p, n) in points.iter().zip(&nav) {
        let src = p.find_all("content")[0].attr("src").unwrap();
        assert_eq!(src, n.2);
        check_link(nav_href, src);
    }
    let uid = ncx.find_all("meta").into_iter().find(|m| m.attr("name") == Some("dtb:uid")).unwrap();
    assert_eq!(uid.attr("content").map(str::to_string), Some(ident[0].text_all()));
    Epub { files, opf, spine, nav }
}

fn convert(data: &[u8], tag: &str) -> Epub {
    let a = to_epub(data).unwrap_or_else(|e| panic!("{tag}: {e}"));
    let b = to_epub(data).unwrap();
    assert!(a == b, "{tag}: output is not deterministic");
    check_epub(&a, tag)
}

// ---------------------------------------------------------------------
// Tests.

#[test]
fn mobi7_converts_with_links_images_and_metadata() {
    let book = mobi7_book(2, true);
    let m = metadata(&book).unwrap();
    assert_eq!(m.format, "mobi");
    assert_eq!(m.title, "Synthetic Seven");
    assert_eq!(m.authors, ["Ann Author", "Bob Writer"]);
    assert_eq!(m.publisher, "Test Press");
    assert_eq!(m.description, "A test & more");
    assert_eq!(m.subjects, ["Fiction", "Tests"]);
    assert_eq!(m.language, "en");
    assert_eq!(m.isbn, "978-0-00-000000-2");
    assert_eq!(m.published, "2001-02-03T00:00:00+00:00");
    assert!(!m.rtl);
    assert_eq!(m.cover, Some((JPEG.to_vec(), "image/jpeg".to_string())));

    let e = convert(&book, "mobi7");
    assert_eq!(e.meta("dc:title"), ["Synthetic Seven"]);
    assert_eq!(e.meta("dc:creator"), ["Ann Author", "Bob Writer"]);
    assert_eq!(e.meta("dc:identifier"), ["urn:isbn:9780000000002"]);
    assert_eq!(e.meta("dc:language"), ["en"]);
    assert_eq!(e.meta("dc:date"), ["2001-02-03"]);
    assert_eq!(e.meta("dc:subject"), ["Fiction", "Tests"]);
    assert_eq!(e.meta("dc:description"), ["A test & more"]);
    // The cover is not in the text, so a cover page comes first.
    assert_eq!(e.spine, ["cover.xhtml", "chapter0001.xhtml", "chapter0002.xhtml", "chapter0003.xhtml"]);
    let cover = e.opf.find_all("item").into_iter().find(|i| i.attr("properties") == Some("cover-image")).unwrap();
    assert_eq!(cover.attr("href"), Some("images/image00002.jpg"));
    assert_eq!(e.files["OEBPS/images/image00001.png"], PNG);

    let html = fill_filepos(MOBI7_HTML, &[]);
    let off = |t: &str| html.find(t).unwrap();
    let c1 = e.text("chapter0001.xhtml");
    let c2 = e.text("chapter0002.xhtml");
    let c3 = e.text("chapter0003.xhtml");
    assert!(c1.contains(&format!("<p id=\"fpos{}\">Contents</p>", off("<p>Contents"))), "{c1}");
    assert!(c1.contains(&format!("href=\"chapter0002.xhtml#fpos{}\"", off("<h2>Chapter One"))), "{c1}");
    // A link to a page break lands at the start of the next chapter.
    let brk = off("<mbp:pagebreak/><h2>Chapter Two");
    assert!(c1.contains(&format!("href=\"chapter0003.xhtml#fpos{brk}\"")), "{c1}");
    assert!(c3.starts_with("<?xml") && c3.contains(&format!("<span id=\"fpos{brk}\"></span><h2")), "{c3}");
    // A link into the middle of a text run.
    let mid = off("middle target");
    assert!(c2.contains(&format!("href=\"chapter0003.xhtml#fpos{mid}\"")), "{c2}");
    assert!(c3.contains(&format!("the <span id=\"fpos{mid}\"></span>middle target")), "{c3}");
    assert!(c2.contains("Caf\u{e9} na\u{ef}ve \u{2014} text"), "{c2}");
    assert!(c2.contains("<img src=\"images/image00001.png\" alt=\"\"/>"), "{c2}");
    assert!(c2.contains("<span style=\"font-size: x-large; color: red\">big red</span>"), "{c2}");
    assert!(c2.contains("style=\"margin-top: 1em; text-indent: 0pt\""), "{c2}");
    assert!(c3.contains("href=\"http://example.com/x?a=1&amp;b=2\""), "{c3}");
    assert!(c3.contains("<a>bad</a>") && !c3.contains("script") && !c3.contains("alert"), "{c3}");
    assert!(c3.contains("<table><tr><td>cell</td><td>two</td></tr></table>"), "{c3}");
    // The NCX, nested by its parent links.
    let nav: Vec<(usize, &str, String)> = e.nav.iter().map(|(d, l, h)| (*d, l.as_str(), h.clone())).collect();
    assert_eq!(
        nav,
        [
            (0, "Contents", format!("chapter0001.xhtml#fpos{}", off("<p>Contents"))),
            (0, "Chapter One", format!("chapter0002.xhtml#fpos{}", off("<h2>Chapter One"))),
            (0, "Chapter Two", format!("chapter0003.xhtml#fpos{}", off("<h2>Chapter Two"))),
            (1, "Middle & target", format!("chapter0003.xhtml#fpos{mid}")),
        ]
    );
}

#[test]
fn compression_does_not_change_the_output() {
    let plain = to_epub(&mobi7_book(1, true)).unwrap();
    let lz = to_epub(&mobi7_book(2, true)).unwrap();
    assert!(plain == lz);
}

#[test]
fn mobi7_toc_falls_back_to_the_guide_page_then_headings() {
    let e = convert(&mobi7_book(1, false), "mobi7-guide");
    let labels: Vec<&str> = e.nav.iter().map(|n| n.1.as_str()).collect();
    assert_eq!(labels, ["Chapter One", "Chapter Two"]);
    // Without a guide either, the headings.
    let html = "<html><body><h1>Alpha</h1><p>a</p><h2>Beta</h2><p>b</p><mbp:pagebreak/><h1>Gamma</h1></body></html>";
    let texts = text_records(html.as_bytes(), 50, 2, 0);
    let mut records =
        vec![rec0(&Rec0 { compression: 2, text_len: html.len(), text_records: texts.len(), ..Rec0::default() })];
    records.extend(texts);
    records.extend(tail());
    let e = convert(&palmdb("Headings", &records), "mobi7-headings");
    let nav: Vec<(usize, &str)> = e.nav.iter().map(|n| (n.0, n.1.as_str())).collect();
    assert_eq!(nav, [(0, "Alpha"), (1, "Beta"), (0, "Gamma")]);
    assert_eq!(e.meta("dc:title"), ["Book"]);
    assert_eq!(e.meta("dc:language"), ["en"]);
    assert!(e.meta("dc:identifier")[0].starts_with("urn:lain:"));
}

#[test]
fn cp1252_text_is_decoded() {
    let html = b"<html><body><p>Caf\xe9 \x93quoted\x94 \x80 &eacute;</p></body></html>";
    let mut records = vec![rec0(&Rec0 {
        utf8: false,
        text_len: html.len(),
        text_records: 1,
        exth: vec![(100, b"Jos\xe9".to_vec())],
        ..Rec0::default()
    })];
    records.push(html.to_vec());
    records.extend(tail());
    let book = palmdb("cp1252", &records);
    assert_eq!(metadata(&book).unwrap().authors, ["Jos\u{e9}"]);
    let e = convert(&book, "cp1252");
    assert!(e.text("chapter0001.xhtml").contains("Caf\u{e9} \u{201c}quoted\u{201d} \u{20ac} \u{e9}"));
}

#[test]
fn huff_cdic_books_decompress() {
    let book = huff_book();
    let e = convert(&book, "huff");
    assert_eq!(e.spine, ["chapter0001.xhtml", "chapter0002.xhtml"]);
    assert!(e.text("chapter0001.xhtml").contains("<p>Hello world</p>"));
    assert!(e.text("chapter0002.xhtml").contains("<p>Huff world!</p>"));
}

#[test]
fn kf8_converts_with_parts_flows_and_resources() {
    let book = kf8_book();
    let m = metadata(&book).unwrap();
    assert_eq!(m.format, "azw3");
    assert_eq!(m.title, "Synthetic Eight");
    assert_eq!(m.authors, ["Cara Coder"]);
    assert_eq!(m.language, "fr-CA");
    assert_eq!(m.cover, Some((JPEG.to_vec(), "image/jpeg".to_string())));

    let e = convert(&book, "kf8");
    assert_eq!(e.spine, ["cover.xhtml", "part0000.xhtml", "part0001.xhtml"]);
    let p0 = e.text("part0000.xhtml");
    let p1 = e.text("part0001.xhtml");
    assert!(p0.contains("<title>Part One</title>"), "{p0}");
    assert!(p0.contains("<link rel=\"stylesheet\" type=\"text/css\" href=\"styles/flow0001.css\"/>"), "{p0}");
    assert!(p0.contains("<div id=\"aid-1\"><p>First part. <a href=\"part0000.xhtml#aid-3\">"), "{p0}");
    assert!(p0.contains("<a href=\"part0001.xhtml#aid-7\">to part two</a>"), "{p0}");
    assert!(p0.contains("<h1 id=\"aid-3\">Second fragment</h1>"), "{p0}");
    assert!(p0.contains("<img src=\"images/image00001.png\" alt=\"\"/>"), "{p0}");
    assert!(p0.contains("<image width=\"10\" height=\"10\" xlink:href=\"images/image00001.png\"/>"), "{p0}");
    assert!(p1.contains("<p id=\"aid-7\">Back to <a href=\"part0000.xhtml#aid-1\">the start</a>"), "{p1}");
    assert!(p1.contains("<span id=\"aid-8\">anchor</span> text \u{e9}t\u{e9}."), "{p1}");
    assert!(p1.contains("h1 { color: blue }"), "{p1}");
    let css = e.text("styles/flow0001.css");
    assert!(css.contains("url(../fonts/font00002.ttf)"), "{css}");
    assert!(css.contains("url(\"../images/image00001.png\")"), "{css}");
    assert!(!css.contains("@import") && !css.contains("example.com"), "{css}");
    assert_eq!(e.files["OEBPS/fonts/font00002.ttf"], font_data());
    let font = e.opf.find_all("item").into_iter().find(|i| i.attr("href") == Some("fonts/font00002.ttf")).unwrap();
    assert_eq!(font.attr("media-type"), Some("font/ttf"));
    let spread: Vec<_> = e.opf.find_all("itemref").into_iter().filter_map(|i| i.attr("properties")).collect();
    assert_eq!(spread, ["page-spread-right"]);
    let nav: Vec<(usize, &str, &str)> = e.nav.iter().map(|(d, l, h)| (*d, l.as_str(), h.as_str())).collect();
    assert_eq!(
        nav,
        [
            (0, "Part One", "part0000.xhtml#aid-1"),
            (1, "Second fragment", "part0000.xhtml#aid-3"),
            (0, "Part Two", "part0001.xhtml#aid-6"),
            (1, "Anchor", "part0001.xhtml#aid-8"),
        ]
    );
    assert_eq!(e.meta("dc:language"), ["fr-CA"]);
}

#[test]
fn joint_files_prefer_kf8_and_fall_back_to_mobi7() {
    let book = joint_book(false);
    let m = metadata(&book).unwrap();
    assert_eq!(m.format, "azw3");
    assert_eq!(m.title, "Synthetic Eight");
    assert_eq!(m.cover, Some((JPEG.to_vec(), "image/jpeg".to_string())));
    let e = convert(&book, "joint");
    assert_eq!(e.spine, ["cover.xhtml", "part0000.xhtml", "part0001.xhtml"]);
    assert!(e.text("part0000.xhtml").contains("<img src=\"images/image00001.png\" alt=\"\"/>"));
    assert_eq!(e.files["OEBPS/fonts/font00002.ttf"], font_data());
    // A broken KF8 half: the MOBI 7 half is used.
    let e = convert(&joint_book(true), "joint-broken");
    assert_eq!(e.spine, ["cover.xhtml", "chapter0001.xhtml"]);
    assert!(e.text("chapter0001.xhtml").contains("Old MOBI text <img src=\"images/image00001.png\""));
}

#[test]
fn encrypted_books_are_drm() {
    for version in [6, 8] {
        let html = b"<p>secret</p>";
        let mut records =
            vec![rec0(&Rec0 { encryption: 2, version, text_len: html.len(), text_records: 1, ..Rec0::default() })];
        records.push(html.to_vec());
        let book = palmdb("Locked", &records);
        assert_eq!(metadata(&book), Err(MobiError::Drm));
        assert_eq!(to_epub(&book), Err(MobiError::Drm));
    }
    // An encrypted KF8 half of a joint file is DRM too.
    let mut book = joint_book(false);
    let boundary = book.windows(8).position(|w| w == b"BOUNDARY").unwrap();
    let k8 = boundary + 8;
    book[k8 + 12] = 0;
    book[k8 + 13] = 1;
    assert_eq!(to_epub(&book), Err(MobiError::Drm));
}

#[test]
fn not_mobi_and_oversized_inputs_are_refused() {
    assert!(matches!(to_epub(b""), Err(MobiError::Invalid(_))));
    assert!(matches!(to_epub(&[0u8; 200]), Err(MobiError::Invalid(_))));
    let mut text = vec![0u8; 100];
    text[60..68].copy_from_slice(b"TEXtREAd");
    assert!(matches!(metadata(&text), Err(MobiError::Unsupported(_))));
    // Records out of order.
    let mut book = mobi7_book(1, false);
    let (a, b) = (78 + 8, 78 + 16);
    let (x, y) = (book[a..a + 4].to_vec(), book[b..b + 4].to_vec());
    book[a..a + 4].copy_from_slice(&y);
    book[b..b + 4].copy_from_slice(&x);
    assert!(matches!(to_epub(&book), Err(MobiError::Invalid(m)) if m.contains("out of order")));
    // Unknown compression.
    let mut book = mobi7_book(1, false);
    let r0 = u32::from_be_bytes(book[78..82].try_into().unwrap()) as usize;
    book[r0..r0 + 2].copy_from_slice(&7u16.to_be_bytes());
    assert!(matches!(to_epub(&book), Err(MobiError::Unsupported(_))));
    let huge = vec![0u8; MAX_FILE + 1];
    assert!(matches!(metadata(&huge), Err(MobiError::Invalid(m)) if m.contains("512 MB")));
}

#[test]
fn index_values_with_byte_counts() {
    // Tag 7 has a two-bit mask with both bits set: its values are given
    // as a byte count instead of a value count.
    let tagx = [(1, 1, 1, 0), (7, 1, 6, 0), (0, 0, 0, 1)];
    let recs = indx(&tagx, &[(b"x".to_vec(), vec![(1, vec![5]), (7, vec![300, 1, 2])])], vec![]);
    let mut all = vec![rec0(&Rec0::default())];
    all.extend(recs);
    let file = palmdb("idx", &all);
    let db = PalmDb::parse(&file).unwrap();
    let index = indx::read(&db, 1).unwrap();
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries[0].label, b"x");
    assert_eq!(index.entries[0].values(1), Some(&[5u64][..]));
    assert_eq!(index.entries[0].values(7), Some(&[300u64, 1, 2][..]));
}

/// Every synthetic book, for the hostile tests.
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("mobi7", mobi7_book(2, true)),
        ("mobi7-plain", mobi7_book(1, false)),
        ("huff", huff_book()),
        ("kf8", kf8_book()),
        ("joint", joint_book(false)),
    ]
}

/// Converts damaged input; a success must still be a sound EPUB.
fn survive(data: &[u8], tag: &str, check: bool) {
    let _ = metadata(data);
    if let Ok(epub) = to_epub(data)
        && check
    {
        check_epub(&epub, tag);
    }
}

#[test]
fn truncations_never_panic() {
    for (name, book) in corpus() {
        let step = (book.len() / 300).max(1);
        let cuts = (0..book.len().min(400)).chain((400..book.len()).step_by(step));
        for (i, cut) in cuts.enumerate() {
            survive(&book[..cut], &format!("{name}-cut{cut}"), i % 16 == 0);
        }
    }
}

#[test]
fn byte_flips_never_panic() {
    for (name, book) in corpus() {
        let step = (book.len() / 500).max(1);
        for (n, at) in (0..book.len()).step_by(step).enumerate() {
            for v in [0x00, 0xff, book[at] ^ 0x80] {
                let mut d = book.clone();
                d[at] = v;
                survive(&d, &format!("{name}-flip{at}"), n % 32 == 0 && v == 0xff);
            }
        }
        // Several random bytes at once (a fixed LCG keeps it repeatable).
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for round in 0..200 {
            let mut d = book.clone();
            for _ in 0..8 {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let at = (seed >> 33) as usize % d.len();
                d[at] = (seed >> 13) as u8;
            }
            survive(&d, &format!("{name}-rand{round}"), round % 25 == 0);
        }
    }
}

#[test]
fn hostile_headers_never_panic() {
    let book = mobi7_book(2, true);
    let r0 = u32::from_be_bytes(book[78..82].try_into().unwrap()) as usize;
    // Extreme values in every 16- and 32-bit field of record 0.
    for at in (0..0x118).step_by(2) {
        for v in [0u32, 1, 0x7fff_ffff, 0xffff_fffe, 0xffff_ffff] {
            let mut d = book.clone();
            if at + 4 <= 0x118 {
                d[r0 + at..r0 + at + 4].copy_from_slice(&v.to_be_bytes());
            }
            survive(&d, "hdr", false);
        }
    }
    let kf8 = kf8_book();
    let r0 = u32::from_be_bytes(kf8[78..82].try_into().unwrap()) as usize;
    for at in (0..0x118).step_by(4) {
        for v in [0u32, 1, 2, 0xffff_ffff] {
            let mut d = kf8.clone();
            d[r0 + at..r0 + at + 4].copy_from_slice(&v.to_be_bytes());
            survive(&d, "hdr8", false);
        }
    }
}

#[test]
fn deep_and_wide_tag_soup_stays_linear_and_well_formed() {
    let mut html = String::from("<html><body>");
    for _ in 0..5000 {
        html.push_str("<div><b><i>");
    }
    for _ in 0..20_000 {
        html.push_str("</b>x<p>y</span>");
    }
    html.push_str("<a filepos=0000000000>z</a></body></html>");
    let texts = text_records(html.as_bytes(), 4096, 1, 0);
    let mut records = vec![rec0(&Rec0 { text_len: html.len(), text_records: texts.len(), ..Rec0::default() })];
    records.extend(texts);
    records.extend(tail());
    convert(&palmdb("Deep", &records), "deep");
}

// ---------------------------------------------------------------------
// Real books and large books (ignored: run with --ignored --release).

#[test]
#[ignore]
fn real_samples() {
    let Ok(dir) = std::env::var("MOBI_SAMPLES") else {
        eprintln!("MOBI_SAMPLES is not set; skipping");
        return;
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(Path::new(&dir)).unwrap().flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut failures = Vec::new();
    for p in paths.iter().filter(|p| p.is_file()) {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(p).unwrap();
        let t = std::time::Instant::now();
        let meta = metadata(&data);
        let tm = t.elapsed();
        let t = std::time::Instant::now();
        let out = to_epub(&data);
        let te = t.elapsed();
        match (meta, out) {
            (Ok(m), Ok(epub)) => {
                let e = std::panic::catch_unwind(|| check_epub(&epub, &name));
                match e {
                    Ok(e) => eprintln!(
                        "{name}: {} {:?} by {:?}: {} KB -> {} KB, {} docs, {} nav, {} files; metadata {:?}, epub {:?}",
                        m.format,
                        m.title,
                        m.authors,
                        data.len() / 1024,
                        epub.len() / 1024,
                        e.spine.len(),
                        e.nav.len(),
                        e.files.len(),
                        tm,
                        te
                    ),
                    Err(_) => failures.push(format!("{name}: EPUB check failed")),
                }
            }
            (m, o) => failures.push(format!("{name}: metadata {:?}, epub {:?}", m.err(), o.err())),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A large MOBI 7 book: `chapters` chapters of `size` bytes, linked to
/// each other; without page breaks when `breaks` is false.
fn large_mobi7(chapters: usize, size: usize, breaks: bool) -> Vec<u8> {
    let mut html = String::from("<html><head><guide></guide></head><body>");
    let mut starts = Vec::new();
    let mut para = String::new();
    while para.len() < 400 {
        para.push_str("All happy families are alike; each unhappy family is unhappy in its own way. ");
    }
    for c in 0..chapters {
        starts.push(html.len());
        html.push_str(&format!("<h2>Chapter {c}</h2>"));
        let begin = html.len();
        let mut k = 0;
        while html.len() - begin < size {
            html.push_str("<p>");
            html.push_str(&para);
            html.push_str(&format!("<a filepos=LINK{:06}>next</a></p>", (c + k + 1) % chapters));
            k += 1;
        }
        if breaks {
            html.push_str("<mbp:pagebreak/>");
        }
    }
    html.push_str("</body></html>");
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(p) = rest.find("LINK") {
        out.push_str(&rest[..p]);
        let n: usize = rest[p + 4..p + 10].parse().unwrap();
        out.push_str(&format!("{:010}", starts[n]));
        rest = &rest[p + 10..];
    }
    out.push_str(rest);
    let texts = text_records(out.as_bytes(), 4096, 2, 0b11);
    let mut records = vec![rec0(&Rec0 {
        compression: 2,
        text_len: out.len(),
        text_records: texts.len(),
        extra_flags: 0b11,
        ..Rec0::default()
    })];
    records.extend(texts);
    records.extend(tail());
    palmdb("Large", &records)
}

#[test]
#[ignore]
fn large_books_convert_quickly() {
    for (label, book) in [
        ("20 MB, 400 chapters", large_mobi7(400, 50_000, true)),
        ("20 MB, no page breaks", large_mobi7(400, 50_000, false)),
    ] {
        let t = std::time::Instant::now();
        let epub = to_epub(&book).unwrap();
        let el = t.elapsed();
        let e = check_epub(&epub, label);
        eprintln!(
            "{label}: {} KB in, {} KB out, {} docs, {} nav, {el:?}",
            book.len() / 1024,
            epub.len() / 1024,
            e.spine.len(),
            e.nav.len()
        );
        assert!(el.as_secs() < 10);
    }
}

/// A one-part KF8 book whose `n` fragments all go to the same position
/// (each insert lands before the previous ones: the slow path).
fn kf8_same_position(n: usize) -> Vec<u8> {
    let skel = "<html><head><title>x</title></head><body aid=\"0\"></body></html>";
    let ip = skel.find("</body>").unwrap();
    let piece = "<p>frag</p>";
    let mut text = skel.to_string();
    for _ in 0..n {
        text.push_str(piece);
    }
    let skel_idx = indx(
        &[(1, 1, 3, 0), (6, 2, 12, 0), (0, 0, 0, 1)],
        &[(b"SKEL0000000".to_vec(), vec![(1, vec![n as u64]), (6, vec![0, skel.len() as u64])])],
        vec![],
    );
    let (sel, so) = cncx(&["P-//*[@aid='0']"]);
    let entries: Vec<IndexEntry> = (0..n)
        .map(|k| {
            (
                format!("{ip:010}").into_bytes(),
                vec![(2, vec![so[0]]), (3, vec![0]), (4, vec![k as u64]), (6, vec![0, piece.len() as u64])],
            )
        })
        .collect();
    // One INDX data record holds at most 64 KB; split the entries.
    let mut frag_recs = Vec::new();
    let chunks: Vec<&[IndexEntry]> = entries.chunks(3000).collect();
    for (i, c) in chunks.iter().enumerate() {
        let mut r = indx(&[(2, 1, 1, 0), (3, 1, 2, 0), (4, 1, 4, 0), (6, 2, 8, 0), (0, 0, 0, 1)], c, vec![]);
        let data = r.remove(1);
        if i == 0 {
            frag_recs.push(r.remove(0));
        }
        frag_recs.push(data);
    }
    put32(&mut frag_recs[0], 24, chunks.len() as u32);
    put32(&mut frag_recs[0], 52, 1);
    frag_recs.push(sel);
    let texts = text_records(text.as_bytes(), 4096, 1, 0);
    let skel_at = 1 + texts.len();
    let frag_at = skel_at + skel_idx.len();
    let mut records = vec![rec0(&Rec0 {
        text_len: text.len(),
        text_records: texts.len(),
        version: 8,
        skel: skel_at as u32,
        frag: frag_at as u32,
        ..Rec0::default()
    })];
    records.extend(texts);
    records.extend(skel_idx);
    records.extend(frag_recs);
    records.extend(tail());
    palmdb("Same", &records)
}

#[test]
fn kf8_fragments_at_one_position() {
    let e = convert(&kf8_same_position(50), "same");
    assert_eq!(e.text("part0000.xhtml").matches("<p>frag</p>").count(), 50);
}

#[test]
#[ignore]
fn hostile_structures_stay_fast() {
    let book = |html: String| {
        let texts = text_records(html.as_bytes(), 4096, 1, 0);
        let mut records = vec![rec0(&Rec0 { text_len: html.len(), text_records: texts.len(), ..Rec0::default() })];
        records.extend(texts);
        records.extend(tail());
        palmdb("Hostile", &records)
    };
    let mut deep_lists = String::from("<html><body>");
    deep_lists.push_str(&"<span>".repeat(255));
    deep_lists.push_str(&"<li>x".repeat(2_000_000));
    let mut blocked = String::from("<html><body><p><table>");
    blocked.push_str(&"<span>".repeat(254));
    blocked.push_str(&"<div>".repeat(2_000_000));
    let mut ends = String::from("<html><body>");
    ends.push_str(&"<b>".repeat(250));
    ends.push_str(&"</i></u></q>".repeat(1_000_000));
    for (label, data) in [
        ("255 open spans, 2M li", book(deep_lists)),
        ("table scope, 2M div", book(blocked)),
        ("3M stray end tags", book(ends)),
        ("KF8, 30k fragments at one position", kf8_same_position(30_000)),
    ] {
        let t = std::time::Instant::now();
        let r = to_epub(&data);
        let el = t.elapsed();
        eprintln!("{label}: {} KB, {:?} in {el:?}", data.len() / 1024, r.as_ref().map(|e| e.len() / 1024));
        assert!(el.as_secs() < 10, "{label}");
    }
}
