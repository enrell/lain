//! The loading screen and the player's chrome over the video.

use iced::widget::{Space, button, column, container, mouse_area, row, shader, slider, stack, text};
use iced::{Background, Border, Color, Element, Fill, Gradient, Padding, gradient, mouse};

use crate::app::{App, Message};
use crate::player::{Menu, Player, fmt_time};
use crate::seekbar::SeekBar;
use crate::theme::{Theme, mix, with_alpha};
use crate::video::VideoView;

pub fn loading(app: &App) -> Element<'_, Message> {
    let t = &app.theme;
    let msg = app.error.clone().unwrap_or_else(|| format!("connecting to {}", app.paths.server));
    container(
        column![
            text("lain").size(t.size_base * 4.0).font(t.font_display).color(t.text),
            text(msg).size(t.size_base * 0.85).font(t.font_mono).color(t.muted),
        ]
        .spacing(t.sp(2))
        .align_x(iced::Center),
    )
    .center(Fill)
    .style(move |_| container::Style { background: Some(Background::Color(t.bg)), ..Default::default() })
    .into()
}

fn chip<'a>(t: &'a Theme, label: String, a: f32, active: bool, msg: Message) -> Element<'a, Message> {
    button(text(label).size(t.size_base * 0.8).font(t.font_mono))
        .padding(Padding::from([t.sp(1), t.sp(2)]))
        .on_press(msg)
        .style(move |_, st| {
            let hot = matches!(st, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(with_alpha(
                    if active { t.accent } else { t.surface },
                    a * if hot { 0.95 } else if active { 0.85 } else { 0.55 },
                ))),
                text_color: with_alpha(t.text, a),
                border: Border { radius: t.radius_button.into(), width: 1.0, color: with_alpha(t.text, a * if hot { 0.35 } else { 0.12 }) },
                ..Default::default()
            }
        })
        .into()
}

fn icon_button<'a>(t: &'a Theme, glyph: &'static str, size: f32, a: f32, msg: Message) -> Element<'a, Message> {
    button(container(text(glyph).size(size).font(t.font_ui).color(with_alpha(t.text, a))).center(size * 2.0))
        .padding(0)
        .on_press(msg)
        .style(move |_, st| button::Style {
            background: Some(Background::Color(with_alpha(
                t.text,
                a * if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.14 } else { 0.0 },
            ))),
            border: Border { radius: (size).into(), ..Border::default() },
            ..Default::default()
        })
        .into()
}

pub fn player<'a>(app: &'a App, p: &'a Player) -> Element<'a, Message> {
    let t = &app.theme;
    let now = app.now;
    let a = p.chrome.interpolate(0.0, 1.0, now);
    let base: Element<'a, Message> = match &p.video {
        Some(v) => shader(VideoView(v.clone())).width(Fill).height(Fill).into(),
        None => Space::new().width(Fill).height(Fill).into(),
    };
    let base = container(base)
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style { background: Some(Background::Color(Color::BLACK)), ..Default::default() });

    let mut layers = stack![base];
    if let Some(e) = &p.error {
        layers = layers.push(
            container(text(format!("Playback failed: {e}")).size(t.size_base).font(t.font_mono).color(t.danger)).center(Fill),
        );
    }
    let visible = a > 0.002;
    if visible {
        layers = layers.push(chrome(app, p, a));
    }
    // Pointer hides with the chrome; the whole area wakes it.
    let interaction = if visible { mouse::Interaction::None } else { mouse::Interaction::Hidden };
    mouse_area(layers.width(Fill).height(Fill)).interaction(interaction).on_double_click(Message::Fullscreen).into()
}

fn chrome<'a>(app: &'a App, p: &'a Player, a: f32) -> Element<'a, Message> {
    let t = &app.theme;
    let bg = t.controls_bg;
    let telemetry = {
        let mut parts = vec![];
        if !p.codec.is_empty() {
            parts.push(p.codec.split_whitespace().next().unwrap_or("").to_string());
        }
        parts.push(if p.hwdec.is_empty() || p.hwdec == "no" { "sw decode".into() } else { format!("hwdec {}", p.hwdec) });
        if let Some(v) = &p.video {
            parts.push((*v.import_mode.lock().unwrap_or_else(|e| e.into_inner())).to_string());
        }
        parts.join("  ·  ")
    };
    let top = container(
        row![
            icon_button(t, "←", t.size_base * 1.2, a, Message::Back),
            Space::new().width(t.sp(3)),
            column![
                text(p.item.title.clone()).size(t.size_base * 1.7).font(t.font_display).color(with_alpha(t.text, a)),
                text(p.item.label()).size(t.size_base * 0.85).font(t.font_mono).color(with_alpha(mix(t.text, t.muted, 0.4), a)),
            ]
            .spacing(2),
            Space::new().width(Fill),
            text(telemetry).size(t.size_base * 0.75).font(t.font_mono).color(with_alpha(mix(t.text, t.muted, 0.5), a)),
        ]
        .align_y(iced::Center),
    )
    .padding(Padding { top: t.sp(4), right: t.sp(5), bottom: t.sp(6), left: t.sp(4) })
    .width(Fill)
    .style(move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(
            gradient::Linear::new(std::f32::consts::PI)
                .add_stop(0.0, with_alpha(bg, a))
                .add_stop(1.0, with_alpha(bg, 0.0)),
        ))),
        ..Default::default()
    });

    let seek = SeekBar {
        position: p.position,
        duration: p.duration,
        buffered: p.buffered,
        track: with_alpha(t.text, a * 0.22),
        buffer: with_alpha(t.text, a * 0.38),
        fill: with_alpha(t.accent, a),
        bar_h: t.seek_h,
        on_seek: Box::new(Message::Seek),
    };
    let time = text(format!("{}  /  {}", fmt_time(p.position), fmt_time(p.duration)))
        .size(t.size_base * 0.85)
        .font(t.font_mono)
        .color(with_alpha(t.text, a));
    let vol = slider(0.0..=130.0, p.volume, Message::Volume).width(110).step(1.0).style(move |_, _| slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(with_alpha(t.text, a * 0.85)), Background::Color(with_alpha(t.text, a * 0.22))),
            width: 3.0,
            border: Border { radius: 2.0.into(), ..Border::default() },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: Background::Color(with_alpha(t.text, a)),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    });
    let audio_n = p.tracks.iter().filter(|x| x.kind == "audio").count();
    let subs_n = p.tracks.iter().filter(|x| x.kind == "sub").count();
    let controls = row![
        icon_button(t, if p.paused { "▶" } else { "❚❚" }, t.size_base * 1.15, a, Message::TogglePause),
        Space::new().width(t.sp(2)),
        time,
        Space::new().width(Fill),
        chip(t, format!("AUDIO {audio_n}"), a, p.menu == Some(Menu::Audio), Message::ToggleMenu(Menu::Audio)),
        Space::new().width(t.sp(1)),
        chip(t, format!("SUBS {subs_n}"), a, p.menu == Some(Menu::Subs), Message::ToggleMenu(Menu::Subs)),
        Space::new().width(t.sp(3)),
        icon_button(t, if p.muted || p.volume < 1.0 { "🔇" } else { "🔊" }, t.size_base, a, Message::ToggleMute),
        vol,
        Space::new().width(t.sp(2)),
        icon_button(t, if app.fullscreen { "🗗" } else { "⛶" }, t.size_base * 1.1, a, Message::Fullscreen),
    ]
    .align_y(iced::Center);
    let bottom = container(column![seek, controls].spacing(t.sp(1)))
        .padding(Padding { top: t.sp(6), right: t.sp(5), bottom: t.sp(3), left: t.sp(5) })
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                gradient::Linear::new(0.0)
                    .add_stop(0.0, with_alpha(bg, a))
                    .add_stop(1.0, with_alpha(bg, 0.0)),
            ))),
            ..Default::default()
        });

    let mut col = column![top, Space::new().height(Fill)];
    if let Some(m) = p.menu {
        col = col.push(track_menu(t, p, m, a));
    }
    col.push(bottom).width(Fill).height(Fill).into()
}

fn track_menu<'a>(t: &'a Theme, p: &'a Player, m: Menu, a: f32) -> Element<'a, Message> {
    let kind = if m == Menu::Audio { "audio" } else { "sub" };
    let mut list = column![text(if m == Menu::Audio { "AUDIO" } else { "SUBTITLES" }).size(t.size_base * 0.7).font(t.font_mono).color(with_alpha(t.muted, a))]
        .spacing(t.sp(1));
    if m == Menu::Subs {
        let none = !p.tracks.iter().any(|x| x.kind == "sub" && x.selected);
        list = list.push(chip(t, "Off".into(), a, none, Message::SetTrack("sub".into(), None)));
    }
    for tr in p.tracks.iter().filter(|x| x.kind == kind) {
        list = list.push(chip(t, tr.label.clone(), a, tr.selected, Message::SetTrack(kind.into(), Some(tr.id))));
    }
    container(
        container(list)
            .padding(t.sp(3))
            .style(move |_| container::Style {
                background: Some(Background::Color(with_alpha(t.bg, 0.92 * a))),
                border: Border { radius: t.radius_card.into(), width: 1.0, color: with_alpha(t.text, 0.1 * a) },
                ..Default::default()
            }),
    )
    .align_right(Fill)
    .padding(Padding { top: 0.0, right: t.sp(5), bottom: 0.0, left: 0.0 })
    .into()
}
