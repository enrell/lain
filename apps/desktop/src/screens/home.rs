//! Home: a quiet top bar, a full-bleed title-first hero and horizontal
//! rails. Rails are virtualized by hand: only the cards in view (plus one
//! on each side) become elements, positioned absolutely from the rail's
//! animated offset, so a 500-card rail costs the same as a 10-card one.
//! GPUI has no element transforms, so the hover "scale" grows the card's
//! layout box around its centre.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    Context, FontWeight, ImageSource, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ScrollDelta, ScrollWheelEvent, Window, div, img, linear_color_stop, linear_gradient, point, px, svg,
};

use crate::backend::Item;
use crate::images::ImgKey;
use crate::{Anim, LainApp, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    Continue,
    Posters,
}

pub struct RailState {
    pub title: String,
    pub kind: RailKind,
    /// Item indices.
    pub cards: Vec<usize>,
    pub off: Anim,
    pub target: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Hero,
    Card(usize, usize),
}

struct Drag {
    rail: usize,
    x0: f32,
    off0: f32,
    moved: bool,
    last_x: f32,
    last_t: Instant,
    vel: f32,
}

pub struct BenchScroll {
    pub rail: usize,
    pub start: Instant,
    pub px_per_s: f32,
    pub done: bool,
}

pub struct HomeState {
    pub rails: Vec<RailState>,
    pub page: Anim,
    pub page_target: f32,
    pub focus: Focus,
    prev: Option<Focus>,
    focus_at: Instant,
    drag: Option<Drag>,
    pub featured: Option<usize>,
    pub bench_scroll: Option<BenchScroll>,
    /// The first frame with hero + visible posters drawn.
    pub art_ready: bool,
}

impl Default for HomeState {
    fn default() -> Self {
        HomeState {
            rails: Vec::new(),
            page: Anim::still(0.),
            page_target: 0.,
            focus: Focus::Hero,
            prev: None,
            focus_at: Instant::now(),
            drag: None,
            featured: None,
            bench_scroll: None,
            art_ready: false,
        }
    }
}

impl HomeState {
    pub fn build(&mut self, items: &[Item], continues: &[(usize, lain_client::types::Progress)]) {
        self.rails.clear();
        self.rebuild_continue(continues);
        self.rails.push(RailState {
            title: format!("All episodes · {}", items.len()),
            kind: RailKind::Posters,
            cards: (0..items.len()).collect(),
            off: Anim::still(0.),
            target: 0.,
        });
        // One rail per ten series.
        let mut slugs: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
        slugs.dedup();
        for (g, chunk) in slugs.chunks(10).enumerate() {
            let cards: Vec<usize> =
                items.iter().enumerate().filter(|(_, it)| chunk.contains(&it.slug.as_str())).map(|(i, _)| i).collect();
            self.rails.push(RailState {
                title: format!("Series {}–{}", g * 10 + 1, g * 10 + chunk.len()),
                kind: RailKind::Posters,
                cards,
                off: Anim::still(0.),
                target: 0.,
            });
        }
        self.featured = continues.first().map(|c| c.0).or(if items.is_empty() { None } else { Some(0) });
        self.focus = Focus::Hero;
    }

    pub fn rebuild_continue(&mut self, continues: &[(usize, lain_client::types::Progress)]) {
        let has = self.rails.first().map(|r| r.kind == RailKind::Continue).unwrap_or(false);
        let cards: Vec<usize> = continues.iter().map(|c| c.0).collect();
        match (has, cards.is_empty()) {
            (true, true) => {
                self.rails.remove(0);
                self.focus = Focus::Hero;
            }
            (true, false) => {
                let r = &mut self.rails[0];
                r.cards = cards;
                r.title = "Continue".into();
                if let Focus::Card(0, i) = self.focus
                    && i >= r.cards.len()
                {
                    self.focus = Focus::Card(0, r.cards.len() - 1);
                }
            }
            (false, false) => {
                self.rails.insert(
                    0,
                    RailState { title: "Continue".into(), kind: RailKind::Continue, cards, off: Anim::still(0.), target: 0. },
                );
                if let Focus::Card(r, i) = self.focus {
                    self.focus = Focus::Card(r + 1, i);
                }
            }
            (false, true) => {}
        }
    }

    fn set_focus(&mut self, f: Focus) {
        if f != self.focus {
            self.prev = Some(self.focus);
            self.focus = f;
            self.focus_at = Instant::now();
        }
    }
}

/// Card box sizes for a rail kind.
#[derive(Debug, Clone, Copy)]
pub struct Dims {
    pub w: f32,
    pub h: f32,
    /// Vertical room for the hover growth and the focus ring.
    pub pad: f32,
    pub label_h: f32,
    pub step: f32,
    pub row_h: f32,
    pub landscape: bool,
}

pub struct RailGeo {
    /// Content-space y of the rail header.
    pub top: f32,
    pub row_top: f32,
    pub d: Dims,
}

pub struct Geo {
    pub vw: f32,
    pub vh: f32,
    pub hero_h: f32,
    pub margin: f32,
    pub rails: Vec<RailGeo>,
    pub content_h: f32,
}

impl LainApp {
    pub fn dims(&self, kind: RailKind) -> Dims {
        let r = &self.theme.rail;
        let (w, h) = match kind {
            RailKind::Posters => (r.card_w, r.card_h),
            RailKind::Continue => {
                let w = (r.card_w * 1.6).max(300.);
                (w, w * 9. / 16.)
            }
        };
        let pad = h * (r.hover_scale.max(1.) - 1.) / 2. + 6.;
        let label_h = self.theme.fonts.size_base * 3.2;
        Dims { w, h, pad, label_h, step: w + r.gap, row_h: pad * 2. + h + label_h, landscape: w > h }
    }

    pub fn geo(&self, vw: f32, vh: f32) -> Geo {
        let th = &self.theme;
        let hero_h = (vh * th.hero.height).max(320.);
        let margin = th.sp(6);
        let header_h = th.fonts.size_base * 1.4 + th.sp(2);
        let mut y = hero_h - th.sp(6);
        let mut rails = Vec::new();
        for r in &self.home.rails {
            let d = self.dims(r.kind);
            rails.push(RailGeo { top: y, row_top: y + header_h, d });
            y += header_h + d.row_h + th.sp(3);
        }
        Geo { vw, vh, hero_h, margin, rails, content_h: y + th.sp(5) }
    }

    fn max_off(&self, geo: &Geo, r: usize) -> f32 {
        let d = geo.rails[r].d;
        let n = self.home.rails[r].cards.len() as f32;
        (n * d.step - self.theme.rail.gap + 2. * geo.margin - geo.vw).max(0.)
    }

    fn max_page(&self, geo: &Geo) -> f32 {
        (geo.content_h - geo.vh).max(0.)
    }

    /// The rail whose card row is under window-space `y`.
    fn rail_at(&self, geo: &Geo, y: f32) -> Option<usize> {
        let page = self.home.page.value(&self.theme);
        geo.rails.iter().position(|g| {
            let top = g.row_top - page;
            y >= top && y < top + g.d.row_h
        })
    }

    fn card_at(&self, geo: &Geo, x: f32, y: f32) -> Option<(usize, usize)> {
        let r = self.rail_at(geo, y)?;
        let g = &geo.rails[r];
        let off = self.home.rails[r].off.value(&self.theme);
        let cx = x + off - geo.margin;
        if cx < 0. {
            return None;
        }
        let i = (cx / g.d.step) as usize;
        let inside = cx - i as f32 * g.d.step <= g.d.w;
        (inside && i < self.home.rails[r].cards.len()).then_some((r, i))
    }

    fn scroll_rail_to(&mut self, geo: &Geo, r: usize, target: f32) {
        let t = target.clamp(0., self.max_off(geo, r));
        let dur = self.theme.motion.normal_ms;
        let rail = &mut self.home.rails[r];
        rail.target = t;
        rail.off.go(t, dur, &self.theme);
    }

    fn scroll_page_to(&mut self, geo: &Geo, target: f32) {
        let t = target.clamp(0., self.max_page(geo));
        self.home.page_target = t;
        let dur = self.theme.motion.normal_ms * 1.4;
        self.home.page.go(t, dur, &self.theme);
    }

    fn ensure_visible(&mut self, geo: &Geo, f: Focus) {
        match f {
            Focus::Hero => self.scroll_page_to(geo, 0.),
            Focus::Card(r, i) => {
                let d = geo.rails[r].d;
                let x = geo.margin + i as f32 * d.step;
                let target = self.home.rails[r].target;
                if x - target < geo.margin {
                    self.scroll_rail_to(geo, r, x - geo.margin);
                } else if x + d.w - target > geo.vw - geo.margin {
                    self.scroll_rail_to(geo, r, x + d.w - geo.vw + geo.margin);
                }
                let g = &geo.rails[r];
                let page = self.home.page_target;
                let nav = 72.;
                if g.top - page < nav {
                    self.scroll_page_to(geo, g.top - nav);
                } else if g.row_top + d.row_h - page > geo.vh - 8. {
                    self.scroll_page_to(geo, g.row_top + d.row_h - geo.vh + 8.);
                }
            }
        }
    }

    pub fn home_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.status, Status::Ready) || self.home.rails.is_empty() {
            return;
        }
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        let n_rails = self.home.rails.len();
        let f = self.home.focus;
        let next = match (ev.keystroke.key.as_str(), f) {
            ("right", Focus::Card(r, i)) => Focus::Card(r, (i + 1).min(self.home.rails[r].cards.len() - 1)),
            ("left", Focus::Card(r, i)) => Focus::Card(r, i.saturating_sub(1)),
            ("down", Focus::Hero) => Focus::Card(0, self.nearest(&geo, 0, None)),
            ("down", Focus::Card(r, i)) if r + 1 < n_rails => Focus::Card(r + 1, self.nearest(&geo, r + 1, Some((r, i)))),
            ("up", Focus::Card(0, _)) => Focus::Hero,
            ("up", Focus::Card(r, i)) => Focus::Card(r - 1, self.nearest(&geo, r - 1, Some((r, i)))),
            ("home", Focus::Card(r, _)) => Focus::Card(r, 0),
            ("end", Focus::Card(r, _)) => Focus::Card(r, self.home.rails[r].cards.len() - 1),
            ("enter" | "space", Focus::Card(r, i)) => {
                let item = self.home.rails[r].cards[i];
                self.open_player(item, cx);
                return;
            }
            ("enter" | "space", Focus::Hero) => {
                if let Some(item) = self.home.featured {
                    self.open_player(item, cx);
                }
                return;
            }
            ("pagedown", _) => {
                self.scroll_page_to(&geo, self.home.page_target + geo.vh * 0.8);
                cx.notify();
                return;
            }
            ("pageup", _) => {
                self.scroll_page_to(&geo, self.home.page_target - geo.vh * 0.8);
                cx.notify();
                return;
            }
            _ => return,
        };
        self.home.set_focus(next);
        self.ensure_visible(&geo, next);
        cx.notify();
    }

    /// The card of rail `r` closest on screen to the focused one.
    fn nearest(&self, geo: &Geo, r: usize, from: Option<(usize, usize)>) -> usize {
        let len = self.home.rails[r].cards.len();
        let screen_x = match from {
            Some((fr, fi)) => geo.margin + fi as f32 * geo.rails[fr].d.step - self.home.rails[fr].target + geo.rails[fr].d.w / 2.,
            None => geo.margin,
        };
        let d = geo.rails[r].d;
        let i = ((screen_x + self.home.rails[r].target - geo.margin) / d.step).max(0.) as usize;
        i.min(len.saturating_sub(1))
    }

    fn home_wheel(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let rail = if x > geo.margin * 0.5 { self.rail_at(&geo, y) } else { None };
        match rail {
            Some(r) => {
                let step = geo.rails[r].d.step;
                let d = match ev.delta {
                    ScrollDelta::Lines(p) => (p.x + p.y) * step / 3.,
                    ScrollDelta::Pixels(p) => f32::from(p.x) + f32::from(p.y),
                };
                let t = self.home.rails[r].target - d;
                self.scroll_rail_to(&geo, r, t);
            }
            None => {
                let d = match ev.delta {
                    ScrollDelta::Lines(p) => p.y * 40.,
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                };
                self.scroll_page_to(&geo, self.home.page_target - d);
            }
        }
        cx.notify();
    }

    fn home_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(r) = self.rail_at(&geo, y) {
            let off = self.home.rails[r].off.value(&self.theme);
            self.home.drag = Some(Drag { rail: r, x0: x, off0: off, moved: false, last_x: x, last_t: Instant::now(), vel: 0. });
        }
    }

    fn home_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(drag) = self.home.drag.as_mut()
            && ev.pressed_button == Some(MouseButton::Left)
        {
            if (x - drag.x0).abs() > 6. {
                drag.moved = true;
            }
            if drag.moved {
                let dt = drag.last_t.elapsed().as_secs_f32().max(1e-3);
                drag.vel = 0.7 * drag.vel + 0.3 * (x - drag.last_x) / dt;
                drag.last_x = x;
                drag.last_t = Instant::now();
                let (r, off) = (drag.rail, drag.off0 - (x - drag.x0));
                let off = off.clamp(0., self.max_off(&geo, r));
                let rail = &mut self.home.rails[r];
                rail.off.set(off);
                rail.target = off;
                cx.notify();
                return;
            }
        }
        if let Some((r, i)) = self.card_at(&geo, x, y)
            && self.home.focus != Focus::Card(r, i)
        {
            self.home.set_focus(Focus::Card(r, i));
            cx.notify();
        }
    }

    fn home_mouse_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        let Some(drag) = self.home.drag.take() else { return };
        if drag.moved {
            // A short fling, then settle.
            let vel = if drag.last_t.elapsed().as_millis() < 80 { drag.vel } else { 0. };
            let off = self.home.rails[drag.rail].off.value(&self.theme);
            self.scroll_rail_to(&geo, drag.rail, off - vel * 0.25);
            cx.notify();
        } else if let Some((r, i)) = self.card_at(&geo, f32::from(ev.position.x), f32::from(ev.position.y)) {
            let item = self.home.rails[r].cards[i];
            self.open_player(item, cx);
        }
    }

    // ---- rendering -------------------------------------------------------

    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let vs = window.viewport_size();
        let (vw, vh) = (f32::from(vs.width), f32::from(vs.height));
        let sf = window.scale_factor();
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let mut animating = false;

        let root = div()
            .id("home")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(pal.bg)
            .on_scroll_wheel(cx.listener(Self::home_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::home_mouse_down))
            .on_mouse_move(cx.listener(Self::home_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::home_mouse_up));

        let msg = match &self.status {
            Status::Loading => Some(("Connecting to the server…".to_string(), pal.muted)),
            Status::Error(e) => Some((format!("Could not load the catalog: {e}"), pal.danger)),
            Status::Ready if self.items.is_empty() => Some(("The catalog is empty.".to_string(), pal.muted)),
            Status::Ready => None,
        };
        if let Some((m, c)) = msg {
            return root.child(
                div().size_full().flex().items_center().justify_center().text_color(c).child(m),
            );
        }

        // Bench: drive the 500-card rail at a fixed speed.
        let geo0 = self.geo(vw, vh);
        if let Some(bs) = self.home.bench_scroll.as_mut() {
            let max = {
                let d = geo0.rails[bs.rail].d;
                let n = self.home.rails[bs.rail].cards.len() as f32;
                (n * d.step - th.rail.gap + 2. * geo0.margin - vw).max(0.)
            };
            let off = (bs.start.elapsed().as_secs_f32() * bs.px_per_s).min(max);
            bs.done = off >= max;
            self.home.rails[bs.rail].off.set(off);
            self.home.rails[bs.rail].target = off;
            animating = true;
        }

        let geo = geo0;
        let page = self.home.page.value(&th);
        animating |= self.home.page.running();

        // ---- hero
        let mut hero_ready = true;
        let mut root = root;
        if let Some(fi) = self.home.featured {
            let item = self.items[fi].clone();
            let q = |v: f32| ((v * sf / 64.).ceil() * 64.) as u32;
            let key = ImgKey {
                path: self.art(&item.slug, "backdrop"),
                w: q(vw),
                h: q(geo.hero_h),
                blur: (th.hero.blur * sf).round() as u32,
            };
            let images = self.images.clone();
            let art = images.update(cx, |s, cx| s.get(&key, cx));
            hero_ready = art.is_some();
            let parallax = -page * 0.4;
            let mut hero =
                div().absolute().left_0().top(px(parallax)).w(px(vw)).h(px(geo.hero_h)).overflow_hidden().bg(pal.bg);
            if let Some((art, at)) = art {
                let fade = th.ease((at.elapsed().as_secs_f32() * 1000. / th.motion.normal_ms.max(1.)).min(1.));
                animating |= fade < 1.;
                hero = hero.child(img(ImageSource::Render(art)).size_full().object_fit(ObjectFit::Cover).opacity(fade));
            }
            let (g0, g1) = match pal.hero_stops.as_slice() {
                [a, .., b] => (*a, *b),
                [a] => (*a, pal.bg),
                [] => (pal.bg.opacity(0.), pal.bg),
            };
            hero = hero
                // the theme's top → bottom scrim
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(linear_gradient(180., linear_color_stop(g0, 0.35), linear_color_stop(g1, 1.))),
                )
                // a left scrim so the title reads on any art
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(linear_gradient(90., linear_color_stop(pal.bg.opacity(0.75), 0.), linear_color_stop(pal.bg.opacity(0.), 0.6))),
                );
            root = root.child(hero);

            let series = self.series_title.get(&item.slug).cloned().unwrap_or(item.title.clone());
            let eps = self.items.iter().filter(|i| i.slug == item.slug).count();
            let resume = self.continues.iter().find(|c| c.0 == fi).map(|c| c.1.position_sec);
            let play_label = match resume {
                Some(p) => format!("Resume {} · {}", item.label(), fmt_time(p)),
                None => format!("Play {}", item.label()),
            };
            let hero_focus = self.home.focus == Focus::Hero;
            let fp = self.focus_progress(Focus::Hero, &th);
            animating |= fp.1;
            let title_size = th.fonts.size_base * 4.4;
            // anchored above the first rail header
            let text_bottom = vh - (geo.hero_h - th.sp(6) - th.sp(5) - page);
            root = root.child(
                div()
                    .absolute()
                    .left(px(geo.margin))
                    .bottom(px(text_bottom))
                    // fade out before the title slides under the top bar
                    .opacity((1. - page / (geo.hero_h * 0.45)).clamp(0., 1.))
                    .w(px((vw * 0.55).max(420.)))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(th.sp(3)))
                    .child(
                        div()
                            .text_color(pal.accent)
                            .text_size(px(th.fonts.size_base * 0.8))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if resume.is_some() { "CONTINUE WATCHING" } else { "FEATURED" }),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.display.clone())
                            .text_size(px(title_size))
                            .line_height(px(title_size * 1.1))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(series),
                    )
                    .child(div().text_color(pal.muted).child(format!(
                        "{eps} episodes · Season {} · {}",
                        item.season.max(1),
                        if self.library.is_empty() { "Library" } else { &self.library }
                    )))
                    .child(
                        div().pt(px(th.sp(2))).child(
                            div()
                                .id("hero-play")
                                .flex()
                                .items_center()
                                .gap(px(th.sp(2)))
                                .h(px(52.))
                                .px(px(th.sp(5)))
                                .rounded(px(th.radius.button))
                                .bg(pal.accent)
                                .text_color(pal.text)
                                .font_weight(FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .border_2()
                                .border_color(pal.text.opacity(if hero_focus { fp.0 } else { 0. }))
                                .hover(|s| s.opacity(0.9))
                                .child(svg().path("icons/play.svg").size(px(18.)).text_color(pal.text))
                                .child(play_label)
                                .on_click(cx.listener(move |app, _, _, cx| app.open_player(fi, cx))),
                        ),
                    ),
            );
        }

        // ---- rails
        let mut posters_ready = true;
        let n_rails = self.home.rails.len();
        for r in 0..n_rails {
            let g = &geo.rails[r];
            let header_y = g.top - page;
            let row_y = g.row_top - page;
            if row_y > vh || row_y + g.d.row_h < 0. {
                continue;
            }
            let rail = &self.home.rails[r];
            animating |= rail.off.running();
            root = root.child(
                div()
                    .absolute()
                    .left(px(geo.margin))
                    .top(px(header_y))
                    .flex()
                    .items_baseline()
                    .gap(px(th.sp(2)))
                    .child(
                        div()
                            .text_size(px(th.fonts.size_base * 1.2))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(rail.title.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono.clone())
                            .text_size(px(th.fonts.size_base * 0.75))
                            .text_color(pal.muted)
                            .child(format!("{}", rail.cards.len())),
                    ),
            );
            let (row, ready, anim) = self.render_row(r, &geo, vw, sf, cx);
            posters_ready &= ready || row_y > vh * 0.9;
            animating |= anim;
            root = root.child(row.absolute().left_0().top(px(row_y)));
        }

        // ---- top bar
        let scrolled = (page / 120.).clamp(0., 1.);
        root = root.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(vw))
                .h(px(64.))
                .px(px(geo.margin))
                .flex()
                .items_center()
                .justify_between()
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(pal.scrim.opacity(0.6 + 0.4 * scrolled), 0.),
                    linear_color_stop(pal.scrim.opacity(scrolled), 1.),
                ))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(th.sp(5)))
                        .child(
                            div()
                                .font_family(self.fonts.display.clone())
                                .text_size(px(th.fonts.size_base * 1.7))
                                .font_weight(FontWeight::BOLD)
                                .child("lain"),
                        )
                        .child(div().font_weight(FontWeight::MEDIUM).child("Home")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(th.sp(2)))
                        .text_color(pal.muted)
                        .text_size(px(th.fonts.size_base * 0.87))
                        .child(div().size(px(7.)).rounded_full().bg(pal.accent))
                        .child(format!("{} · {}", self.library, self.username)),
                ),
        );

        if !self.home.art_ready && hero_ready && posters_ready && self.images.read(cx).loading == 0 {
            self.home.art_ready = true;
            self.bench.home_art_ready();
        }
        if animating {
            window.request_animation_frame();
        }
        root
    }

    /// Focus ring progress for `f` (0..1) and whether it is still moving.
    fn focus_progress(&self, f: Focus, th: &crate::theme::Theme) -> (f32, bool) {
        let t = (self.home.focus_at.elapsed().as_secs_f32() * 1000. / th.motion.fast_ms.max(1.)).min(1.);
        let e = th.ease(t);
        if self.home.focus == f {
            (e, t < 1.)
        } else if self.home.prev == Some(f) {
            (1. - e, t < 1.)
        } else {
            (0., false)
        }
    }

    /// One rail's visible cards. Returns (row, all art ready, animating).
    fn render_row(&mut self, r: usize, geo: &Geo, vw: f32, sf: f32, cx: &mut Context<Self>) -> (gpui::Div, bool, bool) {
        let th = self.theme.clone();
        let pal = &self.pal;
        let g = &geo.rails[r];
        let d = g.d;
        let rail = &self.home.rails[r];
        let off = rail.off.value(&th);
        let n = rail.cards.len();
        let first = (((off - geo.margin - d.w) / d.step).floor().max(0.)) as usize;
        let last = ((((off + vw) / d.step).ceil()) as usize + 1).min(n);
        let mut row = div().w(px(vw)).h(px(d.row_h)).overflow_hidden();
        let mut ready = true;
        let mut animating = false;
        let images = self.images.clone();
        let scale = th.rail.hover_scale.max(1.);
        for i in first..last {
            let item = &self.items[rail.cards[i]];
            let (fp, moving) = self.focus_progress(Focus::Card(r, i), &th);
            animating |= moving;
            let s = 1. + (scale - 1.) * fp;
            let (w, h) = (d.w * s, d.h * s);
            let x = geo.margin + i as f32 * d.step - off - (w - d.w) / 2.;
            let y = d.pad - (h - d.h) / 2.;
            let kind = if d.landscape { "backdrop" } else { "poster" };
            let key = ImgKey {
                path: self.art(&item.slug, kind),
                w: (d.w * scale * sf).round() as u32,
                h: (d.h * scale * sf).round() as u32,
                blur: 0,
            };
            let art = images.update(cx, |st, cx| st.get(&key, cx));
            ready &= art.is_some() || x > vw;
            let radius = th.radius.card * s;

            // focus ring, just outside the art
            if fp > 0. {
                let ring = 3.;
                row = row.child(
                    div()
                        .absolute()
                        .left(px(x - ring))
                        .top(px(y - ring))
                        .w(px(w + ring * 2.))
                        .h(px(h + ring * 2.))
                        .rounded(px(radius + ring))
                        .border_2()
                        .border_color(pal.accent.opacity(fp)),
                );
            }
            let mut card = div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w))
                .h(px(h))
                .rounded(px(radius))
                .overflow_hidden()
                .bg(pal.surface)
                .cursor_pointer();
            if fp > 0. {
                card = card.shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.45 * fp),
                    offset: point(px(0.), px(10. * fp)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                }]);
            }
            if let Some((art, at)) = art {
                let fade = th.ease((at.elapsed().as_secs_f32() * 1000. / th.motion.normal_ms.max(1.)).min(1.));
                animating |= fade < 1.;
                card = card.child(img(ImageSource::Render(art)).size_full().object_fit(ObjectFit::Cover).rounded(px(radius)).opacity(fade));
            }
            // Continue cards: progress along the bottom edge.
            let mut sub = item.label();
            if self.home.rails[r].kind == RailKind::Continue
                && let Some((_, p)) = self.continues.get(i)
            {
                let frac = if p.duration_sec > 0. { (p.position_sec / p.duration_sec).clamp(0., 1.) as f32 } else { 0. };
                let left = (p.duration_sec - p.position_sec).max(0.);
                sub = format!("{} · {} left", item.label(), fmt_time(left));
                let bar_h = th.player.seek_h.max(2.);
                card = card.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(px(bar_h))
                        .bg(pal.bg.opacity(0.6))
                        .child(div().h_full().w(px(w * frac)).bg(pal.accent)),
                );
            }
            row = row.child(card);
            row = row.child(
                div()
                    .absolute()
                    .left(px(geo.margin + i as f32 * d.step - off))
                    .top(px(d.pad + d.h + th.sp(2)))
                    .w(px(d.w))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if fp > 0.5 { pal.text } else { pal.text.opacity(0.85) })
                            .child(item.title.clone()),
                    )
                    .child(div().text_size(px(th.fonts.size_base * 0.85)).text_color(pal.muted).child(sub)),
            );
        }
        (row, ready, animating)
    }
}

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

impl LainApp {
    /// Demo automation: focus a card as if reached by keyboard.
    pub fn demo_focus(&mut self, r: usize, i: usize, window: &mut Window) {
        let vs = window.viewport_size();
        let geo = self.geo(f32::from(vs.width), f32::from(vs.height));
        if r < self.home.rails.len() && i < self.home.rails[r].cards.len() {
            self.home.set_focus(Focus::Card(r, i));
            self.ensure_visible(&geo, Focus::Card(r, i));
        }
    }
}
