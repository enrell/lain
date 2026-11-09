//! The player's seek bar: track, buffered range, position and a knob, as
//! rounded quads in a custom widget (no canvas tessellation per frame).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Border, Color, Element, Event, Length, Rectangle, Size, mouse};

pub struct SeekBar<'a, Message> {
    pub position: f64,
    pub duration: f64,
    pub buffered: f64,
    pub track: Color,
    pub buffer: Color,
    pub fill: Color,
    pub bar_h: f32,
    pub on_seek: Box<dyn Fn(f64) -> Message + 'a>,
}

#[derive(Default)]
struct State {
    dragging: bool,
    hover: bool,
}

impl<'a, Message> SeekBar<'a, Message> {
    fn frac_at(&self, bounds: Rectangle, x: f32) -> f64 {
        (((x - bounds.x) / bounds.width.max(1.0)) as f64).clamp(0.0, 1.0)
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for SeekBar<'a, Message>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size { width: Length::Fill, height: Length::Fixed(self.bar_h * 2.0 + 14.0) }
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let h = self.bar_h * 2.0 + 14.0;
        layout::Node::new(limits.width(Length::Fill).height(h).resolve(Length::Fill, h, Size::ZERO))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let b = layout.bounds();
        let over = cursor.is_over(b);
        if over != st.hover {
            st.hover = over;
            shell.request_redraw();
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if over => {
                if let Some(p) = cursor.position() {
                    st.dragging = true;
                    shell.publish((self.on_seek)(self.frac_at(b, p.x) * self.duration));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if st.dragging => {
                shell.publish((self.on_seek)(self.frac_at(b, position.x) * self.duration));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if st.dragging => {
                st.dragging = false;
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<State>();
        let b = layout.bounds();
        let active = st.hover || st.dragging;
        let h = if active { self.bar_h * 1.75 } else { self.bar_h };
        let y = b.y + (b.height - h) / 2.0;
        let radius = h / 2.0;
        let d = self.duration.max(0.001);
        let quad = |x: f32, w: f32| Quad {
            bounds: Rectangle { x, y, width: w.max(0.0), height: h },
            border: Border { radius: radius.into(), ..Border::default() },
            ..Quad::default()
        };
        renderer.fill_quad(quad(b.x, b.width), self.track);
        let bw = (self.buffered / d).clamp(0.0, 1.0) as f32 * b.width;
        renderer.fill_quad(quad(b.x, bw), self.buffer);
        let pw = (self.position / d).clamp(0.0, 1.0) as f32 * b.width;
        renderer.fill_quad(quad(b.x, pw), self.fill);
        if active {
            let k = h * 2.6;
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle { x: b.x + pw - k / 2.0, y: b.y + (b.height - k) / 2.0, width: k, height: k },
                    border: Border { radius: (k / 2.0).into(), ..Border::default() },
                    ..Quad::default()
                },
                self.fill,
            );
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) { mouse::Interaction::Pointer } else { mouse::Interaction::None }
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<SeekBar<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(s: SeekBar<'a, Message>) -> Self {
        Element::new(s)
    }
}
