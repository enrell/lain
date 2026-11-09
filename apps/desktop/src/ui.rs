//! Shared widgets. Every interactive widget registers itself in the
//! frame's focus order, so Tab/Shift+Tab (and arrows on list screens)
//! reach it and Enter activates it: keyboard-first (AGENTS.md).
//!
//! Text fields are the app's own: a string, a caret and an
//! all-selected flag, edited from the root key handler. GPUI 0.2.2 has
//! no text input widget; its `EntityInputHandler` example is ~750 lines
//! for IME and selection, which v1 does not need (D-181).

use gpui::prelude::*;
use gpui::{ClickEvent, Context, Div, FontWeight, Hsla, SharedString, Stateful, Window, div, px, svg};

use crate::app::{Act, FocusItem, LainApp};

#[derive(Debug, Clone, Default)]
pub struct Field {
    pub text: String,
    /// Caret position in chars.
    pub caret: usize,
    /// Ctrl+A: the next edit replaces everything.
    pub all: bool,
}

impl Field {
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.caret = self.text.chars().count();
        self.all = false;
    }

    fn byte(&self, ch: usize) -> usize {
        self.text.char_indices().nth(ch).map(|(b, _)| b).unwrap_or(self.text.len())
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn take_all(&mut self) -> bool {
        if self.all {
            self.text.clear();
            self.caret = 0;
            self.all = false;
            true
        } else {
            false
        }
    }

    pub fn insert(&mut self, s: &str) {
        self.take_all();
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        let at = self.byte(self.caret);
        self.text.insert_str(at, &s);
        self.caret += s.chars().count();
    }

    /// Applies an editing key; returns whether it was one.
    pub fn key(&mut self, key: &str, ctrl: bool, clip: Option<String>) -> bool {
        match key {
            "backspace" => {
                if !self.take_all() && self.caret > 0 {
                    let mut from = self.caret - 1;
                    if ctrl {
                        let chars: Vec<char> = self.text.chars().collect();
                        while from > 0 && chars[from - 1].is_whitespace() {
                            from -= 1;
                        }
                        while from > 0 && !chars[from - 1].is_whitespace() {
                            from -= 1;
                        }
                    }
                    let (a, b) = (self.byte(from), self.byte(self.caret));
                    self.text.replace_range(a..b, "");
                    self.caret = from;
                }
            }
            "delete" => {
                if !self.take_all() && self.caret < self.len() {
                    let (a, b) = (self.byte(self.caret), self.byte(self.caret + 1));
                    self.text.replace_range(a..b, "");
                }
            }
            "left" => {
                self.all = false;
                self.caret = self.caret.saturating_sub(1);
            }
            "right" => {
                self.all = false;
                self.caret = (self.caret + 1).min(self.len());
            }
            "home" => {
                self.all = false;
                self.caret = 0;
            }
            "end" => {
                self.all = false;
                self.caret = self.len();
            }
            "a" if ctrl => self.all = !self.text.is_empty(),
            "v" if ctrl => {
                if let Some(c) = clip {
                    self.insert(c.lines().next().unwrap_or(""));
                }
            }
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Ghost,
    Quiet,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// 501 not-ported.
    NotPorted,
    /// The module is off on this server.
    Disabled,
    Offline,
    Empty,
    Error,
    Loading,
}

impl LainApp {
    /// Registers a focus stop for this frame.
    pub fn stop(&mut self, id: &str, act: Option<Act>, field: bool) -> bool {
        self.focus_order.push(FocusItem { id: SharedString::from(id.to_string()), act, field });
        self.focused.as_ref().map(|s| s.as_ref()) == Some(id)
    }

    /// A mono uppercase label (D-022).
    pub fn label(&self, text: impl Into<SharedString>) -> Div {
        div()
            .font_family(self.fonts.mono.clone())
            .text_size(px(self.theme.ts(-2)))
            .text_color(self.pal.muted)
            .child(text.into().to_uppercase())
    }

    pub fn kbd(&self, k: &str) -> Div {
        div()
            .px(px(5.))
            .py(px(1.))
            .rounded(px(4.))
            .border_1()
            .border_color(self.pal.line)
            .font_family(self.fonts.mono.clone())
            .text_size(px(self.theme.ts(-2)))
            .text_color(self.pal.muted)
            .child(k.to_string())
    }

    pub fn icon(&self, name: &str, size: f32, color: Hsla) -> gpui::Svg {
        svg().path(self.icon_path(name)).size(px(size)).text_color(color)
    }

    /// `icons/<name>.svg`, with the theme generation so overrides apply
    /// on reload (GPUI caches SVG renders by path).
    pub fn icon_path(&self, name: &str) -> SharedString {
        SharedString::from(format!("icons/{name}.svg?g={}", self.theme_gen))
    }

    /// A button that is also a focus stop. `kbd` is shown next to it.
    pub fn btn(
        &mut self,
        id: &str,
        text: impl Into<SharedString>,
        kbd: Option<&str>,
        tone: Tone,
        act: Act,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = self.stop(id, Some(act.clone()), false);
        let (pal, th) = (&self.pal, &self.theme);
        let (bg, fg, border) = match tone {
            Tone::Primary => (pal.accent, pal.on_accent, pal.accent),
            Tone::Ghost => (pal.text.opacity(0.06), pal.text, pal.line),
            Tone::Quiet => (pal.text.opacity(0.), pal.text.opacity(0.85), pal.text.opacity(0.)),
            Tone::Danger => (pal.danger.opacity(0.14), pal.danger, pal.danger.opacity(0.4)),
        };
        let mut b = div()
            .id(SharedString::from(id.to_string()))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(th.sp(1)))
            .h(px(th.ts(0) * 2.4))
            .px(px(th.sp(3)))
            .rounded(px(th.radius.button))
            .bg(bg)
            .text_color(fg)
            .font_weight(FontWeight::MEDIUM)
            .border_1()
            .border_color(if focused { pal.focus } else { border })
            .cursor_pointer()
            .hover(|s| s.opacity(0.88))
            .child(text.into());
        if focused {
            b = b.shadow(vec![gpui::BoxShadow {
                color: pal.focus.opacity(0.45),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
            }]);
        }
        if let Some(k) = kbd {
            b = b.child(self.kbd(k).text_color(fg.opacity(0.7)).border_color(fg.opacity(0.25)));
        }
        b.on_click(cx.listener(move |app, _: &ClickEvent, w, cx| {
            app.run(act.clone(), w, cx);
        }))
    }

    /// A one-line text field; Enter runs `submit`.
    pub fn field(
        &mut self,
        id: &'static str,
        placeholder: &str,
        secret: bool,
        submit: Act,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = self.stop(id, Some(submit), true);
        let f = self.fields.entry(id).or_default().clone();
        let (pal, th) = (&self.pal, &self.theme);
        let shown: String = if secret { "•".repeat(f.text.chars().count()) } else { f.text.clone() };
        let caret_b = shown.char_indices().nth(f.caret).map(|(b, _)| b).unwrap_or(shown.len());
        let (before, after) = shown.split_at(caret_b);
        let mut inner = div().flex().items_center().overflow_hidden().whitespace_nowrap();
        if f.text.is_empty() && !focused {
            inner = inner.child(div().text_color(pal.muted).child(placeholder.to_string()));
        } else if f.all && focused {
            inner = inner.child(div().bg(pal.accent.opacity(0.35)).child(shown.clone()));
        } else {
            inner = inner.child(div().child(before.to_string()));
            if focused {
                inner = inner.child(div().w(px(1.5)).h(px(th.ts(0) * 1.2)).bg(pal.accent));
            }
            inner = inner.child(div().child(after.to_string()));
            if f.text.is_empty() {
                inner = inner.child(div().pl(px(4.)).text_color(pal.muted).child(placeholder.to_string()));
            }
        }
        div()
            .id(id)
            .flex()
            .items_center()
            .h(px(th.ts(0) * 2.5))
            .px(px(th.sp(2)))
            .rounded(px(th.radius.button))
            .bg(pal.text.opacity(0.05))
            .border_1()
            .border_color(if focused { pal.focus } else { pal.line })
            .cursor_text()
            .child(inner)
            .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                app.focused = Some(SharedString::from(id));
                cx.notify();
            }))
    }

    pub fn text(&self, id: &'static str) -> String {
        self.fields.get(id).map(|f| f.text.clone()).unwrap_or_default()
    }

    pub fn set_text(&mut self, id: &'static str, s: &str) {
        self.fields.entry(id).or_default().set(s);
    }

    /// A glass panel (D-022): the theme's `raised` color, a hairline and
    /// the panel radius.
    pub fn panel(&self) -> Div {
        div().bg(self.pal.raised).border_1().border_color(self.pal.line).rounded(px(self.theme.radius.panel))
    }

    /// One settings row (D-087): label + one-line hint left, control right.
    pub fn row(&self, label: &str, hint: &str, control: impl IntoElement) -> Div {
        let th = &self.theme;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(th.sp(4)))
            .py(px(th.sp(3)))
            .border_b_1()
            .border_color(self.pal.line)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w(px(160.))
                    .flex_1()
                    .child(div().font_weight(FontWeight::MEDIUM).child(label.to_string()))
                    .child(div().text_size(px(th.ts(-1))).text_color(self.pal.muted).child(hint.to_string())),
            )
            .child(div().flex().flex_none().items_center().gap(px(th.sp(2))).child(control))
    }

    /// A screen-level state: not ported, module off, offline, empty...
    pub fn notice(&self, kind: Notice, title: &str, body: &str) -> Div {
        let th = &self.theme;
        let (icon, color) = match kind {
            Notice::NotPorted => ("clock", self.pal.warn),
            Notice::Disabled => ("off", self.pal.muted),
            Notice::Offline => ("offline", self.pal.danger),
            Notice::Empty => ("empty", self.pal.muted),
            Notice::Error => ("alert", self.pal.danger),
            Notice::Loading => ("clock", self.pal.muted),
        };
        let tag = match kind {
            Notice::NotPorted => "not available yet",
            Notice::Disabled => "module disabled",
            Notice::Offline => "offline",
            Notice::Empty => "empty",
            Notice::Error => "error",
            Notice::Loading => "loading",
        };
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(th.sp(2)))
            .max_w(px(560.))
            .p(px(th.sp(5)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(2)))
                    .child(self.icon(icon, 16., color))
                    .child(self.label(tag).text_color(color)),
            )
            .child(
                div()
                    .font_family(self.fonts.display.clone())
                    .text_size(px(th.ts(3)))
                    .line_height(px(th.ts(3) * 1.15))
                    .child(title.to_string()),
            )
            .child(div().text_color(self.pal.muted).child(body.to_string()))
    }

    /// A notice for a failed call, mapped from its error.
    pub fn error_notice(&self, what: &str, e: &lain_client::ClientError) -> Div {
        use lain_client::ClientError as E;
        match e {
            E::NotPorted { .. } => self.notice(
                Notice::NotPorted,
                &format!("{what} is not available on this server yet"),
                "This server answers 501 not-ported for it: the module is still being moved to the new server. It shows up here as soon as the server serves it.",
            ),
            E::ModuleDisabled { module } => self.notice(
                Notice::Disabled,
                &format!("The {module} module is off"),
                "An admin turned this module off on the server (Settings → Modules). Its screens come back when it is on again.",
            ),
            E::Offline { .. } | E::Timeout => self.notice(
                Notice::Offline,
                "The server is unreachable",
                "Offline copies still play from Downloads. This screen reloads when the connection is back.",
            ),
            other => self.notice(Notice::Error, &format!("{what} failed"), &crate::backend::reason(other)),
        }
    }

    pub fn section_title(&self, text: &str) -> Div {
        div()
            .font_family(self.fonts.display.clone())
            .text_size(px(self.theme.ts(2)))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text.to_string())
    }

    /// A small status pill.
    pub fn pill(&self, text: &str, color: Hsla) -> Div {
        div()
            .px(px(8.))
            .py(px(2.))
            .rounded_full()
            .bg(color.opacity(0.14))
            .text_color(color)
            .font_family(self.fonts.mono.clone())
            .text_size(px(self.theme.ts(-2)))
            .child(text.to_uppercase())
    }
}

/// `1:02:03` or `2:03`.
pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn fmt_bytes(b: u64) -> String {
    let f = b as f64;
    if f >= 1e9 {
        format!("{:.1} GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.0} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.0} kB", f / 1e3)
    } else {
        format!("{b} B")
    }
}

/// A stable hue for a title (placeholder art).
pub fn title_hue(s: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    (h % 360) as f32
}

/// Window helper used by screens that need the viewport.
pub fn viewport(window: &Window) -> (f32, f32) {
    let vs = window.viewport_size();
    (f32::from(vs.width), f32::from(vs.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_edits() {
        let mut f = Field::default();
        f.insert("héllo world");
        assert_eq!(f.caret, 11);
        f.key("backspace", true, None);
        assert_eq!(f.text, "héllo ");
        f.key("home", false, None);
        f.key("delete", false, None);
        assert_eq!(f.text, "éllo ");
        f.key("a", true, None);
        f.insert("x");
        assert_eq!(f.text, "x");
        f.key("v", true, Some("pasted\nsecond".into()));
        assert_eq!(f.text, "xpasted");
    }
}
