//! First run: the server URL (manual discovery), setup of the first
//! admin on a fresh server, and login. The token is kept by
//! `FileTokenStore` in the client dir, so the next start skips this.

use gpui::prelude::*;
use gpui::{Context, FontWeight, ImageSource, ObjectFit, Window, div, img, linear_color_stop, linear_gradient, px};
use lain_client::ClientError;

use crate::app::{Act, LainApp};
use crate::images::{ImgKey, Source};
use crate::ui::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Step {
    #[default]
    Server,
    Setup,
    Login,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectState {
    pub step: Step,
    pub busy: bool,
    pub error: Option<String>,
    pub server_version: String,
}

impl LainApp {
    pub fn run_setup(&mut self, cx: &mut Context<Self>) {
        let (user, pass, again) = (self.text("setup.user"), self.text("setup.pass"), self.text("setup.again"));
        if user.trim().is_empty() || pass.is_empty() {
            self.connect.error = Some("Choose a username and a password.".into());
            return;
        }
        if pass != again {
            self.connect.error = Some("The two passwords differ.".into());
            self.focused = Some("setup.again".into());
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.connect.busy = true;
        self.connect.error = None;
        let u = user.trim().to_string();
        self.spawn_ui(
            cx,
            async move {
                c.setup(&u, &pass).await?;
                c.login(&u, &pass).await
            },
            |app, r, cx| {
                app.connect.busy = false;
                match r {
                    Ok(me) => {
                        app.set_text("setup.pass", "");
                        app.set_text("setup.again", "");
                        app.toast(format!("Welcome, {}. You are this server's admin.", me.username), false);
                        app.enter_main(me, cx);
                    }
                    Err(e) => app.connect.error = Some(crate::backend::reason(&e)),
                }
            },
        );
    }

    pub fn run_login(&mut self, cx: &mut Context<Self>) {
        let (user, pass) = (self.text("login.user"), self.text("login.pass"));
        if user.trim().is_empty() || pass.is_empty() {
            self.connect.error = Some("Enter your username and password.".into());
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.connect.busy = true;
        self.connect.error = None;
        self.spawn_ui(cx, async move { c.login(user.trim(), &pass).await }, |app, r, cx| {
            app.connect.busy = false;
            match r {
                Ok(me) => {
                    app.set_text("login.pass", "");
                    app.enter_main(me, cx);
                }
                Err(ClientError::LoginFailed { message }) => {
                    app.connect.error = Some(if message.is_empty() { "invalid credentials".into() } else { message });
                    app.focused = Some("login.pass".into());
                }
                Err(e) => app.connect.error = Some(crate::backend::reason(&e)),
            }
        });
    }

    pub fn render_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let (vw, vh) = crate::ui::viewport(window);
        let sf = window.scale_factor();
        let mut root = div().size_full().relative().overflow_hidden().bg(pal.bg);

        // Backdrop: the theme's wallpaper, else a quiet accent glow.
        let wall = crate::theme::theme_file(&self.theme_dir, &th.wallpaper);
        let mut has_art = false;
        if let Some(p) = wall {
            let key = ImgKey {
                src: Source::File(p),
                w: (vw * sf) as u32,
                h: (vh * sf) as u32,
                blur: (th.blur.hero * sf) as u32,
            };
            if let Some((art, _)) = self.images.update(cx, |s, cx| s.get(&key, cx)) {
                has_art = true;
                root = root
                    .child(img(ImageSource::Render(art)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover));
            }
        }
        if !has_art {
            root = root.child(div().absolute().inset_0().bg(linear_gradient(
                135.,
                linear_color_stop(pal.accent.opacity(0.28), 0.),
                linear_color_stop(pal.bg.opacity(0.), 0.7),
            )));
        }
        root = root.child(div().absolute().inset_0().bg(linear_gradient(
            90.,
            linear_color_stop(pal.bg.opacity(0.9), 0.),
            linear_color_stop(pal.bg.opacity(0.2), 1.),
        )));

        let step = self.connect.step;
        let busy = self.connect.busy;
        let (kicker, title, body) = match step {
            Step::Server => {
                ("connect", "Your server", "Where your lain server answers. A local server listens on port 9360.")
            }
            Step::Setup => {
                ("first run", "Make the first admin", "This server has no accounts yet. The first one administers it.")
            }
            Step::Login => ("log in", "Welcome back", ""),
        };
        let server = self.backend.as_ref().map(|b| b.client.server()).unwrap_or_default();
        let mut form = div().flex().flex_col().gap(px(th.sp(3))).w(px(420.));
        match step {
            Step::Server => {
                form = form
                    .child(self.label("server url"))
                    .child(self.field("connect.url", "http://host:9360", false, Act::ConnectServer, cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(th.sp(2)))
                            .child(self.btn(
                                "connect.go",
                                if busy { "Connecting…" } else { "Connect" },
                                Some("Enter"),
                                Tone::Primary,
                                Act::ConnectServer,
                                cx,
                            ))
                            .child(self.btn(
                                "connect.offline",
                                "Offline copies",
                                None,
                                Tone::Ghost,
                                Act::OfflineMode,
                                cx,
                            )),
                    );
            }
            Step::Setup => {
                form = form
                    .child(self.label("username"))
                    .child(self.field("setup.user", "admin", false, Act::Focus("setup.pass"), cx))
                    .child(self.label("password"))
                    .child(self.field("setup.pass", "", true, Act::Focus("setup.again"), cx))
                    .child(self.label("password again"))
                    .child(self.field("setup.again", "", true, Act::Setup, cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(th.sp(2)))
                            .child(self.btn(
                                "setup.go",
                                if busy { "Creating…" } else { "Create admin" },
                                Some("Enter"),
                                Tone::Primary,
                                Act::Setup,
                                cx,
                            ))
                            .child(self.btn("setup.server", "Other server", None, Tone::Quiet, Act::ChangeServer, cx)),
                    );
            }
            Step::Login => {
                form = form
                    .child(self.label("username"))
                    .child(self.field("login.user", "", false, Act::Focus("login.pass"), cx))
                    .child(self.label("password"))
                    .child(self.field("login.pass", "", true, Act::Login, cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(th.sp(2)))
                            .child(self.btn(
                                "login.go",
                                if busy { "Logging in…" } else { "Log in" },
                                Some("Enter"),
                                Tone::Primary,
                                Act::Login,
                                cx,
                            ))
                            .child(self.btn("login.server", "Other server", None, Tone::Quiet, Act::ChangeServer, cx)),
                    );
            }
        }
        if let Some(e) = &self.connect.error {
            form = form.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(2)))
                    .text_color(pal.danger)
                    .child(self.icon("alert", 14., pal.danger))
                    .child(e.clone()),
            );
        }
        let title_size = th.ts(5);
        root.child(
            div()
                .absolute()
                .left(px(th.sp(6) * 1.5))
                .top_0()
                .bottom_0()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(th.sp(4)))
                .child(
                    div()
                        .font_family(self.fonts.display.clone())
                        .text_size(px(th.ts(3)))
                        .font_weight(FontWeight::BOLD)
                        .child("lain"),
                )
                .child(self.label(kicker).text_color(pal.accent))
                .child(
                    div()
                        .font_family(self.fonts.display.clone())
                        .text_size(px(title_size))
                        .line_height(px(title_size * 1.05))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .when(!body.is_empty(), |d| d.child(div().max_w(px(460.)).text_color(pal.muted).child(body)))
                .when(step != Step::Server && !server.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(th.sp(2)))
                            .text_color(pal.muted)
                            .child(self.icon("server", 14., pal.muted))
                            .child(format!("{server}  ·  {}", self.connect.server_version)),
                    )
                })
                .child(div().h(px(th.sp(2))))
                .child(form)
                .child(
                    div()
                        .pt(px(th.sp(4)))
                        .flex()
                        .gap(px(th.sp(3)))
                        .text_size(px(th.ts(-2)))
                        .text_color(pal.muted)
                        .child(div().flex().gap(px(4.)).child(self.kbd("Tab")).child("next field"))
                        .child(div().flex().gap(px(4.)).child(self.kbd("Enter")).child("continue")),
                ),
        )
    }
}
