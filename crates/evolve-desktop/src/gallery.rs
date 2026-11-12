//! Texture gallery + lightbox for browsing DDS files in the asset viewer.
//! Thumbnails decode lazily, one page at a time, through the shared
//! `asset_cache` on blocking workers; nothing decoded is written to disk.
//! Clicking a thumbnail opens a full-size lightbox (wheel zoom, drag pan,
//! Esc closes, ←/→ step between textures).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use iced::widget::image::Handle;
use iced::widget::{button, column, container, image, row, space, text};
use iced::{Background, Color, Element, Fill, Length, Task, Theme};

use crate::asset_cache::{SharedCache, decoded_dds};
use crate::design;
use crate::{Message, State};

const PAGE: usize = 48;
const THUMB: u32 = 128;

#[derive(Debug, Clone)]
pub enum Msg {
    Toggle,
    Page(i32),
    Thumb(usize, Result<Handle, String>),
    Open(usize),
    Full(usize, Result<Handle, String>),
    Close,
    Step(i32),
}

#[derive(Debug, Default)]
pub struct GalleryState {
    pub open: bool,
    page: usize,
    thumbs: HashMap<usize, Handle>,
    failed: HashSet<usize>,
    loading: HashSet<usize>,
    /// Inventory index shown in the lightbox.
    pub lightbox: Option<usize>,
    full: Option<(usize, Handle)>,
}

impl GalleryState {
    /// Drop everything when the inventory changes.
    pub fn reset(&mut self) {
        let open = self.open;
        *self = Self {
            open,
            ..Self::default()
        };
    }
}

fn is_texture(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".dds")
}

/// Inventory indices of DDS textures, in inventory order.
pub fn textures(state: &State) -> Vec<usize> {
    state
        .inventory
        .as_ref()
        .map(|inventory| {
            inventory
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| is_texture(&entry.relative_path))
                .map(|(index, _)| index)
                .collect()
        })
        .unwrap_or_default()
}

fn path_of(state: &State, index: usize) -> Option<PathBuf> {
    let inventory = state.inventory.as_ref()?;
    let entry = inventory.entries.get(index)?;
    Some(PathBuf::from(&inventory.root).join(&entry.relative_path))
}

fn decode(cache: SharedCache, path: PathBuf, max: Option<u32>) -> Result<Handle, String> {
    let (_, rgba) = decoded_dds(&cache, &path)?;
    let image = match max {
        Some(max) if rgba.width() > max || rgba.height() > max => ::image::imageops::thumbnail(
            rgba.as_ref(),
            max,
            max * rgba.height() / rgba.width().max(1),
        ),
        _ => (*rgba).clone(),
    };
    Ok(Handle::from_rgba(
        image.width(),
        image.height(),
        image.into_raw(),
    ))
}

fn load(state: &State, index: usize, full: bool) -> Task<Message> {
    let Some(path) = path_of(state, index) else {
        return Task::none();
    };
    let cache = state.preview_cache.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || decode(cache, path, (!full).then_some(THUMB)))
                .await
                .map_err(|e| format!("Texture worker stopped: {e}"))
                .and_then(|result| result)
        },
        move |result| {
            Message::Gallery(if full {
                Msg::Full(index, result)
            } else {
                Msg::Thumb(index, result)
            })
        },
    )
}

/// Queue decodes for the current page's missing thumbnails.
fn load_page(state: &mut State) -> Task<Message> {
    let list = textures(state);
    let start = state.gallery.page * PAGE;
    let wanted: Vec<usize> = list
        .iter()
        .skip(start)
        .take(PAGE)
        .copied()
        .filter(|index| {
            !state.gallery.thumbs.contains_key(index)
                && !state.gallery.failed.contains(index)
                && !state.gallery.loading.contains(index)
        })
        .collect();
    state.gallery.loading.extend(wanted.iter().copied());
    Task::batch(
        wanted
            .into_iter()
            .map(|index| load(state, index, false))
            .collect::<Vec<_>>(),
    )
}

pub fn update(state: &mut State, message: Msg) -> Task<Message> {
    match message {
        Msg::Toggle => {
            state.gallery.open = !state.gallery.open;
            if state.gallery.open {
                return load_page(state);
            }
        }
        Msg::Page(delta) => {
            let pages = textures(state).len().div_ceil(PAGE).max(1);
            state.gallery.page =
                (state.gallery.page as i64 + i64::from(delta)).clamp(0, pages as i64 - 1) as usize;
            return load_page(state);
        }
        Msg::Thumb(index, result) => {
            state.gallery.loading.remove(&index);
            match result {
                Ok(handle) => {
                    state.gallery.thumbs.insert(index, handle);
                }
                Err(_) => {
                    state.gallery.failed.insert(index);
                }
            }
        }
        Msg::Open(index) => {
            state.gallery.lightbox = Some(index);
            if state
                .gallery
                .full
                .as_ref()
                .is_none_or(|(shown, _)| *shown != index)
            {
                state.gallery.full = None;
                return load(state, index, true);
            }
        }
        Msg::Full(index, result) => {
            if state.gallery.lightbox == Some(index) {
                match result {
                    Ok(handle) => state.gallery.full = Some((index, handle)),
                    Err(error) => state.status = format!("Could not open texture: {error}"),
                }
            }
        }
        Msg::Close => state.gallery.lightbox = None,
        Msg::Step(delta) => {
            let list = textures(state);
            if let Some(current) = state.gallery.lightbox
                && let Some(position) = list.iter().position(|index| *index == current)
            {
                let next = (position as i64 + i64::from(delta)).clamp(0, list.len() as i64 - 1);
                return update(state, Msg::Open(list[next as usize]));
            }
        }
    }
    Task::none()
}

fn gallery(message: Msg) -> Message {
    Message::Gallery(message)
}

/// The thumbnail grid that replaces the viewport while the gallery is open.
pub fn view(state: &State) -> Element<'_, Message> {
    let list = textures(state);
    if list.is_empty() {
        return crate::ui_kit::empty_state(
            None,
            "No textures here",
            "Open an extracted folder or PAK that contains DDS textures.",
            Some(("Close gallery", gallery(Msg::Toggle))),
        );
    }
    let pages = list.len().div_ceil(PAGE).max(1);
    let page = state.gallery.page.min(pages - 1);
    let alpha = crate::ui_kit::pulse(
        state.ui.clock(std::time::Instant::now()),
        state.preferences.reduce_motion,
    );
    let mut tiles = row![].spacing(10);
    for &index in list.iter().skip(page * PAGE).take(PAGE) {
        let name = state
            .inventory
            .as_ref()
            .and_then(|inventory| inventory.entries.get(index))
            .map(|entry| {
                entry
                    .relative_path
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .unwrap_or_default();
        let preview: Element<'_, Message> = if let Some(handle) = state.gallery.thumbs.get(&index) {
            image(handle.clone())
                .width(Length::Fixed(THUMB as f32))
                .height(Length::Fixed(THUMB as f32))
                .content_fit(iced::ContentFit::Contain)
                .into()
        } else if state.gallery.failed.contains(&index) {
            container(text("No preview").size(10).style(design::muted))
                .center_x(THUMB as f32)
                .center_y(THUMB as f32)
                .into()
        } else {
            crate::ui_kit::skeleton(Length::Fixed(THUMB as f32), THUMB as f32, alpha)
        };
        tiles = tiles.push(
            button(
                column![
                    preview,
                    text(name).size(10).width(Length::Fixed(THUMB as f32)),
                ]
                .spacing(4),
            )
            .padding(6)
            .style(design::text_button)
            .on_press(gallery(Msg::Open(index))),
        );
    }
    column![
        row![
            text(format!("TEXTURE GALLERY · {} textures", list.len()))
                .size(13)
                .width(Fill),
            design::button("‹").on_press_maybe((page > 0).then(|| gallery(Msg::Page(-1)))),
            text(format!("{} / {pages}", page + 1)).size(12),
            design::button("›").on_press_maybe((page + 1 < pages).then(|| gallery(Msg::Page(1)))),
            design::button("Close gallery")
                .style(design::secondary)
                .on_press(gallery(Msg::Toggle)),
        ]
        .spacing(8)
        .align_y(iced::Center),
        iced::widget::scrollable(tiles.wrap()).height(Fill),
    ]
    .spacing(10)
    .padding(8)
    .height(Fill)
    .into()
}

/// Full-size lightbox laid over the viewer window.
pub fn lightbox<'a>(state: &'a State, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(index) = state.gallery.lightbox else {
        return base;
    };
    let name = path_of(state, index)
        .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let body: Element<'a, Message> = match &state.gallery.full {
        Some((shown, handle)) if *shown == index => image::viewer(handle.clone())
            .width(Fill)
            .height(Fill)
            .min_scale(0.1)
            .max_scale(16.0)
            .into(),
        _ => container(text("Decoding texture…").size(13))
            .center(Fill)
            .into(),
    };
    let bar = row![
        text(name).size(13).width(Fill),
        text("Wheel zoom · drag pan · ←/→ next · Esc close")
            .size(11)
            .style(design::muted),
        design::button("‹").on_press(gallery(Msg::Step(-1))),
        design::button("›").on_press(gallery(Msg::Step(1))),
        design::button("Close")
            .style(design::primary)
            .on_press(gallery(Msg::Close)),
    ]
    .spacing(10)
    .align_y(iced::Center);
    let panel = container(column![bar, body].spacing(10))
        .padding(14)
        .width(Fill)
        .height(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.94,
                ..design::sidebar(theme)
            })),
            ..container::Style::default()
        });
    iced::widget::stack![
        base,
        iced::widget::mouse_area(space().width(Fill).height(Fill)).on_press(gallery(Msg::Close)),
        container(panel).padding(24),
    ]
    .into()
}

/// A full-window image viewer for any image (map thumbnails), with the
/// same frame as the texture lightbox.
pub fn image_lightbox<'a>(
    base: Element<'a, Message>,
    title: String,
    handle: Handle,
    close: Message,
) -> Element<'a, Message> {
    let bar = row![
        text(title).size(13).width(Fill),
        text("Wheel zoom · drag pan · Esc close")
            .size(11)
            .style(design::muted),
        design::button("Close")
            .style(design::primary)
            .on_press(close.clone()),
    ]
    .spacing(10)
    .align_y(iced::Center);
    let panel = container(
        column![
            bar,
            image::viewer(handle)
                .width(Fill)
                .height(Fill)
                .min_scale(0.5)
                .max_scale(16.0)
                .filter_method(image::FilterMethod::Nearest)
        ]
        .spacing(10),
    )
    .padding(14)
    .width(Fill)
    .height(Fill)
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(Color {
            a: 0.94,
            ..design::sidebar(theme)
        })),
        ..container::Style::default()
    });
    iced::widget::stack![
        base,
        iced::widget::mouse_area(space().width(Fill).height(Fill)).on_press(close),
        container(panel).padding(24),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dds_files_are_gallery_textures() {
        assert!(is_texture("Objects/Hunter/body_diff.dds"));
        assert!(is_texture("a/B.DDS"));
        assert!(!is_texture("a/b.dds.1"));
        assert!(!is_texture("a/b.png"));
    }

    #[test]
    fn empty_inventory_has_no_pages_and_toggle_opens() {
        let mut state = State::default();
        assert!(textures(&state).is_empty());
        let _ = update(&mut state, Msg::Toggle);
        assert!(state.gallery.open);
        let _ = update(&mut state, Msg::Page(5));
        assert_eq!(state.gallery.page, 0);
    }
}
