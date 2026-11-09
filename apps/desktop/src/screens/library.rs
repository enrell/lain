//! Library: one library at a time (tabs), its titles as a poster grid
//! with a filter, three sorts and pages sized to the window. A private
//! library this session has not unlocked shows its unlock form (D-156).

use gpui::prelude::*;
use gpui::{ClickEvent, Context, FontWeight, ImageSource, ObjectFit, SharedString, Window, div, img, px};
use lain_client::types::Library;

use crate::app::{Act, LainApp, LibSort, Load, Screen, Section};
use crate::images::{ImgKey, Source};
use crate::ui::{Notice, Tone};

#[derive(Debug, Clone)]
pub struct LibState {
    pub sort: LibSort,
    pub page: usize,
    /// Focused card within the page (spatial keys), when no widget has focus.
    pub sel: usize,
    /// Columns and rows of the last layout (for the arrow keys).
    pub cols: usize,
    pub per_page: usize,
    pub pages: usize,
    pub shown: Vec<usize>,
}

impl Default for LibState {
    fn default() -> Self {
        LibState { sort: LibSort::Title, page: 0, sel: 0, cols: 1, per_page: 1, pages: 1, shown: Vec::new() }
    }
}

pub fn is_reading_kind(kind: &str) -> bool {
    matches!(kind, "manga" | "comic" | "book")
}

impl LainApp {
    /// The libraries of one group (watch or read), honoring modules.
    pub fn group_libraries(&self, read: bool) -> Vec<Library> {
        let Some(libs) = self.libraries.ready() else { return Vec::new() };
        let on = if read { self.module_on("read") } else { self.module_on("watch") };
        if !on {
            return Vec::new();
        }
        libs.iter().filter(|l| is_reading_kind(&l.kind) == read).cloned().collect()
    }

    pub fn lib_page(&mut self, d: i32) {
        let p = self.lib.page as i32 + d;
        self.lib.page = p.clamp(0, self.lib.pages.saturating_sub(1) as i32) as usize;
        self.lib.sel = 0;
    }

    pub fn library_key(&mut self, ev: &gpui::KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) -> bool {
        let n = self.lib.shown.len();
        let cols = self.lib.cols.max(1);
        let s = self.lib.sel;
        match ev.keystroke.key.as_str() {
            "right" if n > 0 => self.lib.sel = (s + 1).min(n - 1),
            "left" => self.lib.sel = s.saturating_sub(1),
            "down" if n > 0 => {
                if s + cols < n {
                    self.lib.sel = s + cols
                } else if self.lib.page + 1 < self.lib.pages {
                    self.lib_page(1);
                }
            }
            "up" => {
                if s >= cols {
                    self.lib.sel = s - cols
                } else if self.lib.page > 0 {
                    self.lib_page(-1);
                } else {
                    self.move_focus(-1);
                }
            }
            "pagedown" => self.lib_page(1),
            "pageup" => self.lib_page(-1),
            "home" => self.lib.sel = 0,
            "end" => self.lib.sel = n.saturating_sub(1),
            "f" => self.focused = Some("library.filter".into()),
            "enter" | "space" => {
                if let Some(&si) = self.lib.shown.get(s)
                    && let Some(&first) = self.series.get(si).and_then(|x| x.items.first())
                {
                    let id = self.catalog[first].id.clone();
                    self.navigate(Screen::Item(id), cx);
                }
            }
            _ => return false,
        }
        true
    }

    pub fn render_library(
        &mut self,
        read: bool,
        lib_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let (vw, vh) = crate::ui::viewport(window);
        let sf = window.scale_factor();
        let margin = th.sp(6);
        let mut root =
            div().size_full().flex().flex_col().pt(px(76.)).px(px(margin)).gap(px(th.sp(3))).overflow_hidden();

        let module = if read { "read" } else { "watch" };
        if !self.module_on(module) {
            return root.child(self.notice(
                Notice::Disabled,
                &format!("The {module} module is off"),
                "An admin turned it off on this server (Settings → Modules).",
            ));
        }
        match &self.libraries {
            Load::Failed(e) => return root.child(self.error_notice("Libraries", &e.clone())),
            Load::Ready(_) => {}
            _ => return root.child(self.notice(Notice::Loading, "Loading libraries…", "")),
        }
        let libs = self.group_libraries(read);
        if libs.is_empty() {
            let mut n = div().child(self.notice(
                Notice::Empty,
                if read { "No reading libraries" } else { "No libraries" },
                if read {
                    "Add a manga or comic library to read here."
                } else {
                    "Add a folder of anime, movies or videos to watch here."
                },
            ));
            if self.is_admin() {
                n = n.child(div().pl(px(th.sp(5))).child(self.btn(
                    "lib.add",
                    "Add a library",
                    None,
                    Tone::Primary,
                    Act::Nav(Screen::Settings(Section::Libraries)),
                    cx,
                )));
            }
            return root.child(n);
        }
        let lib = libs.iter().find(|l| l.id == lib_id).cloned().unwrap_or_else(|| libs[0].clone());

        // ---- header: tabs, filter, sort
        let mut tabs = div().flex().items_center().gap(px(th.sp(1)));
        for l in &libs {
            let on = l.id == lib.id;
            let id = format!("lib.tab.{}", l.id);
            let focused = self.stop(&id, Some(Act::Nav(Screen::Library { read, lib: l.id.clone() })), false);
            let target = Screen::Library { read, lib: l.id.clone() };
            tabs = tabs.child(
                div()
                    .id(SharedString::from(id))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(th.sp(3)))
                    .py(px(th.sp(1)))
                    .rounded(px(th.radius.button))
                    .border_1()
                    .border_color(if focused { pal.focus } else { pal.text.opacity(0.) })
                    .bg(if on { pal.text.opacity(0.1) } else { pal.text.opacity(0.) })
                    .cursor_pointer()
                    .child(
                        div()
                            .font_family(self.fonts.display.clone())
                            .text_size(px(th.ts(2)))
                            .font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                            .text_color(if on { pal.text } else { pal.muted })
                            .child(l.name.clone()),
                    )
                    .when(l.private, |d| {
                        d.child(self.icon(if l.locked == Some(true) { "lock" } else { "unlock" }, 14., pal.muted))
                    })
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.screen = target.clone();
                        app.lib.page = 0;
                        app.lib.sel = 0;
                        cx.notify();
                    })),
            );
        }
        root = root.child(tabs);

        if lib.locked == Some(true) {
            let field = self.field("unlock.secret", "PIN or password", true, Act::Unlock(lib.id.clone()), cx);
            let go = self.btn("unlock.go", "Unlock", Some("Enter"), Tone::Primary, Act::Unlock(lib.id.clone()), cx);
            return root.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(th.sp(3)))
                    .max_w(px(460.))
                    .child(self.notice(
                        Notice::Disabled,
                        "This library is private",
                        "Unlock it for this session with your library PIN or your password.",
                    ))
                    .child(
                        div().px(px(th.sp(5))).flex().gap(px(th.sp(2))).child(div().w(px(260.)).child(field)).child(go),
                    ),
            );
        }

        let filter = self.field("library.filter", "Filter titles  (F)", false, Act::Focus("library.filter"), cx);
        let sorts = [(LibSort::Title, "Title"), (LibSort::Recent, "Recent"), (LibSort::Year, "Year")];
        let mut sort_row = div().flex().items_center().gap(px(th.sp(1))).child(self.label("sort"));
        for (s, label) in sorts {
            let tone = if self.lib.sort == s { Tone::Ghost } else { Tone::Quiet };
            sort_row = sort_row.child(self.btn(&format!("lib.sort.{label}"), label, None, tone, Act::LibSort(s), cx));
        }

        // ---- data
        let q = self.text("library.filter").to_lowercase();
        let mut shown: Vec<usize> = self
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| s.library_id == lib.id && (q.is_empty() || s.title.to_lowercase().contains(&q)))
            .map(|(i, _)| i)
            .collect();
        match self.lib.sort {
            LibSort::Title => {}
            LibSort::Recent => shown.sort_by(|&a, &b| self.series[b].latest.cmp(&self.series[a].latest)),
            LibSort::Year => shown.sort_by(|&a, &b| self.series[b].year.cmp(&self.series[a].year)),
        }
        let total = shown.len();

        // ---- grid geometry: pages sized to the window
        let (cw, ch) = (th.rail.card_w * 0.85, th.rail.card_h * 0.85);
        let gap = th.rail.gap;
        let label_h = th.ts(0) * 3.;
        let cols = (((vw - 2. * margin + gap) / (cw + gap)).floor() as usize).max(1);
        let grid_top = 76. + th.ts(2) * 2. + th.sp(3) * 3. + th.ts(0) * 2.5;
        let rows = (((vh - grid_top - 60.) / (ch + label_h + gap)).floor() as usize).max(1);
        let per_page = cols * rows;
        let pages = total.div_ceil(per_page).max(1);
        self.lib.cols = cols;
        self.lib.per_page = per_page;
        self.lib.pages = pages;
        if self.lib.page >= pages {
            self.lib.page = pages - 1;
        }
        let start = self.lib.page * per_page;
        let page_items: Vec<usize> = shown.iter().skip(start).take(per_page).copied().collect();
        self.lib.sel = self.lib.sel.min(page_items.len().saturating_sub(1));
        self.lib.shown = page_items.clone();

        let prev = self.btn("lib.prev", "Prev", Some("PgUp"), Tone::Quiet, Act::LibPage(-1), cx);
        let next = self.btn("lib.next", "Next", Some("PgDn"), Tone::Quiet, Act::LibPage(1), cx);
        root = root.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(th.sp(3)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(th.sp(3)))
                        .child(div().w(px(300.)).child(filter))
                        .child(sort_row),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(th.sp(2)))
                        .child(self.label(format!("{total} titles · page {}/{pages}", self.lib.page + 1)))
                        .child(prev)
                        .child(next),
                ),
        );

        if matches!(self.items, Load::Loading) && total == 0 {
            return root.child(self.notice(Notice::Loading, "Loading titles…", ""));
        }
        if let Load::Failed(e) = &self.items {
            return root.child(self.error_notice("The catalog", &e.clone()));
        }
        if total == 0 {
            let body = if q.is_empty() {
                "Scan the library to fill it (Settings → Libraries)."
            } else {
                "No title matches the filter."
            };
            return root.child(self.notice(Notice::Empty, "No titles", body));
        }

        let mut grid = div().flex().flex_wrap().gap(px(gap)).w(px(cols as f32 * (cw + gap)));
        for (k, &si) in page_items.iter().enumerate() {
            let ser = self.series[si].clone();
            let sel = self.focused.is_none() && k == self.lib.sel;
            let first_id = self.catalog[ser.items[0]].id.clone();
            let key =
                ImgKey { src: Source::Thumb(first_id.clone()), w: (cw * sf) as u32, h: (ch * sf) as u32, blur: 0 };
            let art = self.images.update(cx, |s, cx| s.get(&key, cx));
            let mut card = div()
                .w(px(cw))
                .h(px(ch))
                .rounded(px(th.radius.card))
                .overflow_hidden()
                .bg(pal.surface)
                .border_2()
                .border_color(if sel { pal.focus } else { pal.text.opacity(0.) });
            card = match art {
                Some((a, _)) => card.child(img(ImageSource::Render(a)).size_full().object_fit(ObjectFit::Cover)),
                None => card.child(self.placeholder(&ser.title, cw, ch, false)),
            };
            let watched = ser
                .items
                .iter()
                .filter(|&&i| self.progress.get(&self.catalog[i].id).is_some_and(|p| p.completed))
                .count();
            let unit = if ser.reading() { "vol" } else { "ep" };
            let target = Screen::Item(first_id);
            grid = grid.child(
                div()
                    .id(("card", si))
                    .w(px(cw))
                    .flex()
                    .flex_col()
                    .gap(px(th.sp(1)))
                    .cursor_pointer()
                    .child(card)
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(ser.title.clone()))
                    .child(div().text_size(px(th.ts(-1))).text_color(pal.muted).child(format!(
                        "{} {unit}{}{}",
                        ser.items.len(),
                        if ser.year > 0 { format!(" · {}", ser.year) } else { String::new() },
                        if watched > 0 { format!(" · {watched} seen") } else { String::new() }
                    )))
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.navigate(target.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        root.child(grid)
    }
}
