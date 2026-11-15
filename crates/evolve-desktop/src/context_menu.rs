//! Native Windows 11 style dark floating context menu.
//! Drawn floating over the screen (not inline) at the mouse cursor position.
//! Features subtle acrylic background, fine border, rounded corners,
//! hover item pills, icons, keyboard shortcut hints, separators,
//! and full-window backdrop dismissal.

use crate::{Message, State, icons};
use iced::widget::{
    button, column, container, mouse_area, pin, row, scrollable, space, stack, text,
};
use iced::{Background, Border, Color, Element, Fill, Length, Point, Vector};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextIcon {
    Eye,
    Edit,
    Copy,
    Folder,
    File,
    Play,
    Unpack,
    Export,
    Search,
    Discard,
}

#[derive(Clone, Debug)]
pub struct ContextMenuItem {
    pub label: String,
    pub icon: Option<ContextIcon>,
    pub shortcut: Option<String>,
    pub action: Option<Message>,
    pub is_separator: bool,
    pub disabled: bool,
}

impl ContextMenuItem {
    pub fn action(label: impl Into<String>, msg: Message) -> Self {
        Self {
            label: label.into(),
            icon: None,
            shortcut: None,
            action: Some(msg),
            is_separator: false,
            disabled: false,
        }
    }

    pub fn with_icon(mut self, icon: ContextIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    #[allow(dead_code)]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            icon: None,
            shortcut: None,
            action: None,
            is_separator: true,
            disabled: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub position: Point,
    pub items: Vec<ContextMenuItem>,
    pub width: f32,
}

impl ContextMenuState {
    pub fn new(position: Point, items: Vec<ContextMenuItem>) -> Self {
        Self {
            position,
            items,
            width: 220.0,
        }
    }

    #[allow(dead_code)]
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            position: Point::new(100.0, 100.0),
            items: Vec::new(),
            width: 220.0,
        }
    }
}

fn icon_handle(icon: ContextIcon) -> iced::widget::svg::Handle {
    match icon {
        ContextIcon::Eye => icons::eye(),
        ContextIcon::Edit => icons::pencil(),
        ContextIcon::Copy => icons::details(),
        ContextIcon::Folder => icons::folder(),
        ContextIcon::File => icons::file(),
        ContextIcon::Play => icons::play(),
        ContextIcon::Unpack => icons::unpack(),
        ContextIcon::Export => icons::export(),
        ContextIcon::Search => icons::search(),
        ContextIcon::Discard => icons::discard(),
    }
}

pub fn overlay<'a>(
    base: impl Into<Element<'a, Message>>,
    state: &'a State,
) -> Element<'a, Message> {
    let base = base.into();
    let Some(ref menu) = state.context_menu else {
        return base;
    };
    if menu.items.is_empty() {
        return base;
    }

    let backdrop = mouse_area(container(space().width(Fill).height(Fill)))
        .on_press(Message::DismissContextMenu)
        .on_right_press(Message::DismissContextMenu);

    let mut list = column![].spacing(2);
    for item in &menu.items {
        if item.is_separator {
            list = list.push(
                container(space().width(Fill).height(Length::Fixed(1.0)))
                    .padding([3, 6])
                    .style(|_theme| container::Style {
                        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08))),
                        ..container::Style::default()
                    }),
            );
            continue;
        }

        let mut row_content = row![].spacing(8).align_y(iced::Alignment::Center);
        if let Some(icon) = item.icon {
            row_content = row_content.push(
                container(icons::glyph(icon_handle(icon), 14.0))
                    .width(Length::Fixed(18.0))
                    .center_x(18.0),
            );
        } else {
            row_content = row_content.push(space().width(Length::Fixed(4.0)));
        }

        row_content = row_content.push(text(item.label.clone()).size(13).width(Length::Fill));

        if let Some(ref shortcut) = item.shortcut {
            row_content =
                row_content.push(text(shortcut.clone()).size(11).style(|_theme| text::Style {
                    color: Some(Color::from_rgb(0.55, 0.55, 0.55)),
                }));
        }

        let disabled = item.disabled || item.action.is_none();
        let action = item.action.clone();
        let btn = button(row_content)
            .width(Length::Fill)
            .padding([5, 8])
            .style(move |_theme, status| {
                let is_hovered = !disabled
                    && matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if is_hovered {
                        Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)))
                    } else {
                        None
                    },
                    text_color: if disabled {
                        Color::from_rgb(0.42, 0.42, 0.44)
                    } else if is_hovered {
                        Color::WHITE
                    } else {
                        Color::from_rgb(0.90, 0.90, 0.92)
                    },
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                }
            })
            .on_press_maybe(
                (!disabled)
                    .then_some(action)
                    .flatten()
                    .map(|msg| Message::ContextAction(Box::new(msg))),
            );

        list = list.push(btn);
    }

    let popup = container(scrollable(list).height(Length::Shrink))
        .padding(4)
        .width(Length::Fixed(menu.width))
        .max_height(480)
        .style(|_theme| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.14, 0.14, 0.15))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.10),
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.50),
                offset: Vector::new(0.0, 6.0),
                blur_radius: 20.0,
            },
            ..container::Style::default()
        });

    let left = menu.position.x.max(8.0);
    let top = menu.position.y.max(8.0);

    stack![base, backdrop, pin(popup).x(left).y(top)].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_overlay_without_active_menu_returns_base() {
        let state = State::default();
        let base: Element<'static, Message> = container(space()).into();
        let _ = overlay(base, &state);
    }

    #[test]
    fn context_menu_items_builder() {
        let item = ContextMenuItem::action("Test", Message::DismissContextMenu)
            .with_icon(ContextIcon::Eye)
            .with_shortcut("Ctrl+T");
        assert_eq!(item.label, "Test");
        assert_eq!(item.icon, Some(ContextIcon::Eye));
        assert_eq!(item.shortcut.as_deref(), Some("Ctrl+T"));
        assert!(!item.is_separator);

        let sep = ContextMenuItem::separator();
        assert!(sep.is_separator);
        assert!(sep.disabled);
    }
}
