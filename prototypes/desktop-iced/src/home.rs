//! Home: the hero and the rails. Rails are virtualized: each one keeps a
//! scroll offset (an iced `Animation<f32>`) and only the cards in view are
//! built, placed by `Strip`.

use std::collections::HashMap;
use std::time::Instant;

use iced::widget::{Space, button, column, container, float, image, mouse_area, row, scrollable, stack, text};
use iced::{Animation, Background, Border, Color, Element, Fill, Gradient, Length, Padding, Shadow, Vector, gradient, mouse};

use crate::app::{App, HOME_SCROLL, Message};
use crate::data::{ArtKey, ArtKind, Item};
use crate::strip::Strip;
use crate::theme::{Theme, mix, with_alpha};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    /// Landscape cards with a progress bar.
    Continue,
    Items,
}

pub struct Drag {
    pub start_x: f32,
    pub start_off: f32,
    pub last_x: f32,
    pub last_t: Instant,
    pub vel: f32,
    pub moved: bool,
}

pub struct Rail {
    pub title: String,
    pub kind: RailKind,
    /// Catalog indices.
    pub items: Vec<usize>,
    /// Continue rail only: (position, duration) per entry.
    pub progress: Vec<(f64, f64)>,
    pub offset: Animation<f32>,
    pub target: f32,
    pub drag: Option<Drag>,
    pub pointer: Option<(f32, f32)>,
}

impl Rail {
    pub fn new(title: String, kind: RailKind, items: Vec<usize>, t: &Theme) -> Self {
        Rail {
            title,
            kind,
            items,
            progress: Vec::new(),
            offset: anim(0.0, t),
            target: 0.0,
            drag: None,
            pointer: None,
        }
    }

    pub fn value(&self, now: Instant) -> f32 {
        self.offset.interpolate_with(|v| v, now)
    }
}

pub fn anim(v: f32, t: &Theme) -> Animation<f32> {
    Animation::new(v).duration(t.normal).easing(t.easing)
}

/// Geometry of one rail under the current theme.
#[derive(Debug, Clone, Copy)]
pub struct Geo {
    pub w: f32,
    pub h: f32,
    pub stride: f32,
    pub pad: f32,
    /// Room above and below the cards for the hover lift.
    pub lift: f32,
    pub label_h: f32,
    pub title_h: f32,
}

impl Geo {
    pub fn of(kind: RailKind, t: &Theme) -> Geo {
        let (w, h) = match kind {
            RailKind::Continue => {
                let w = t.card_w * 1.6;
                (w, (w * 9.0 / 16.0).round())
            }
            RailKind::Items => (t.card_w, t.card_h),
        };
        let lift = (h * (t.hover_scale - 1.0).max(0.0) / 2.0).ceil() + 4.0;
        Geo {
            w,
            h,
            stride: w + t.gap,
            pad: t.sp(6),
            lift,
            label_h: t.size_base * 2.6 + t.sp(2),
            title_h: t.size_base * 1.35 + t.sp(3),
        }
    }

    pub fn band_h(&self) -> f32 {
        self.h + self.label_h + self.lift * 2.0
    }

    /// Height of the whole rail block (title + band + spacing below).
    pub fn block_h(&self, t: &Theme) -> f32 {
        self.title_h + self.band_h() + t.sp(4)
    }

    pub fn max_offset(&self, n: usize, view_w: f32) -> f32 {
        let content = self.pad * 2.0 + n as f32 * self.stride - self.gap_of();
        (content - view_w).max(0.0)
    }

    fn gap_of(&self) -> f32 {
        self.stride - self.w
    }

    /// Which card the pointer is over (x in band coordinates).
    pub fn hit(&self, x: f32, y: f32, offset: f32, n: usize) -> Option<usize> {
        let cx = x + offset - self.pad;
        if cx < 0.0 || y < self.lift || y > self.lift + self.h + self.label_h {
            return None;
        }
        let i = (cx / self.stride).floor() as usize;
        (i < n && cx - i as f32 * self.stride <= self.w).then_some(i)
    }
}

/// Which art a card wants: posters for portrait cards, backdrops for
/// landscape ones (theme-alt's 240x180 cards, the Continue rail).
pub fn card_art_key(slug: &str, w: f32, h: f32, scale: f32) -> ArtKey {
    let kind = if w > h { ArtKind::Backdrop } else { ArtKind::Poster };
    ArtKey { slug: slug.to_string(), kind, w: (w * scale).round() as u32, h: (h * scale).round() as u32, blur: 0 }
}

pub fn hero_key(slug: &str, blur: f32) -> ArtKey {
    ArtKey { slug: slug.to_string(), kind: ArtKind::Backdrop, w: 1920, h: 1080, blur: blur.round() as u32 }
}

fn txt<'a>(s: impl text::IntoFragment<'a>, size: f32, font: iced::Font, color: Color) -> text::Text<'a> {
    text(s).size(size).font(font).color(color)
}

pub fn view(app: &App) -> Element<'_, Message> {
    let t = &app.theme;
    let now = app.now;
    let mut col = column![hero(app)].spacing(0);
    for (r, rail) in app.rails.iter().enumerate() {
        col = col.push(rail_view(app, r, rail, now));
    }
    col = col.push(Space::new().height(t.sp(6)));
    let page = scrollable(col)
        .id(HOME_SCROLL)
        .on_scroll(Message::HomeScrolled)
        .width(Fill)
        .height(Fill)
        .style(move |_th, _st| {
            let mut s = scrollable::default(_th, _st);
            let rail = scrollable::Rail {
                background: None,
                border: Border::default(),
                scroller: scrollable::Scroller {
                    background: Background::Color(with_alpha(app.theme.muted, 0.35)),
                    border: Border { radius: 3.0.into(), ..Border::default() },
                },
            };
            s.vertical_rail = rail;
            s.horizontal_rail = rail;
            s.container = container::Style::default();
            s
        });
    let nav = top_nav(app);
    let status = app.status_line().map(|s| {
        container(txt(s, t.size_base * 0.8, t.font_mono, t.muted))
            .padding(Padding::from([t.sp(1), t.sp(3)]))
            .style(move |_| container::Style {
                background: Some(Background::Color(with_alpha(t.surface, 0.85))),
                border: Border { radius: t.radius_button.into(), ..Border::default() },
                ..Default::default()
            })
    });
    let mut s = stack![page, nav];
    if let Some(st) = status {
        s = s.push(container(st).align_bottom(Fill).align_right(Fill).padding(t.sp(4)).width(Fill).height(Fill));
    }
    container(s)
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style { background: Some(Background::Color(t.bg)), ..Default::default() })
        .into()
}

/// Quiet top navigation (D-022): wordmark left, account right. It fades
/// in a backing once the page scrolls past the hero.
fn top_nav(app: &App) -> Element<'_, Message> {
    let t = &app.theme;
    let past = (app.home_scroll_y / (app.hero_h() * 0.6)).clamp(0.0, 1.0);
    let user = app.username.clone().unwrap_or_default();
    let bar = row![
        txt("lain", t.size_base * 1.6, t.font_display, t.text),
        Space::new().width(t.sp(5)),
        txt("Home", t.size_base * 0.95, t.font_ui_medium, t.text),
        Space::new().width(Fill),
        txt(user, t.size_base * 0.9, t.font_ui, t.muted),
    ]
    .align_y(iced::Center)
    .padding(Padding::from([t.sp(3), t.sp(6)]));
    container(bar)
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(with_alpha(t.bg, 0.97 * past))),
            ..Default::default()
        })
        .into()
}

fn hero(app: &App) -> Element<'_, Message> {
    let t = &app.theme;
    let h = app.hero_h();
    let Some((slug, title)) = app.featured() else {
        return container(Space::new()).height(h).width(Fill).into();
    };
    let art: Element<'_, Message> = match app.art.get(&hero_key(&slug, t.hero_blur)) {
        Some(handle) => image(handle.clone()).width(Fill).height(h).content_fit(iced::ContentFit::Cover).into(),
        None => container(Space::new()).width(Fill).height(h).style(move |_| container::Style {
            background: Some(Background::Color(t.surface)),
            ..Default::default()
        }).into(),
    };
    let (g0, g1) = t.hero_gradient;
    // Theme gradient top → bottom, plus a left scrim for the title.
    let scrim_v = container(Space::new()).width(Fill).height(h).style(move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            gradient::Linear::new(std::f32::consts::PI)
                .add_stop(0.0, g0)
                .add_stop(0.45, mix(g0, g1, 0.25))
                .add_stop(1.0, g1),
        ))),
        ..Default::default()
    });
    let scrim_h = container(Space::new()).width(Fill).height(h).style(move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            gradient::Linear::new(std::f32::consts::FRAC_PI_2)
                .add_stop(0.0, with_alpha(t.scrim, 0.9))
                .add_stop(0.55, with_alpha(t.scrim, 0.0)),
        ))),
        ..Default::default()
    });
    let resume = app.hero_target();
    let label = match &resume {
        Some((_, Some(_))) => "Resume",
        _ => "Play",
    };
    let sub = match &resume {
        Some((idx, p)) => {
            let it = &app.catalog.items[*idx];
            match p {
                Some((pos, dur)) => format!("{} · {} of {}", it.label(), crate::player::fmt_time(*pos), crate::player::fmt_time(*dur)),
                None => it.label(),
            }
        }
        None => String::new(),
    };
    let n_eps = app.catalog.items.iter().filter(|i| i.slug == slug).count();
    let play = button(
        row![txt("▶", t.size_base * 0.9, t.font_ui, t.bg), txt(label, t.size_base, t.font_ui_semibold, t.bg)]
            .spacing(t.sp(1))
            .align_y(iced::Center),
    )
    .padding(Padding::from([t.sp(2), t.sp(4)]))
    .on_press(Message::PlayHero)
    .style(move |_, st| button::Style {
        background: Some(Background::Color(match st {
            button::Status::Hovered | button::Status::Pressed => mix(t.text, t.accent, 0.25),
            _ => t.text,
        })),
        text_color: t.bg,
        border: Border { radius: t.radius_button.into(), ..Border::default() },
        shadow: Shadow { color: with_alpha(Color::BLACK, 0.35), offset: Vector::new(0.0, 6.0), blur_radius: 18.0 },
        snap: true,
    });
    let content = column![
        txt("FEATURED", t.size_base * 0.75, t.font_mono, t.accent),
        txt(title, (h * 0.13).clamp(40.0, 96.0), t.font_display, t.text).line_height(1.0),
        txt(format!("{n_eps} episodes  ·  {sub}"), t.size_base * 0.95, t.font_ui, mix(t.text, t.muted, 0.35)),
        Space::new().height(t.sp(2)),
        play,
    ]
    .spacing(t.sp(2))
    .max_width(820);
    let content = container(content)
        .padding(Padding { top: 0.0, right: 0.0, bottom: t.sp(6), left: t.sp(6) })
        .width(Fill)
        .height(h)
        .align_bottom(h);
    stack![art, scrim_v, scrim_h, content].width(Fill).height(h).into()
}

fn rail_view<'a>(app: &'a App, r: usize, rail: &'a Rail, now: Instant) -> Element<'a, Message> {
    let t = &app.theme;
    let g = Geo::of(rail.kind, t);
    let off = rail.value(now);
    let view_w = app.window.width;
    let n = rail.items.len();
    let first = (((off - g.pad) / g.stride).floor().max(0.0)) as usize;
    let last = ((((off + view_w - g.pad) / g.stride).ceil()) as usize + 1).min(n);
    let mut strip = Strip::new(g.band_h());
    for i in first..last {
        let x = g.pad + i as f32 * g.stride - off;
        strip = strip.push(x, card(app, r, rail, i, &g, now));
    }
    let count = if n > 12 { format!("{n}") } else { String::new() };
    let header = row![
        txt(rail.title.clone(), t.size_base * 1.2, t.font_ui_semibold, t.text),
        Space::new().width(t.sp(2)),
        txt(count, t.size_base * 0.8, t.font_mono, t.muted),
    ]
    .align_y(iced::Alignment::End)
    .height(g.title_h)
    .padding(Padding { top: 0.0, right: 0.0, bottom: 0.0, left: g.pad });
    let band = mouse_area(strip)
        .on_scroll(move |d| Message::RailWheel(r, d))
        .on_press(Message::RailPress(r))
        .on_release(Message::RailRelease(r))
        .on_move(move |p| Message::RailMove(r, p))
        .on_exit(Message::RailExit(r))
        .interaction(if rail.drag.as_ref().is_some_and(|d| d.moved) {
            mouse::Interaction::Grabbing
        } else if app.hovered.is_some_and(|(hr, _)| hr == r) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        });
    column![header, band].height(g.block_h(t)).into()
}

fn card<'a>(app: &'a App, r: usize, rail: &'a Rail, i: usize, g: &Geo, now: Instant) -> Element<'a, Message> {
    let t = &app.theme;
    let item: &Item = &app.catalog.items[rail.items[i]];
    let lifted = app.lift_amount((r, i), now);
    let focused = app.focus == Some((r, i));
    let key = card_art_key(&item.slug, g.w, g.h, app.art_scale());
    let radius = t.radius_card;
    let art: Element<'a, Message> = match app.art.get(&key) {
        Some(h) => image(h.clone())
            .width(g.w)
            .height(g.h)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(radius)
            .into(),
        None => container(txt(item.title.chars().take(1).collect::<String>(), g.h * 0.3, t.font_display, with_alpha(t.muted, 0.5)))
            .center(g.w)
            .height(g.h)
            .style(move |_| container::Style {
                background: Some(Background::Color(t.surface)),
                border: Border { radius: radius.into(), ..Border::default() },
                ..Default::default()
            })
            .into(),
    };
    let mut layers = stack![art].width(g.w).height(g.h);
    if rail.kind == RailKind::Continue {
        if let Some(&(pos, dur)) = rail.progress.get(i) {
            let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let seg = |c: Color, w: f32| {
                container(Space::new()).width(Length::FillPortion(((w * 1000.0) as u16).max(1))).height(3).style(move |_| {
                    container::Style { background: Some(Background::Color(c)), ..Default::default() }
                })
            };
            let bar = row![seg(t.accent, frac), seg(with_alpha(Color::WHITE, 0.35), 1.0 - frac)].height(3);
            layers = layers.push(
                container(container(bar).width(Fill).padding(Padding { top: 0.0, right: 12.0, bottom: 10.0, left: 12.0 }))
                    .align_bottom(g.h)
                    .width(g.w)
                    .height(g.h),
            );
        }
    }
    // Focus ring (keyboard), drawn above the art.
    if focused {
        layers = layers.push(container(Space::new()).width(g.w).height(g.h).style(move |_| container::Style {
            border: Border { radius: radius.into(), width: 2.5, color: t.accent },
            ..Default::default()
        }));
    }
    let framed = layers;
    let title_color = mix(mix(t.text, t.muted, 0.15), t.text, lifted);
    let body = column![
        framed,
        Space::new().height(t.sp(1) + 2.0),
        txt(item.title.clone(), t.size_base * 0.95, t.font_ui_medium, title_color).wrapping(text::Wrapping::None),
        txt(item.label(), t.size_base * 0.8, t.font_mono, t.muted),
    ]
    .width(g.w);
    let scale = 1.0 + (t.hover_scale - 1.0) * lifted;
    // Float only draws above its neighbours when it scales; no lift, no shadow.
    let shadow_a = if t.hover_scale > 1.0 { 0.55 * lifted } else { 0.0 };
    let lifted_el = float(body)
        .scale(scale)
        .style(move |_| iced::widget::float::Style {
            shadow: Shadow {
                color: with_alpha(Color::BLACK, shadow_a),
                offset: Vector::new(0.0, 10.0 * lifted),
                blur_radius: 28.0 * lifted,
            },
            shadow_border_radius: radius.into(),
        });
    container(lifted_el).padding(Padding { top: g.lift, right: 0.0, bottom: 0.0, left: 0.0 }).into()
}

/// Precomputed rail layout: y of each rail's band top in page coordinates.
pub fn rail_tops(app: &App) -> Vec<f32> {
    let mut y = app.hero_h();
    let mut out = Vec::with_capacity(app.rails.len());
    for rail in &app.rails {
        let g = Geo::of(rail.kind, &app.theme);
        out.push(y);
        y += g.block_h(&app.theme);
    }
    out
}

pub type ArtMap = HashMap<ArtKey, iced::widget::image::Handle>;
