//! Settings → Account → Linked accounts (`/api/me/links`, the lists
//! module): link AniList with the zero-setup pin flow (D-083), toggle
//! scrobbling, sync the list now, unlink. Tokens never reach the app.

use gpui::prelude::*;
use gpui::{Context, div, px};
use lain_client::types::Link;

use crate::app::{Act, LainApp, Load};
use crate::ui::Tone;

const PLATFORMS: &[(&str, &str)] = &[("anilist", "AniList")];

impl LainApp {
    pub fn load_links(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.links = Load::Loading;
        self.spawn_ui(cx, async move { c.links().await }, |app, r, _| app.links = Load::from(r));
    }

    pub fn link_op(&mut self, platform: String, op: &'static str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        match op {
            "open" => {
                // The pin page opens in the browser; the code comes back here.
                self.spawn_ui(cx, async move { c.link_pin_url(&platform).await }, |app, r, cx| match r {
                    Ok(url) => {
                        cx.open_url(&url);
                        app.toast("Approve lain in the browser, then paste the code here", false);
                        app.focused = Some("link.code".into());
                    }
                    Err(e) => app.toast(crate::backend::reason(&e), true),
                });
            }
            "code" => {
                let code = self.text("link.code").trim().to_string();
                if code.is_empty() {
                    self.toast("Paste the code the pin page showed", true);
                    return;
                }
                self.spawn_ui(cx, async move { c.link_with_code(&platform, &code).await }, |app, r, cx| {
                    if app.report(&r, "Account linked") {
                        app.set_text("link.code", "");
                        app.load_links(cx);
                    }
                });
            }
            "sync" => self.spawn_ui(cx, async move { c.sync_link(&platform).await }, |app, r, cx| {
                app.report(&r, "List synced");
                app.load_links(cx);
            }),
            "unlink" => self.spawn_ui(cx, async move { c.unlink(&platform).await }, |app, r, cx| {
                app.report(&r, "Unlinked");
                app.load_links(cx);
            }),
            "scrobble-on" | "scrobble-off" => {
                let on = op == "scrobble-on";
                self.spawn_ui(cx, async move { c.set_link_scrobble(&platform, on).await }, |app, r, cx| {
                    app.report(&r, "");
                    app.load_links(cx);
                });
            }
            _ => {}
        }
    }

    /// The linked-accounts rows of Settings → Account.
    pub fn links_rows(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let mut col = div().flex().flex_col().child(div().pt(px(th.sp(5))).child(self.label("linked accounts")));
        let links: Vec<Link> = match self.links.clone() {
            Load::Ready(l) => l,
            Load::Failed(e) => {
                return col.child(div().py(px(th.sp(2))).text_color(pal.muted).child(crate::backend::reason(&e)));
            }
            _ => return col,
        };
        for &(id, name) in PLATFORMS {
            let base = format!("link.{id}");
            match links.iter().find(|l| l.platform == id) {
                Some(l) => {
                    let scrobble = self.btn(
                        &format!("{base}.scrobble"),
                        if l.scrobble { "Scrobbling on" } else { "Scrobbling off" },
                        None,
                        Tone::Ghost,
                        Act::LinkOp(id.into(), if l.scrobble { "scrobble-off" } else { "scrobble-on" }),
                        cx,
                    );
                    let sync = self.btn(
                        &format!("{base}.sync"),
                        "Sync now",
                        None,
                        Tone::Quiet,
                        Act::LinkOp(id.into(), "sync"),
                        cx,
                    );
                    let unlink = self.btn(
                        &format!("{base}.unlink"),
                        "Unlink",
                        None,
                        Tone::Danger,
                        Act::LinkOp(id.into(), "unlink"),
                        cx,
                    );
                    let mut hint = format!("{} · {} list entries", l.remote_username, l.entry_count);
                    if l.token_expired {
                        hint.push_str(" · the token expired: link again");
                    } else if !l.last_sync_error.is_empty() {
                        hint.push_str(&format!(" · last sync failed: {}", l.last_sync_error));
                    }
                    col = col.child(self.row(
                        name,
                        &hint,
                        div().flex().gap(px(th.sp(1))).child(scrobble).child(sync).child(unlink),
                    ));
                }
                None => {
                    let open = self.btn(
                        &format!("{base}.open"),
                        "Get a code…",
                        None,
                        Tone::Ghost,
                        Act::LinkOp(id.into(), "open"),
                        cx,
                    );
                    let field = self.field("link.code", "paste the code", false, Act::LinkOp(id.into(), "code"), cx);
                    let link = self.btn(
                        &format!("{base}.link"),
                        "Link",
                        None,
                        Tone::Primary,
                        Act::LinkOp(id.into(), "code"),
                        cx,
                    );
                    col = col.child(self.row(
                        name,
                        "Not linked: open the pin page, approve, paste the code",
                        div().flex().gap(px(th.sp(1))).child(open).child(div().w(px(200.)).child(field)).child(link),
                    ));
                }
            }
        }
        col
    }
}
