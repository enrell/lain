//! Extensions in the app (P6, D-258): the session glue (start workers
//! against the logged-in server, forward events, apply their output),
//! the permission prompt and Settings → Extensions.
//!
//! The prompt appears the first time an extension is found (installed
//! from Settings, or dropped into the extensions folder) and again when an
//! update asks for scopes the user has not seen. Enter grants every
//! scope it lists and enables the extension; Esc leaves it off with
//! nothing granted. Each scope can be granted or revoked later on its
//! own; a change restarts the extension with the new grants.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{Context, FontWeight, PathPromptOptions, div, px};

use crate::app::{Act, LainApp};
use crate::ext::{self, Cmd, Notice, Scope, Status};
use crate::ui::Tone;

impl LainApp {
    /// Points the extension host at the current session and starts what
    /// the settings allow.
    pub fn ext_connect(&mut self, cx: &mut Context<Self>) {
        let api: Option<Arc<dyn ext::HostApi>> = self.backend.as_ref().and_then(|b| {
            match ext::api::ClientApi::new(b.client.clone(), b.net.rt.handle().clone()) {
                Ok(a) => Some(Arc::new(a) as Arc<dyn ext::HostApi>),
                Err(e) => {
                    eprintln!("extensions: {e}");
                    None
                }
            }
        });
        self.ext.set_api(api);
        self.ext_sync(cx);
    }

    /// Applies the settings to the workers and queues a prompt if one is due.
    pub fn ext_sync(&mut self, _cx: &mut Context<Self>) {
        self.ext.rescan();
        for n in self.ext.sync(&self.config.extensions) {
            self.ext_notice(n);
        }
        if self.ext_prompt.is_none() && self.me.is_some() {
            self.ext_prompt = self.ext.pending_review(&self.config.extensions);
        }
        self.home.dirty = true;
    }

    pub fn on_ext(&mut self, out: ext::Out, cx: &mut Context<Self>) {
        if let Some(n) = self.ext.on_out(out, &mut self.config.extensions) {
            if matches!(n, Notice::Stopped { .. }) {
                self.config.save(&self.dir);
            }
            self.ext_notice(n);
            cx.notify();
        }
    }

    fn ext_notice(&mut self, n: Notice) {
        match n {
            Notice::Changed => self.home.dirty = true,
            Notice::Stopped { name, reason } => {
                self.home.dirty = true;
                self.toast(format!("Extension {name} stopped and was turned off: {reason}"), true);
            }
        }
    }

    pub fn ext_event(&self, e: ext::Event) {
        self.ext.event(e);
    }

    pub fn run_ext(&mut self, act: Act, cx: &mut Context<Self>) {
        let cfg = &mut self.config.extensions;
        match act {
            Act::ExtAllow | Act::ExtDeny => {
                let Some(id) = self.ext_prompt.take() else { return };
                self.focused = None;
                let requested: Vec<String> = self
                    .ext
                    .find(&id)
                    .and_then(|i| i.manifest.as_ref().ok())
                    .and_then(|m| m.requested().ok())
                    .unwrap_or_default()
                    .iter()
                    .map(Scope::id)
                    .collect();
                let s = cfg.entry(id.clone()).or_default();
                for r in &requested {
                    if !s.reviewed.contains(r) {
                        s.reviewed.push(r.clone());
                    }
                }
                if matches!(act, Act::ExtAllow) {
                    s.granted = requested.clone();
                    s.enabled = true;
                    s.error.clear();
                    let name = self.ext.name(&id);
                    self.toast(format!("{name} is on"), false);
                } else {
                    s.enabled = false;
                    s.granted.retain(|g| requested.contains(g));
                }
            }
            Act::ExtReview(id) => {
                self.ext_prompt = Some(id);
                return;
            }
            Act::ExtToggle(id) => {
                let unreviewed = !self.ext.unreviewed(&id, cfg).is_empty();
                let s = cfg.entry(id.clone()).or_default();
                if !s.enabled && unreviewed {
                    // Turning on something never reviewed asks first.
                    self.ext_prompt = Some(id);
                    return;
                }
                s.enabled = !s.enabled;
                s.error.clear();
            }
            Act::ExtScope(id, scope) => {
                let s = cfg.entry(id).or_default();
                if let Some(i) = s.granted.iter().position(|g| *g == scope) {
                    s.granted.remove(i);
                } else {
                    s.granted.push(scope.clone());
                }
                if !s.reviewed.contains(&scope) {
                    s.reviewed.push(scope);
                }
            }
            Act::ExtAskUninstall(id) => {
                let name = self.ext.name(&id);
                self.ask(format!("Uninstall {name}? Its data folder is removed too."), Act::ExtUninstall(id));
                return;
            }
            Act::ExtUninstall(id) => match ext::manifest::uninstall(&self.ext.root, &id) {
                Ok(()) => {
                    let name = self.ext.name(&id);
                    self.config.extensions.remove(&id);
                    self.toast(format!("{name} uninstalled"), false);
                }
                Err(e) => self.toast(format!("Uninstall: {e}"), true),
            },
            Act::InstallExtension => {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: true,
                    multiple: false,
                    prompt: Some("Extension folder or zip".into()),
                });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(paths))) = rx.await else { return };
                    let Some(p) = paths.into_iter().next() else { return };
                    let _ = this.update(cx, |app, cx| {
                        app.install_extension(&p, cx);
                        cx.notify();
                    });
                })
                .detach();
                return;
            }
            Act::OpenExtensionsFolder => {
                let _ = std::fs::create_dir_all(&self.ext.root);
                crate::platform::open_folder(&self.ext.root);
                return;
            }
            _ => return,
        }
        self.config.save(&self.dir);
        self.ext_sync(cx);
    }

    /// Installs from a folder or zip, then asks for its permissions.
    pub fn install_extension(&mut self, p: &std::path::Path, cx: &mut Context<Self>) {
        match ext::manifest::install(p, &self.ext.root) {
            Ok(m) => {
                self.toast(format!("{} {} installed", m.name, m.version), false);
                // A reinstall restarts it with the new code.
                self.ext.stop_one(&m.id);
                self.ext_sync(cx);
                if !self.ext.unreviewed(&m.id, &self.config.extensions).is_empty() {
                    self.ext_prompt = Some(m.id);
                }
            }
            Err(e) => self.toast(format!("Extension: {e}"), true),
        }
    }

    /// Asks every rail extension for fresh rails (F5).
    pub fn ext_refresh(&self) {
        self.ext.broadcast(&Scope::UiRail, Cmd::Rails);
    }

    // ---- the permission prompt -------------------------------------------------

    pub fn render_ext_prompt(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let id = self.ext_prompt.clone()?;
        let inst = self.ext.find(&id)?.clone();
        let m = inst.manifest.as_ref().ok()?.clone();
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let unreviewed = self.ext.unreviewed(&id, &self.config.extensions);
        // The prompt traps focus and lands on its primary action.
        if !matches!(self.focused.as_ref().map(|f| f.as_ref()), Some("ext.allow" | "ext.deny")) {
            self.focused = Some("ext.allow".into());
        }
        let scopes = m.requested().unwrap_or_default();
        let allow = self.btn("ext.allow", "Allow and turn on", Some("Enter"), Tone::Primary, Act::ExtAllow, cx);
        let deny = self.btn("ext.deny", "Not now", Some("Esc"), Tone::Ghost, Act::ExtDeny, cx);
        let mut list = div().flex().flex_col().gap(px(th.sp(2)));
        for s in &scopes {
            let new = unreviewed.contains(s);
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(th.sp(3)))
                    .child(div().pt(px(6.)).size(px(7.)).rounded_full().bg(if s.sensitive() {
                        pal.warn
                    } else {
                        pal.accent
                    }))
                    .child(
                        div().flex().flex_col().child(div().child(s.describe())).child(
                            div()
                                .flex()
                                .gap(px(th.sp(2)))
                                .child(self.label(s.id()))
                                .when(new && scopes.len() != unreviewed.len(), |d| {
                                    d.child(self.label("new").text_color(pal.warn))
                                }),
                        ),
                    ),
            );
        }
        if scopes.is_empty() {
            list = list.child(div().text_color(pal.muted).child("It asks for nothing: it can only compute."));
        }
        let by = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(pal.bg.opacity(0.6))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    self.panel()
                        .w(px(560.))
                        .p(px(th.sp(5)))
                        .flex()
                        .flex_col()
                        .gap(px(th.sp(4)))
                        .child(self.label("extension permissions"))
                        .child(
                            div()
                                .font_family(self.fonts.display.clone())
                                .text_size(px(th.ts(3)))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{} {}", m.name, m.version)),
                        )
                        .child(div().text_color(pal.muted).child(format!("{}{by}", m.description)))
                        .child(div().child(if scopes.is_empty() {
                            "It may:".to_string()
                        } else {
                            "It asks to:".to_string()
                        }))
                        .child(list)
                        .child(div().text_size(px(th.ts(-1))).text_color(pal.muted).child(
                            "It runs sandboxed: no files, network or other extensions beyond what you allow. \
                             You can revoke any of these in Settings → Extensions.",
                        ))
                        .child(div().flex().gap(px(th.sp(2))).child(allow).child(deny)),
                )
                .into_any_element(),
        )
    }

    // ---- Settings → Extensions -------------------------------------------------------

    pub fn set_extensions(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let install = self.btn("ext.install", "Install folder or zip…", None, Tone::Ghost, Act::InstallExtension, cx);
        let open = self.btn("ext.open", "Open extensions folder", None, Tone::Quiet, Act::OpenExtensionsFolder, cx);
        let mut col = div().flex().flex_col().child(self.row(
            "Extensions",
            "WebAssembly components with the scopes you grant (lain.extension@1)",
            div().flex().gap(px(th.sp(1))).child(install).child(open),
        ));
        if let Some(e) = &self.ext.runtime_error {
            col = col.child(
                div().py(px(th.sp(2))).text_color(pal.danger).child(format!("The extension runtime failed: {e}")),
            );
        }
        let installed = self.ext.installed.clone();
        if installed.is_empty() {
            col = col.child(
                div()
                    .py(px(th.sp(3)))
                    .text_color(pal.muted)
                    .child(format!("None installed. Install one, or put its folder in {}.", self.ext.root.display())),
            );
        }
        for inst in installed {
            let id = inst.id.clone();
            let status = self.ext.status.get(&id).cloned().unwrap_or(Status::Off);
            let s = self.config.extensions.get(&id).cloned().unwrap_or_default();
            let tone_color = match status {
                Status::Running => pal.ok,
                Status::Failed(_) | Status::Broken(_) => pal.danger,
                Status::NeedsReview | Status::Starting => pal.warn,
                Status::Off => pal.muted,
            };
            let mut card = self.panel().p(px(th.sp(4))).mt(px(th.sp(3))).flex().flex_col().gap(px(th.sp(2)));
            let (title, sub) = match &inst.manifest {
                Ok(m) => (
                    format!("{} {}", m.name, m.version),
                    format!(
                        "{}{}",
                        m.description,
                        if m.author.is_empty() { String::new() } else { format!(" · {}", m.author) }
                    ),
                ),
                Err(_) => (id.clone(), inst.dir.display().to_string()),
            };
            let toggle = match &status {
                Status::Broken(_) => None,
                Status::NeedsReview => Some(self.btn(
                    &format!("ext.{id}.review"),
                    "Review permissions",
                    None,
                    Tone::Primary,
                    Act::ExtReview(id.clone()),
                    cx,
                )),
                _ => Some(self.btn(
                    &format!("ext.{id}.toggle"),
                    if s.enabled { "Turn off" } else { "Turn on" },
                    None,
                    if s.enabled { Tone::Ghost } else { Tone::Primary },
                    Act::ExtToggle(id.clone()),
                    cx,
                )),
            };
            let remove = self.btn(
                &format!("ext.{id}.remove"),
                "Uninstall",
                None,
                Tone::Danger,
                Act::ExtAskUninstall(id.clone()),
                cx,
            );
            card = card.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(div().text_size(px(th.ts(-1))).text_color(pal.muted).child(sub)),
                    )
                    .child(div().flex().gap(px(th.sp(1))).flex_none().children(toggle).child(remove)),
            );
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(2)))
                    .child(div().size(px(7.)).rounded_full().bg(tone_color))
                    .child(div().text_size(px(th.ts(-1))).child(status.label())),
            );
            if let Ok(m) = &inst.manifest {
                card = card.child(self.label("scopes"));
                let scopes = m.requested().unwrap_or_default();
                if scopes.is_empty() {
                    card = card.child(div().text_color(pal.muted).text_size(px(th.ts(-1))).child("none"));
                }
                for sc in scopes {
                    let sid = sc.id();
                    let granted = s.granted.contains(&sid);
                    let b = self.btn(
                        &format!("ext.{id}.scope.{sid}"),
                        if granted { "Granted" } else { "Not granted" },
                        None,
                        if granted { Tone::Ghost } else { Tone::Quiet },
                        Act::ExtScope(id.clone(), sid.clone()),
                        cx,
                    );
                    card = card.child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .gap(px(th.sp(3)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_size(px(th.ts(-1))).child(sc.describe()))
                                    .child(self.label(sid)),
                            )
                            .child(b),
                    );
                }
                let rails: usize = self.ext.rails.get(&id).map(|r| r.len()).unwrap_or(0);
                if rails > 0 {
                    card = card.child(div().text_size(px(th.ts(-1))).text_color(pal.muted).child(format!(
                        "{rails} rail{} on Home (layout slot ext:{id})",
                        if rails == 1 { "" } else { "s" }
                    )));
                }
            }
            if let Some(log) = self.ext.logs.get(&id).filter(|l| !l.is_empty()) {
                let mut l = div().flex().flex_col().pt(px(th.sp(1))).child(self.label("log"));
                for line in log.iter().rev().take(5) {
                    l = l.child(
                        div()
                            .font_family(self.fonts.mono.clone())
                            .text_size(px(th.ts(-2)))
                            .text_color(pal.muted)
                            .truncate()
                            .child(line.clone()),
                    );
                }
                card = card.child(l);
            }
            col = col.child(card);
        }
        col
    }

    /// One extension panel on a title page.
    pub fn ext_panel(&self, ext: &str, p: &ext::Panel) -> gpui::Div {
        let th = &self.theme;
        let pal = &self.pal;
        let mut d = self.panel().p(px(th.sp(4))).flex().flex_col().gap(px(th.sp(2))).child(
            div()
                .flex()
                .justify_between()
                .child(self.label(p.title.clone()).text_color(pal.accent))
                .child(self.label(self.ext.name(ext))),
        );
        for r in &p.rows {
            d = d.child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .child(div().text_color(pal.muted).child(r.label.clone()))
                    .child(div().text_size(px(th.ts(-1))).child(r.value.clone())),
            );
        }
        if !p.text.is_empty() {
            d = d.child(div().text_size(px(th.ts(-1))).text_color(pal.text.opacity(0.8)).child(p.text.clone()));
        }
        d
    }
}
