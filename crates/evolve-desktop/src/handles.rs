//! Image and SVG handles built once and reused.
//!
//! iced caches decoded images and rasterised SVGs by handle ID, and every
//! `Handle::from_bytes` / `from_memory` call makes a new ID. Building a
//! handle inside `view` therefore re-decodes the asset on every redraw;
//! while an animation (the Home embers, a hover tween) redraws the window
//! that showed up as the logo flashing. Clones keep the ID, so these
//! helpers hand out clones of one handle per asset.

use iced::widget::image::Handle as ImageHandle;
use iced::widget::svg::Handle as SvgHandle;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// An embedded image, keyed by the address of its bytes.
pub fn image(bytes: &'static [u8]) -> ImageHandle {
    static CACHE: LazyLock<Mutex<HashMap<usize, ImageHandle>>> = LazyLock::new(Default::default);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(bytes.as_ptr() as usize)
        .or_insert_with(|| ImageHandle::from_bytes(bytes))
        .clone()
}

/// An embedded SVG file, keyed by the address of its bytes.
pub fn svg(bytes: &'static [u8]) -> SvgHandle {
    static CACHE: LazyLock<Mutex<HashMap<usize, SvgHandle>>> = LazyLock::new(Default::default);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(bytes.as_ptr() as usize)
        .or_insert_with(|| SvgHandle::from_memory(bytes))
        .clone()
}

/// An SVG built from generated markup, keyed by the markup itself.
pub fn svg_markup(markup: String) -> SvgHandle {
    static CACHE: LazyLock<Mutex<HashMap<String, SvgHandle>>> = LazyLock::new(Default::default);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(handle) = cache.get(&markup) {
        return handle.clone();
    }
    let handle = SvgHandle::from_memory(markup.clone().into_bytes());
    cache.insert(markup, handle.clone());
    handle
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_same_asset_keeps_one_handle_id() {
        static BYTES: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
        assert_eq!(super::svg(BYTES).id(), super::svg(BYTES).id());
        assert_eq!(
            super::svg_markup("<svg/>".into()).id(),
            super::svg_markup("<svg/>".into()).id()
        );
        assert_eq!(super::image(BYTES).id(), super::image(BYTES).id());
    }
}
