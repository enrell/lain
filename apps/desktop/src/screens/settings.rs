//! Settings in one shape (D-087): a YOU/SERVER rail and one row pattern.
//! YOU: account, profile and avatar, private libraries and the PIN,
//! appearance (themes), playback and offline copies. SERVER (admins):
//! users, libraries with scan, plugins (swap/withdraw), modules,
//! transcode and integrations, backup. Personal settings apply at once.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    ClickEvent, Context, FontWeight, ImageSource, ObjectFit, PathPromptOptions, SharedString, Window, div, img, px,
};
use lain_client::types::{
    BrowseResult, LibraryPatch, NewLibrary, PluginsView, ProfilePatch, ScanStatus, SwapRequest, User, UserPatch,
};
use lain_client::{ClientError, Unlock};

use crate::app::{Act, LainApp, Load, Screen, Section};
use crate::images::{ImgKey, Source};
use crate::ui::{Notice, Tone};

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub users: Load<Vec<User>>,
    pub plugins: Load<PluginsView>,
    pub scan: Load<ScanStatus>,
    pub browse: Option<Load<BrowseResult>>,
    pub mascots: Load<Vec<String>>,
    pub pin_set: Load<bool>,
    pub new_kind: String,
    pub scan_dirty: bool,
    pub plugins_dirty: bool,
    pub scan_poll_at: Instant,
    pub busy: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState {
            users: Load::Idle,
            plugins: Load::Idle,
            scan: Load::Idle,
            browse: None,
            mascots: Load::Idle,
            pin_set: Load::Idle,
            new_kind: "anime".into(),
            scan_dirty: false,
            plugins_dirty: false,
            scan_poll_at: Instant::now(),
            busy: false,
        }
    }
}

const LIB_KINDS: &[&str] = &["anime", "movie", "video", "manga", "comic"];

impl LainApp {
    pub fn enter_settings(&mut self, sec: Section, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        match sec {
            Section::Account => {
                let lang = self.me.as_ref().map(|u| u.preferred_language.clone()).unwrap_or_default();
                self.set_text("acct.lang", &lang);
            }
            Section::Profile => {
                if let Some(u) = &self.me {
                    let (n, bio) = (u.profile.display_name.clone(), u.profile.bio.clone());
                    self.set_text("profile.name", &n);
                    self.set_text("profile.bio", &bio);
                }
                self.spawn_ui(cx, async move { c.mascots().await }, |app, r, _| app.settings.mascots = Load::from(r));
            }
            Section::Privacy => {
                self.spawn_ui(cx, async move { c.pin_status().await }, |app, r, _| {
                    app.settings.pin_set = Load::from(r)
                });
                self.reload_libraries(cx);
            }
            Section::Appearance => self.themes = crate::theme::list(&crate::theme::themes_dir(&self.dir)),
            Section::Playback => {
                let gib = self.config.offline_max_gib.to_string();
                self.set_text("play.gib", &gib);
                let map = self.config.path_map.first().map(|(a, b)| format!("{a} => {b}")).unwrap_or_default();
                self.set_text("play.map", &map);
            }
            Section::Users => {
                self.settings.users = Load::Loading;
                self.spawn_ui(cx, async move { c.users().await }, |app, r, _| app.settings.users = Load::from(r));
            }
            Section::Libraries => {
                self.reload_libraries(cx);
                self.poll_scan(cx);
            }
            Section::Plugins | Section::Modules => {
                self.settings.plugins = Load::Loading;
                self.spawn_ui(cx, async move { c.plugins().await }, |app, r, _| app.settings.plugins = Load::from(r));
                let c2 = b.client.clone();
                self.spawn_ui(cx, async move { c2.modules().await }, |app, r, _| app.modules = Load::from(r));
            }
            Section::Server | Section::Backup => {}
        }
    }

    fn poll_scan(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.settings.scan_poll_at = Instant::now();
        self.spawn_ui(cx, async move { c.scan_status().await }, |app, r, cx| {
            let finished = matches!(&app.settings.scan, Load::Ready(s) if s.state == "running")
                && matches!(&r, Ok(s) if s.state != "running");
            app.settings.scan = Load::from(r);
            if finished {
                app.toast("Scan finished", false);
                app.reload_catalog(cx);
            }
        });
    }

    /// Live scan progress while a scan runs; plugin list after events.
    pub fn settings_tick(&mut self, cx: &mut Context<Self>) {
        let Screen::Settings(sec) = self.screen.clone() else { return };
        let running = matches!(&self.settings.scan, Load::Ready(s) if s.state == "running");
        if sec == Section::Libraries && (running || self.settings.scan_dirty) {
            self.settings.scan_dirty = false;
            self.poll_scan(cx);
        }
        if matches!(sec, Section::Plugins | Section::Modules) && self.settings.plugins_dirty {
            self.settings.plugins_dirty = false;
            self.enter_settings(sec, cx);
        }
    }

    /// Settings actions (everything `run` does not handle itself).
    pub fn run_settings(&mut self, act: Act, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        match act {
            Act::SetLanguage => {
                let lang = self.text("acct.lang").trim().to_lowercase();
                self.spawn_ui(cx, async move { c.set_preferred_language(&lang).await }, |app, r, _| {
                    if app.report(&r, "Language saved") {
                        app.me = r.ok();
                    }
                });
            }
            Act::ChangePassword => {
                let (old, new, again) = (self.text("acct.old"), self.text("acct.new"), self.text("acct.again"));
                if new.is_empty() || new != again {
                    self.toast("The new passwords are empty or differ", true);
                    return;
                }
                let user = self.me.as_ref().map(|u| u.username.clone()).unwrap_or_default();
                self.spawn_ui(
                    cx,
                    async move {
                        c.change_password(&old, &new).await?;
                        // Every session ended, this one included: log in again.
                        c.login(&user, &new).await
                    },
                    |app, r, _| {
                        if app.report(&r, "Password changed") {
                            for k in ["acct.old", "acct.new", "acct.again"] {
                                app.set_text(k, "");
                            }
                            app.me = r.ok();
                        }
                    },
                );
            }
            Act::SaveProfile => {
                let patch = ProfilePatch {
                    display_name: Some(self.text("profile.name")),
                    bio: Some(self.text("profile.bio")),
                    mascot: None,
                };
                self.spawn_ui(cx, async move { c.update_profile(patch).await }, |app, r, _| {
                    if app.report(&r, "Profile saved") {
                        app.me = r.ok();
                    }
                });
            }
            Act::SetMascot(m) => {
                let patch = ProfilePatch { mascot: Some(m), ..Default::default() };
                self.spawn_ui(cx, async move { c.update_profile(patch).await }, |app, r, _| {
                    if app.report(&r, "Avatar changed") {
                        app.me = r.ok();
                    }
                });
            }
            Act::UploadAvatar => {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Avatar image".into()),
                });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(paths))) = rx.await else { return };
                    let Some(p) = paths.into_iter().next() else { return };
                    let _ = this.update(cx, |app, cx| app.upload_avatar(p, cx));
                })
                .detach();
            }
            Act::ClearAvatar => {
                self.spawn_ui(cx, async move { c.clear_avatar().await }, |app, r, _| {
                    if app.report(&r, "Avatar cleared") {
                        app.me = r.ok();
                    }
                });
            }
            Act::SetPin => {
                let (pin, pass) = (self.text("pin.new"), self.text("pin.pass"));
                self.spawn_ui(cx, async move { c.set_pin(&pin, &pass).await }, |app, r, _| {
                    if app.report(&r, "Library PIN set") {
                        app.settings.pin_set = Load::Ready(true);
                        app.set_text("pin.new", "");
                        app.set_text("pin.pass", "");
                    }
                });
            }
            Act::ClearPin => {
                let pass = self.text("pin.pass");
                self.spawn_ui(cx, async move { c.clear_pin(&pass).await }, |app, r, _| {
                    if app.report(&r, "Library PIN removed") {
                        app.settings.pin_set = Load::Ready(false);
                        app.set_text("pin.pass", "");
                    }
                });
            }
            Act::Unlock(id) => {
                let secret = self.text("unlock.secret");
                let how = if !secret.is_empty() && secret.chars().all(|c| c.is_ascii_digit()) && secret.len() <= 12 {
                    Unlock::Pin(secret.clone())
                } else {
                    Unlock::Password(secret.clone())
                };
                self.spawn_ui(
                    cx,
                    async move {
                        match c.unlock_library(&id, how).await {
                            // A numeric password is still a password.
                            Err(ClientError::Forbidden { .. }) if secret.chars().all(|c| c.is_ascii_digit()) => {
                                c.unlock_library(&id, Unlock::Password(secret)).await
                            }
                            r => r,
                        }
                    },
                    |app, r, cx| {
                        if app.report(&r, "Unlocked for this session") {
                            app.set_text("unlock.secret", "");
                            app.reload_libraries(cx);
                            app.reload_catalog(cx);
                        }
                    },
                );
            }
            Act::Lock(id) => {
                self.spawn_ui(cx, async move { c.lock_library(&id).await }, |app, r, cx| {
                    if app.report(&r, "Locked") {
                        app.reload_libraries(cx);
                        app.reload_catalog(cx);
                    }
                });
            }
            Act::PickTheme(id) => {
                let dir = crate::theme::themes_dir(&self.dir).join(&id);
                self.apply_theme_dir(dir, cx);
                self.config.theme = id;
                self.config.save(&self.dir);
            }
            Act::InstallTheme => {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Theme zip".into()),
                });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(paths))) = rx.await else { return };
                    let Some(p) = paths.into_iter().next() else { return };
                    let _ = this.update(cx, |app, cx| app.install_theme(&p, cx));
                })
                .detach();
            }
            Act::OpenThemesFolder => crate::platform::open_folder(&crate::theme::themes_dir(&self.dir)),
            Act::ToggleAutoplay => {
                self.config.autoplay_next = !self.config.autoplay_next;
                self.config.save(&self.dir);
            }
            Act::SaveOffline => match self.text("play.gib").trim().parse::<u64>() {
                Ok(g) if g > 0 => {
                    self.config.offline_max_gib = g;
                    self.config.save(&self.dir);
                    self.toast("Saved; applies on the next start", false);
                }
                _ => self.toast("Enter a whole number of GiB", true),
            },
            Act::SavePathMap => {
                let raw = self.text("play.map");
                self.config.path_map = match raw.split_once("=>") {
                    Some((a, b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
                        vec![(a.trim().to_string(), b.trim().to_string())]
                    }
                    _ => Vec::new(),
                };
                self.config.save(&self.dir);
                self.toast(if self.config.path_map.is_empty() { "Path map cleared" } else { "Path map saved" }, false);
            }
            Act::CreateUser => {
                let (u, p) = (self.text("users.name"), self.text("users.pass"));
                self.spawn_ui(cx, async move { c.create_user(u.trim(), &p, "user").await }, |app, r, cx| {
                    if app.report(&r, "User created") {
                        app.set_text("users.name", "");
                        app.set_text("users.pass", "");
                        app.enter_settings(Section::Users, cx);
                    }
                });
            }
            Act::UserDisabled(id, d) => {
                let patch = UserPatch { disabled: Some(d), ..Default::default() };
                self.user_patch(id, patch, if d { "User disabled" } else { "User enabled" }, cx);
            }
            Act::UserRole(id, role) => {
                let patch = UserPatch { role: Some(role), ..Default::default() };
                self.user_patch(id, patch, "Role changed", cx);
            }
            Act::UserPassword(id) => {
                let p = self.text("users.reset");
                if p.is_empty() {
                    self.toast("Type the new password in the field first", true);
                    self.focused = Some("users.reset".into());
                    return;
                }
                let patch = UserPatch { password: Some(p), ..Default::default() };
                self.user_patch(id, patch, "Password reset", cx);
                self.set_text("users.reset", "");
            }
            Act::CreateLibrary => {
                let lib = NewLibrary {
                    name: self.text("lib.name").trim().to_string(),
                    kind: self.settings.new_kind.clone(),
                    path: self.text("lib.path").trim().to_string(),
                    root: String::new(),
                };
                if lib.name.is_empty() || lib.path.is_empty() {
                    self.toast("A library needs a name and a folder", true);
                    return;
                }
                self.spawn_ui(cx, async move { c.create_library(lib).await }, |app, r, cx| {
                    if app.report(&r, "Library added; scan it to fill it") {
                        app.set_text("lib.name", "");
                        app.set_text("lib.path", "");
                        app.settings.browse = None;
                        app.reload_libraries(cx);
                    }
                });
            }
            Act::AskDeleteLibrary(id, name) => {
                self.ask(
                    format!("Remove the library {name}? Its catalog entries go; the files stay on disk."),
                    Act::DeleteLibrary(id),
                );
            }
            Act::DeleteLibrary(id) => {
                self.spawn_ui(cx, async move { c.delete_library(&id).await }, |app, r, cx| {
                    let msg = match &r {
                        Ok(d) => format!("Library removed ({} catalog entries; files untouched)", d.items_removed),
                        Err(_) => String::new(),
                    };
                    if app.report(&r, &msg) {
                        app.reload_libraries(cx);
                        app.reload_catalog(cx);
                    }
                });
            }
            Act::LibraryPrivate(id, private) => {
                let owner = self.me.as_ref().map(|u| u.id.clone());
                let patch = LibraryPatch {
                    private: Some(private),
                    owner: if private { owner } else { None },
                    ..Default::default()
                };
                self.spawn_ui(cx, async move { c.update_library(&id, patch).await }, move |app, r, cx| {
                    if app.report(&r, if private { "Library is private" } else { "Library is shared" }) {
                        app.reload_libraries(cx);
                    }
                });
            }
            Act::Scan => {
                self.spawn_ui(cx, async move { c.start_scan().await }, |app, r, cx| {
                    if app.report(&r, "Scan started") {
                        app.settings.scan = Load::Ready(ScanStatus { state: "running".into(), ..Default::default() });
                        app.poll_scan(cx);
                    }
                });
            }
            Act::BrowseDir(path) => {
                self.settings.browse = Some(Load::Loading);
                self.spawn_ui(cx, async move { c.browse(&path).await }, |app, r, _| {
                    if let Ok(b) = &r {
                        let p = b.path.clone();
                        app.set_text("lib.path", &p);
                    }
                    app.settings.browse = Some(Load::from(r));
                });
            }
            Act::Swap(cap, providers, generation) => {
                let req = SwapRequest { capability: cap, providers, generation };
                self.spawn_ui(cx, async move { c.swap_providers(req).await }, |app, r, cx| {
                    if app.report(&r, "Provider swapped") {
                        app.enter_settings(Section::Plugins, cx);
                    } else {
                        app.enter_settings(Section::Plugins, cx);
                    }
                });
            }
            Act::Withdraw(p) => {
                self.spawn_ui(cx, async move { c.withdraw_provider(&p).await }, |app, r, cx| {
                    app.report(&r, "Provider withdrawn");
                    app.enter_settings(Section::Plugins, cx);
                });
            }
            Act::Backup => {
                let dir = dirs_home();
                let rx = cx.prompt_for_new_path(&dir, Some("lain-backup.zip"));
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(path))) = rx.await else { return };
                    let _ = this.update(cx, |app, cx| app.save_backup(path, cx));
                })
                .detach();
            }
            _ => {}
        }
    }

    fn user_patch(&mut self, id: String, patch: UserPatch, ok: &'static str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.spawn_ui(cx, async move { c.update_user(&id, patch).await }, move |app, r, cx| {
            app.report(&r, ok);
            app.enter_settings(Section::Users, cx);
        });
    }

    fn upload_avatar(&mut self, p: std::path::PathBuf, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        let ctype = match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            _ => "image/jpeg",
        };
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                self.toast(format!("{}: {e}", p.display()), true);
                return;
            }
        };
        self.spawn_ui(cx, async move { c.set_avatar(bytes, ctype).await }, |app, r, cx| {
            if app.report(&r, "Avatar uploaded") {
                if let Ok(u) = &r {
                    let src = Source::Avatar(u.id.clone(), u.profile.avatar.version);
                    app.images.update(cx, |s, _| s.forget(&src));
                }
                app.me = r.ok();
            }
        });
    }

    fn install_theme(&mut self, p: &std::path::Path, cx: &mut Context<Self>) {
        match crate::theme::install_zip(p, &crate::theme::themes_dir(&self.dir)) {
            Ok(id) => {
                self.toast(format!("Theme {id} installed"), false);
                let w = Act::PickTheme(id);
                if let Act::PickTheme(id) = w {
                    let dir = crate::theme::themes_dir(&self.dir).join(&id);
                    self.apply_theme_dir(dir, cx);
                    self.config.theme = id;
                    self.config.save(&self.dir);
                }
            }
            Err(e) => self.toast(format!("Theme: {e}"), true),
        }
    }

    fn save_backup(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.settings.busy = true;
        self.spawn_ui(cx, async move { c.backup().await }, move |app, r, _| {
            app.settings.busy = false;
            match r {
                Ok(bytes) => match std::fs::write(&path, &bytes) {
                    Ok(()) => app.toast(
                        format!("Backup saved to {} ({})", path.display(), crate::ui::fmt_bytes(bytes.len() as u64)),
                        false,
                    ),
                    Err(e) => app.toast(format!("{}: {e}", path.display()), true),
                },
                Err(e) => app.toast(crate::backend::reason(&e), true),
            }
        });
    }

    // ---- rendering --------------------------------------------------------------

    pub fn render_settings(
        &mut self,
        sec: Section,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let margin = th.sp(6);
        let admin = self.is_admin();
        let sec = if sec.admin() && !admin { Section::Account } else { sec };

        // ---- rail
        let mut rail = div().w(px(230.)).flex_none().flex().flex_col().gap(px(2.));
        let groups: Vec<(&str, &[Section])> =
            if admin { vec![("you", Section::YOU), ("server", Section::SERVER)] } else { vec![("you", Section::YOU)] };
        for (g, list) in groups {
            rail = rail.child(div().pt(px(th.sp(3))).pb(px(th.sp(1))).child(self.label(g)));
            for &s in list {
                let id = format!("set.rail.{}", s.id());
                let focused = self.stop(&id, Some(Act::Nav(Screen::Settings(s))), false);
                let on = s == sec;
                rail = rail.child(
                    div()
                        .id(SharedString::from(id))
                        .px(px(th.sp(3)))
                        .py(px(th.sp(1) + 2.))
                        .rounded(px(th.radius.button))
                        .border_1()
                        .border_color(if focused { pal.focus } else { pal.text.opacity(0.) })
                        .bg(if on { pal.text.opacity(0.08) } else { pal.text.opacity(0.) })
                        .text_color(if on { pal.text } else { pal.text.opacity(0.7) })
                        .font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                        .cursor_pointer()
                        .hover(|s| s.bg(gpui::white().opacity(0.05)))
                        .child(s.label())
                        .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                            app.screen = Screen::Settings(s);
                            app.enter_settings(s, cx);
                            cx.notify();
                        })),
                );
            }
        }

        let body = match sec {
            Section::Account => self.set_account(cx),
            Section::Profile => self.set_profile(cx),
            Section::Privacy => self.set_privacy(cx),
            Section::Appearance => self.set_appearance(cx),
            Section::Playback => self.set_playback(cx),
            Section::Users => self.set_users(cx),
            Section::Libraries => self.set_libraries(cx),
            Section::Plugins => self.set_plugins(cx),
            Section::Modules => self.set_modules(),
            Section::Server => self.set_server(),
            Section::Backup => self.set_backup(cx),
        };
        div().size_full().flex().pt(px(76.)).px(px(margin)).gap(px(th.sp(6))).child(rail).child(
            div()
                .id("set.body")
                .flex_1()
                .h_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .pb(px(th.sp(6)))
                .child(div().max_w(px(860.)).flex().flex_col().child(self.section_title(sec.label())).child(body)),
        )
    }

    fn set_account(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let u = self.me.clone().unwrap_or_default();
        let server = self.backend.as_ref().map(|b| b.client.server()).unwrap_or_default();
        let lang = self.field("acct.lang", "jpn, eng…", false, Act::SetLanguage, cx);
        let lang_save = self.btn("acct.lang.save", "Save", None, Tone::Ghost, Act::SetLanguage, cx);
        let old = self.field("acct.old", "current password", true, Act::Focus("acct.new"), cx);
        let new = self.field("acct.new", "new password", true, Act::Focus("acct.again"), cx);
        let again = self.field("acct.again", "new password again", true, Act::ChangePassword, cx);
        let change = self.btn("acct.pw", "Change password", None, Tone::Ghost, Act::ChangePassword, cx);
        let switch = self.btn("acct.server", "Other server", None, Tone::Ghost, Act::ChangeServer, cx);
        let logout = self.btn("acct.logout", "Log out", None, Tone::Danger, Act::Logout, cx);
        div()
            .flex()
            .flex_col()
            .child(self.row("Username", &format!("{} · {}", u.username, u.role), div()))
            .child(self.row("Server", &server, switch))
            .child(self.row(
                "Preferred language",
                "ISO 639-2 code; picks audio and subtitle tracks",
                div().flex().gap(px(th.sp(1))).child(div().w(px(140.)).child(lang)).child(lang_save),
            ))
            .child(self.row(
                "Password",
                "Changing it ends every session of this account; this app logs in again",
                div().flex().flex_col().gap(px(th.sp(1))).w(px(280.)).child(old).child(new).child(again).child(change),
            ))
            .child(self.row("Session", "Forget the saved token on this device", logout))
    }

    fn set_profile(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let u = self.me.clone().unwrap_or_default();
        let name = self.field("profile.name", "display name", false, Act::SaveProfile, cx);
        let bio = self.field("profile.bio", "one line about you", false, Act::SaveProfile, cx);
        let save = self.btn("profile.save", "Save profile", None, Tone::Primary, Act::SaveProfile, cx);
        let upload = self.btn("profile.upload", "Upload image…", None, Tone::Ghost, Act::UploadAvatar, cx);
        let clear = self.btn("profile.clear", "Clear", None, Tone::Quiet, Act::ClearAvatar, cx);
        let src = Source::Avatar(u.id.clone(), u.profile.avatar.version);
        let key = ImgKey { src, w: 128, h: 128, blur: 0 };
        let art =
            if u.profile.avatar.kind == "upload" { self.images.update(cx, |s, cx| s.get(&key, cx)) } else { None };
        let mut avatar =
            div().size(px(64.)).rounded_full().overflow_hidden().bg(pal.surface).flex().items_center().justify_center();
        avatar = match art {
            Some((a, _)) => avatar.child(img(ImageSource::Render(a)).size_full().object_fit(ObjectFit::Cover)),
            None => avatar.child(div().font_family(self.fonts.display.clone()).text_size(px(th.ts(2))).child(
                if u.profile.avatar.mascot.is_empty() {
                    u.username.chars().next().unwrap_or('?').to_uppercase().to_string()
                } else {
                    u.profile.avatar.mascot.chars().take(2).collect()
                },
            )),
        };
        let mut mascots = div().flex().flex_wrap().gap(px(th.sp(1)));
        match self.settings.mascots.clone() {
            Load::Ready(list) => {
                for m in list {
                    let tone = if u.profile.avatar.mascot == m { Tone::Ghost } else { Tone::Quiet };
                    mascots = mascots.child(self.btn(
                        &format!("mascot.{m}"),
                        m.clone(),
                        None,
                        tone,
                        Act::SetMascot(m.clone()),
                        cx,
                    ));
                }
            }
            Load::Failed(e) => mascots = mascots.child(div().text_color(pal.muted).child(crate::backend::reason(&e))),
            _ => {}
        }
        div()
            .flex()
            .flex_col()
            .child(self.row("Display name", "What other people on this server see", div().w(px(300.)).child(name)))
            .child(self.row("Bio", "Optional", div().w(px(300.)).child(bio)))
            .child(self.row("", "", save))
            .child(self.row(
                "Avatar",
                "An image, or one of the server's mascots",
                div().flex().items_center().gap(px(th.sp(2))).child(avatar).child(upload).child(clear),
            ))
            .child(div().py(px(th.sp(2))).child(mascots))
    }

    fn set_privacy(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let pin_state = match &self.settings.pin_set {
            Load::Ready(true) => "set",
            Load::Ready(false) => "not set",
            Load::Failed(_) => "unknown",
            _ => "…",
        };
        let pin = self.field("pin.new", "new PIN (4–12 digits)", true, Act::SetPin, cx);
        let pass = self.field("pin.pass", "your password", true, Act::SetPin, cx);
        let set = self.btn("pin.set", "Set PIN", None, Tone::Ghost, Act::SetPin, cx);
        let clear = self.btn("pin.clear", "Remove PIN", None, Tone::Quiet, Act::ClearPin, cx);
        let unlock_field = self.field("unlock.secret", "PIN or password", true, Act::Focus("unlock.secret"), cx);
        let mut col = div()
            .flex()
            .flex_col()
            .child(
                self.row(
                    "Library PIN",
                    &format!("Unlocks your private libraries for a session; currently {pin_state}"),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(th.sp(1)))
                        .w(px(280.))
                        .child(pin)
                        .child(pass)
                        .child(div().flex().gap(px(th.sp(1))).child(set).child(clear)),
                ),
            )
            .child(self.row(
                "Unlock with",
                "Type the PIN or password, then Unlock on a library",
                div().w(px(220.)).child(unlock_field),
            ));
        let libs: Vec<_> =
            self.libraries.ready().map(|l| l.iter().filter(|l| l.private).cloned().collect()).unwrap_or_default();
        if libs.is_empty() {
            col =
                col.child(div().py(px(th.sp(3))).text_color(pal.muted).child(
                    "You have no private libraries. An admin can make a library private (Settings → Libraries).",
                ));
        }
        for l in libs {
            let locked = l.locked == Some(true);
            let b = if locked {
                self.btn(&format!("priv.{}", l.id), "Unlock", None, Tone::Primary, Act::Unlock(l.id.clone()), cx)
            } else {
                self.btn(&format!("priv.{}", l.id), "Lock", None, Tone::Ghost, Act::Lock(l.id.clone()), cx)
            };
            col = col.child(self.row(
                &l.name,
                if locked { "locked: hidden everywhere" } else { "unlocked for this session" },
                b,
            ));
        }
        col
    }

    fn set_appearance(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let install = self.btn("theme.install", "Install from zip…", None, Tone::Ghost, Act::InstallTheme, cx);
        let open = self.btn("theme.open", "Open themes folder", None, Tone::Quiet, Act::OpenThemesFolder, cx);
        let active_dir = self.theme_dir.clone();
        let mut col = div().flex().flex_col().child(self.row(
            "Themes",
            "lain.ui.theme@1 folders: edit theme.json and the app reloads it live",
            div().flex().gap(px(th.sp(1))).child(install).child(open),
        ));
        let themes = self.themes.clone();
        for t in themes {
            let on = t.dir == active_dir;
            let hint = match &t.error {
                Some(e) => e.clone(),
                None => format!("{} · {}", if t.scheme.is_empty() { "dark" } else { &t.scheme }, t.dir.display()),
            };
            let b = if on {
                self.btn(&format!("theme.{}", t.id), "Active", None, Tone::Ghost, Act::PickTheme(t.id.clone()), cx)
            } else {
                self.btn(&format!("theme.{}", t.id), "Use", None, Tone::Primary, Act::PickTheme(t.id.clone()), cx)
            };
            col = col.child(self.row(&t.name, &hint, b));
        }
        if !self.theme_errors.is_empty() || !self.fonts.notes.is_empty() {
            let mut notes = div().flex().flex_col().gap(px(4.)).py(px(th.sp(3))).child(self.label("theme notes"));
            for n in self.theme_errors.iter().chain(self.fonts.notes.iter()) {
                notes = notes.child(div().text_size(px(th.ts(-1))).text_color(pal.muted).child(n.clone()));
            }
            col = col.child(notes);
        }
        // A swatch strip of the active palette.
        let mut sw = div().flex().gap(px(th.sp(1))).py(px(th.sp(3)));
        for (name, c) in [
            ("bg", pal.bg),
            ("surface", pal.surface),
            ("text", pal.text),
            ("muted", pal.muted),
            ("accent", pal.accent),
            ("danger", pal.danger),
            ("ok", pal.ok),
            ("warn", pal.warn),
        ] {
            sw = sw.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(div().size(px(40.)).rounded(px(th.radius.button)).bg(c).border_1().border_color(pal.line))
                    .child(self.label(name)),
            );
        }
        col.child(sw)
    }

    fn set_playback(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let auto = self.btn(
            "play.auto",
            if self.config.autoplay_next { "On" } else { "Off" },
            None,
            Tone::Ghost,
            Act::ToggleAutoplay,
            cx,
        );
        let gib = self.field("play.gib", "10", false, Act::SaveOffline, cx);
        let gib_save = self.btn("play.gib.save", "Save", None, Tone::Ghost, Act::SaveOffline, cx);
        let map = self.field("play.map", "/srv/media => \\\\nas\\media", false, Act::SavePathMap, cx);
        let map_save = self.btn("play.map.save", "Save", None, Tone::Ghost, Act::SavePathMap, cx);
        let engine = crate::screens::player::engine_status();
        let offline_dir = if self.config.offline_dir.is_empty() {
            self.dir.join("offline").display().to_string()
        } else {
            self.config.offline_dir.clone()
        };
        div()
            .flex()
            .flex_col()
            .child(self.row("Player", &engine, div()))
            .child(self.row("Play the next episode", "When an episode ends, the next one starts after 5 s", auto))
            .child(self.row(
                "Offline budget (GiB)",
                &format!("Copies go to {offline_dir}"),
                div().flex().gap(px(th.sp(1))).child(div().w(px(90.)).child(gib)).child(gib_save),
            ))
            .child(self.row(
                "Shared storage",
                "server path => local path: play files straight from a share when the server cannot stream",
                div().flex().gap(px(th.sp(1))).child(div().w(px(320.)).child(map)).child(map_save),
            ))
    }

    fn set_users(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let name = self.field("users.name", "username", false, Act::Focus("users.pass"), cx);
        let pass = self.field("users.pass", "password", true, Act::CreateUser, cx);
        let create = self.btn("users.create", "Create user", None, Tone::Primary, Act::CreateUser, cx);
        let reset = self.field("users.reset", "new password for a reset", true, Act::Focus("users.reset"), cx);
        let mut col = div()
            .flex()
            .flex_col()
            .child(
                self.row(
                    "New user",
                    "A regular account; promote it below",
                    div()
                        .flex()
                        .gap(px(th.sp(1)))
                        .child(div().w(px(160.)).child(name))
                        .child(div().w(px(160.)).child(pass))
                        .child(create),
                ),
            )
            .child(self.row("Password reset", "Type it here, then Reset on a user", div().w(px(240.)).child(reset)));
        match self.settings.users.clone() {
            Load::Ready(users) => {
                let me = self.me.as_ref().map(|u| u.id.clone()).unwrap_or_default();
                for u in users {
                    let base = format!("user.{}", u.id);
                    let mut ctl = div().flex().gap(px(th.sp(1)));
                    if u.id != me {
                        ctl = ctl
                            .child(self.btn(
                                &format!("{base}.role"),
                                if u.role == "admin" { "Make user" } else { "Make admin" },
                                None,
                                Tone::Quiet,
                                Act::UserRole(
                                    u.id.clone(),
                                    if u.role == "admin" { "user".into() } else { "admin".into() },
                                ),
                                cx,
                            ))
                            .child(self.btn(
                                &format!("{base}.dis"),
                                if u.disabled { "Enable" } else { "Disable" },
                                None,
                                Tone::Quiet,
                                Act::UserDisabled(u.id.clone(), !u.disabled),
                                cx,
                            ));
                    }
                    ctl = ctl.child(self.btn(
                        &format!("{base}.pw"),
                        "Reset",
                        None,
                        Tone::Quiet,
                        Act::UserPassword(u.id.clone()),
                        cx,
                    ));
                    let hint = format!(
                        "{}{}{}",
                        u.role,
                        if u.disabled { " · disabled" } else { "" },
                        if u.id == me { " · you" } else { "" }
                    );
                    col = col.child(self.row(&u.username, &hint, ctl));
                }
            }
            Load::Failed(e) => col = col.child(self.error_notice("Users", &e)),
            _ => {}
        }
        col
    }

    fn set_libraries(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        // scan
        let (scan_hint, running) = match &self.settings.scan {
            Load::Ready(s) => {
                let stats = s
                    .stats
                    .as_ref()
                    .map(|st| {
                        format!(
                            " · {} files, {} identified, {} new missing, {} errors",
                            st.candidates, st.identified, st.missing, st.errors
                        )
                    })
                    .unwrap_or_default();
                (format!("{}{stats}", s.state), s.state == "running")
            }
            Load::Failed(e) => (crate::backend::reason(e), false),
            _ => ("…".into(), false),
        };
        let scan =
            self.btn("libs.scan", if running { "Scanning…" } else { "Scan all" }, None, Tone::Primary, Act::Scan, cx);
        let mut col = div().flex().flex_col().child(self.row("Scan", &scan_hint, scan));
        if running {
            col = col.child(
                div().h(px(3.)).w_full().rounded_full().bg(pal.accent.opacity(0.25)).child(
                    div()
                        .h_full()
                        .w(gpui::relative(
                            0.35 + 0.3 * ((self.settings.scan_poll_at.elapsed().as_secs_f32() * 2.).sin().abs()),
                        ))
                        .bg(pal.accent),
                ),
            );
        }
        // existing libraries
        match self.libraries.clone() {
            Load::Ready(libs) => {
                for l in libs {
                    let base = format!("libs.{}", l.id);
                    let ctl = div()
                        .flex()
                        .gap(px(th.sp(1)))
                        .child(self.btn(
                            &format!("{base}.priv"),
                            if l.private { "Make shared" } else { "Make private" },
                            None,
                            Tone::Quiet,
                            Act::LibraryPrivate(l.id.clone(), !l.private),
                            cx,
                        ))
                        .child(self.btn(
                            &format!("{base}.del"),
                            "Remove",
                            None,
                            Tone::Danger,
                            Act::AskDeleteLibrary(l.id.clone(), l.name.clone()),
                            cx,
                        ));
                    let hint = format!("{} · {}{}", l.kind, l.path, if l.private { " · private" } else { "" });
                    col = col.child(self.row(&l.name, &hint, ctl));
                }
            }
            Load::Failed(e) => col = col.child(self.error_notice("Libraries", &e)),
            _ => {}
        }
        // add
        let name = self.field("lib.name", "name", false, Act::Focus("lib.path"), cx);
        let path = self.field("lib.path", "folder on the server", false, Act::CreateLibrary, cx);
        let browse = self.btn("lib.browse", "Browse…", None, Tone::Ghost, Act::BrowseDir(self.text("lib.path")), cx);
        let add = self.btn("lib.create", "Add library", None, Tone::Primary, Act::CreateLibrary, cx);
        let mut kinds = div().flex().gap(px(th.sp(1)));
        for k in LIB_KINDS {
            let on = self.settings.new_kind == *k;
            let id = format!("lib.kind.{k}");
            let focused = self.stop(&id, None, false);
            let kk = k.to_string();
            kinds = kinds.child(
                div()
                    .id(SharedString::from(id))
                    .px(px(th.sp(2)))
                    .py(px(4.))
                    .rounded(px(th.radius.button))
                    .border_1()
                    .border_color(if focused { pal.focus } else { pal.line })
                    .bg(if on { pal.accent.opacity(0.25) } else { pal.text.opacity(0.) })
                    .cursor_pointer()
                    .child(k.to_string())
                    .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                        app.settings.new_kind = kk.clone();
                        cx.notify();
                    })),
            );
        }
        col = col
            .child(div().pt(px(th.sp(5))).child(self.label("add a library")))
            .child(self.row(
                "Name and kind",
                "The kind picks the module (watch or read)",
                div().flex().flex_col().gap(px(th.sp(1))).child(div().w(px(300.)).child(name)).child(kinds),
            ))
            .child(self.row(
                "Folder",
                "A path the server can read",
                div().flex().gap(px(th.sp(1))).child(div().w(px(300.)).child(path)).child(browse),
            ));
        if let Some(b) = self.settings.browse.clone() {
            match b {
                Load::Ready(br) => {
                    let mut dirs = div().flex().flex_col().gap(px(2.)).py(px(th.sp(2)));
                    if !br.parent.is_empty() {
                        dirs = dirs.child(self.btn(
                            "browse.up",
                            format!("↑ {}", br.parent),
                            None,
                            Tone::Quiet,
                            Act::BrowseDir(br.parent.clone()),
                            cx,
                        ));
                    }
                    for (i, d) in br.dirs.iter().take(40).enumerate() {
                        dirs = dirs.child(self.btn(
                            &format!("browse.{i}"),
                            format!("{}/", d.name),
                            None,
                            Tone::Quiet,
                            Act::BrowseDir(d.path.clone()),
                            cx,
                        ));
                    }
                    col = col.child(dirs);
                }
                Load::Failed(e) => {
                    col = col.child(
                        div()
                            .py(px(th.sp(2)))
                            .text_color(pal.muted)
                            .child(format!("Browse: {}", crate::backend::reason(&e))),
                    )
                }
                _ => {}
            }
        }
        col.child(self.row("", "", add))
    }

    fn set_plugins(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let view = match self.settings.plugins.clone() {
            Load::Ready(v) => v,
            Load::Failed(e) => return div().child(self.error_notice("Plugins", &e)),
            _ => return div().child(self.notice(Notice::Loading, "Loading plugins…", "")),
        };
        let mut col = div().flex().flex_col();
        col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child("Each capability is bound to providers. Pick another provider to swap it; withdraw takes a provider out of every binding."));
        for b in &view.composition {
            let candidates: Vec<String> = view
                .provider_info
                .iter()
                .filter(|p| p.capabilities.contains(&b.capability))
                .map(|p| p.id.clone())
                .collect();
            let mut ctl = div().flex().flex_wrap().gap(px(th.sp(1))).justify_end().max_w(px(420.));
            for p in candidates {
                let bound = b.providers.contains(&p);
                let healthy = view.provider_info.iter().find(|x| x.id == p).map(|x| x.healthy).unwrap_or(false);
                let label = format!("{}{}", p, if healthy { "" } else { " (down)" });
                let id = format!("swap.{}.{}", b.capability, p);
                ctl = ctl.child(self.btn(
                    &id,
                    label,
                    None,
                    if bound { Tone::Ghost } else { Tone::Quiet },
                    Act::Swap(b.capability.clone(), vec![p.clone()], b.generation),
                    cx,
                ));
            }
            col = col.child(self.row(&b.capability, &format!("{} · generation {}", b.mode, b.generation), ctl));
        }
        col = col.child(div().pt(px(th.sp(5))).child(self.label("providers")));
        for p in &view.provider_info {
            let w =
                self.btn(&format!("withdraw.{}", p.id), "Withdraw", None, Tone::Quiet, Act::Withdraw(p.id.clone()), cx);
            col = col.child(self.row(
                &p.id,
                &format!("{} · {} capabilities", if p.healthy { "healthy" } else { "unhealthy" }, p.capabilities.len()),
                w,
            ));
        }
        if !view.events.is_empty() {
            col = col.child(div().pt(px(th.sp(5))).child(self.label("recent changes")));
            for e in view.events.iter().rev().take(12) {
                col = col.child(
                    div()
                        .py(px(2.))
                        .text_size(px(th.ts(-1)))
                        .text_color(pal.muted)
                        .child(format!("{} {} {} {}", e.kind, e.capability, e.provider, e.detail)),
                );
            }
        }
        col
    }

    fn set_modules(&mut self) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let mods = match self.modules.clone() {
            Load::Ready(m) => m,
            Load::Failed(e) => return div().child(self.error_notice("Modules", &e)),
            _ => return div(),
        };
        let mut col = div().flex().flex_col().child(div().py(px(th.sp(2))).text_color(pal.muted).child("Modules are switched in the server's modules.json; this app follows them live (hidden destinations, disabled states)."));
        for m in mods.modules {
            let color = match m.state.as_str() {
                "active" => pal.ok,
                "degraded" => pal.warn,
                "disabled" => pal.muted,
                _ => pal.danger,
            };
            let caps = m.capabilities.iter().filter(|c| c.bound).count();
            col = col.child(self.row(
                &m.name,
                &format!(
                    "{} · {caps}/{} capabilities bound{}",
                    m.id,
                    m.capabilities.len(),
                    if m.required { " · required" } else { "" }
                ),
                self.pill(&m.state, color),
            ));
        }
        col
    }

    fn set_server(&mut self) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(self.notice(
                Notice::NotPorted,
                "Transcode settings, integrations and the transcode list are not available on this server yet",
                "Their routes (/api/admin/settings/*, /api/admin/transcodes) belong to modules still being moved to the new server. This section fills in when the server serves them.",
            ))
    }

    fn set_backup(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let b = self.btn(
            "backup.go",
            if self.settings.busy { "Downloading…" } else { "Download backup…" },
            None,
            Tone::Primary,
            Act::Backup,
            cx,
        );
        div().flex().flex_col().child(self.row("Backup", "The server's database and settings as one archive", b))
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
