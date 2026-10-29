//! Compact, Blender-style floating menus shared by Modding, Asset Library
//! and the model viewer. The open menu is a Stack overlay: unlike an inline
//! row it never changes the viewport, explorer or table's layout geometry.
use crate::{Message, design};
use iced::widget::{container, mouse_area, pin, scrollable, space, stack};
use iced::{Background, Border, Element, Fill, Length};

/// X is the left edge of the selected menu label; Y is just below its bar.
/// Keep menus within a modest height so long command sets scroll vertically.
pub fn overlay<'a>(
    base: impl Into<Element<'a, Message>>,
    opened: Option<Element<'a, Message>>,
    left: f32,
    top: f32,
    menu_width: f32,
) -> Element<'a, Message> {
    let base = base.into();
    let Some(items) = opened else {
        return base;
    };
    let backdrop =
        mouse_area(container(space().width(Fill).height(Fill))).on_press(Message::DismissMenus);
    // Flat, rectangular Windows-desktop context menus: dense items, a
    // single crisp border and no rounded floating-card silhouette.
    // Preserve Iced's accessible menu actions without native Win32 unsafe.
    let popup = container(scrollable(items).height(Length::Shrink))
        .padding(4)
        .width(Length::Fixed(menu_width))
        .max_height(440)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });
    stack![base, backdrop, pin(popup).x(left).y(top)].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_without_selection_does_not_change_page_layout() {
        let _: Element<'static, Message> =
            overlay(container("Content").height(Fill), None, 0.0, 35.0, 280.0);
    }
}
