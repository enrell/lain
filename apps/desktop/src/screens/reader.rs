//! The comic and manga reader (`/api/items/{id}/pages`, `lain.comic.pages@1`):
//! one page fit to the window, turned with the arrows in the archive's
//! reading direction (`rtl` for manga). Progress uses the same record as
//! video: `position_sec` is the 1-based page, `duration_sec` the count.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{Context, ImageSource, ObjectFit, Window, div, img, px};
use lain_client::types::ComicPages;

use crate::app::{Act, LainApp, Load};
use crate::images::{ImgKey, Source};
use crate::ui::{Notice, Tone};

pub struct ReaderState {
    pub id: String,
    pub pages: Load<ComicPages>,
    pub page: usize,
    pub saved_page: usize,
    pub last_turn: Instant,
}

impl LainApp {
    pub fn open_reader(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.reader = Some(ReaderState {
            id: id.to_string(),
            pages: Load::Loading,
            page: 0,
            saved_page: 0,
            last_turn: Instant::now(),
        });
        let (c, want) = (b.client.clone(), id.to_string());
        let sync = b.sync.clone();
        self.spawn_ui(
            cx,
            async move {
                let pages = c.comic_pages(&want).await;
                let p = match sync {
                    Some(s) => s.fetch(&want).await.ok(),
                    None => c.progress(&want).await.ok(),
                };
                (pages, p)
            },
            |app, (pages, p), _| {
                if let Some(r) = app.reader.as_mut() {
                    let n = pages.as_ref().map(|p| p.pages.len()).unwrap_or(0);
                    let resume = p
                        .filter(|p| !p.completed && p.position_sec >= 1.)
                        .map(|p| p.position_sec as usize - 1)
                        .unwrap_or(0);
                    r.page = resume.min(n.saturating_sub(1));
                    r.saved_page = r.page;
                    r.pages = Load::from(pages);
                }
            },
        );
    }

    /// Turns `d` pages forward (negative: back).
    pub fn reader_turn(&mut self, d: i64, cx: &mut Context<Self>) {
        let Some(r) = self.reader.as_mut() else { return };
        let Some(n) = r.pages.ready().map(|p| p.pages.len()) else { return };
        let next = (r.page as i64 + d).clamp(0, n as i64 - 1) as usize;
        if next == r.page {
            return;
        }
        r.page = next;
        r.last_turn = Instant::now();
        let (id, page) = (r.id.clone(), next);
        if let Some(b) = self.backend.clone() {
            let done = page + 1 == n;
            self.reader.as_mut().unwrap().saved_page = page;
            self.spawn_ui(cx, crate::backend::save_progress(b, id, (page + 1) as f64, n as f64, done), |app, r, cx| {
                if r.is_err() {
                    app.report(&r, "");
                } else {
                    app.refresh_continues(cx);
                }
            });
        }
    }

    pub fn reader_key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let rtl = self.reader.as_ref().and_then(|r| r.pages.ready()).is_some_and(|p| p.direction == "rtl");
        let fwd = if rtl { -1 } else { 1 };
        match ev.keystroke.key.as_str() {
            "right" | "l" => self.reader_turn(fwd, cx),
            "left" | "h" => self.reader_turn(-fwd, cx),
            "space" | "pagedown" | "down" => self.reader_turn(1, cx),
            "pageup" | "up" | "backspace" => self.reader_turn(-1, cx),
            "home" => self.reader_turn(-100_000, cx),
            "end" => self.reader_turn(100_000, cx),
            _ => return false,
        }
        true
    }

    pub fn render_reader(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let (vw, vh) = crate::ui::viewport(window);
        let sf = window.scale_factor();
        if self.reader.as_ref().map(|r| r.id != id).unwrap_or(true) {
            self.open_reader(id, cx);
        }
        let title = self.by_id.get(id).map(|&i| self.catalog[i].title.clone()).unwrap_or_default();
        let back = self.btn("reader.back", "Back", Some("Esc"), Tone::Quiet, Act::Back, cx);
        let mut root = div().size_full().relative().bg(gpui::black());
        let top = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(52.))
            .px(px(th.sp(4)))
            .flex()
            .items_center()
            .justify_between()
            .bg(pal.bg.opacity(0.7))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .child(back)
                    .child(div().font_family(self.fonts.display.clone()).text_size(px(th.ts(1))).child(title)),
            );
        if !self.module_on("read") {
            return root
                .child(div().size_full().flex().items_center().justify_center().child(self.notice(
                    Notice::Disabled,
                    "The read module is off",
                    "An admin turned reading off on this server.",
                )))
                .child(top);
        }
        let (pages, page) = match self.reader.as_ref().map(|r| (&r.pages, r.page)) {
            Some((Load::Ready(p), page)) => (p.clone(), page),
            Some((Load::Failed(e), _)) => {
                let e = e.clone();
                return root
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(self.error_notice("The reader", &e)),
                    )
                    .child(top);
            }
            _ => {
                return root
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(pal.muted)
                            .child("Opening…"),
                    )
                    .child(top);
            }
        };
        let n = pages.pages.len();
        if n == 0 {
            return root
                .child(div().size_full().flex().items_center().justify_center().child(self.notice(
                    Notice::Empty,
                    "No pages",
                    "The archive has no images.",
                )))
                .child(top);
        }
        let pg = &pages.pages[page];
        let (avail_w, avail_h) = (vw - 40., vh - 52. - 44.);
        let aspect = if pg.width > 0 && pg.height > 0 { pg.width as f32 / pg.height as f32 } else { 2. / 3. };
        let (w, h) = if avail_w / avail_h > aspect { (avail_h * aspect, avail_h) } else { (avail_w, avail_w / aspect) };
        let key =
            ImgKey { src: Source::Page(id.to_string(), pg.index), w: (w * sf) as u32, h: (h * sf) as u32, blur: 0 };
        let art = self.images.update(cx, |s, cx| s.get(&key, cx));
        // Prefetch the next page.
        let ahead = if page + 1 < n { Some(&pages.pages[page + 1]) } else { None };
        if let Some(nx) = ahead {
            let aspect = if nx.width > 0 && nx.height > 0 { nx.width as f32 / nx.height as f32 } else { aspect };
            let (w2, h2) =
                if avail_w / avail_h > aspect { (avail_h * aspect, avail_h) } else { (avail_w, avail_w / aspect) };
            let k2 = ImgKey {
                src: Source::Page(id.to_string(), nx.index),
                w: (w2 * sf) as u32,
                h: (h2 * sf) as u32,
                blur: 0,
            };
            self.images.update(cx, |s, cx| {
                s.get(&k2, cx);
            });
        }
        let mut stage =
            div().absolute().top(px(52.)).left_0().right_0().bottom(px(44.)).flex().items_center().justify_center();
        stage = match art {
            Some((a, _)) => stage.child(img(ImageSource::Render(a)).w(px(w)).h(px(h)).object_fit(ObjectFit::Contain)),
            None => stage.child(div().w(px(w)).h(px(h)).bg(pal.surface.opacity(0.4))),
        };
        let rtl = pages.direction == "rtl";
        let bar_w = vw - th.sp(5) * 2.;
        let frac = (page + 1) as f32 / n as f32;
        root = root.child(stage).child(top).child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(44.))
                .px(px(th.sp(5)))
                .flex()
                .items_center()
                .gap(px(th.sp(3)))
                .child(
                    div()
                        .font_family(self.fonts.mono.clone())
                        .text_size(px(th.ts(-1)))
                        .child(format!("{} / {n}", page + 1)),
                )
                .child(
                    div().flex_1().h(px(3.)).rounded_full().bg(pal.text.opacity(0.12)).child(
                        div()
                            .h_full()
                            .w(px((bar_w - 260.).max(40.) * frac))
                            .bg(pal.accent)
                            .rounded_full()
                            .when(rtl, |d| d.ml_auto()),
                    ),
                )
                .child(self.label(if rtl { "right to left" } else { "left to right" }))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .text_size(px(th.ts(-2)))
                        .text_color(pal.muted)
                        .child(self.kbd("← →"))
                        .child("turn"),
                ),
        );
        root
    }
}
