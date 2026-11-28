use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::image::test_png;
use crate::zip::Writer;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lain-book-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="uid">urn:uuid:12345678-1234-1234-1234-123456789abc</dc:identifier>
    <dc:title>A Test Book</dc:title>
    <dc:creator id="a1">First Author</dc:creator>
    <meta refines="#a1" property="role" scheme="marc:relators">aut</meta>
    <dc:creator id="e1">An Editor</dc:creator>
    <meta refines="#e1" property="role" scheme="marc:relators">edt</meta>
    <dc:language>en</dc:language>
    <dc:description>&lt;p&gt;A &lt;b&gt;fine&lt;/b&gt; book.&lt;/p&gt;</dc:description>
    <dc:subject>Fiction</dc:subject><dc:subject>Tests</dc:subject>
    <dc:date>1999-05-01</dc:date>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c1" href="text/one.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/two%20b.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="css/s.css" media-type="text/css"/>
    <item id="cover-img" href="img/cover.png" media-type="image/png"/>
    <item id="font" href="fonts/f.ttf" media-type="font/ttf"/>
  </manifest>
  <spine page-progression-direction="rtl">
    <itemref idref="c1"/>
    <itemref idref="c2"/>
    <itemref idref="nav" linear="no"/>
  </spine>
</package>"##;

const NAV: &str = r##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="landmarks"><ol><li><a href="text/one.xhtml">Start</a></li></ol></nav>
<nav epub:type="toc"><ol>
  <li><a href="text/one.xhtml">One</a><ol><li><a href="text/one.xhtml#s2">One, part two</a></li></ol></li>
  <li><a href="text/two%20b.xhtml">Two</a></li>
</ol></nav></body></html>"##;

const ONE: &str = r##"<html xmlns="http://www.w3.org/1999/xhtml"><head><link rel="stylesheet" href="../css/s.css"/></head>
<body><h1>Chapter one</h1><p>Hello <a href="two%20b.xhtml#end">there</a>.</p><p id="s2">Part two.</p>
<img src="../img/cover.png" alt=""/><script>evil()</script></body></html>"##;

const TWO: &str = "<html><body><p id=\"end\">The end.</p></body></html>";

fn epub(path: &Path, extra: &[(&str, Vec<u8>)]) {
    let mut w = Writer::new();
    w.add("mimetype", b"application/epub+zip", false);
    w.add(
        "META-INF/container.xml",
        br##"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"##,
        true,
    );
    w.add("OEBPS/content.opf", OPF.as_bytes(), true);
    w.add("OEBPS/nav.xhtml", NAV.as_bytes(), true);
    w.add("OEBPS/text/one.xhtml", ONE.as_bytes(), true);
    w.add("OEBPS/text/two b.xhtml", TWO.as_bytes(), true);
    w.add("OEBPS/css/s.css", b"body { background: url(../img/cover.png) } @import url(http://evil/x.css);", true);
    w.add("OEBPS/img/cover.png", &test_png(3, 4), false);
    for (n, b) in extra {
        w.add(n, b, true);
    }
    std::fs::write(path, w.finish()).unwrap();
}

#[test]
fn epub_manifest_chapters_and_resources() {
    let d = dir("epub");
    let p = d.join("A Test Book.epub");
    epub(&p, &[("OEBPS/fonts/f.ttf", b"font bytes".to_vec())]);
    let books = Books::new();
    let m = books.manifest(&p).unwrap();
    assert_eq!((m.format.as_str(), m.layout.as_str(), m.title.as_str()), ("epub", "reflowable", "A Test Book"));
    assert_eq!(m.authors, vec!["First Author"], "the editor is not an author");
    assert_eq!((m.language.as_str(), m.published.as_str(), m.direction.as_str()), ("en", "1999-05-01", "rtl"));
    assert_eq!(m.description, "A fine book.");
    assert_eq!(m.subjects, vec!["Fiction", "Tests"]);
    assert_eq!(m.chapters.len(), 3);
    assert!(m.chapters[0].chars > 0 && !m.chapters[2].linear);
    assert_eq!(m.count_rule, lain_api::BOOK_COUNT_RULE);
    assert_eq!(m.pages, 1);
    assert_eq!(m.toc.len(), 2, "the toc nav, not the landmarks");
    assert_eq!((m.toc[0].title.as_str(), m.toc[0].chapter), ("One", 0));
    assert_eq!((m.toc[0].children[0].anchor.as_str(), m.toc[1].chapter), ("s2", 1));

    let html = books.chapter(&p, 0, "/api/items/X/book/res/").unwrap();
    assert!(html.contains("data-lain-chapter=\"1\" data-lain-anchor=\"end\""), "{html}");
    assert!(html.contains("src=\"/api/items/X/book/res/OEBPS/img/cover.png\""));
    assert!(html.contains("url(\"/api/items/X/book/res/OEBPS/img/cover.png\")"), "the stylesheet is inlined");
    assert!(!html.contains("evil"));
    assert!(books.chapter(&p, 9, "/x/").is_err());

    let (png, mime) = books.resource(&p, "OEBPS/img/cover.png", "/r/").unwrap();
    assert_eq!((png, mime.as_str()), (test_png(3, 4), "image/png"));
    let (css, mime) = books.resource(&p, "OEBPS/css/s.css", "/r/").unwrap();
    assert_eq!(mime, "text/css");
    let css = String::from_utf8(css).unwrap();
    assert!(css.contains("url(\"/r/OEBPS/img/cover.png\")") && !css.contains("evil"), "{css}");
    for bad in ["META-INF/container.xml", "../etc/passwd", "OEBPS/missing.png"] {
        assert!(books.resource(&p, bad, "/r/").is_err(), "{bad}");
    }
    let (cover, mime) = books.cover(&p).unwrap();
    assert_eq!((cover, mime.as_str()), (test_png(3, 4), "image/png"));
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn obfuscated_fonts_and_drm() {
    let d = dir("enc");
    let p = d.join("f.epub");
    // IDPF obfuscation: XOR with the SHA-1 of the identifier.
    let font: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let key = {
        use sha1::{Digest, Sha1};
        Sha1::digest(b"urn:uuid:12345678-1234-1234-1234-123456789abc").to_vec()
    };
    let mut obf = font.clone();
    for (i, b) in obf.iter_mut().take(1040).enumerate() {
        *b ^= key[i % key.len()];
    }
    let enc = br##"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
      <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
      <enc:CipherData><enc:CipherReference URI="OEBPS/fonts/f.ttf"/></enc:CipherData></enc:EncryptedData></encryption>"##;
    epub(&p, &[("OEBPS/fonts/f.ttf", obf), ("META-INF/encryption.xml", enc.to_vec())]);
    let books = Books::new();
    assert_eq!(books.resource(&p, "OEBPS/fonts/f.ttf", "/r/").unwrap().0, font);
    // A content document under real encryption is DRM.
    let q = d.join("drm.epub");
    let enc =
        br##"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/>
      <CipherData><CipherReference URI="OEBPS/text/one.xhtml"/></CipherData></EncryptedData></encryption>"##;
    epub(&q, &[("META-INF/encryption.xml", enc.to_vec())]);
    assert_eq!(books.manifest(&q).unwrap_err().code, DRM_PROTECTED);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn broken_books_are_typed_errors() {
    let d = dir("bad");
    let books = Books::new();
    for (name, data) in [
        ("a.epub", b"not a zip".to_vec()),
        ("b.pdf", b"%PDF-1.4 nothing".to_vec()),
        ("c.mobi", vec![0u8; 100]),
        ("d.azw3", Vec::new()),
    ] {
        let p = d.join(name);
        std::fs::write(&p, data).unwrap();
        let e = books.manifest(&p).unwrap_err();
        assert!(matches!(e.code, INVALID_BOOK | UNSUPPORTED_BOOK), "{name}: {e:?}");
    }
    assert_eq!(books.manifest(&d.join("missing.epub")).unwrap_err().code, "dependency-unavailable");
    let p = d.join("x.txt");
    std::fs::write(&p, "x").unwrap();
    assert_eq!(books.manifest(&p).unwrap_err().code, UNSUPPORTED_BOOK);
    // Truncated EPUBs never panic.
    let full = d.join("full.epub");
    epub(&full, &[]);
    let bytes = std::fs::read(&full).unwrap();
    for cut in (0..bytes.len()).step_by(97) {
        let p = d.join(format!("cut{cut}.epub"));
        std::fs::write(&p, &bytes[..cut]).unwrap();
        let _ = books.manifest(&p);
        std::fs::remove_file(&p).ok();
    }
    std::fs::remove_dir_all(d).ok();
}

fn tiny_pdf() -> Vec<u8> {
    let content = b"1 0 0 rg 10 10 50 50 re f";
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 200 100] >>".to_vec(),
        b"<< /Title (PDF Title) /Author (A; B) /Keywords (x, y) /CreationDate (D:20200101) >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 7 0 R /Rotate 90 >>".to_vec(),
        b"<< /First 8 0 R >>".to_vec(),
    ];
    let mut s = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    s.extend_from_slice(content);
    s.extend_from_slice(b"\nendstream");
    objs.push(s);
    objs.push(b"<< /Title (Second page) /Dest [5 0 R /Fit] >>".to_vec());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes(),
    );
    out
}

#[test]
fn pdf_manifest_pages_and_cover() {
    let d = dir("pdf");
    let p = d.join("b.pdf");
    std::fs::write(&p, tiny_pdf()).unwrap();
    let books = Books::new();
    let m = books.manifest(&p).unwrap();
    assert_eq!((m.format.as_str(), m.layout.as_str(), m.pages), ("pdf", "pages", 2));
    assert_eq!((m.title.as_str(), m.published.as_str()), ("PDF Title", "2020"));
    assert_eq!(m.authors, vec!["A", "B"]);
    assert_eq!(m.subjects, vec!["x", "y"]);
    assert_eq!((m.toc[0].title.as_str(), m.toc[0].page), ("Second page", 1));
    assert_eq!(books.page_sizes(&p).unwrap(), vec![(200.0, 100.0), (100.0, 200.0)]);
    let page = books.page(&p, 0, 400).unwrap();
    assert_eq!(page.1, "image/png");
    assert!(page.0.starts_with(b"\x89PNG"));
    assert!(Arc::ptr_eq(&page, &books.page(&p, 0, 400).unwrap()), "rendered pages are cached");
    assert!(books.page(&p, 7, 400).is_err());
    let (cover, mime) = books.cover(&p).unwrap();
    assert!(cover.starts_with(b"\x89PNG") && mime == "image/png");
    assert!(books.chapter(&p, 0, "/x/").is_err(), "a PDF has no chapters");
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn providers() {
    let d = dir("prov");
    let p = d.join("A Test Book.epub");
    epub(&p, &[]);
    let books = Arc::new(Books::new());
    let prov = BookProvider { books: books.clone() };
    let m = prov.invoke(CAP_BOOK_MANIFEST, "manifest", json!({"path": p})).unwrap();
    assert_eq!(m["title"], "A Test Book");
    assert_eq!(prov.invoke(CAP_BOOK_MANIFEST, "", json!({})).unwrap_err().code, "invalid-message");
    let meta = BookMetadata { books };
    let hits = meta.invoke(CAP_SEARCH, "search", json!({"kind": "book", "path": p, "query": "x"})).unwrap();
    assert_eq!(hits[0]["title"], "A Test Book");
    assert_eq!(meta.invoke(CAP_SEARCH, "search", json!({"kind": "anime", "path": p})).unwrap(), json!([]));
    let rec = meta.invoke(CAP_RESOLVE, "resolve", json!({"remote_id": p})).unwrap();
    assert_eq!((rec["year"].as_i64(), rec["synopsis"].as_str()), (Some(1999), Some("A fine book.")));
    assert_eq!(rec["genres"], json!(["Fiction", "Tests"]));
    assert!(meta.invoke(CAP_RESOLVE, "resolve", json!({"remote_id": "/etc/passwd"})).is_err());
    std::fs::remove_dir_all(d).ok();
}

/// Real books from `LAIN_BOOK_SAMPLES` (any of the four formats):
/// every chapter sanitizes, every resource a chapter names exists.
#[test]
#[ignore]
fn real_books() {
    let Ok(dir) = std::env::var("LAIN_BOOK_SAMPLES") else { return };
    let books = Books::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if format(&p).is_empty() {
            continue;
        }
        let t = std::time::Instant::now();
        let m = match books.manifest(&p) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: {e:?}", p.display());
                continue;
            }
        };
        let opened = t.elapsed();
        let t = std::time::Instant::now();
        let mut bytes = 0usize;
        let mut missing = 0usize;
        for c in 0..m.chapters.len() {
            let html = books.chapter(&p, c, "/r/").unwrap();
            bytes += html.len();
            for part in html.split("\"/r/").skip(1) {
                let end = part.find('"').unwrap_or(part.len());
                let member = super::markup::percent_decode(&part[..end]);
                if books.resource(&p, &member, "/r/").is_err() {
                    missing += 1;
                }
            }
        }
        eprintln!(
            "{}: {} {:?} by {:?}, {} chapters, {} toc, {} pages, cover {}, opened {:?}, sanitized {} KB in {:?}, missing resources {}",
            p.file_name().unwrap().to_string_lossy(),
            m.format,
            m.title,
            m.authors,
            m.chapters.len(),
            m.toc.len(),
            m.pages,
            books.cover(&p).is_ok(),
            opened,
            bytes / 1024,
            t.elapsed(),
            missing
        );
    }
}

/// Regressions from the security review: a chapter's policy blocks
/// outside fetches, a sheet linked many times is inlined once, and a
/// deeply nested contents list neither overflows the stack nor grows.
#[test]
fn hostile_books_stay_bounded() {
    let d = dir("hostile");
    let p = d.join("h.epub");
    let mut one = String::from("<html><head>");
    for _ in 0..500 {
        one.push_str("<link rel=\"stylesheet\" href=\"../css/s.css\"/>");
    }
    one.push_str("</head><body><p>x</p></body></html>");
    let mut nav = String::from("<html><body><nav epub:type=\"toc\">");
    for _ in 0..100_000 {
        nav.push_str("<ol><li><a href=\"text/one.xhtml\">x</a>");
    }
    nav.push_str("</nav></body></html>");
    let mut w = Writer::new();
    w.add("mimetype", b"application/epub+zip", false);
    w.add(
        "META-INF/container.xml",
        br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
        true,
    );
    w.add("OEBPS/content.opf", OPF.as_bytes(), true);
    w.add("OEBPS/nav.xhtml", nav.as_bytes(), true);
    w.add("OEBPS/text/one.xhtml", one.as_bytes(), true);
    w.add("OEBPS/text/two b.xhtml", TWO.as_bytes(), true);
    w.add("OEBPS/css/s.css", " ".repeat(1 << 20).as_bytes(), true);
    std::fs::write(&p, w.finish()).unwrap();
    let books = Books::new();
    let m = books.manifest(&p).unwrap();
    let mut depth = 0;
    let mut level = &m.toc;
    while let Some(first) = level.first() {
        depth += 1;
        level = &first.children;
    }
    assert!(depth <= 9, "contents depth {depth}");
    let html = books.chapter(&p, 0, "/r/").unwrap();
    assert!(html.len() < 2 << 20, "the sheet is inlined once: {} bytes", html.len());
    assert!(html.contains("http-equiv=\"Content-Security-Policy\""));
    assert!(html.find("Content-Security-Policy").unwrap() < html.find("<style>").unwrap_or(usize::MAX));
    std::fs::remove_dir_all(d).ok();
}
