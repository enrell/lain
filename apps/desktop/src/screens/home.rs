//! Home: a full-bleed, title-first hero and horizontal rails (Continue,
//! Recently added, one per library). Rails are virtualized by hand: only
//! the cards in view (plus one on each side) become elements, positioned
//! from the rail's animated offset, so a 5,000-card rail costs the same
//! as a 10-card one. GPUI has no element transforms, so the hover
//! "scale" grows the card's layout box around its centre.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    Context, FontWeight, ImageSource, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ScrollDelta, ScrollWheelEvent, Window, div, img, linear_color_stop, linear_gradient, point, px,
};

use crate::app::{Act, Anim, LainApp, Load, Screen, Section};
use crate::images::{ImgKey, Source};
use crate::ui::{Notice, Tone, fmt_time, title_hue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    Continue,
    Posters,
}

/// What a card shows: a title (series index) or one item (catalog index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    Series(usize),
    Item(usize),
}

pub struct RailState {
    pub title: String,
    pub kind: RailKind,
    pub cards: Vec<Card>,
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

pub struct HomeState {
    pub rails: Vec<RailState>,
    pub page: Anim,
    pub page_target: f32,
    pub focus: Focus,
    prev: Option<Focus>,
    focus_at: Instant,
    drag: Option<Drag>,
    pub featured: Option<Card>,
    /// Data changed: rebuild the rails on the next frame.
    pub dirty: bool,
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
            dirty: true,
        }
    }
}

impl HomeState {
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
    pub step: f32,
    pub row_h: f32,
}

pub struct RailGeo {
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
    /// Rebuilds the rails from the catalog, Continue and the libraries,
    /// keeping offsets and focus where the rails still exist.
    pub fn build_home(&mut self) {
        let old: Vec<(String, Anim, f32)> =
            self.home.rails.iter().map(|r| (r.title.clone(), r.off, r.target)).collect();
        let mut rails: Vec<RailState> = Vec::new();
        let mut push = |title: String, kind: RailKind, cards: Vec<Card>| {
            if cards.is_empty() {
                return;
            }
            let (off, target) = old.iter().find(|o| o.0 == title).map(|o| (o.1, o.2)).unwrap_or((Anim::still(0.), 0.));
            rails.push(RailState { title, kind, cards, off, target });
        };
        let cont: Vec<Card> =
            self.continues.iter().filter_map(|p| self.by_id.get(&p.item_id).map(|&i| Card::Item(i))).collect();
        push("Continue".into(), RailKind::Continue, cont.clone());
        let mut recent: Vec<usize> = (0..self.series.len()).collect();
        recent.sort_by(|&a, &b| self.series[b].latest.cmp(&self.series[a].latest));
        push("Recently added".into(), RailKind::Posters, recent.into_iter().take(30).map(Card::Series).collect());
        if let Some(libs) = self.libraries.ready() {
            for l in libs.iter().filter(|l| l.locked != Some(true)) {
                let cards: Vec<Card> = self
                    .series
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.library_id == l.id)
                    .map(|(i, _)| Card::Series(i))
                    .collect();
                push(l.name.clone(), RailKind::Posters, cards);
            }
        }
        self.home.featured = cont
            .first()
            .copied()
            .or_else(|| rails.iter().find(|r| r.kind == RailKind::Posters).and_then(|r| r.cards.first().copied()));
        if let Focus::Card(r, i) = self.home.focus
            && (r >= rails.len() || i >= rails[r].cards.len())
        {
            self.home.focus = Focus::Hero;
        }
        self.home.rails = rails;
        self.home.dirty = false;
    }

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
        let label_h = self.theme.ts(0) * 3.2;
        Dims { w, h, pad, step: w + r.gap, row_h: pad * 2. + h + label_h }
    }

    pub fn geo(&self, vw: f32, vh: f32) -> Geo {
        let th = &self.theme;
        let hero_h = (vh * th.hero.height).max(320.);
        let margin = th.sp(6);
        let header_h = th.ts(1) * 1.4 + th.sp(2);
        let mut y = hero_h - th.sp(6);
        let mut rails = Vec::new();
        for r in &self.home.rails {
            let d = self.dims(r.kind);
            rails.push(RailGeo { top: y, row_top: y + header_h, d });
            y += header_h + d.row_h + th.sp(3);
        }
        Geo { vw, vh, hero_h, margin, rails, content_h: y + th.sp(5) }
    }

    pub fn home_nav_solid(&self) -> f32 {
        (self.home.page.value(&self.theme) / 120.).clamp(0., 1.)
    }

    fn max_off(&self, geo: &Geo, r: usize) -> f32 {
        let d = geo.rails[r].d;
        let n = self.home.rails[r].cards.len() as f32;
        (n * d.step - self.theme.rail.gap + 2. * geo.margin - geo.vw).max(0.)
    }

    fn max_page(&self, geo: &Geo) -> f32 {
        (geo.content_h - geo.vh).max(0.)
    }

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

    fn open_card(&mut self, c: Card, cx: &mut Context<Self>) {
        match c {
            Card::Item(i) => {
                let id = self.catalog[i].id.clone();
                if self.series_of.get(&id).is_some_and(|&s| self.series[s].reading()) {
                    self.navigate(Screen::Reader(id), cx);
                } else {
                    self.open_player(&id, None, cx);
                }
            }
            Card::Series(s) => {
                if let Some(&first) = self.series[s].items.first() {
                    let id = self.catalog[first].id.clone();
                    self.navigate(Screen::Item(id), cx);
                }
            }
        }
    }

    pub fn home_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.home.rails.is_empty() {
            return false;
        }
        let (vw, vh) = crate::ui::viewport(window);
        let geo = self.geo(vw, vh);
        let n_rails = self.home.rails.len();
        let f = self.home.focus;
        let next = match (ev.keystroke.key.as_str(), f) {
            ("right", Focus::Card(r, i)) => Focus::Card(r, (i + 1).min(self.home.rails[r].cards.len() - 1)),
            ("left", Focus::Card(r, i)) => Focus::Card(r, i.saturating_sub(1)),
            ("down", Focus::Hero) => Focus::Card(0, self.nearest(&geo, 0, None)),
            ("down", Focus::Card(r, i)) if r + 1 < n_rails => {
                Focus::Card(r + 1, self.nearest(&geo, r + 1, Some((r, i))))
            }
            ("up", Focus::Card(0, _)) => Focus::Hero,
            ("up", Focus::Card(r, i)) => Focus::Card(r - 1, self.nearest(&geo, r - 1, Some((r, i)))),
            ("up", Focus::Hero) => {
                // Up from the hero reaches the top navigation.
                self.move_focus(1);
                return true;
            }
            ("home", Focus::Card(r, _)) => Focus::Card(r, 0),
            ("end", Focus::Card(r, _)) => Focus::Card(r, self.home.rails[r].cards.len() - 1),
            ("enter" | "space", Focus::Card(r, i)) => {
                let c = self.home.rails[r].cards[i];
                self.open_card(c, cx);
                return true;
            }
            ("enter" | "space", Focus::Hero) => {
                self.play_featured(cx);
                return true;
            }
            ("i", Focus::Card(r, i)) => {
                // Details of the focused card's title.
                if let Card::Item(ci) = self.home.rails[r].cards[i] {
                    let id = self.catalog[ci].id.clone();
                    self.navigate(Screen::Item(id), cx);
                } else {
                    let c = self.home.rails[r].cards[i];
                    self.open_card(c, cx);
                }
                return true;
            }
            ("pagedown", _) => {
                self.scroll_page_to(&geo, self.home.page_target + geo.vh * 0.8);
                return true;
            }
            ("pageup", _) => {
                self.scroll_page_to(&geo, self.home.page_target - geo.vh * 0.8);
                return true;
            }
            _ => return false,
        };
        self.home.set_focus(next);
        self.ensure_visible(&geo, next);
        true
    }

    fn play_featured(&mut self, cx: &mut Context<Self>) {
        match self.home.featured {
            Some(Card::Item(i)) => {
                let c = Card::Item(i);
                self.open_card(c, cx);
            }
            Some(Card::Series(s)) => {
                if let Some(id) = self.next_up(s) {
                    if self.series[s].reading() {
                        self.navigate(Screen::Reader(id), cx);
                    } else {
                        self.open_player(&id, None, cx);
                    }
                }
            }
            None => {}
        }
    }

    /// The first unfinished item of a title.
    pub fn next_up(&self, s: usize) -> Option<String> {
        let ser = self.series.get(s)?;
        let ids = ser.items.iter().map(|&i| &self.catalog[i].id);
        ids.clone()
            .find(|id| !self.progress.get(*id).is_some_and(|p| p.completed))
            .or_else(|| ids.clone().next())
            .cloned()
    }

    fn nearest(&self, geo: &Geo, r: usize, from: Option<(usize, usize)>) -> usize {
        let len = self.home.rails[r].cards.len();
        let screen_x = match from {
            Some((fr, fi)) => {
                geo.margin + fi as f32 * geo.rails[fr].d.step - self.home.rails[fr].target + geo.rails[fr].d.w / 2.
            }
            None => geo.margin,
        };
        let d = geo.rails[r].d;
        let i = ((screen_x + self.home.rails[r].target - geo.margin) / d.step).max(0.) as usize;
        i.min(len.saturating_sub(1))
    }

    fn home_wheel(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (vw, vh) = crate::ui::viewport(window);
        let geo = self.geo(vw, vh);
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
        let (vw, vh) = crate::ui::viewport(window);
        let geo = self.geo(vw, vh);
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(r) = self.rail_at(&geo, y) {
            let off = self.home.rails[r].off.value(&self.theme);
            self.home.drag =
                Some(Drag { rail: r, x0: x, off0: off, moved: false, last_x: x, last_t: Instant::now(), vel: 0. });
        }
    }

    fn home_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (vw, vh) = crate::ui::viewport(window);
        let geo = self.geo(vw, vh);
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
        if y > 64.
            && let Some((r, i)) = self.card_at(&geo, x, y)
            && self.home.focus != Focus::Card(r, i)
        {
            self.home.set_focus(Focus::Card(r, i));
            self.focused = None;
            cx.notify();
        }
    }

    fn home_mouse_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (vw, vh) = crate::ui::viewport(window);
        let geo = self.geo(vw, vh);
        let Some(drag) = self.home.drag.take() else { return };
        if drag.moved {
            let vel = if drag.last_t.elapsed().as_millis() < 80 { drag.vel } else { 0. };
            let off = self.home.rails[drag.rail].off.value(&self.theme);
            self.scroll_rail_to(&geo, drag.rail, off - vel * 0.25);
            cx.notify();
        } else if let Some((r, i)) = self.card_at(&geo, f32::from(ev.position.x), f32::from(ev.position.y)) {
            let c = self.home.rails[r].cards[i];
            self.open_card(c, cx);
        }
    }

    // ---- rendering -------------------------------------------------------

    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        if self.home.dirty && matches!(self.items, Load::Ready(_)) {
            self.build_home();
        }
        let (vw, vh) = crate::ui::viewport(window);
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

        // States before there is anything to show.
        let state = match &self.items {
            Load::Idle | Load::Loading => Some(self.notice(Notice::Loading, "Loading your library…", "")),
            Load::Failed(e) => Some(self.error_notice("The catalog", &e.clone())),
            Load::Ready(_) if self.home.rails.is_empty() => Some(if self.is_admin() {
                self.notice(
                    Notice::Empty,
                    "Nothing here yet",
                    "Add a library (a folder of anime, movies or manga) and scan it. Settings → Libraries.",
                )
            } else {
                self.notice(Notice::Empty, "Nothing here yet", "An admin has not added a library you can see yet.")
            }),
            Load::Ready(_) => None,
        };
        if let Some(n) = state {
            let mut col = div().flex().flex_col().items_start().gap(px(th.sp(2))).child(n);
            if self.is_admin() && matches!(self.items, Load::Ready(_)) {
                col = col.child(div().pl(px(th.sp(5))).child(self.btn(
                    "home.addlib",
                    "Add a library",
                    None,
                    Tone::Primary,
                    Act::Nav(Screen::Settings(Section::Libraries)),
                    cx,
                )));
            }
            return root.child(div().size_full().flex().items_center().pl(px(th.sp(6))).child(col));
        }

        let geo = self.geo(vw, vh);
        let page = self.home.page.value(&th);
        animating |= self.home.page.running();
        let mut root = root;

        // ---- hero
        if let Some(fc) = self.home.featured {
            let (item_id, series_idx) = match fc {
                Card::Item(i) => (self.catalog[i].id.clone(), self.series_of.get(&self.catalog[i].id).copied()),
                Card::Series(s) => {
                    (self.series[s].items.first().map(|&i| self.catalog[i].id.clone()).unwrap_or_default(), Some(s))
                }
            };
            let q = |v: f32| ((v * sf / 64.).ceil() * 64.) as u32;
            let key = ImgKey {
                src: Source::Thumb(item_id.clone()),
                w: q(vw),
                h: q(geo.hero_h),
                blur: (th.blur.hero * sf).round() as u32,
            };
            let art = self.images.update(cx, |s, cx| s.get(&key, cx));
            let parallax = -page * 0.4;
            let title = series_idx.map(|s| self.series[s].title.clone()).unwrap_or_default();
            let hue = title_hue(&title);
            let mut hero =
                div().absolute().left_0().top(px(parallax)).w(px(vw)).h(px(geo.hero_h)).overflow_hidden().bg(pal.bg);
            match art {
                Some((art, at)) => {
                    let fade = th.ease((at.elapsed().as_secs_f32() * 1000. / th.motion.normal_ms.max(1.)).min(1.));
                    animating |= fade < 1.;
                    hero = hero
                        .child(img(ImageSource::Render(art)).size_full().object_fit(ObjectFit::Cover).opacity(fade));
                }
                None => {
                    // No art from the server: a field in the title's hue.
                    let a = crate::theme::parse_color(&format!("oklch(0.45 0.12 {hue})")).unwrap_or(pal.accent);
                    let b = crate::theme::parse_color(&format!("oklch(0.3 0.08 {})", (hue + 60.) % 360.))
                        .unwrap_or(pal.surface);
                    hero = hero.child(div().absolute().inset_0().bg(linear_gradient(
                        120.,
                        linear_color_stop(a.opacity(0.55), 0.),
                        linear_color_stop(b.opacity(0.2), 1.),
                    )));
                }
            }
            let (g0, g1) = match pal.hero_stops.as_slice() {
                [a, .., b] => (*a, *b),
                [a] => (*a, pal.bg),
                [] => (pal.bg.opacity(0.), pal.bg),
            };
            hero = hero
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(g0, 0.35),
                    linear_color_stop(g1, 1.),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(pal.bg.opacity(0.75), 0.),
                    linear_color_stop(pal.bg.opacity(0.), 0.6),
                )));
            root = root.child(hero);

            let (sub, play_label, kicker) = match (fc, series_idx) {
                (Card::Item(i), _) => {
                    let it = &self.catalog[i];
                    let p = self.progress.get(&it.id);
                    let label = episode_label(it.season, it.episode);
                    (
                        format!("{label} · {}", self.library_name(&it.library_id)),
                        format!("Resume {label} · {}", fmt_time(p.map(|p| p.position_sec).unwrap_or(0.))),
                        "CONTINUE WATCHING",
                    )
                }
                (Card::Series(s), _) => {
                    let ser = &self.series[s];
                    let reading = ser.reading();
                    (
                        format!(
                            "{} {} · {}{}",
                            ser.items.len(),
                            if reading {
                                "volumes"
                            } else if ser.items.len() == 1 {
                                "file"
                            } else {
                                "episodes"
                            },
                            self.library_name(&ser.library_id),
                            if ser.year > 0 { format!(" · {}", ser.year) } else { String::new() }
                        ),
                        if reading { "Read".to_string() } else { "Play".to_string() },
                        "RECENTLY ADDED",
                    )
                }
            };
            let hero_focus = self.home.focus == Focus::Hero && self.focused.is_none();
            let fp = self.focus_progress(Focus::Hero, &th);
            animating |= fp.1;
            let title_size = th.ts(5);
            let text_bottom = vh - (geo.hero_h - th.sp(6) - th.sp(5) - page);
            let details_id = item_id.clone();
            root = root.child(
                div()
                    .absolute()
                    .left(px(geo.margin))
                    .bottom(px(text_bottom))
                    .opacity((1. - page / (geo.hero_h * 0.45)).clamp(0., 1.))
                    .w(px((vw * 0.55).max(420.)))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(th.sp(3)))
                    .child(self.label(kicker).text_color(pal.accent))
                    .child(
                        div()
                            .font_family(self.fonts.display.clone())
                            .text_size(px(title_size))
                            .line_height(px(title_size * 1.08))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(div().text_color(pal.text.opacity(0.75)).child(sub))
                    .child(
                        div()
                            .pt(px(th.sp(2)))
                            .flex()
                            .gap(px(th.sp(2)))
                            .child(
                                div()
                                    .id("hero-play")
                                    .flex()
                                    .items_center()
                                    .gap(px(th.sp(2)))
                                    .h(px(50.))
                                    .px(px(th.sp(5)))
                                    .rounded(px(th.radius.button))
                                    .bg(pal.accent)
                                    .text_color(pal.on_accent)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .border_2()
                                    .border_color(pal.text.opacity(if hero_focus { fp.0 } else { 0. }))
                                    .hover(|s| s.opacity(0.9))
                                    .child(self.icon("play", 16., pal.on_accent))
                                    .child(play_label)
                                    .child(
                                        self.kbd("Enter")
                                            .text_color(pal.on_accent.opacity(0.7))
                                            .border_color(pal.on_accent.opacity(0.3)),
                                    )
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.play_featured(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("hero-info")
                                    .flex()
                                    .items_center()
                                    .h(px(50.))
                                    .px(px(th.sp(4)))
                                    .rounded(px(th.radius.button))
                                    .bg(pal.text.opacity(0.1))
                                    .border_1()
                                    .border_color(pal.line)
                                    .cursor_pointer()
                                    .child("Details")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.navigate(Screen::Item(details_id.clone()), cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }

        // ---- rails
        for r in 0..self.home.rails.len() {
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
                    .child(div().text_size(px(th.ts(1))).font_weight(FontWeight::SEMIBOLD).child(rail.title.clone()))
                    .child(
                        div()
                            .font_family(self.fonts.mono.clone())
                            .text_size(px(th.ts(-2)))
                            .text_color(pal.muted)
                            .child(format!("{}", rail.cards.len())),
                    ),
            );
            let (row, anim) = self.render_row(r, &geo, vw, sf, cx);
            animating |= anim;
            root = root.child(row.absolute().left_0().top(px(row_y)));
        }

        // keys legend
        root = root.child(
            div()
                .absolute()
                .right(px(geo.margin))
                .bottom(px(th.sp(3)))
                .flex()
                .gap(px(th.sp(3)))
                .text_size(px(th.ts(-2)))
                .text_color(pal.muted)
                .child(div().flex().gap(px(4.)).child(self.kbd("←↑↓→")).child("move"))
                .child(div().flex().gap(px(4.)).child(self.kbd("Enter")).child("open"))
                .child(div().flex().gap(px(4.)).child(self.kbd("I")).child("details"))
                .child(div().flex().gap(px(4.)).child(self.kbd("/")).child("search")),
        );
        if animating {
            window.request_animation_frame();
        }
        root
    }

    pub fn library_name(&self, id: &str) -> String {
        self.libraries.ready().and_then(|l| l.iter().find(|l| l.id == id)).map(|l| l.name.clone()).unwrap_or_default()
    }

    fn focus_progress(&self, f: Focus, th: &crate::theme::Theme) -> (f32, bool) {
        let t = (self.home.focus_at.elapsed().as_secs_f32() * 1000. / th.motion.fast_ms.max(1.)).min(1.);
        let e = th.ease(t);
        if self.focused.is_some() {
            return (0., false);
        }
        if self.home.focus == f {
            (e, t < 1.)
        } else if self.home.prev == Some(f) {
            (1. - e, t < 1.)
        } else {
            (0., false)
        }
    }

    /// One rail's visible cards. Returns (row, animating).
    fn render_row(&mut self, r: usize, geo: &Geo, vw: f32, sf: f32, cx: &mut Context<Self>) -> (gpui::Div, bool) {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let d = geo.rails[r].d;
        let rail = &self.home.rails[r];
        let kind = rail.kind;
        let off = rail.off.value(&th);
        let n = rail.cards.len();
        let first = (((off - geo.margin - d.w) / d.step).floor().max(0.)) as usize;
        let last = ((((off + vw) / d.step).ceil()) as usize + 1).min(n);
        let first = first.min(last);
        let cards: Vec<Card> = rail.cards[first..last].to_vec();
        let mut row = div().w(px(vw)).h(px(d.row_h)).overflow_hidden();
        let mut animating = false;
        let scale = th.rail.hover_scale.max(1.);
        for (k, card) in cards.into_iter().enumerate() {
            let i = first + k;
            let (item_idx, title, sub, progress) = match card {
                Card::Item(ci) => {
                    let it = &self.catalog[ci];
                    let p = self.progress.get(&it.id);
                    let frac = p
                        .filter(|p| p.duration_sec > 0.)
                        .map(|p| (p.position_sec / p.duration_sec).clamp(0., 1.) as f32);
                    let left = p.map(|p| (p.duration_sec - p.position_sec).max(0.)).unwrap_or(0.);
                    (
                        ci,
                        it.title.clone(),
                        format!("{} · {} left", episode_label(it.season, it.episode), fmt_time(left)),
                        frac,
                    )
                }
                Card::Series(s) => {
                    let ser = &self.series[s];
                    let n = ser.items.len();
                    let unit = if ser.reading() {
                        "vol"
                    } else if n == 1 {
                        "file"
                    } else {
                        "ep"
                    };
                    (
                        ser.items[0],
                        ser.title.clone(),
                        format!("{n} {unit}{}", if ser.year > 0 { format!(" · {}", ser.year) } else { String::new() }),
                        None,
                    )
                }
            };
            let (fp, moving) = self.focus_progress(crate::screens::home::Focus::Card(r, i), &th);
            animating |= moving;
            let s = 1. + (scale - 1.) * fp;
            let (w, h) = (d.w * s, d.h * s);
            let x = geo.margin + i as f32 * d.step - off - (w - d.w) / 2.;
            let y = d.pad - (h - d.h) / 2.;
            let key = ImgKey {
                src: Source::Thumb(self.catalog[item_idx].id.clone()),
                w: (d.w * scale * sf).round() as u32,
                h: (d.h * scale * sf).round() as u32,
                blur: 0,
            };
            let art = self.images.update(cx, |st, cx| st.get(&key, cx));
            let radius = th.radius.card * s;
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
                        .border_color(pal.focus.opacity(fp)),
                );
            }
            let mut c = div()
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
                c = c.shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.45 * fp),
                    offset: point(px(0.), px(10. * fp)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                }]);
            }
            match art {
                Some((art, at)) => {
                    let fade = th.ease((at.elapsed().as_secs_f32() * 1000. / th.motion.normal_ms.max(1.)).min(1.));
                    animating |= fade < 1.;
                    c = c.child(
                        img(ImageSource::Render(art))
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .rounded(px(radius))
                            .opacity(fade),
                    );
                }
                None => c = c.child(self.placeholder(&title, w, h, kind == RailKind::Continue)),
            }
            if let Some(frac) = progress {
                c = c.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(px(th.player.seek_h.max(3.)))
                        .bg(pal.bg.opacity(0.6))
                        .child(div().h_full().w(px(w * frac)).bg(pal.accent)),
                );
            }
            row = row.child(c);
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
                            .child(title),
                    )
                    .child(div().truncate().text_size(px(th.ts(-1))).text_color(pal.muted).child(sub)),
            );
        }
        (row, animating)
    }

    /// Typographic placeholder art: the title in the display face over a
    /// field in the title's own hue (no server art yet).
    pub fn placeholder(&self, title: &str, w: f32, h: f32, landscape: bool) -> gpui::Div {
        let hue = title_hue(title);
        let dark = self.theme.scheme != "light";
        let (l1, l2) = if dark { (0.36, 0.22) } else { (0.86, 0.74) };
        let a = crate::theme::parse_color(&format!("oklch({l1} 0.09 {hue})")).unwrap_or(self.pal.surface);
        let b =
            crate::theme::parse_color(&format!("oklch({l2} 0.06 {})", (hue + 40.) % 360.)).unwrap_or(self.pal.surface);
        let size = if landscape { (h * 0.16).clamp(14., 30.) } else { (w * 0.13).clamp(13., 28.) };
        div()
            .size_full()
            .bg(linear_gradient(160., linear_color_stop(a, 0.), linear_color_stop(b, 1.)))
            .flex()
            .flex_col()
            .justify_end()
            .p(px(w * 0.08))
            .child(
                div()
                    .font_family(self.fonts.display.clone())
                    .text_size(px(size))
                    .line_height(px(size * 1.1))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(self.pal.text.opacity(0.92))
                    .child(title.to_string()),
            )
            .child(div().pt(px(6.)).w(px(w * 0.18)).h(px(2.)).bg(self.pal.accent))
    }
}

pub fn episode_label(season: i64, episode: i64) -> String {
    match (season, episode) {
        (_, 0) => "Movie".into(),
        (0, e) => format!("E{e:02}"),
        (s, e) => format!("S{s} · E{e:02}"),
    }
}
