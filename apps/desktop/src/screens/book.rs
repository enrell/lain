//! The reflowable book reader (EPUB, MOBI, AZW3; D-306): the app reads
//! the server's sanitized chapters itself instead of sending the user to
//! the browser. A chapter is parsed into blocks (`crate::book`) and drawn
//! as one centered, scrolling column in the theme's fonts. Progress is
//! the shared record in virtual pages (D-307), so the web reader, the
//! Android app and this one resume each other's place.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, FontStyle, FontWeight, ImageSource, InteractiveText, ScrollHandle, StrikethroughStyle,
    StyledText, TextRun, UnderlineStyle, Window, div, img, point, px,
};
use lain_client::types::{BookManifest, BookTocEntry, Progress};

use crate::app::{Act, LainApp, Load};
use crate::book::{self, Align, Block, Doc, Kind, Link};
use crate::images::{ImgKey, Source};
use crate::ui::{Notice, Tone};

/// How long a landing (resume, link, chapter turn) is held while the
/// chapter's images load and push the text down.
const SETTLE: Duration = Duration::from_secs(3);
/// The shortest gap between two progress saves while reading.
const SAVE_GAP: Duration = Duration::from_secs(2);
const BAR_TOP: f32 = 52.;
const BAR_BOTTOM: f32 = 44.;

/// Where the scroll goes once a chapter is laid out.
#[derive(Debug, Clone)]
pub enum Land {
    Start,
    End,
    /// A fraction of the chapter's characters (a saved position).
    Fraction(f64),
    Anchor(String),
}

struct TocRow {
    depth: usize,
    title: String,
    /// Position in the reading order.
    chapter: usize,
    anchor: String,
}

pub struct BookView {
    pub id: String,
    pub manifest: BookManifest,
    chars: Vec<i64>,
    pub chapter: usize,
    pub doc: Load<Arc<Doc>>,
    cache: HashMap<usize, Arc<Doc>>,
    /// Bumped per chapter load, so a late answer for another one is dropped.
    load_gen: u64,
    scroll: ScrollHandle,
    land: Option<Land>,
    land_until: Instant,
    /// The last measured place in the chapter (0..1 of its characters).
    fraction: f64,
    saved: f64,
    saved_at: Instant,
    save_timer: bool,
    toc: Vec<TocRow>,
    toc_open: bool,
    toc_sel: usize,
    toc_scroll: ScrollHandle,
    /// The link Tab focused (an index into `Doc::links`): Enter follows it.
    link_sel: Option<usize>,
}

fn flatten(out: &mut Vec<TocRow>, entries: &[BookTocEntry], depth: usize, order: &dyn Fn(i64) -> Option<usize>) {
    for e in entries {
        if let Some(chapter) = order(e.chapter) {
            out.push(TocRow { depth, title: e.title.trim().to_string(), chapter, anchor: e.anchor.clone() });
        }
        flatten(out, &e.children, depth + 1, order);
    }
}

impl BookView {
    fn new(id: &str, manifest: BookManifest) -> BookView {
        let chars: Vec<i64> = manifest.chapters.iter().map(|c| c.chars).collect();
        let index: Vec<i64> = manifest.chapters.iter().map(|c| c.index).collect();
        let order = |n: i64| index.iter().position(|&i| i == n);
        let mut toc = Vec::new();
        flatten(&mut toc, &manifest.toc, 0, &order);
        if toc.is_empty() {
            // No table of contents: the reading order is one.
            toc = (0..manifest.chapters.len())
                .filter(|&i| manifest.chapters[i].linear)
                .map(|i| TocRow { depth: 0, title: format!("Chapter {}", i + 1), chapter: i, anchor: String::new() })
                .collect();
        }
        BookView {
            id: id.to_string(),
            manifest,
            chars,
            chapter: 0,
            doc: Load::Loading,
            cache: HashMap::new(),
            load_gen: 0,
            scroll: ScrollHandle::new(),
            land: None,
            land_until: Instant::now(),
            fraction: 0.,
            saved: 0.,
            saved_at: Instant::now(),
            save_timer: false,
            toc,
            toc_open: false,
            toc_sel: 0,
            toc_scroll: ScrollHandle::new(),
            link_sel: None,
        }
    }

    /// The reading-order position of a chapter index the server names.
    fn order(&self, index: i64) -> Option<usize> {
        self.manifest.chapters.iter().position(|c| c.index == index)
    }

    /// The table of contents row the current chapter belongs to.
    fn toc_row(&self) -> Option<usize> {
        let best = self.toc.iter().filter(|r| r.chapter <= self.chapter).map(|r| r.chapter).max()?;
        self.toc.iter().position(|r| r.chapter == best)
    }

    fn progress(&self) -> book::BookProgress {
        book::progress_at(self.chapter, self.fraction, &self.chars)
    }

    fn view(&self) -> (f32, f32, f32) {
        let off = -f32::from(self.scroll.offset().y);
        let max = f32::from(self.scroll.max_offset().height).max(0.);
        let h = f32::from(self.scroll.bounds().size.height);
        (off, max, h)
    }

    /// Whether the scroll view holds the current chapter's laid-out blocks.
    fn laid_out(&self) -> Option<&Arc<Doc>> {
        let doc = self.doc.ready()?;
        (!doc.blocks.is_empty() && self.scroll.children_count() == doc.blocks.len()).then_some(doc)
    }

    /// The place on screen as a fraction of the chapter's characters: the
    /// block at the top of the view and how far into it the view starts.
    fn measure(&self) -> Option<f64> {
        let doc = self.laid_out()?;
        let (off, max, _) = self.view();
        if max <= 1. {
            return Some(0.);
        }
        if off >= max - 1. {
            return Some(1.);
        }
        let total: usize = doc.blocks.iter().map(|b| b.chars).sum();
        if total == 0 {
            return Some((off / max) as f64);
        }
        let i = self.scroll.top_item();
        let b = self.scroll.bounds_for_item(i)?;
        let y = f32::from(self.scroll.bounds().top()) + off;
        let within = ((y - f32::from(b.top())) / f32::from(b.size.height).max(1.)).clamp(0., 1.) as f64;
        let before: usize = doc.blocks[..i].iter().map(|b| b.chars).sum();
        Some(((before as f64 + doc.blocks[i].chars as f64 * within) / total as f64).clamp(0., 1.))
    }

    /// Puts the scroll where `land` says; false until the chapter is laid out.
    fn apply_land(&self, gap: f32) -> bool {
        let (Some(land), Some(doc)) = (&self.land, self.laid_out()) else { return false };
        let (_, max, _) = self.view();
        let top = f32::from(self.scroll.bounds().top());
        let block_top = |i: usize| self.scroll.bounds_for_item(i).map(|b| f32::from(b.top()) - top);
        let y = match land {
            Land::Start => 0.,
            Land::End => max,
            Land::Fraction(f) if *f <= 0. => 0.,
            Land::Fraction(f) => {
                let total: usize = doc.blocks.iter().map(|b| b.chars).sum();
                if total == 0 {
                    *f as f32 * max
                } else {
                    let want = f * total as f64;
                    let mut before = 0usize;
                    let mut y = max;
                    for (i, b) in doc.blocks.iter().enumerate() {
                        if b.chars > 0 && (before + b.chars) as f64 > want {
                            let within = ((want - before as f64) / b.chars as f64) as f32;
                            let h = self.scroll.bounds_for_item(i).map_or(0., |b| f32::from(b.size.height));
                            y = block_top(i).unwrap_or(0.) + within * h;
                            break;
                        }
                        before += b.chars;
                    }
                    y
                }
            }
            Land::Anchor(a) => doc.anchor(a).and_then(block_top).map_or(0., |y| y - gap),
        };
        self.scroll.set_offset(point(px(0.), px(-y.clamp(0., max))));
        true
    }

    fn scroll_to(&mut self, y: f32) {
        let (_, max, _) = self.view();
        self.scroll.set_offset(point(px(0.), px(-y.clamp(0., max))));
        self.land = None;
        // The focused link may be off screen now: Enter must not follow
        // what the reader cannot see.
        self.link_sel = None;
    }

    /// A laid-out block's top and height in the scroll's content.
    fn block_span(&self, i: usize) -> Option<(f32, f32)> {
        let top = f32::from(self.scroll.bounds().top());
        self.scroll.bounds_for_item(i).map(|b| (f32::from(b.top()) - top, f32::from(b.size.height)))
    }

    /// The first and last block on screen.
    fn visible_blocks(&self) -> Option<(usize, usize)> {
        let doc = self.laid_out()?;
        let (off, _, h) = self.view();
        let first = self.scroll.top_item().min(doc.blocks.len() - 1);
        let mut last = first;
        for i in first..doc.blocks.len() {
            match self.block_span(i) {
                Some((y, _)) if y < off + h => last = i,
                _ => break,
            }
        }
        Some((first, last))
    }

    /// Moves the link focus (Tab, Shift+Tab) and brings the link's block
    /// into view; false when the chapter has no links.
    fn link_move(&mut self, fwd: bool) -> bool {
        let Some(doc) = self.laid_out().cloned() else { return false };
        let links = doc.links();
        let Some(view) = self.visible_blocks() else { return false };
        let Some(sel) = book::link_step(&links, self.link_sel, view, fwd) else { return false };
        let block = links[sel].0;
        if let Some((y, bh)) = self.block_span(block) {
            let (off, _, h) = self.view();
            if y < off || y + bh.min(h) > off + h {
                // A third down the screen: the text around it stays readable.
                self.scroll_to(y - h / 3.);
            }
        }
        self.land = None;
        self.link_sel = Some(sel);
        true
    }

    /// The focused link's target.
    fn focused_link(&self) -> Option<Link> {
        let doc = self.doc.ready()?;
        let (block, n) = *doc.links().get(self.link_sel?)?;
        doc.blocks.get(block)?.links().into_iter().nth(n).map(|(_, l)| l)
    }
}

impl LainApp {
    fn bv(&mut self) -> Option<&mut BookView> {
        self.reader.as_mut().and_then(|r| r.book.as_mut())
    }

    /// Starts reading a reflowable book at the saved place.
    pub fn open_book(&mut self, id: &str, manifest: BookManifest, saved: Option<Progress>, cx: &mut Context<Self>) {
        let mut v = BookView::new(id, manifest);
        if v.manifest.chapters.is_empty() {
            v.doc = Load::Ready(Arc::new(Doc::default()));
        }
        let (chapter, fraction) =
            saved.as_ref().map(|p| book::position_of(p.position_sec, p.completed, &v.chars)).unwrap_or((0, 0.));
        // The record as stored: reading on from it is what saves next.
        v.saved = saved.as_ref().map_or(1., |p| p.position_sec);
        let empty = v.manifest.chapters.is_empty();
        if let Some(r) = self.reader.as_mut() {
            r.book = Some(v);
        }
        if !empty {
            self.book_load(chapter, Land::Fraction(fraction), false, cx);
        }
    }

    /// Shows chapter `n` (reading order) and lands at `land`; `save`
    /// records the move at once (a chapter turn, not the first open).
    fn book_load(&mut self, n: usize, land: Land, save: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(v) = self.bv() else { return };
        if n >= v.manifest.chapters.len() {
            return;
        }
        v.chapter = n;
        v.load_gen += 1;
        v.fraction = match &land {
            Land::Fraction(f) => f.clamp(0., 1.),
            Land::End => 1.,
            _ => 0.,
        };
        v.land = Some(land);
        v.land_until = Instant::now() + SETTLE;
        v.scroll = ScrollHandle::new();
        v.toc_open = false;
        v.link_sel = None;
        let (want, id, index) = (v.load_gen, v.id.clone(), v.manifest.chapters[n].index);
        let cached = v.cache.get(&n).cloned();
        match cached {
            Some(d) => v.doc = Load::Ready(d),
            None => {
                v.doc = Load::Loading;
                let c = b.client.clone();
                let local = crate::local_book::path(&b, &id);
                // Parsed on the network runtime: a long chapter never
                // stalls the UI thread. A downloaded book reads its file
                // (on the blocking pool), with no server.
                self.spawn_ui(
                    cx,
                    async move {
                        let html = match local {
                            Some(path) => {
                                tokio::task::spawn_blocking(move || crate::local_book::chapter(&path, &id, index))
                                    .await
                                    .unwrap_or_else(|e| Err(lain_client::ClientError::Store { message: e.to_string() }))
                            }
                            None => c.book_chapter(&id, index).await,
                        };
                        html.map(|h| Arc::new(book::parse(&h)))
                    },
                    move |app, r, _| {
                        let Some(v) = app.bv() else { return };
                        if v.load_gen != want {
                            return;
                        }
                        if let Ok(d) = &r {
                            if v.cache.len() >= 8 {
                                // Keep the chapters nearest the reader.
                                if let Some(&far) = v.cache.keys().max_by_key(|&&k| k.abs_diff(n)) {
                                    v.cache.remove(&far);
                                }
                            }
                            v.cache.insert(n, d.clone());
                        }
                        v.land_until = Instant::now() + SETTLE;
                        v.doc = Load::from(r);
                    },
                );
            }
        }
        if save {
            self.book_save(false, cx);
        }
    }

    /// Records the current place (throttling is the caller's).
    fn book_save(&mut self, finished: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(v) = self.bv() else { return };
        let p = v.progress();
        v.saved = p.position;
        v.saved_at = Instant::now();
        let id = v.id.clone();
        let fut = crate::backend::save_progress(b, id, p.position, p.duration, p.completed || finished);
        self.spawn_ui(cx, fut, |app, r, cx| {
            if r.is_err() {
                app.report(&r, "");
            } else {
                app.refresh_continues(cx);
            }
        });
    }

    fn book_turn(&mut self, d: i32, cx: &mut Context<Self>) {
        let Some(v) = self.bv() else { return };
        if v.doc.ready().is_some_and(|d| d.blocks.is_empty()) {
            // Nothing to scroll in an empty chapter: turn past it.
            let land = if d > 0 { Land::Start } else { Land::End };
            return self.book_chapter_step(d, land, cx);
        }
        if v.laid_out().is_none() {
            return;
        }
        let (off, max, h) = v.view();
        // One screen less a line or two, so the reader keeps their place.
        let step = (h * 0.9).max(40.);
        if d > 0 {
            if off >= max - 2. {
                return self.book_chapter_step(1, Land::Start, cx);
            }
            v.scroll_to(off + step);
        } else {
            if off <= 2. {
                return self.book_chapter_step(-1, Land::End, cx);
            }
            v.scroll_to(off - step);
        }
    }

    fn book_chapter_step(&mut self, d: i32, land: Land, cx: &mut Context<Self>) {
        let Some(v) = self.bv() else { return };
        let (cur, n) = (v.chapter, v.manifest.chapters.len());
        if d > 0 && cur + 1 >= n {
            // Past the last page: the book is finished (as the web reader).
            v.fraction = 1.;
            v.scroll_to(f32::MAX);
            self.book_save(true, cx);
            self.toast("End of the book", false);
            return;
        }
        if d < 0 && cur == 0 {
            if let Some(v) = self.bv() {
                v.scroll_to(0.);
            }
            return;
        }
        let next = if d > 0 { cur + 1 } else { cur - 1 };
        self.book_load(next, land, true, cx);
    }

    /// Follows a link in a chapter.
    pub fn book_follow(&mut self, link: Link, cx: &mut Context<Self>) {
        let Some(v) = self.bv() else { return };
        v.link_sel = None;
        match link {
            Link::Anchor(a) => {
                v.land = Some(Land::Anchor(a));
                v.land_until = Instant::now() + Duration::from_millis(400);
            }
            Link::Chapter(n, a) => {
                let Some(n) = v.order(n) else { return };
                let land = if a.is_empty() { Land::Start } else { Land::Anchor(a) };
                if n == v.chapter {
                    v.land = Some(land);
                    v.land_until = Instant::now() + Duration::from_millis(400);
                } else {
                    self.book_load(n, land, true, cx);
                }
            }
            Link::External(url) => cx.open_url(&url),
        }
        cx.notify();
    }

    fn book_text(&mut self, step: Option<i32>) {
        let size = match step {
            Some(d) => (self.config.book_text + d).clamp(-4, 8),
            None => 0,
        };
        if size == self.config.book_text {
            return;
        }
        self.config.book_text = size;
        self.config.save(&self.dir);
        // The text reflows: hold the reader's place through it (a landing
        // still under way keeps its own target).
        if let Some(v) = self.bv() {
            if v.land.is_none() {
                v.land = Some(Land::Fraction(v.fraction));
            }
            v.land_until = v.land_until.max(Instant::now() + Duration::from_millis(600));
        }
    }

    fn book_open_toc_row(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(v) = self.bv() else { return };
        let Some(r) = v.toc.get(i) else { return };
        let land = if r.anchor.is_empty() { Land::Start } else { Land::Anchor(r.anchor.clone()) };
        let n = r.chapter;
        v.toc_open = false;
        if n == v.chapter {
            v.land = Some(land);
            v.land_until = Instant::now() + Duration::from_millis(400);
        } else {
            self.book_load(n, land, true, cx);
        }
    }

    /// Records the place if it moved since the last save; `back()` calls
    /// it whichever way the reader is left (B, Esc, the button, Alt+←).
    pub fn book_flush(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.bv() {
            if let Some(f) = v.land.is_none().then(|| v.measure()).flatten() {
                v.fraction = f;
            }
            if (v.progress().position - v.saved).abs() > 0.0005 {
                self.book_save(false, cx);
            }
        }
    }

    /// The reader's keys (the web reader's, D-306).
    pub fn book_key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let m = ev.keystroke.modifiers;
        if m.control || m.platform || m.alt {
            return false;
        }
        let k = ev.keystroke.key.as_str();
        let Some(v) = self.bv() else { return false };
        if v.toc_open {
            let n = v.toc.len();
            let sel = v.toc_sel;
            let to = |d: i64| (sel as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
            match k {
                "down" | "j" => v.toc_sel = to(1),
                "up" | "k" => v.toc_sel = to(-1),
                "pagedown" => v.toc_sel = to(10),
                "pageup" => v.toc_sel = to(-10),
                "home" => v.toc_sel = 0,
                "end" => v.toc_sel = n.saturating_sub(1),
                "enter" | "space" => {
                    self.book_open_toc_row(sel, cx);
                    return true;
                }
                "escape" | "t" | "b" | "backspace" => v.toc_open = false,
                _ => {}
            }
            v.toc_scroll.scroll_to_item(v.toc_sel);
            // The overlay is modal: every other key stays with it.
            return true;
        }
        let fwd = if v.manifest.direction == "rtl" { -1 } else { 1 };
        match k {
            // Links: Tab walks them, Enter follows, Esc lets go. A chapter
            // without links leaves Tab to the window's focus order.
            "tab" => return v.link_move(!m.shift),
            "enter" if v.link_sel.is_some() => {
                if let Some(l) = v.focused_link() {
                    self.book_follow(l, cx);
                }
            }
            "escape" if v.link_sel.is_some() => v.link_sel = None,
            "right" => self.book_turn(fwd, cx),
            "left" => self.book_turn(-fwd, cx),
            "pagedown" => self.book_turn(1, cx),
            "pageup" => self.book_turn(-1, cx),
            "space" => self.book_turn(if m.shift { -1 } else { 1 }, cx),
            "down" | "up" => {
                if v.laid_out().is_some() {
                    let (off, _, _) = v.view();
                    v.scroll_to(off + if k == "down" { 80. } else { -80. });
                }
            }
            "home" => v.scroll_to(0.),
            "end" => v.scroll_to(f32::MAX),
            "n" | "]" => self.book_chapter_step(1, Land::Start, cx),
            "p" | "[" => self.book_chapter_step(-1, Land::Start, cx),
            "t" => {
                v.toc_sel = v.toc_row().unwrap_or(0);
                v.toc_open = !v.toc.is_empty();
                v.toc_scroll.scroll_to_item(v.toc_sel);
            }
            "+" | "=" => self.book_text(Some(1)),
            "-" => self.book_text(Some(-1)),
            "0" => self.book_text(None),
            "b" | "backspace" | "escape" => self.back(cx),
            _ => return false,
        }
        true
    }

    /// The text size of body copy, from the theme and the reader's step.
    fn book_font(&self) -> f32 {
        self.theme.ts(0) * 1.15 * 1.1f32.powi(self.config.book_text)
    }

    pub fn render_book(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (vw, vh) = crate::ui::viewport(window);
        let sf = window.scale_factor();
        let fs = self.book_font();
        let gap = fs;
        // Land, then measure and save: both read the last frame's layout.
        let mut landing = false;
        let mut save = false;
        if let Some(v) = self.bv() {
            if v.land.is_some() {
                if v.apply_land(gap) && Instant::now() >= v.land_until {
                    v.land = None;
                }
                landing = v.land.is_some();
            }
            if v.land.is_none()
                && let Some(f) = v.measure()
            {
                v.fraction = f;
                let moved = (v.progress().position - v.saved).abs() > 0.0005;
                if moved && v.saved_at.elapsed() >= SAVE_GAP {
                    save = true;
                } else if moved && !v.save_timer {
                    // A trailing save once the reader stops moving.
                    v.save_timer = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SAVE_GAP).await;
                        let _ = this.update(cx, |app, cx| {
                            if let Some(v) = app.bv() {
                                v.save_timer = false;
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
        }
        if save {
            self.book_save(false, cx);
        }
        if landing {
            // Layout settles over a few frames (images, reflow).
            window.request_animation_frame();
        }
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let Some(v) = self.reader.as_ref().and_then(|r| r.book.as_ref()) else { return div() };
        let (doc, chapter, n) = (v.doc.clone(), v.chapter, v.manifest.chapters.len());
        let p = v.progress();
        let title = if v.manifest.title.is_empty() {
            self.by_id.get(&v.id).map(|&i| self.catalog[i].title.clone()).unwrap_or_default()
        } else {
            v.manifest.title.clone()
        };
        let chapter_title = v
            .toc_row()
            .map(|i| v.toc[i].title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| format!("Chapter {}", chapter + 1));
        let scroll = v.scroll.clone();
        let id = v.id.clone();
        let toc_open = v.toc_open;
        // The focused link as (block, n-th link of the block).
        let focus = match &doc {
            Load::Ready(d) => v.link_sel.and_then(|i| d.links().get(i).copied()),
            _ => None,
        };
        let has_links = doc.ready().is_some_and(|d| d.blocks.iter().any(|b| b.spans.iter().any(|s| s.link.is_some())));
        let root = div().size_full().relative().bg(pal.bg);
        let back = self.btn("reader.back", "Back", Some("B"), Tone::Quiet, Act::Back, cx);
        let top = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(BAR_TOP))
            .px(px(th.sp(4)))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(th.sp(3)))
            .bg(pal.bg.opacity(0.92))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .child(back)
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(vw * 0.35))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(self.fonts.display.clone())
                            .text_size(px(th.ts(1)))
                            .child(title),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(th.ts(-1)))
                            .text_color(pal.muted)
                            .child(chapter_title),
                    ),
            )
            .child(self.label(format!("page {} of {}", p.position.floor() as i64, p.duration as i64)));
        let col_w = (vw - 2. * th.sp(5)).min(fs * 35.).max(160.);
        let stage = div().absolute().top(px(BAR_TOP)).left_0().right_0().bottom(px(BAR_BOTTOM));
        let stage = match &doc {
            Load::Ready(d) if d.blocks.is_empty() => stage.flex().items_center().justify_center().child(self.notice(
                Notice::Empty,
                "An empty chapter",
                "This part of the book has no text. → or N goes on.",
            )),
            Load::Ready(d) => {
                let weak = cx.weak_entity();
                let mut col = div()
                    .id("book.scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(fs * 2.))
                    .pb(px(fs * 4.))
                    .text_size(px(fs))
                    .line_height(px(fs * 1.6))
                    .on_scroll_wheel(cx.listener(|app, _: &gpui::ScrollWheelEvent, _, _| {
                        // The reader took over: stop holding a landing.
                        if let Some(v) = app.bv() {
                            v.land = None;
                            v.link_sel = None;
                        }
                    }));
                for (i, b) in d.blocks.iter().enumerate() {
                    let f = focus.filter(|f| f.0 == i).map(|f| f.1);
                    col = col.child(self.book_block(&id, i, b, f, fs, col_w, sf, &weak, cx));
                }
                stage.child(col)
            }
            Load::Failed(e) => stage.flex().items_center().justify_center().child(self.error_notice("This chapter", e)),
            _ => stage.flex().items_center().justify_center().text_color(pal.muted).child("Opening…"),
        };
        let bar_w = vw - th.sp(5) * 2.;
        let frac = (p.position / p.duration.max(1.)) as f32;
        let bottom = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(BAR_BOTTOM))
            .px(px(th.sp(5)))
            .flex()
            .items_center()
            .gap(px(th.sp(3)))
            .child(div().font_family(self.fonts.mono.clone()).text_size(px(th.ts(-1))).child(format!(
                "{} / {}",
                p.position.floor() as i64,
                p.duration as i64
            )))
            .child(div().flex_1().h(px(3.)).rounded_full().bg(pal.text.opacity(0.12)).child(
                div().h_full().w(px((bar_w - 420.).max(40.) * frac.clamp(0., 1.))).bg(pal.accent).rounded_full(),
            ))
            .child(self.label(format!("chapter {} / {n}", chapter + 1)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(th.ts(-2)))
                    .text_color(pal.muted)
                    // A focused link swaps in its own keys: the row stays one line.
                    .when(focus.is_none(), |d| {
                        d.child(self.kbd("← →"))
                            .child("turn")
                            .child(self.kbd("T"))
                            .child("contents")
                            .child(self.kbd("+ -"))
                            .child("size")
                            .when(has_links, |d| d.child(self.kbd("Tab")).child("links"))
                    })
                    .when(focus.is_some(), |d| {
                        d.child(self.kbd("Tab"))
                            .child(self.kbd("Shift+Tab"))
                            .child("move")
                            .child(self.kbd("Enter"))
                            .child("follow")
                            .child(self.kbd("Esc"))
                            .child("clear")
                    }),
            );
        let mut root = root.child(stage).child(top).child(bottom);
        if toc_open {
            root = root.child(self.book_toc(vw, vh, cx));
        }
        root
    }

    #[allow(clippy::too_many_arguments)]
    fn book_block(
        &self,
        id: &str,
        i: usize,
        b: &Block,
        focus: Option<usize>,
        fs: f32,
        col_w: f32,
        sf: f32,
        weak: &gpui::WeakEntity<LainApp>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pal = &self.pal;
        let indent = b.indent as f32 * fs * 1.5;
        let cell = div().w(px(col_w)).flex_none().pl(px(indent));
        // GPUI has no justified text: `justify` reads as start-aligned.
        let cell = match b.align {
            Align::Center => cell.text_center(),
            Align::Right => cell.text_right(),
            Align::Start | Align::Justify => cell,
        };
        match &b.kind {
            Kind::Rule => cell
                .py(px(fs))
                .flex()
                .justify_center()
                .child(div().w(px(col_w * 0.3)).h(px(1.)).bg(pal.line))
                .into_any(),
            Kind::Image { src, alt } => {
                let key =
                    ImgKey { src: Source::BookRes(id.to_string(), src.clone()), w: (col_w * sf) as u32, h: 0, blur: 0 };
                let art = self.images.update(cx, |s, cx| s.get(&key, cx));
                let failed = art.is_none() && self.images.read(cx).failed(&key);
                let cell = cell.py(px(fs * 0.5)).flex().justify_center();
                match art {
                    Some((a, _)) => {
                        let sz = a.size(0);
                        let (nw, nh) = (sz.width.0.max(1) as f32 / sf, sz.height.0.max(1) as f32 / sf);
                        let w = nw.min(col_w - indent);
                        cell.child(img(ImageSource::Render(a)).w(px(w)).h(px(w * nh / nw))).into_any()
                    }
                    None if failed => cell
                        .text_color(pal.muted)
                        .text_size(px(fs * 0.85))
                        .when(!alt.is_empty(), |c| c.child(format!("[{alt}]")))
                        .into_any(),
                    None => cell
                        .child(div().w(px(col_w * 0.6)).h(px(fs * 10.)).rounded(px(4.)).bg(pal.surface.opacity(0.5)))
                        .into_any(),
                }
            }
            kind => {
                let (size, lh, family, before, after) = match kind {
                    Kind::Heading(h) => {
                        let s = [2.0, 1.6, 1.3, 1.15, 1.05, 1.0][(*h as usize).clamp(1, 6) - 1];
                        (fs * s, 1.25, self.fonts.display.clone(), fs * 1.2, fs * 0.6)
                    }
                    Kind::Pre => (fs * 0.88, 1.45, self.fonts.mono.clone(), 0., fs * 0.9),
                    Kind::Row => (fs * 0.92, 1.45, self.fonts.ui.clone(), 0., fs * 0.3),
                    _ => (fs, 1.6, self.fonts.ui.clone(), 0., fs * 0.8),
                };
                let (text, runs, links) = self.book_runs(b, &family, matches!(kind, Kind::Heading(_)), focus);
                let styled = StyledText::new(text).with_runs(runs);
                let body: AnyElement = if links.is_empty() {
                    styled.into_any_element()
                } else {
                    let ranges: Vec<_> = links.iter().map(|(r, _)| r.clone()).collect();
                    let targets: Vec<Link> = links.into_iter().map(|(_, l)| l).collect();
                    let weak = weak.clone();
                    InteractiveText::new(("book.text", i), styled)
                        .on_click(ranges, move |ix, _, cx| {
                            if let Some(l) = targets.get(ix).cloned() {
                                let _ = weak.update(cx, |app, cx| app.book_follow(l, cx));
                            }
                        })
                        .into_any_element()
                };
                let cell = cell.pt(px(before)).pb(px(after)).text_size(px(size)).line_height(px(size * lh));
                match kind {
                    Kind::Item(marker) => cell
                        .flex()
                        .child(div().flex_none().w(px(fs * 1.8)).text_color(pal.muted).child(marker.clone()))
                        .child(div().flex_1().min_w_0().child(body))
                        .into_any(),
                    Kind::Pre => cell
                        .child(div().p(px(fs * 0.6)).rounded(px(4.)).bg(pal.surface.opacity(0.6)).child(body))
                        .into_any(),
                    _ => cell.child(body).into_any(),
                }
            }
        }
    }

    /// A block's text, its font runs and its links (byte ranges, one per
    /// link of `Block::links`); the `focus`-th link wears the focus ring.
    #[allow(clippy::type_complexity)]
    fn book_runs(
        &self,
        b: &Block,
        family: &gpui::SharedString,
        heading: bool,
        focus: Option<usize>,
    ) -> (String, Vec<TextRun>, Vec<(std::ops::Range<usize>, Link)>) {
        let pal = &self.pal;
        let groups = b.links();
        let mut text = String::new();
        let mut runs = Vec::with_capacity(b.spans.len());
        let mut starts = vec![0usize; b.spans.len() + 1];
        for (si, s) in b.spans.iter().enumerate() {
            starts[si] = text.len();
            text.push_str(&s.text);
            let focused = focus.and_then(|n| groups.get(n)).is_some_and(|(r, _)| r.contains(&si));
            let mut f = gpui::font(if s.style & book::CODE != 0 { self.fonts.mono.clone() } else { family.clone() });
            f.weight = if s.style & book::BOLD != 0 {
                FontWeight::BOLD
            } else if heading {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            };
            if s.style & book::ITALIC != 0 {
                f.style = FontStyle::Italic;
            }
            let color = if s.link.is_some() {
                pal.accent
            } else if s.style & (book::SMALL | book::SUP | book::SUB) != 0 {
                pal.muted
            } else {
                pal.text
            };
            let line = |c: gpui::Hsla, w: f32| Some(UnderlineStyle { thickness: px(w), color: Some(c), wavy: false });
            runs.push(TextRun {
                len: s.text.len(),
                font: f,
                color: if focused { pal.text } else { color },
                background_color: if focused {
                    Some(pal.accent.opacity(0.32))
                } else {
                    (s.style & book::CODE != 0).then(|| pal.surface.opacity(0.7))
                },
                underline: if focused {
                    line(pal.accent, 2.)
                } else if s.link.is_some() {
                    line(pal.accent.opacity(0.5), 1.)
                } else if s.style & book::UNDERLINE != 0 {
                    line(color, 1.)
                } else {
                    None
                },
                strikethrough: (s.style & book::STRIKE != 0)
                    .then_some(StrikethroughStyle { thickness: px(1.), color: Some(color) }),
            });
        }
        starts[b.spans.len()] = text.len();
        let links = groups.into_iter().map(|(r, l)| (starts[r.start]..starts[r.end], l)).collect();
        (text, runs, links)
    }

    /// The table of contents overlay (T): ↑/↓ choose, Enter opens.
    fn book_toc(&mut self, vw: f32, vh: f32, cx: &mut Context<Self>) -> Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let Some(v) = self.reader.as_ref().and_then(|r| r.book.as_ref()) else { return div() };
        let (sel, current) = (v.toc_sel, v.toc_row());
        let rows: Vec<(usize, String)> = v.toc.iter().map(|r| (r.depth, r.title.clone())).collect();
        let scroll = v.toc_scroll.clone();
        let mut list =
            div().id("book.toc").flex().flex_col().max_h(px(vh * 0.6)).overflow_y_scroll().track_scroll(&scroll);
        for (i, (depth, title)) in rows.into_iter().enumerate() {
            let on = i == sel;
            list = list.child(
                div()
                    .id(("book.toc.row", i))
                    .flex()
                    .items_center()
                    .gap(px(th.sp(2)))
                    .px(px(th.sp(3)))
                    .py(px(th.sp(1)))
                    .pl(px(th.sp(3) + depth as f32 * th.sp(4)))
                    .rounded(px(th.radius.button))
                    .when(on, |d| d.bg(pal.accent.opacity(0.18)).text_color(pal.text))
                    .when(!on, |d| d.text_color(pal.text.opacity(0.85)))
                    .when(Some(i) == current, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    .hover(|s| s.bg(pal.text.opacity(0.06)))
                    .on_click(cx.listener(move |app, _: &gpui::ClickEvent, _, cx| app.book_open_toc_row(i, cx)))
                    .child(if title.is_empty() { "—".to_string() } else { title }),
            );
        }
        div().absolute().inset_0().bg(pal.scrim).flex().items_center().justify_center().child(
            div()
                .w(px((vw - 48.).min(560.)))
                .flex()
                .flex_col()
                .gap(px(th.sp(2)))
                .p(px(th.sp(4)))
                .rounded(px(th.radius.panel))
                .bg(pal.raised)
                .border_1()
                .border_color(pal.line)
                .child(
                    div().flex().items_center().justify_between().child(self.label("contents")).child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(th.ts(-2)))
                            .text_color(pal.muted)
                            .child(self.kbd("↑ ↓"))
                            .child("choose")
                            .child(self.kbd("Enter"))
                            .child("open")
                            .child(self.kbd("Esc"))
                            .child("close"),
                    ),
                )
                .child(list),
        )
    }
}
