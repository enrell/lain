//! Search and browse: server search as you type (debounced), with kind
//! chips; an empty query browses every title of the chosen kind. When
//! the server cannot search, titles are filtered locally and the screen
//! says so.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{ClickEvent, Context, FontWeight, Window, div, px};
use lain_client::SearchQuery;
use lain_client::types::CatalogItem;

use crate::app::{Act, LainApp, Load, Screen};
use crate::screens::home::episode_label;
use crate::ui::{Notice, Tone};

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub kind: String,
    pub results: Option<Load<Vec<CatalogItem>>>,
    pub total: i64,
    pub seq: u64,
    pub local_note: bool,
}

const KINDS: &[&str] = &["anime", "movie", "episode", "video", "manga", "comic"];

impl LainApp {
    pub fn search_typed(&mut self, cx: &mut Context<Self>) {
        self.search.seq += 1;
        let seq = self.search.seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            let _ = this.update(cx, |app, cx| {
                if app.search.seq == seq {
                    app.run_search(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn run_search(&mut self, cx: &mut Context<Self>) {
        let q = self.text("search.q").trim().to_string();
        if q.is_empty() {
            self.search.results = None;
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        let kind = self.search.kind.clone();
        self.search.results = Some(Load::Loading);
        let seq = self.search.seq;
        self.spawn_ui(
            cx,
            async move { c.search(SearchQuery { q, kind, limit: 100, ..Default::default() }).await },
            move |app, r, _| {
                if app.search.seq != seq {
                    return;
                }
                match r {
                    Ok(page) => {
                        app.search.total = page.total;
                        app.search.local_note = false;
                        app.search.results = Some(Load::Ready(page.items));
                    }
                    Err(lain_client::ClientError::NotPorted { .. }) => {
                        app.search.local_note = true;
                        app.search.results = Some(Load::Ready(Vec::new()));
                    }
                    Err(e) => app.search.results = Some(Load::Failed(e)),
                }
            },
        );
    }

    pub fn render_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let margin = th.sp(6);
        let q = self.text("search.q");
        let field = self.field("search.q", "Search titles, episodes, files…", false, Act::Focus("search.q"), cx);
        let mut chips = div().flex().items_center().gap(px(th.sp(1))).child(self.label("kind"));
        for k in KINDS {
            let tone = if self.search.kind == *k { Tone::Ghost } else { Tone::Quiet };
            chips =
                chips.child(self.btn(&format!("search.kind.{k}"), *k, None, tone, Act::SearchKind(k.to_string()), cx));
        }
        let mut root = div()
            .id("search")
            .size_full()
            .flex()
            .flex_col()
            .pt(px(84.))
            .px(px(margin))
            .gap(px(th.sp(3)))
            .child(
                div()
                    .font_family(self.fonts.display.clone())
                    .text_size(px(th.ts(4)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Search"),
            )
            .child(div().w(px(560.)).child(field))
            .child(chips);

        // Rows: (title, sub, target)
        let mut rows: Vec<(String, String, Screen)> = Vec::new();
        let mut note: Option<String> = None;
        match (&self.search.results, q.trim().is_empty()) {
            (_, true) => {
                // Browse: every title of the kind.
                for s in self.series.iter().filter(|s| self.search.kind.is_empty() || s.kind == self.search.kind) {
                    let first = &self.catalog[s.items[0]];
                    rows.push((
                        s.title.clone(),
                        format!("{} · {} · {}", s.kind, s.items.len(), self.library_name(&s.library_id)),
                        Screen::Item(first.id.clone()),
                    ));
                }
                note = Some(format!("Browsing {} titles", rows.len()));
            }
            (Some(Load::Ready(items)), false) if self.search.local_note => {
                let ql = q.to_lowercase();
                for s in self.series.iter().filter(|s| {
                    s.title.to_lowercase().contains(&ql) && (self.search.kind.is_empty() || s.kind == self.search.kind)
                }) {
                    let first = &self.catalog[s.items[0]];
                    rows.push((
                        s.title.clone(),
                        format!("{} · {}", s.kind, s.items.len()),
                        Screen::Item(first.id.clone()),
                    ));
                }
                let _ = items;
                note = Some("Server search is not available on this server yet: titles are filtered here.".into());
            }
            (Some(Load::Ready(items)), false) => {
                for it in items {
                    rows.push((
                        it.title.clone(),
                        format!("{} · {}", it.kind, episode_label(it.season, it.episode)),
                        Screen::Item(it.id.clone()),
                    ));
                }
                note = Some(format!("{} results", self.search.total));
            }
            (Some(Load::Failed(e)), false) => {
                let e = e.clone();
                return root.child(self.error_notice("Search", &e));
            }
            (Some(_), false) => note = Some("Searching…".into()),
            (None, false) => {}
        }
        if let Some(n) = note {
            root = root.child(self.label(n));
        }
        if rows.is_empty() && !q.trim().is_empty() && !matches!(self.search.results, Some(Load::Loading)) {
            return root.child(self.notice(Notice::Empty, "Nothing found", "Try fewer words, or another kind."));
        }
        let mut list =
            div().id("search.list").flex().flex_col().flex_1().overflow_y_scroll().track_scroll(&self.scroll);
        for (i, (title, sub, target)) in rows.into_iter().take(300).enumerate() {
            self.cur_row = i;
            let id = format!("search.r.{i}");
            let focused = self.stop(&id, Some(Act::Nav(target.clone())), false);
            self.row_of.insert(id.clone().into(), i);
            list = list.child(
                div()
                    .id(("sr", i))
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(px(th.sp(3)))
                    .py(px(th.sp(2)))
                    .rounded(px(th.radius.button))
                    .border_1()
                    .border_color(if focused { pal.focus } else { pal.text.opacity(0.) })
                    .cursor_pointer()
                    .hover(|s| s.bg(gpui::white().opacity(0.05)))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().text_color(pal.muted).text_size(px(th.ts(-1))).child(sub))
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.navigate(target.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        root.child(list)
    }
}
