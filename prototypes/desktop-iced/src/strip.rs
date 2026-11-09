//! `Strip`: children at explicit x positions inside a clipped, fixed-height
//! band, drawn in one layer. iced has no virtualized list, so a rail only
//! builds the dozen cards in view and places them itself; `Stack` + `pin`
//! would work too, but `Stack` opens a render layer per child.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

pub struct Strip<'a, Message, Theme, Renderer> {
    xs: Vec<f32>,
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    height: f32,
}

impl<'a, Message, Theme, Renderer> Strip<'a, Message, Theme, Renderer> {
    pub fn new(height: f32) -> Self {
        Strip { xs: Vec::new(), children: Vec::new(), height }
    }

    pub fn push(mut self, x: f32, child: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        self.xs.push(x);
        self.children.push(child.into());
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Strip<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size { width: Length::Fill, height: Length::Fixed(self.height) }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let size = limits.width(Length::Fill).height(self.height).resolve(Length::Fill, self.height, Size::ZERO);
        let child_limits = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, self.height));
        let nodes = self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(&self.xs)
            .map(|((c, t), &x)| c.as_widget_mut().layout(t, renderer, &child_limits).move_to(Point::new(x, 0.0)))
            .collect();
        layout::Node::with_children(size, nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for ((c, t), l) in self.children.iter_mut().zip(&mut tree.children).zip(layout.children()) {
            c.as_widget_mut().update(t, event, l, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(clip) = layout.bounds().intersection(viewport) else { return };
        renderer.with_layer(clip, |renderer| {
            for ((c, t), l) in self.children.iter().zip(&tree.children).zip(layout.children()) {
                if l.bounds().intersects(&clip) {
                    c.as_widget().draw(t, renderer, theme, style, l, cursor, &clip);
                }
            }
        });
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        mouse::Interaction::None
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(&mut self.children, tree, layout, renderer, viewport, translation)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |op| {
            for ((c, t), l) in self.children.iter_mut().zip(&mut tree.children).zip(layout.children()) {
                c.as_widget_mut().operate(t, l, renderer, op);
            }
        });
    }
}

impl<'a, Message, Theme, Renderer> From<Strip<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(s: Strip<'a, Message, Theme, Renderer>) -> Self {
        Element::new(s)
    }
}
