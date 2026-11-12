//! The app's outline icon set: 24×24, 1.65 stroke, round caps and joins,
//! the same style as `assets/nav`. These are drawn for this app rather than
//! copied from a third-party set, so there is no licence to track.
//! [`glyph`] draws one through `design::icon`, which recolours it to the
//! theme's text colour.

use iced::widget::Svg;
use iced::widget::svg;

fn icon(body: &str) -> svg::Handle {
    crate::handles::svg_markup(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#D6454D" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
    ))
}

pub fn frame_all() -> svg::Handle {
    icon(
        r#"<path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4"/><circle cx="12" cy="12" r="2.5"/>"#,
    )
}

pub fn view_front() -> svg::Handle {
    icon(r#"<rect x="5" y="5" width="14" height="14" rx="1.5"/><path d="M12 9v6M9 12h6"/>"#)
}

pub fn view_right() -> svg::Handle {
    icon(r#"<path d="M4 6l7-2v16l-7-2z"/><path d="M14 12h6M17 9l3 3-3 3"/>"#)
}

pub fn view_top() -> svg::Handle {
    icon(r#"<path d="M12 3v8M9 8l3 3 3-3"/><path d="m12 13 8 4-8 4-8-4z"/>"#)
}

pub fn view_iso() -> svg::Handle {
    icon(r#"<path d="m12 2 9 5-9 5-9-5 9-5zM3 7v10l9 5 9-5V7"/><path d="M12 12v10"/>"#)
}

pub fn wireframe() -> svg::Handle {
    icon(
        r#"<path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 3v18M4 7.5l16 9M20 7.5l-16 9"/>"#,
    )
}

pub fn zoom_in() -> svg::Handle {
    icon(r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2M11 8v6M8 11h6"/>"#)
}

pub fn zoom_out() -> svg::Handle {
    icon(r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2M8 11h6"/>"#)
}

pub fn fullscreen() -> svg::Handle {
    icon(r#"<path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5"/>"#)
}

pub fn chevron() -> svg::Handle {
    icon(r#"<path d="m9 6 6 6-6 6"/>"#)
}

pub fn search() -> svg::Handle {
    icon(r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>"#)
}

pub fn close() -> svg::Handle {
    icon(r#"<path d="M6 6l12 12M18 6 6 18"/>"#)
}

pub fn check() -> svg::Handle {
    icon(r#"<path d="m5 12.5 4.5 4.5L19 7.5"/>"#)
}

pub fn error() -> svg::Handle {
    icon(r#"<circle cx="12" cy="12" r="8.5"/><path d="m9 9 6 6M15 9l-6 6"/>"#)
}

pub fn warning() -> svg::Handle {
    icon(r#"<path d="M12 3.5 21 19.5H3z"/><path d="M12 10v4.5M12 17.2v.1"/>"#)
}

pub fn info() -> svg::Handle {
    icon(r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 7.8v.1"/>"#)
}

pub fn plus() -> svg::Handle {
    icon(r#"<path d="M12 5v14M5 12h14"/>"#)
}

pub fn minus() -> svg::Handle {
    icon(r#"<path d="M5 12h14"/>"#)
}

pub fn pin() -> svg::Handle {
    icon(r#"<path d="M9 4h6l-1 6 3 3H7l3-3z"/><path d="M12 13v7"/>"#)
}

pub fn folder() -> svg::Handle {
    icon(
        r#"<path d="M3.5 7a1.5 1.5 0 0 1 1.5-1.5h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/>"#,
    )
}

pub fn file() -> svg::Handle {
    icon(r#"<path d="M6.5 3.5h7l4 4v13h-11z"/><path d="M13.5 3.5v4h4"/>"#)
}

pub fn details() -> svg::Handle {
    icon(
        r#"<rect x="3.5" y="4.5" width="17" height="15" rx="1.5"/><path d="M14 4.5v15M16.5 9h1.5M16.5 12h1.5"/>"#,
    )
}

pub fn shield() -> svg::Handle {
    icon(r#"<path d="M12 3.5 19 6v5.5c0 4.2-3 7.4-7 9-4-1.6-7-4.8-7-9V6z"/>"#)
}

pub fn flask() -> svg::Handle {
    icon(
        r#"<path d="M9.5 3.5h5M10.5 3.5v6L5 19a1 1 0 0 0 .9 1.5h12.2A1 1 0 0 0 19 19l-5.5-9.5v-6"/><path d="M7.5 15h9"/>"#,
    )
}

// Level editor tools.

pub fn cursor() -> svg::Handle {
    icon(r#"<path d="M6 3.5 18 10l-5.2 1.7-2.3 5.3z"/><path d="m12.8 11.7 4.7 4.8"/>"#)
}

pub fn mountain() -> svg::Handle {
    icon(r#"<path d="M3 19.5 9.5 8l3.5 6 2.5-4 5.5 9.5z"/><path d="m8 10.6 1.5 1.4 1.4-1.3"/>"#)
}

pub fn brush() -> svg::Handle {
    icon(
        r#"<path d="M19.5 4.5 11 13"/><path d="M11 13c-2.8-.6-5.2 1-5.5 3.6L5 20l3.4-.4C11 19.2 12.6 16 11 13z"/>"#,
    )
}

pub fn raise() -> svg::Handle {
    icon(r#"<path d="M3.5 19.5h17"/><path d="M12 15.5V5M8 9l4-4 4 4"/>"#)
}

pub fn flatten() -> svg::Handle {
    icon(r#"<path d="M3.5 19.5h17M5 13.5h14"/><path d="M12 4v6M9.5 7.5 12 10l2.5-2.5"/>"#)
}

pub fn smooth() -> svg::Handle {
    icon(r#"<path d="M3.5 19.5h17"/><path d="M3.5 13c3-4 5-4 8.5 0s5.5 4 8.5 0"/>"#)
}

pub fn create_ground() -> svg::Handle {
    icon(r#"<path d="M3.5 19.5h17M6 15.5h12"/><path d="M12 4v8M8 8h8"/>"#)
}

pub fn undo() -> svg::Handle {
    icon(r#"<path d="M9 14 4.5 9.5 9 5"/><path d="M4.5 9.5H14a5.5 5.5 0 0 1 0 11h-3"/>"#)
}

pub fn redo() -> svg::Handle {
    icon(r#"<path d="m15 14 4.5-4.5L15 5"/><path d="M19.5 9.5H10a5.5 5.5 0 0 0 0 11h3"/>"#)
}

pub fn discard() -> svg::Handle {
    icon(r#"<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5"/><path d="M10 11v5M14 11v5"/>"#)
}

pub fn keyboard() -> svg::Handle {
    icon(
        r#"<rect x="2.5" y="6" width="19" height="12" rx="1.5"/><path d="M6 9.5h.1M9.5 9.5h.1M13 9.5h.1M16.5 9.5h.1M6 12.5h.1M18 12.5h.1M8 15h8"/>"#,
    )
}

pub fn help() -> svg::Handle {
    icon(
        r#"<circle cx="12" cy="12" r="8.5"/><path d="M9.6 9.5a2.5 2.5 0 1 1 3.4 2.3c-.7.3-1 .8-1 1.5v.4M12 16.6v.1"/>"#,
    )
}

pub fn tree() -> svg::Handle {
    icon(r#"<path d="M12 3.5 6 12h3l-3.5 5h13L15 12h3z"/><path d="M12 17v3.5"/>"#)
}

pub fn rotate() -> svg::Handle {
    icon(r#"<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.5 4v4.5H15"/>"#)
}

pub fn scale() -> svg::Handle {
    icon(
        r#"<rect x="3.5" y="12.5" width="8" height="8" rx="1"/><path d="M11 13 20 4M14.5 4H20v5.5"/>"#,
    )
}

pub fn move_tool() -> svg::Handle {
    icon(
        r#"<path d="M12 3v18M3 12h18"/><path d="m9 6 3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3"/>"#,
    )
}

// Modding actions.

pub fn pencil() -> svg::Handle {
    icon(r#"<path d="M15.5 4.5 19.5 8.5 9 19H5v-4z"/><path d="m13.5 6.5 4 4"/>"#)
}

pub fn map() -> svg::Handle {
    icon(
        r#"<path d="m3.5 6.5 5.5-2.5 6 2.5 5.5-2.5v13.5L15 20l-6-2.5-5.5 2.5z"/><path d="M9 4v13.5M15 6.5V20"/>"#,
    )
}

pub fn export() -> svg::Handle {
    icon(
        r#"<path d="M12 15V4M8 8l4-4 4 4"/><path d="M5 13v5.5a1.5 1.5 0 0 0 1.5 1.5h11a1.5 1.5 0 0 0 1.5-1.5V13"/>"#,
    )
}

pub fn play() -> svg::Handle {
    icon(r#"<path d="M8 5.5v13l10-6.5z"/>"#)
}

pub fn refresh() -> svg::Handle {
    icon(r#"<path d="M19.5 5v5h-5"/><path d="M19 10a7.5 7.5 0 1 0 .4 5"/>"#)
}

pub fn unpack() -> svg::Handle {
    icon(
        r#"<path d="M3.5 9.5 12 5l8.5 4.5L12 14z"/><path d="M3.5 9.5v7L12 21l8.5-4.5v-7M12 14v7"/>"#,
    )
}

pub fn chevron_left() -> svg::Handle {
    icon(r#"<path d="m15 6-6 6 6 6"/>"#)
}

pub fn chevron_up() -> svg::Handle {
    icon(r#"<path d="m6 15 6-6 6 6"/>"#)
}

pub fn chevron_down() -> svg::Handle {
    icon(r#"<path d="m6 9 6 6 6-6"/>"#)
}

pub fn lock() -> svg::Handle {
    icon(
        r#"<rect x="5" y="10.5" width="14" height="10" rx="1.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>"#,
    )
}

pub fn eye() -> svg::Handle {
    icon(
        r#"<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>"#,
    )
}

pub fn eye_off() -> svg::Handle {
    icon(
        r#"<path d="M4 4l16 16"/><path d="M10 6c.6-.3 1.3-.5 2-.5 6 0 9.5 6.5 9.5 6.5a17 17 0 0 1-2.8 3.5M6.6 7.6A17 17 0 0 0 2.5 12S6 18.5 12 18.5c1.6 0 3-.4 4.2-1.1"/><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/>"#,
    )
}

// Developer tabs.

pub fn network() -> svg::Handle {
    icon(
        r#"<rect x="9" y="3.5" width="6" height="5" rx="1"/><rect x="3" y="15.5" width="6" height="5" rx="1"/><rect x="15" y="15.5" width="6" height="5" rx="1"/><path d="M12 8.5v3.5M6 15.5V12h12v3.5"/>"#,
    )
}

pub fn actors() -> svg::Handle {
    icon(
        r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20c.5-3.5 3.2-5.5 6.5-5.5s6 2 6.5 5.5"/><path d="M15.5 4.8a3.5 3.5 0 0 1 0 6.4M18 14.8c2 .7 3.3 2.5 3.5 5.2"/>"#,
    )
}

pub fn chip() -> svg::Handle {
    icon(
        r#"<rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/><path d="M10 10h4v4h-4zM9.5 3v3.5M14.5 3v3.5M9.5 17.5V21M14.5 17.5V21M3 9.5h3.5M3 14.5h3.5M17.5 9.5H21M17.5 14.5H21"/>"#,
    )
}

pub fn teleport() -> svg::Handle {
    icon(
        r#"<path d="M12 3v18M3 12h18"/><path d="m9 6 3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3"/>"#,
    )
}

pub fn layers() -> svg::Handle {
    icon(r#"<path d="m12 3.5 9 4.5-9 4.5L3 8z"/><path d="m3 12 9 4.5 9-4.5M3 16l9 4.5 9-4.5"/>"#)
}

// Settings tabs.

pub fn palette() -> svg::Handle {
    icon(
        r#"<path d="M12 3.5a8.5 8.5 0 1 0 0 17c1.2 0 1.8-.8 1.5-1.8-.4-1.2.3-2.2 1.5-2.2H17a3.5 3.5 0 0 0 3.5-3.5c0-5.3-3.8-9.5-8.5-9.5z"/><path d="M7.5 11.5h.1M10 7.5h.1M14.5 7.5h.1"/>"#,
    )
}

pub fn drive() -> svg::Handle {
    icon(
        r#"<rect x="3" y="13" width="18" height="6.5" rx="1.5"/><path d="M5 13 7.5 5h9L19 13M16.5 16.3h.1"/>"#,
    )
}

pub fn plug() -> svg::Handle {
    icon(
        r#"<path d="M9 3.5v4.5M15 3.5v4.5M6.5 8h11v3a5.5 5.5 0 0 1-11 0z"/><path d="M12 16.5V21"/>"#,
    )
}

pub fn download() -> svg::Handle {
    icon(r#"<path d="M12 4v11M8 11l4 4 4-4"/><path d="M5 19.5h14"/>"#)
}

pub fn pulse() -> svg::Handle {
    icon(r#"<path d="M3 12h4l2.5-6 4 12 2.5-6h5"/>"#)
}

pub fn sliders() -> svg::Handle {
    icon(
        r#"<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>"#,
    )
}

/// Draws an icon at `size` px, recoloured to the theme's text colour.
/// Sidebar panel: a window with its left column marked.
pub fn sidebar() -> svg::Handle {
    icon(
        r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M9 4.5v15M5.6 8.5h1M5.6 11h1"/>"#,
    )
}

/// Discord: a rounded controller-like face with two eyes.
pub fn discord() -> svg::Handle {
    icon(
        r#"<path d="M8.6 6.4c-1.7.3-3.1.8-4.3 1.6-1.8 2.9-2.4 5.9-2.2 8.9 1.4 1.1 3 1.8 4.6 2.3l1.1-1.7M15.4 6.4c1.7.3 3.1.8 4.3 1.6 1.8 2.9 2.4 5.9 2.2 8.9-1.4 1.1-3 1.8-4.6 2.3l-1.1-1.7M7.2 16.6c3.1 1.3 6.5 1.3 9.6 0M8.4 8.1c2.4-.6 4.8-.6 7.2 0"/><circle cx="9.2" cy="12.6" r="1.2"/><circle cx="14.8" cy="12.6" r="1.2"/>"#,
    )
}

/// Source code: angle brackets with a slash.
pub fn code() -> svg::Handle {
    icon(r#"<path d="m8 8-4 4 4 4M16 8l4 4-4 4M13.6 5l-3.2 14"/>"#)
}

/// A globe, for websites.
pub fn globe() -> svg::Handle {
    icon(
        r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.4 2.5 3.6 5.5 3.6 9s-1.2 6.5-3.6 9c-2.4-2.5-3.6-5.5-3.6-9S9.6 5.5 12 3z"/>"#,
    )
}

/// A person, for profiles.
pub fn user() -> svg::Handle {
    icon(r#"<circle cx="12" cy="8" r="3.8"/><path d="M4.5 20c1.4-3.4 4.1-5 7.5-5s6.1 1.6 7.5 5"/>"#)
}

/// An arrow leaving a box, for links that open in the browser.
pub fn external() -> svg::Handle {
    icon(
        r#"<path d="M14 4h6v6M20 4l-9 9M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10"/>"#,
    )
}

pub fn glyph<'a>(handle: svg::Handle, size: f32) -> Svg<'a> {
    iced::widget::svg(handle)
        .width(size)
        .height(size)
        .style(crate::design::icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_build_valid_svg_documents() {
        for handle in [
            search(),
            close(),
            check(),
            error(),
            warning(),
            info(),
            plus(),
            minus(),
            pin(),
            folder(),
            file(),
            details(),
            shield(),
            flask(),
        ] {
            let iced::advanced::svg::Data::Bytes(bytes) = handle.data() else {
                panic!("icons are built from memory");
            };
            let source = std::str::from_utf8(bytes).unwrap();
            assert!(source.starts_with("<svg") && source.ends_with("</svg>"));
        }
    }
}
