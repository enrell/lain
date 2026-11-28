//! A chapter made safe to show (D-305): the content document re-written
//! as HTML from an allowlist of elements and attributes, with no script,
//! no event handler, no form, no frame and no URL that leaves the book.
//! Links to other chapters carry `data-lain-chapter`/`data-lain-anchor`,
//! resources point at the book's resource route, and the chapter's
//! stylesheets are inlined (their URLs rewritten the same way), so the
//! reader can render the result in a sandboxed frame without scripts.

use std::collections::HashMap;

use super::markup::{self, Ev, escape, local};

/// Elements kept with their content.
const HTML: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "picture",
    "pre",
    "q",
    "rb",
    "rp",
    "rt",
    "rtc",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

const SVG: &[&str] = &[
    "svg",
    "g",
    "image",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "path",
    "text",
    "tspan",
    "textpath",
    "defs",
    "title",
    "desc",
    "lineargradient",
    "radialgradient",
    "stop",
    "clippath",
    "mask",
    "pattern",
    "symbol",
    "use",
    "marker",
];

const MATHML: &[&str] = &[
    "math",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "mspace",
    "mrow",
    "mfrac",
    "msqrt",
    "mroot",
    "mstyle",
    "merror",
    "mpadded",
    "mphantom",
    "mfenced",
    "menclose",
    "msub",
    "msup",
    "msubsup",
    "munder",
    "mover",
    "munderover",
    "mmultiscripts",
    "mprescripts",
    "none",
    "mtable",
    "mtr",
    "mtd",
    "mlabeledtr",
    "semantics",
    "annotation",
];

/// Elements dropped with everything inside them.
const DROP: &[&str] = &[
    "script",
    "noscript",
    "template",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "form",
    "input",
    "select",
    "option",
    "textarea",
    "button",
    "base",
    "meta",
    "link",
    "title",
    "head",
    "style",
    "foreignobject",
    "animate",
    "animatemotion",
    "animatetransform",
    "set",
    "audio-source",
    "canvas",
    "dialog",
    "portal",
];

const VOID: &[&str] = &["br", "hr", "img", "wbr", "col", "source", "track"];

const ATTRS: &[&str] = &[
    "id",
    "class",
    "lang",
    "dir",
    "title",
    "alt",
    "width",
    "height",
    "colspan",
    "rowspan",
    "span",
    "align",
    "valign",
    "start",
    "type",
    "reversed",
    "value",
    "datetime",
    "abbr",
    "scope",
    "headers",
    "role",
    "border",
    "cellpadding",
    "cellspacing",
    "face",
    "size",
    "color",
    "controls",
    "loop",
    "muted",
    "open",
    "summary",
];

const SVG_ATTRS: &[&str] = &[
    "viewbox",
    "preserveaspectratio",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "d",
    "points",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "transform",
    "opacity",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientunits",
    "gradienttransform",
    "font-size",
    "font-family",
    "font-weight",
    "text-anchor",
    "dx",
    "dy",
    "clip-path",
    "clippathunits",
    "mask",
    "patternunits",
    "version",
    "xmlns",
    "xmlns:xlink",
];

const MATH_ATTRS: &[&str] = &[
    "display",
    "mathvariant",
    "mathsize",
    "mathcolor",
    "mathbackground",
    "accent",
    "accentunder",
    "fence",
    "separator",
    "stretchy",
    "symmetric",
    "largeop",
    "movablelimits",
    "lspace",
    "rspace",
    "linethickness",
    "columnalign",
    "rowalign",
    "columnspan",
    "notation",
    "open",
    "close",
    "separators",
    "encoding",
    "xmlns",
];

/// What the sanitizer needs to know about the book.
pub struct Book<'a> {
    /// Spine documents: member path → chapter index.
    pub chapters: &'a HashMap<String, usize>,
    /// Whether a member exists.
    pub has: &'a dyn Fn(&str) -> bool,
    /// Reads a stylesheet member.
    pub css: &'a dyn Fn(&str) -> Option<String>,
    /// URL prefix of the resource route (`/api/items/ID/book/res/`).
    pub prefix: &'a str,
}

/// A rewritten URL.
enum Target {
    Chapter(usize, String),
    Resource(String),
    External(String),
    /// A fragment in the same document.
    Local(String),
    Drop,
}

fn target(book: &Book, dir: &str, raw: &str) -> Target {
    let v = raw.trim();
    if v.is_empty() {
        return Target::Drop;
    }
    if let Some(frag) = v.strip_prefix('#') {
        return Target::Local(markup::percent_decode(frag));
    }
    let lower = v.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        return Target::External(v.to_string());
    }
    // Any other scheme (javascript:, data:, file:, …) is dropped.
    if let Some(colon) = v.find(':')
        && v[..colon].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Target::Drop;
    }
    if v.starts_with("//") {
        return Target::Drop;
    }
    let no_query = v.split('?').next().unwrap_or("");
    let (path, frag) = no_query.split_once('#').unwrap_or((no_query, ""));
    let Some(path) = markup::resolve(dir, &markup::percent_decode(path)) else { return Target::Drop };
    if let Some(&n) = book.chapters.get(&path) {
        return Target::Chapter(n, markup::percent_decode(frag));
    }
    if (book.has)(&path) {
        return Target::Resource(path);
    }
    Target::Drop
}

fn resource_url(book: &Book, path: &str) -> String {
    format!("{}{}", book.prefix, markup::percent_encode(path))
}

/// A stylesheet with its URLs pointing at the resource route (or
/// emptied), `@import` rules removed, and constructs that could run
/// code neutralised.
pub fn rewrite_css(css: &str, book: &Book, dir: &str) -> String {
    const NEUTER: [&str; 5] = ["expression(", "javascript:", "vbscript:", "-moz-binding", "behavior:"];
    let lower = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < css.len() {
        let rest = &lower[i..];
        if rest.starts_with("@import") {
            i += rest.find(';').map_or(rest.len(), |p| p + 1);
            continue;
        }
        if rest.starts_with("url(") {
            let close = rest.find(')').map_or(rest.len(), |p| p + 1);
            let inner = css.get(i + 4..i + close.saturating_sub(1).max(4)).unwrap_or("");
            let t = inner.trim().trim_matches(['"', '\'']).trim();
            match target(book, dir, t) {
                Target::Resource(p) => {
                    out.push_str("url(\"");
                    out.push_str(&resource_url(book, &p));
                    out.push_str("\")");
                }
                Target::Local(f) => {
                    out.push_str("url(\"#");
                    out.push_str(&markup::percent_encode(&f));
                    out.push_str("\")");
                }
                _ => out.push_str("url()"),
            }
            i += close;
            continue;
        }
        if NEUTER.iter().any(|n| rest.starts_with(n)) {
            out.push_str("x-");
        }
        let Some(c) = css[i..].chars().next() else { break };
        // A `<` could end the inlined `style` element.
        if c == '<' {
            out.push_str("\\3c ");
        } else {
            out.push(c);
        }
        i += c.len_utf8();
    }
    out
}

/// The policy of a sanitized chapter: only this server, no script.
pub const FRAME_CSP: &str =
    "default-src 'none'; img-src 'self' data:; media-src 'self'; font-src 'self'; style-src 'unsafe-inline'";

/// Stylesheets inlined into one chapter, at most (count and bytes): a
/// hostile book naming one big sheet a thousand times stays small.
const MAX_SHEETS: usize = 32;
const MAX_INLINE_CSS: usize = 4 << 20;

/// The sanitized chapter and its character count.
pub struct Chapter {
    pub html: String,
    pub chars: i64,
}

#[derive(Clone, Copy, PartialEq)]
enum Ns {
    Html,
    Svg,
    Math,
}

/// Sanitizes the content document at member `path`.
pub fn chapter(src: &[u8], path: &str, book: &Book) -> Chapter {
    let dir = markup::dir_of(path);
    let ev = markup::events(src);
    let mut styles: Vec<String> = Vec::new();
    let mut linked: Vec<String> = Vec::new();
    let mut inlined = 0usize;
    let mut lang = String::new();
    let mut html_dir = String::new();
    let mut body_attrs = String::new();
    let mut out = String::with_capacity(src.len());
    let mut chars: i64 = 0;
    // Open elements: (name, namespace).
    let mut stack: Vec<(String, Ns)> = Vec::new();
    let mut skip: Option<(String, usize)> = None;
    let mut in_body = false;
    let mut seen_body = false;
    let mut i = 0;
    while i < ev.len() {
        let e = &ev[i];
        i += 1;
        // Inside a dropped element: wait for its end.
        if let Some((name, depth)) = &mut skip {
            match e {
                Ev::Start { name: n, closed: false, .. } if n == name => *depth += 1,
                Ev::End { name: n } if n == name => {
                    if *depth == 0 {
                        skip = None;
                    } else {
                        *depth -= 1;
                    }
                }
                _ => {}
            }
            continue;
        }
        match e {
            Ev::Start { name, attrs: _, closed } => {
                let ln = local(name).to_string();
                match ln.as_str() {
                    "html" => {
                        lang = e.attr("xml:lang").or_else(|| e.attr("lang")).unwrap_or("").to_string();
                        html_dir = e.attr("dir").unwrap_or("").to_string();
                        continue;
                    }
                    "head" => {
                        // Stylesheets and styles; nothing else from the head.
                        let mut j = i;
                        while j < ev.len() {
                            match &ev[j] {
                                Ev::End { name } if local(name) == "head" => break,
                                Ev::Start { name, .. } if local(name) == "body" => break,
                                Ev::Start { name, .. } if local(name) == "link" => {
                                    let rel = ev[j].attr("rel").unwrap_or("").to_ascii_lowercase();
                                    if rel.split_whitespace().any(|r| r == "stylesheet")
                                        && let Some(h) = ev[j].attr("href")
                                        && let Target::Resource(p) = target(book, dir, h)
                                        && !linked.contains(&p)
                                        && linked.len() < MAX_SHEETS
                                        && let Some(css) = (book.css)(&p)
                                    {
                                        let css = rewrite_css(&css, book, markup::dir_of(&p));
                                        if inlined + css.len() <= MAX_INLINE_CSS {
                                            inlined += css.len();
                                            styles.push(css);
                                        }
                                        linked.push(p);
                                    }
                                }
                                Ev::Start { name, .. } if local(name) == "style" => {
                                    if let Some(Ev::Raw(css)) = ev.get(j + 1)
                                        && inlined + css.len() <= MAX_INLINE_CSS
                                    {
                                        inlined += css.len();
                                        styles.push(rewrite_css(css, book, dir));
                                    }
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        i = if j < ev.len() && matches!(&ev[j], Ev::End { .. }) { j + 1 } else { j };
                        continue;
                    }
                    "body" => {
                        in_body = true;
                        seen_body = true;
                        body_attrs = attributes(e, Ns::Html, "body", book, dir);
                        continue;
                    }
                    _ => {}
                }
                // Content before `<body>` (a fragment without one) counts.
                if !in_body && !seen_body {
                    in_body = true;
                }
                if !in_body {
                    continue;
                }
                let parent = stack.last().map_or(Ns::Html, |s| s.1);
                let ns = match (parent, ln.as_str()) {
                    (_, "svg") => Ns::Svg,
                    (_, "math") => Ns::Math,
                    (p, _) => p,
                };
                if DROP.contains(&ln.as_str()) {
                    // A `style` in the body still styles the chapter.
                    if ln == "style"
                        && let Some(Ev::Raw(css)) = ev.get(i)
                        && inlined + css.len() <= MAX_INLINE_CSS
                    {
                        inlined += css.len();
                        styles.push(rewrite_css(css, book, dir));
                    }
                    if !closed {
                        skip = Some((name.clone(), 0));
                    }
                    continue;
                }
                let known = match ns {
                    Ns::Html => HTML.contains(&ln.as_str()),
                    Ns::Svg => SVG.contains(&ln.as_str()) || ln == "a",
                    Ns::Math => MATHML.contains(&ln.as_str()),
                };
                if !known {
                    // Unknown element: dropped, its content kept.
                    continue;
                }
                // `img` without a usable source shows nothing.
                if ln == "img" && !matches!(e.attr("src").map(|s| target(book, dir, s)), Some(Target::Resource(_))) {
                    if let Some(alt) = e.attr("alt").filter(|a| !a.trim().is_empty()) {
                        escape(&mut out, alt, false);
                    }
                    continue;
                }
                out.push('<');
                out.push_str(&ln);
                out.push_str(&attributes(e, ns, &ln, book, dir));
                out.push('>');
                let void = ns == Ns::Html && VOID.contains(&ln.as_str());
                if *closed || void {
                    if !void {
                        out.push_str("</");
                        out.push_str(&ln);
                        out.push('>');
                    }
                } else if stack.len() < 512 {
                    stack.push((ln, ns));
                }
            }
            Ev::End { name } => {
                let ln = local(name);
                if ln == "body" || ln == "html" {
                    in_body = false;
                    continue;
                }
                // Close up to the matching open element; ignore strays.
                if let Some(at) = stack.iter().rposition(|(n, _)| n == ln) {
                    while stack.len() > at {
                        if let Some((n, _)) = stack.pop() {
                            out.push_str("</");
                            out.push_str(&n);
                            out.push('>');
                        }
                    }
                }
            }
            Ev::Text(t) => {
                if !in_body && seen_body {
                    continue;
                }
                if !in_body && t.trim().is_empty() {
                    continue;
                }
                chars += t.chars().filter(|c| !c.is_whitespace()).count() as i64;
                escape(&mut out, t, false);
            }
            Ev::Raw(_) => {}
        }
    }
    while let Some((n, _)) = stack.pop() {
        out.push_str("</");
        out.push_str(&n);
        out.push('>');
    }
    let mut doc = String::with_capacity(out.len() + 512);
    doc.push_str("<!DOCTYPE html><html");
    if !lang.is_empty() {
        doc.push_str(" lang=\"");
        escape(&mut doc, &lang, true);
        doc.push('"');
    }
    if html_dir == "rtl" || html_dir == "ltr" {
        doc.push_str(" dir=\"");
        doc.push_str(&html_dir);
        doc.push('"');
    }
    // The frame takes this policy (a `srcdoc` document honours a meta
    // CSP): whatever a stylesheet manages to name, nothing is fetched
    // from outside this server and no script runs.
    doc.push_str("><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"");
    doc.push_str(FRAME_CSP);
    doc.push_str("\">");
    for s in &styles {
        doc.push_str("<style>");
        doc.push_str(s);
        doc.push_str("</style>");
    }
    doc.push_str("</head><body");
    doc.push_str(&body_attrs);
    doc.push('>');
    doc.push_str(&out);
    doc.push_str("</body></html>");
    Chapter { html: doc, chars }
}

/// The allowed attributes of a start tag, rewritten, with a leading space.
fn attributes(e: &Ev, ns: Ns, tag: &str, book: &Book, dir: &str) -> String {
    let Ev::Start { attrs, .. } = e else { return String::new() };
    let mut out = String::new();
    let mut push = |k: &str, v: &str| {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape(&mut out, v, true);
        out.push('"');
    };
    for (k, v) in attrs {
        let key = k.as_str();
        match key {
            "href" | "xlink:href" | "src" | "poster" => {
                let t = target(book, dir, v);
                let attr = if key == "xlink:href" { "href" } else { key };
                match (t, tag) {
                    (Target::Chapter(n, a), "a") => {
                        push("href", &format!("#{}", markup::percent_encode(&a)));
                        push("data-lain-chapter", &n.to_string());
                        push("data-lain-anchor", &a);
                    }
                    (Target::Local(a), _) => push(attr, &format!("#{}", markup::percent_encode(&a))),
                    (Target::External(u), "a") => {
                        push("href", &u);
                        push("target", "_blank");
                        push("rel", "noopener noreferrer");
                    }
                    (Target::Resource(p), "img" | "image" | "source" | "track" | "audio" | "video") => {
                        push(attr, &resource_url(book, &p));
                    }
                    (Target::Resource(p), "use") => {
                        // `use` may only point inside the document.
                        let _ = p;
                    }
                    _ => {}
                }
            }
            "style" => {
                let css = rewrite_css(v, book, dir);
                if !css.trim().is_empty() {
                    push("style", &css);
                }
            }
            "xml:lang" => push("lang", v),
            "epub:type" => push("data-epub-type", v),
            _ if key.starts_with("aria-") => push(key, v),
            _ if key.starts_with("on") => {}
            _ => {
                let ok = match ns {
                    Ns::Html => ATTRS.contains(&key),
                    Ns::Svg => ATTRS.contains(&key) || SVG_ATTRS.contains(&key),
                    Ns::Math => ATTRS.contains(&key) || MATH_ATTRS.contains(&key),
                };
                // Attribute values in SVG may hold `url(#id)` only.
                let safe = ns != Ns::Svg
                    || !v.to_ascii_lowercase().contains("url(")
                    || v.to_ascii_lowercase()
                        .match_indices("url(")
                        .all(|(p, _)| v[p + 4..].trim_start().trim_start_matches(['"', '\'']).starts_with('#'));
                if ok && safe {
                    push(key, v);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book<'a>(
        chapters: &'a HashMap<String, usize>,
        has: &'a dyn Fn(&str) -> bool,
        css: &'a dyn Fn(&str) -> Option<String>,
    ) -> Book<'a> {
        Book { chapters, has, css, prefix: "/api/items/I/book/res/" }
    }

    fn run(src: &str) -> Chapter {
        let mut ch = HashMap::new();
        ch.insert("OEBPS/text/c1.xhtml".to_string(), 0usize);
        ch.insert("OEBPS/text/c2.xhtml".to_string(), 1usize);
        let has = |p: &str| matches!(p, "OEBPS/img/a b.png" | "OEBPS/css/s.css" | "OEBPS/fonts/f.ttf");
        let css = |p: &str| {
            (p == "OEBPS/css/s.css").then(|| "@import url(x.css); p { background: url(../img/a%20b.png) } @font-face { src: url('../fonts/f.ttf') } h1 { x: url(http://evil/x) }".to_string())
        };
        chapter(src.as_bytes(), "OEBPS/text/c1.xhtml", &book(&ch, &has, &css))
    }

    #[test]
    fn scripts_handlers_and_foreign_urls_are_gone() {
        let c = run(
            r##"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xml:lang="pt"><head><title>T</title>
            <link rel="stylesheet" href="../css/s.css"/><style>.x{color:red}</style><script>alert(1)</script></head>
            <body class="b" onload="x()"><h1 id="t" onclick="evil()">Hello <b>world</b></h1>
            <script>steal()</script><iframe src="http://evil"></iframe><form><input value="x"/></form>
            <p><a href="c2.xhtml#sec">next</a> <a href="javascript:alert(1)">js</a> <a href="http://example.com/">ext</a>
            <a href="#t">top</a></p><img src="../img/a%20b.png" alt="pic"/><img src="http://evil/x.png" alt="remote"/>
            <div/><svg><image xlink:href="../img/a%20b.png"/><foreignObject><p>x</p></foreignObject></svg>
            <unknown>kept text</unknown></body></html>"##,
        );
        let h = &c.html;
        for bad in [
            "script",
            "alert",
            "onload",
            "onclick",
            "iframe",
            "evil",
            "<form",
            "<input",
            "javascript",
            "foreignObject",
            "@import",
        ] {
            assert!(!h.contains(bad), "{bad} in {h}");
        }
        assert!(h.starts_with("<!DOCTYPE html><html lang=\"pt\">"));
        assert!(h.contains("<body class=\"b\">"));
        assert!(h.contains("href=\"#sec\" data-lain-chapter=\"1\" data-lain-anchor=\"sec\""));
        assert!(h.contains("href=\"http://example.com/\" target=\"_blank\" rel=\"noopener noreferrer\""));
        assert!(h.contains("<img src=\"/api/items/I/book/res/OEBPS/img/a%20b.png\" alt=\"pic\">"));
        assert!(h.contains("remote"), "a dropped image keeps its alt text");
        assert!(h.contains("<div></div>"), "a self-closed div is closed in HTML");
        assert!(h.contains("<image href=\"/api/items/I/book/res/OEBPS/img/a%20b.png\"></image>"));
        assert!(h.contains("url(\"/api/items/I/book/res/OEBPS/fonts/f.ttf\")"));
        assert!(h.contains(".x{color:red}"));
        assert!(h.contains("kept text"));
        // "Hello world next js ext top kept text": no title, no alt text.
        assert_eq!(c.chars, "Helloworldnextjsexttopkepttext".len() as i64);
    }

    #[test]
    fn malformed_markup_is_balanced() {
        let c = run("<body><p>one<b>two</p><i>three</body>");
        assert!(c.html.contains("<p>one<b>two</b></p><i>three</i>"), "{}", c.html);
        let c = run("no body at all <em>x</em>");
        assert!(c.html.contains("no body at all <em>x</em>"), "{}", c.html);
        let c = run("<p></style></p><style>body{}</style>");
        assert!(!c.html.contains("</style></p>"));
    }

    #[test]
    fn css_cannot_close_the_style_element() {
        let mut ch = HashMap::new();
        ch.insert("a.xhtml".to_string(), 0usize);
        let has = |_: &str| false;
        let css = |_: &str| None;
        let b = book(&ch, &has, &css);
        assert_eq!(rewrite_css("p{}</style><script>", &b, ""), "p{}\\3c /style>\\3c script>");
        assert_eq!(rewrite_css("x{a:expression(1)}", &b, ""), "x{a:x-expression(1)}");
    }
}
