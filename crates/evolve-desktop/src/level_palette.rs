//! The Paint and Vegetation tools' pickers: a strip under the top bar of the
//! map's terrain materials or vegetation types, each with a preview.
use crate::{Message, State, design};
use iced::widget::{button, column, container, image, row, scrollable, text};
use iced::{Background, Border, Element, Fill, Length};

/// A terrain material to paint with.
#[derive(Clone, Debug)]
pub(super) struct SurfaceItem {
    pub id: u8,
    pub name: String,
    pub preview: Option<image::Handle>,
}

/// A kind of vegetation the map has, with an instance to copy.
#[derive(Clone, Debug)]
pub(super) struct VegetationItem {
    pub model: String,
    pub name: String,
    pub count: usize,
    /// A scene object of this kind, the template for painted copies.
    pub origin: usize,
    pub preview: Option<image::Handle>,
}

/// A preview from RGBA pixels.
pub(super) fn handle(thumbnail: &image_crate::RgbaImage) -> image::Handle {
    image::Handle::from_rgba(
        thumbnail.width(),
        thumbnail.height(),
        thumbnail.as_raw().clone(),
    )
}

use ::image as image_crate;

fn swatch<'a>(
    name: String,
    detail: Option<String>,
    preview: Option<image::Handle>,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    let picture: Element<'a, Message> = match preview {
        Some(preview) => image(preview)
            .width(Length::Fixed(48.0))
            .height(Length::Fixed(48.0))
            .into(),
        None => container(text("—").size(14).style(design::muted))
            .center_x(Length::Fixed(48.0))
            .center_y(Length::Fixed(48.0))
            .into(),
    };
    let mut label = column![picture, text(name).size(10).width(Length::Fixed(76.0))]
        .spacing(3)
        .align_x(iced::Alignment::Center);
    if let Some(detail) = detail {
        label = label.push(text(detail).size(9).style(design::muted));
    }
    button(label)
        .padding(4)
        .width(Length::Fixed(86.0))
        .style(if selected {
            design::primary
        } else {
            design::secondary
        })
        .on_press(message)
        .into()
}

/// The picker strip for the active tool, if it has one.
pub(super) fn strip(state: &State) -> Option<Element<'_, Message>> {
    use crate::terrain_brush::LevelTool;
    let items: Vec<Element<'_, Message>> = match state.level_tool {
        LevelTool::Paint => match &state.level_surface_palette {
            Some(palette) => palette
                .iter()
                .map(|item| {
                    swatch(
                        item.name.clone(),
                        None,
                        item.preview.clone(),
                        state.terrain_brush.surface == item.id,
                        Message::LevelPickSurface(item.id),
                    )
                })
                .collect(),
            None => vec![text("Loading the map's terrain materials…").size(12).into()],
        },
        LevelTool::Vegetation => match &state.level_vegetation_palette {
            Some(palette) if !palette.is_empty() => palette
                .iter()
                .map(|item| {
                    swatch(
                        item.name.clone(),
                        Some(format!("{} placed", item.count)),
                        item.preview.clone(),
                        state.level_vegetation_pick.as_deref() == Some(item.model.as_str()),
                        Message::LevelPickVegetation(item.model.clone()),
                    )
                })
                .collect(),
            _ => vec![
                text("This map has no terrain.dat vegetation to paint with.")
                    .size(12)
                    .into(),
            ],
        },
        _ => return None,
    };
    Some(
        container(
            scrollable(row(items).spacing(6).padding([0, 0]))
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(4).scroller_width(4),
                ))
                .width(Fill),
        )
        .padding([6, 11])
        .width(Fill)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into(),
    )
}
