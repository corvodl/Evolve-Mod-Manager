//! A popup anchored under its trigger (menubar menus, the build dropdown).
//!
//! The popup is drawn as an overlay at the trigger's real on-screen
//! position, so it follows the layout through font, theme and UI scale
//! changes instead of being placed at hand-measured x offsets. A press
//! outside both the trigger and the popup sends `dismiss`.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

pub struct Anchored<'a, Message> {
    base: Element<'a, Message>,
    popup: Option<Element<'a, Message>>,
    dismiss: Message,
    gap: f32,
}

/// Shows `popup` (when `Some`) directly below `base`.
pub fn anchored<'a, Message: Clone + 'a>(
    base: impl Into<Element<'a, Message>>,
    popup: Option<Element<'a, Message>>,
    dismiss: Message,
) -> Anchored<'a, Message> {
    Anchored {
        base: base.into(),
        popup,
        dismiss,
        gap: 2.0,
    }
}

impl<'a, Message: Clone + 'a> Widget<Message, Theme, Renderer> for Anchored<'a, Message> {
    fn children(&self) -> Vec<Tree> {
        let mut children = vec![Tree::new(&self.base)];
        if let Some(popup) = &self.popup {
            children.push(Tree::new(popup));
        }
        children
    }

    fn diff(&self, tree: &mut Tree) {
        match &self.popup {
            Some(popup) => tree.diff_children(&[&self.base, popup]),
            None => tree.diff_children(&[&self.base]),
        }
    }

    fn size(&self) -> Size<Length> {
        self.base.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.base.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.base
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
        self.base.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.base.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.base.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.base
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let mut children = tree.children.iter_mut();
        let base_tree = children.next()?;
        let base = self.base.as_widget_mut().overlay(
            base_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let bounds = layout.bounds();
        let popup = match (&mut self.popup, children.next()) {
            (Some(popup), Some(popup_tree)) => Some(overlay::Element::new(Box::new(Popup {
                content: popup,
                tree: popup_tree,
                anchor: Rectangle {
                    x: bounds.x + translation.x,
                    y: bounds.y + translation.y,
                    ..bounds
                },
                gap: self.gap,
                dismiss: self.dismiss.clone(),
            }))),
            _ => None,
        };
        if base.is_none() && popup.is_none() {
            return None;
        }
        Some(overlay::Group::with_children(base.into_iter().chain(popup).collect()).overlay())
    }
}

struct Popup<'a, 'b, Message> {
    content: &'b mut Element<'a, Message>,
    tree: &'b mut Tree,
    /// The trigger's on-screen bounds.
    anchor: Rectangle,
    gap: f32,
    dismiss: Message,
}

impl<Message: Clone> overlay::Overlay<Message, Theme, Renderer> for Popup<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let below = self.anchor.y + self.anchor.height + self.gap;
        let limits = layout::Limits::new(
            Size::ZERO,
            Size::new(bounds.width, (bounds.height - below).max(80.0)),
        );
        let node = self
            .content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);
        let size = node.size();
        // Keep the popup on screen: shift left at the right edge.
        let x = self
            .anchor
            .x
            .min((bounds.width - size.width).max(0.0))
            .max(0.0);
        node.move_to(Point::new(x, below))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.content.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &Rectangle::with_size(Size::INFINITE),
        );
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event
            && !cursor.is_over(layout.bounds())
            && !cursor.is_over(self.anchor)
        {
            shell.publish(self.dismiss.clone());
            return;
        }
        self.content.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(self.tree, layout, renderer, operation);
    }
}

impl<'a, Message: Clone + 'a> From<Anchored<'a, Message>> for Element<'a, Message> {
    fn from(anchored: Anchored<'a, Message>) -> Self {
        Element::new(anchored)
    }
}
