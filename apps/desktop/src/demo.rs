//! `--demo "<step>;<step>;…"`: in-app automation for screenshots and
//! smoke tests, so test runs never need simulated input (the owner's
//! rule). Steps:
//!
//! - `wait:<ms>`
//! - `connect:<url>`, `login` (credentials from `LAIN_DEMO_USER` and
//!   `LAIN_DEMO_PASS`; on a fresh server it runs the first-admin setup)
//! - `home`, `library`, `read`, `search:<text>`, `downloads`, `acquire`,
//!   `settings:<section id>`, `palette[:<text>]`
//! - `item:<n>` (the n-th title), `play:<n>` (its next episode),
//!   `reader:<n>` (the n-th reading title), `controls` (keep the player
//!   chrome visible), `seek:<s>`, `sub:<id>`
//! - `theme:<id>`, `focus:<widget id>`, `tab`, `key:<down|up|right|left>`
//!   (in-app focus moves, not OS input)
//! - `set:<field>=<text>`, `kind:<library kind>`, `run:<create-library|scan|refresh|set-pin|save-profile|save-pathmap|download>`
//!   (fill fields and run actions as a user would)
//! - `ext-install:<path>` (an extension folder or zip), `ext-allow`,
//!   `ext-deny` (answer the permission prompt), `ext-toggle:<id>`,
//!   `ext-scope:<id>/<scope>` (grant or revoke one scope)
//! - `shot:<name>`: a PNG of this window into the `--shot` directory
//! - `log:<text>`, `quit`

use std::time::{Duration, Instant};

use gpui::{AsyncWindowContext, WeakEntity};

use crate::app::{Act, LainApp, Phase, Screen, Section};

pub struct Demo {
    script: Option<String>,
    shot_dir: Option<std::path::PathBuf>,
    controls: bool,
    pub frames: u64,
}

impl Demo {
    pub fn new(script: Option<String>, shot: Option<std::path::PathBuf>) -> Demo {
        Demo { script, shot_dir: shot, controls: false, frames: 0 }
    }
    pub fn enabled(&self) -> bool {
        self.script.is_some()
    }
    pub fn force_controls(&self) -> bool {
        self.controls
    }
    pub fn on_frame(&mut self) {
        self.frames += 1;
    }
}

async fn wait(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

/// Waits until `f` holds (or the timeout passes).
async fn until(this: &WeakEntity<LainApp>, cx: &mut AsyncWindowContext, ms: u64, f: impl Fn(&LainApp) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        if this.read_with(cx, |app, _| f(app)).unwrap_or(false) {
            return true;
        }
        wait(cx, 100).await;
    }
    false
}

pub async fn run(this: WeakEntity<LainApp>, cx: &mut AsyncWindowContext) {
    let Some(script) = this.read_with(cx, |app, _| app.demo.script.clone()).ok().flatten() else { return };
    wait(cx, 400).await;
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (cmd, arg) = step.split_once(':').unwrap_or((step, ""));
        let arg = arg.to_string();
        eprintln!("demo: {step}");
        match cmd {
            "wait" => wait(cx, arg.parse().unwrap_or(500)).await,
            "log" => {
                let info = this
                    .read_with(cx, |app, _| {
                        let rails: Vec<String> =
                            app.home.rails.iter().map(|r| format!("{} ({})", r.title, r.cards.len())).collect();
                        let exts: Vec<String> =
                            app.ext.status.iter().map(|(id, s)| format!("{id}={}", s.label())).collect();
                        let panels: Vec<String> = app.ext.panels.1.iter().map(|(e, p)| format!("{e}/{}", p.id)).collect();
                        format!(
                            "frames {} · screen {:?} · {} items, {} titles · rails: {} · focus {:?} · layout {:?} hero {:?} · extensions: {} · panels: {} · prompt {:?}",
                            app.demo.frames,
                            app.screen,
                            app.catalog.len(),
                            app.series.len(),
                            rails.join(", "),
                            app.focused,
                            app.layout.name,
                            app.layout.hero,
                            exts.join(", "),
                            panels.join(", "),
                            app.ext_prompt
                        )
                    })
                    .unwrap_or_default();
                eprintln!("demo log: {arg}: {info}");
            }
            "quit" => {
                let _ = this.update(cx, |app, _| app.save_now());
                let _ = cx.update(|_, cx| cx.quit());
                return;
            }
            "connect" => {
                let _ = this.update(cx, |app, cx| {
                    app.set_text("connect.url", &arg);
                    app.run_connect(cx);
                });
                until(&this, cx, 8000, |a| !a.connect.busy).await;
            }
            "login" => {
                until(&this, cx, 8000, |a| !a.connect.busy).await;
                let user = std::env::var("LAIN_DEMO_USER").unwrap_or_default();
                let pass = std::env::var("LAIN_DEMO_PASS").unwrap_or_default();
                let _ = this.update(cx, |app, cx| {
                    if app.phase == Phase::Main {
                        return;
                    }
                    match app.connect.step {
                        crate::screens::connect::Step::Setup => {
                            app.set_text("setup.user", &user);
                            app.set_text("setup.pass", &pass);
                            app.set_text("setup.again", &pass);
                            app.run_setup(cx);
                        }
                        _ => {
                            app.set_text("login.user", &user);
                            app.set_text("login.pass", &pass);
                            app.run_login(cx);
                        }
                    }
                    cx.notify();
                });
                until(&this, cx, 10000, |a| a.phase == Phase::Main).await;
                until(&this, cx, 10000, |a| {
                    a.items.ready().is_some() || matches!(a.items, crate::app::Load::Failed(_))
                })
                .await;
            }
            "shot" => {
                let path = this.read_with(cx, |app, _| app.demo.shot_dir.clone().map(|d| d.join(&arg))).ok().flatten();
                let hwnd = this.read_with(cx, |app, _| app.hwnd).unwrap_or(0);
                // Let the latest state reach the screen.
                let _ = this.update(cx, |_, cx| cx.notify());
                wait(cx, 300).await;
                match path {
                    // Off the UI thread: PrintWindow sends WM_PRINT to the
                    // window, which the UI thread must be free to answer.
                    Some(p) => match cx
                        .background_executor()
                        .spawn({
                            let p = p.clone();
                            async move { crate::platform::capture(hwnd, &p) }
                        })
                        .await
                    {
                        Ok(()) => eprintln!("demo: wrote {}", p.display()),
                        Err(e) => eprintln!("demo: shot {}: {e}", p.display()),
                    },
                    None => eprintln!("demo: shot needs --shot <dir>"),
                }
            }
            _ => {
                let _ = this.update_in(cx, |app, window, cx| {
                    step_now(app, cmd, &arg, window, cx);
                    cx.notify();
                });
            }
        }
    }
}

fn step_now(app: &mut LainApp, cmd: &str, arg: &str, window: &mut gpui::Window, cx: &mut gpui::Context<LainApp>) {
    let nth = |n: &str| n.parse::<usize>().unwrap_or(0);
    match cmd {
        "home" => app.navigate(Screen::Home, cx),
        "library" => app.navigate(Screen::Library { read: false, lib: arg.to_string() }, cx),
        "read" => app.navigate(Screen::Library { read: true, lib: arg.to_string() }, cx),
        "downloads" => app.navigate(Screen::Downloads, cx),
        "acquire" => app.navigate(Screen::Acquire, cx),
        "search" => {
            app.navigate(Screen::Search, cx);
            app.set_text("search.q", arg);
            app.run_search(cx);
        }
        "settings" => app.navigate(Screen::Settings(Section::parse(arg).unwrap_or(Section::Account)), cx),
        "palette" => {
            app.run(Act::Palette, window, cx);
            if !arg.is_empty() {
                app.set_text("palette.q", arg);
                let e = app.palette_entries(arg);
                if let Some(p) = app.palette.as_mut() {
                    p.entries = e;
                }
            }
        }
        "item" | "play" | "reader" => {
            let reading = cmd == "reader";
            let list: Vec<usize> =
                (0..app.series.len()).filter(|&i| app.series[i].reading() == reading || cmd == "item").collect();
            let Some(&s) = list.get(nth(arg)) else {
                eprintln!("demo: no title {arg}");
                return;
            };
            let first = app.catalog[app.series[s].items[0]].id.clone();
            match cmd {
                "item" => app.navigate(Screen::Item(first), cx),
                "reader" => app.navigate(Screen::Reader(first), cx),
                _ => {
                    if let Some(id) = app.next_up(s) {
                        app.open_player(&id, None, cx);
                    }
                }
            }
        }
        "controls" => {
            app.demo.controls = true;
            if let Some(p) = app.player.as_mut() {
                p.force_controls = true;
            }
        }
        "menu" => {
            if let Some(p) = app.player.as_mut() {
                p.menu = Some(if arg == "audio" {
                    crate::screens::player::Menu::Audio
                } else {
                    crate::screens::player::Menu::Subs
                });
            }
        }
        "seek" => {
            if let Some(e) = app.player.as_ref().and_then(|p| p.engine.clone()) {
                e.command(&["seek", arg, "absolute"]);
            }
        }
        "sub" => {
            if let Some(e) = app.player.as_ref().and_then(|p| p.engine.clone()) {
                e.command(&["set", "sid", arg]);
            }
        }
        "close" => app.close_player(cx),
        "set" => {
            // set:<field id>=<text> (fills an in-app field)
            if let Some((k, v)) = arg.split_once('=') {
                let key: &'static str = Box::leak(k.to_string().into_boxed_str());
                app.set_text(key, v);
            }
        }
        "kind" => app.settings.new_kind = arg.to_string(),
        "run" => {
            let act = match arg {
                "create-library" => Some(Act::CreateLibrary),
                "scan" => Some(Act::Scan),
                "refresh" => Some(Act::Refresh),
                "set-pin" => Some(Act::SetPin),
                "save-profile" => Some(Act::SaveProfile),
                "save-pathmap" => Some(Act::SavePathMap),
                "download" => {
                    app.item.episodes.ready().and_then(|e| e.get(app.item.sel)).map(|e| Act::Download(e.id.clone()))
                }
                _ => None,
            };
            match act {
                Some(a) => app.run(a, window, cx),
                None => eprintln!("demo: unknown action {arg}"),
            }
        }
        "theme" => app.run(Act::PickTheme(arg.to_string()), window, cx),
        "ext-install" => app.install_extension(std::path::Path::new(arg), cx),
        "ext-allow" => app.run(Act::ExtAllow, window, cx),
        "ext-deny" => app.run(Act::ExtDeny, window, cx),
        "ext-toggle" => app.run(Act::ExtToggle(arg.to_string()), window, cx),
        "ext-scope" => {
            if let Some((id, scope)) = arg.split_once('/') {
                app.run(Act::ExtScope(id.to_string(), scope.to_string()), window, cx);
            }
        }
        "focus" => app.focused = Some(arg.to_string().into()),
        "tab" => app.move_focus(1),
        "key" => {
            // In-app focus movement for screenshots of focus states.
            let ev = gpui::KeyDownEvent {
                keystroke: gpui::Keystroke { modifiers: Default::default(), key: arg.to_string(), key_char: None },
                is_held: false,
            };
            match app.screen {
                Screen::Home => {
                    app.home_key(&ev, window, cx);
                }
                Screen::Library { .. } => {
                    app.library_key(&ev, window, cx);
                }
                _ => app.move_focus(if matches!(arg, "down" | "right") { 1 } else { -1 }),
            }
        }
        other => eprintln!("demo: unknown step {other}"),
    }
}
