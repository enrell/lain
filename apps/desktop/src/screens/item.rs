//! A title's page: art, facts, the episodes (or volumes) with progress,
//! mark watched, offline copies, and the selected file's details with its
//! subtitle sidecars. The header's parts and the columns (episodes,
//! details, extension panels) follow the theme's layout slots
//! (`layout.rs`); extension panels come from `ui:panel` extensions.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    ClickEvent, Context, FontWeight, ImageSource, ObjectFit, Window, div, img, linear_color_stop, linear_gradient, px,
};
use lain_client::types::{CatalogItem, Sidecar};
use lain_client::{OfflineEntry, OfflineState};

use crate::app::{Act, LainApp, Load, Screen};
use crate::images::{ImgKey, Source};
use crate::screens::home::episode_label;
use crate::ui::{Notice, Tone, fmt_bytes, fmt_time};

#[derive(Debug, Clone)]
pub struct ItemState {
    pub id: String,
    pub episodes: Load<Vec<CatalogItem>>,
    pub sel: usize,
    pub sidecars: Load<Vec<Sidecar>>,
    pub sidecars_for: String,
    pub offline: HashMap<String, OfflineEntry>,
    pub dirty: bool,
    /// The metadata overlay, when the server has one (D-185..D-189).
    pub meta: Option<lain_client::types::Enrichment>,
}

impl Default for ItemState {
    fn default() -> Self {
        ItemState {
            id: String::new(),
            episodes: Load::Idle,
            sel: 0,
            sidecars: Load::Idle,
            sidecars_for: String::new(),
            offline: HashMap::new(),
            dirty: false,
            meta: None,
        }
    }
}

impl ItemState {
    pub fn has(&self, id: &str) -> bool {
        self.episodes.ready().is_some_and(|e| e.iter().any(|x| x.id == id))
    }
    fn selected(&self) -> Option<&CatalogItem> {
        self.episodes.ready().and_then(|e| e.get(self.sel))
    }
}

impl LainApp {
    pub fn load_item(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.item.id != id {
            self.item = ItemState { id: id.to_string(), episodes: Load::Loading, ..Default::default() };
        }
        let (c2, want2) = (b.client.clone(), id.to_string());
        self.spawn_ui(cx, async move { c2.enrichment(&want2).await }, |app, r, _| app.item.meta = r.ok());
        self.ext.request_panels(id);
        self.ext_event(crate::ext::Event::ItemOpened(id.to_string()));
        let c = b.client.clone();
        let want = id.to_string();
        self.spawn_ui(cx, async move { c.episodes(&want).await }, |app, r, cx| {
            let r = r.map(|mut v| {
                v.sort_by(|a, b| (a.season, a.episode, &a.file_path).cmp(&(b.season, b.episode, &b.file_path)));
                v
            });
            if let Ok(eps) = &r {
                // Select the clicked item, else the next one to watch.
                let id = app.item.id.clone();
                let next = eps.iter().position(|e| !app.progress.get(&e.id).is_some_and(|p| p.completed)).unwrap_or(0);
                let clicked = eps.iter().position(|e| e.id == id);
                let in_progress = app.progress.get(&id).is_some_and(|p| !p.completed && p.position_sec > 0.);
                app.item.sel = match clicked {
                    Some(pos) if in_progress => pos,
                    _ => next,
                };
            }
            app.item.episodes = Load::from(r);
            let id = app.item.id.clone();
            app.load_item_progress(&id, cx);
            app.load_sidecars(cx);
        });
        self.refresh_offline_entries();
    }

    pub fn load_item_progress(&mut self, _id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(eps) = self.item.episodes.ready() else { return };
        let ids: Vec<String> = eps.iter().map(|e| e.id.clone()).collect();
        self.spawn_ui(cx, crate::backend::progress_many(b.sync.clone(), b.client.clone(), ids), |app, list, _| {
            for (id, p) in list {
                app.progress.insert(id, p);
            }
        });
    }

    fn load_sidecars(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(sel) = self.item.selected().map(|e| e.id.clone()) else { return };
        if self.item.sidecars_for == sel {
            return;
        }
        self.item.sidecars_for = sel.clone();
        self.item.sidecars = Load::Loading;
        let c = b.client.clone();
        self.spawn_ui(cx, async move { c.sidecars(&sel).await }, |app, r, _| app.item.sidecars = Load::from(r));
    }

    pub fn refresh_offline_entries(&mut self) {
        let Some(b) = self.backend.clone() else { return };
        let Some(o) = &b.offline else { return };
        let server = b.client.server();
        self.item.offline = o
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.server == server)
            .map(|e| (e.item_id.clone(), e))
            .collect();
    }

    pub fn select_episode(&mut self, i: usize, cx: &mut Context<Self>) {
        self.item.sel = i;
        self.load_sidecars(cx);
    }

    pub fn item_nav_solid(&self) -> f32 {
        0.
    }

    pub fn item_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let n = self.item.episodes.ready().map(|e| e.len()).unwrap_or(0);
        let sel = self.item.selected().cloned();
        match ev.keystroke.key.as_str() {
            "down" | "j" if n > 0 => {
                let i = (self.item.sel + 1).min(n - 1);
                self.select_episode(i, cx);
                self.scroll.scroll_to_item(i);
            }
            "up" | "k" => {
                let i = self.item.sel.saturating_sub(1);
                self.select_episode(i, cx);
                self.scroll.scroll_to_item(i);
            }
            "enter" | "space" | "p" => {
                if let Some(e) = sel {
                    self.play_or_read(&e, cx);
                }
            }
            "w" => {
                if let Some(e) = sel {
                    let done = self.progress.get(&e.id).is_some_and(|p| p.completed);
                    self.run(Act::MarkWatched(e.id.clone(), !done), window, cx);
                }
            }
            "d" => {
                if let Some(e) = sel {
                    self.download(&e.id, cx);
                }
            }
            _ => return false,
        }
        true
    }

    pub fn play_or_read(&mut self, e: &CatalogItem, cx: &mut Context<Self>) {
        if crate::screens::library::is_reading_kind(&e.kind) {
            self.navigate(Screen::Reader(e.id.clone()), cx);
        } else {
            self.open_player(&e.id, None, cx);
        }
    }

    pub fn render_item(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let (vw, vh) = crate::ui::viewport(window);
        let sf = window.scale_factor();
        let margin = th.sp(6);
        let series = self.series_of.get(id).map(|&s| self.series[s].clone());
        let title = series
            .as_ref()
            .map(|s| s.title.clone())
            .or_else(|| self.item.selected().map(|e| e.title.clone()))
            .unwrap_or_default();
        let hero_h = (vh * 0.42).max(280.);
        let mut root = div().size_full().relative().overflow_hidden();

        // backdrop
        let key = ImgKey {
            src: Source::Thumb(id.to_string()),
            w: (vw * sf) as u32,
            h: (hero_h * sf) as u32,
            blur: ((th.blur.hero + 8.) * sf) as u32,
        };
        let art = self.images.update(cx, |s, cx| s.get(&key, cx));
        let mut hero = div().absolute().top_0().left_0().w(px(vw)).h(px(hero_h)).overflow_hidden();
        hero = match art {
            Some((a, _)) => {
                hero.child(img(ImageSource::Render(a)).size_full().object_fit(ObjectFit::Cover).opacity(0.7))
            }
            None => hero.child(self.placeholder("", vw, hero_h, true).opacity(0.8)),
        };
        hero = hero.child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(pal.bg.opacity(0.2), 0.),
            linear_color_stop(pal.bg, 1.),
        )));
        root = root.child(hero);

        let eps: Vec<CatalogItem> = match &self.item.episodes {
            Load::Ready(v) => v.clone(),
            Load::Failed(e) => {
                let e = e.clone();
                return root
                    .child(div().absolute().top(px(90.)).left(px(margin)).child(self.error_notice("This title", &e)));
            }
            _ => Vec::new(),
        };
        let reading = eps.first().map(|e| crate::screens::library::is_reading_kind(&e.kind)).unwrap_or(false);
        let sel = self.item.sel.min(eps.len().saturating_sub(1));
        let selected = eps.get(sel).cloned();
        let lib_name = series.as_ref().map(|s| self.library_name(&s.library_id)).unwrap_or_default();
        let watched = eps.iter().filter(|e| self.progress.get(&e.id).is_some_and(|p| p.completed)).count();
        let kind = eps.first().map(|e| e.kind.clone()).unwrap_or_default();
        let year = series.as_ref().map(|s| s.year).unwrap_or(0);

        // ---- header
        let title_size = th.ts(5);
        let mut actions = div().flex().items_center().gap(px(th.sp(2)));
        if let Some(e) = &selected {
            let p = self.progress.get(&e.id).cloned();
            let resume = p.as_ref().filter(|p| !p.completed && p.position_sec > 3.).map(|p| p.position_sec);
            let label = match (reading, resume) {
                (true, Some(pg)) => format!("Read {} · page {}", episode_label(e.season, e.episode), pg as i64),
                (true, None) => format!("Read {}", episode_label(e.season, e.episode)),
                (false, Some(t)) => format!("Resume {} · {}", episode_label(e.season, e.episode), fmt_time(t)),
                (false, None) => format!("Play {}", episode_label(e.season, e.episode)),
            };
            let act = if reading { Act::Nav(Screen::Reader(e.id.clone())) } else { Act::Play(e.id.clone()) };
            actions = actions.child(self.btn("item.play", label, Some("Enter"), Tone::Primary, act, cx));
            if !reading && resume.is_some() {
                actions = actions.child(self.btn(
                    "item.restart",
                    "From the start",
                    None,
                    Tone::Ghost,
                    Act::PlayAt(e.id.clone(), 0.),
                    cx,
                ));
            }
            let done = p.as_ref().is_some_and(|p| p.completed);
            actions = actions.child(self.btn(
                "item.watched",
                if done { "Mark unwatched" } else { "Mark watched" },
                Some("W"),
                Tone::Ghost,
                Act::MarkWatched(e.id.clone(), !done),
                cx,
            ));
            let off = self.item.offline.get(&e.id).cloned();
            let (dl_label, dl_act) = match off.as_ref().map(|o| o.state) {
                Some(OfflineState::Done) => ("Remove offline copy".to_string(), Act::RemoveDownload(e.id.clone())),
                Some(OfflineState::Queued) => {
                    (format!("Downloading {}%", pct(off.as_ref())), Act::PauseDownload(e.id.clone()))
                }
                Some(OfflineState::Paused) | Some(OfflineState::Failed) => {
                    ("Resume download".to_string(), Act::ResumeDownload(e.id.clone()))
                }
                None => ("Download".to_string(), Act::Download(e.id.clone())),
            };
            actions = actions.child(self.btn("item.download", dl_label, Some("D"), Tone::Ghost, dl_act, cx));
        }
        let facts = format!(
            "{} {}{} · {watched} of {} {}",
            eps.len(),
            if reading {
                "volumes"
            } else if eps.len() == 1 {
                "file"
            } else {
                "episodes"
            },
            if year > 0 { format!(" · {year}") } else { String::new() },
            eps.len(),
            if reading { "read" } else { "watched" }
        );
        let mut actions = Some(actions);
        let mut header = div().flex().flex_col().gap(px(th.sp(2)));
        for part in self.layout.header.clone() {
            use crate::layout::HeaderPart as H;
            header = match part {
                H::Kicker => header.child(
                    self.label(format!(
                        "{kind}{}",
                        if lib_name.is_empty() { String::new() } else { format!(" · {lib_name}") }
                    ))
                    .text_color(pal.accent),
                ),
                H::Title => header.child(
                    div()
                        .font_family(self.fonts.display.clone())
                        .text_size(px(title_size))
                        .line_height(px(title_size * 1.05))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.clone()),
                ),
                H::Facts => header.child(div().text_color(pal.text.opacity(0.75)).child(facts.clone())),
                H::Genres => match self.item.meta.as_ref().filter(|m| !m.genres.is_empty()) {
                    Some(m) => header.child(self.label(m.genres.join(" · "))),
                    None => header,
                },
                H::Synopsis => {
                    let syn = self.item.meta.as_ref().map(|m| m.synopsis.clone()).unwrap_or_default();
                    let mut synopsis: String = syn.chars().take(420).collect();
                    if syn.chars().count() > 420 {
                        synopsis.push('…');
                    }
                    if synopsis.is_empty() {
                        header
                    } else {
                        header.child(div().max_w(px(760.)).text_color(pal.text.opacity(0.8)).child(synopsis))
                    }
                }
                H::Actions => match actions.take() {
                    Some(a) => header.child(div().pt(px(th.sp(2))).child(a)),
                    None => header,
                },
            };
        }

        // ---- episodes list
        let mut list = div()
            .id("item.list")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pr(px(th.sp(2)));
        if self.item.episodes.is_loading() {
            list = list.child(div().text_color(pal.muted).child("Loading…"));
        }
        for (i, e) in eps.iter().enumerate() {
            let p = self.progress.get(&e.id).cloned();
            let on = i == sel;
            let frac = p
                .as_ref()
                .filter(|p| p.duration_sec > 0.)
                .map(|p| (p.position_sec / p.duration_sec).clamp(0., 1.) as f32)
                .unwrap_or(0.);
            let done = p.as_ref().is_some_and(|p| p.completed);
            let off = self.item.offline.get(&e.id).map(|o| o.state);
            let name = std::path::Path::new(&e.file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| e.title.clone());
            list = list.child(
                div()
                    .id(("ep", i))
                    .flex()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .px(px(th.sp(3)))
                    .py(px(th.sp(2)))
                    .rounded(px(th.radius.button))
                    .bg(if on { pal.text.opacity(0.08) } else { pal.text.opacity(0.) })
                    .border_1()
                    .border_color(if on && self.focused.is_none() { pal.focus } else { pal.text.opacity(0.) })
                    .cursor_pointer()
                    .hover(|s| s.bg(gpui::white().opacity(0.04)))
                    .child(
                        div()
                            .w(px(64.))
                            .font_family(self.fonts.mono.clone())
                            .text_size(px(th.ts(-1)))
                            .text_color(if on { pal.accent } else { pal.muted })
                            .child(episode_label(e.season, e.episode)),
                    )
                    .child(div().flex_1().flex().flex_col().gap(px(4.)).child(div().truncate().child(name)).child(
                        div().h(px(3.)).w_full().rounded_full().bg(pal.text.opacity(0.08)).child(
                            div().h_full().w(gpui::relative(if done { 1. } else { frac })).rounded_full().bg(if done {
                                pal.ok
                            } else {
                                pal.accent
                            }),
                        ),
                    ))
                    .when(off == Some(OfflineState::Done), |d| d.child(self.icon("download", 14., pal.ok)))
                    .when(done, |d| d.child(self.icon("check", 14., pal.ok)))
                    .child(
                        div()
                            .w(px(56.))
                            .text_size(px(th.ts(-2)))
                            .text_color(pal.muted)
                            .child(fmt_bytes(e.size.max(0) as u64)),
                    )
                    .on_click(cx.listener(move |app, ev: &ClickEvent, _, cx| {
                        app.focused = None;
                        app.select_episode(i, cx);
                        if ev.click_count() >= 2 {
                            let e = app.item.episodes.ready().and_then(|v| v.get(i)).cloned();
                            if let Some(e) = e {
                                app.play_or_read(&e, cx);
                            }
                        }
                        cx.notify();
                    })),
            );
        }

        // ---- details of the selected file
        let mut details = self.panel().w(px(380.)).flex_none().p(px(th.sp(4))).flex().flex_col().gap(px(th.sp(3)));
        if let Some(e) = &selected {
            details = details
                .child(self.label("file"))
                .child(div().text_size(px(th.ts(-1))).text_color(pal.text.opacity(0.85)).child(e.file_path.clone()))
                .child(self.row_kv("size", &fmt_bytes(e.size.max(0) as u64)))
                .child(self.row_kv("identified by", &format!("{} ({:.0}%)", e.origin, e.confidence * 100.)));
            if let Some(o) = self.item.offline.get(&e.id) {
                let state = match o.state {
                    OfflineState::Done => "on this device".to_string(),
                    OfflineState::Queued => format!("downloading {}%", pct(Some(o))),
                    OfflineState::Paused => "paused".into(),
                    OfflineState::Failed => format!("failed: {}", o.error),
                };
                details = details.child(self.row_kv("offline", &state));
            }
            details = details.child(self.label("subtitle files"));
            details = match &self.item.sidecars {
                Load::Ready(v) if v.is_empty() => {
                    details.child(div().text_color(pal.muted).child("None next to this file."))
                }
                Load::Ready(v) => {
                    let mut col = div().flex().flex_col().gap(px(4.));
                    for s in v {
                        let mut tags = vec![s.language.clone(), s.format.clone()];
                        if s.forced {
                            tags.push("forced".into());
                        }
                        if s.hi {
                            tags.push("SDH".into());
                        }
                        col = col.child(
                            div()
                                .flex()
                                .justify_between()
                                .gap(px(8.))
                                .child(div().truncate().child(s.name.clone()))
                                .child(self.pill(&tags.join(" "), pal.muted)),
                        );
                    }
                    details.child(col)
                }
                Load::Failed(err) => details
                    .child(div().text_color(pal.muted).text_size(px(th.ts(-1))).child(crate::backend::reason(err))),
                _ => details.child(div().text_color(pal.muted).child("…")),
            };
        }

        // ---- columns, in the layout's order
        let panels: Vec<gpui::Div> = if self.ext.panels.0 == id {
            self.ext.panels.1.iter().map(|(e, p)| self.ext_panel(e, p)).collect()
        } else {
            Vec::new()
        };
        let mut episodes = Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .gap(px(th.sp(2)))
                .child(self.label(if reading { "volumes" } else { "episodes" }))
                .child(list),
        );
        let mut details = Some(details);
        let mut panels = Some(panels);
        let mut columns = div().flex().flex_1().gap(px(th.sp(5))).min_h(px(0.));
        for c in self.layout.columns.clone() {
            use crate::layout::Column as C;
            match c {
                C::Episodes => columns = columns.children(episodes.take()),
                C::Details => columns = columns.children(details.take()),
                C::Panels => {
                    if let Some(p) = panels.take().filter(|p| !p.is_empty()) {
                        columns = columns.child(
                            div()
                                .id("item.panels")
                                .w(px(320.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(px(th.sp(3)))
                                .overflow_y_scroll()
                                .children(p),
                        );
                    }
                }
            }
        }

        let body = div()
            .absolute()
            .top(px(hero_h * 0.42))
            .left(px(margin))
            .right(px(margin))
            .bottom(px(th.sp(4)))
            .flex()
            .flex_col()
            .gap(px(th.sp(5)))
            .child(header)
            .child(columns)
            .child(
                div()
                    .flex()
                    .gap(px(th.sp(3)))
                    .text_size(px(th.ts(-2)))
                    .text_color(pal.muted)
                    .child(div().flex().gap(px(4.)).child(self.kbd("↑↓")).child("select"))
                    .child(div().flex().gap(px(4.)).child(self.kbd("Enter")).child(if reading {
                        "read"
                    } else {
                        "play"
                    }))
                    .child(div().flex().gap(px(4.)).child(self.kbd("W")).child("watched"))
                    .child(div().flex().gap(px(4.)).child(self.kbd("D")).child("download"))
                    .child(div().flex().gap(px(4.)).child(self.kbd("Esc")).child("back")),
            );
        if eps.is_empty() && !self.item.episodes.is_loading() {
            return root.child(div().absolute().top(px(90.)).left(px(margin)).child(self.notice(
                Notice::Empty,
                "No files",
                "This title has no files.",
            )));
        }
        root.child(body)
    }

    fn row_kv(&self, k: &str, v: &str) -> gpui::Div {
        div()
            .flex()
            .justify_between()
            .gap(px(8.))
            .child(div().text_color(self.pal.muted).child(k.to_string()))
            .child(div().text_size(px(self.theme.ts(-1))).child(v.to_string()))
    }
}

fn pct(o: Option<&OfflineEntry>) -> u64 {
    o.filter(|o| o.size > 0).map(|o| o.bytes * 100 / o.size).unwrap_or(0)
}
