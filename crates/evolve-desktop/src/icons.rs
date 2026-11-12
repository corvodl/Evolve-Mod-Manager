//! The app's outline icon set: 24×24, 1.65 stroke, round caps and joins,
//! the same style as `assets/nav`. These are drawn for this app rather than
//! copied from a third-party set, so there is no licence to track.
//! [`glyph`] draws one through `design::icon`, which recolours it to the
//! theme's text colour.

use iced::widget::Svg;
use iced::widget::svg;

fn icon(body: &str) -> svg::Handle {
    svg::Handle::from_memory(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#D6454D" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
    ).into_bytes())
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

pub fn overview() -> svg::Handle {
    icon(r#"<path d="m3 6 6-3 6 3 6-3v15l-6 3-6-3-6 3zM9 3v15M15 6v15"/>"#)
}

pub fn focus() -> svg::Handle {
    icon(
        r#"<circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="1.5"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/>"#,
    )
}

pub fn reset() -> svg::Handle {
    icon(r#"<path d="M4 4v6h6"/><path d="M5.5 15a7.5 7.5 0 1 0 1.7-8L4 10"/>"#)
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

/// Draws an icon at `size` px, recoloured to the theme's text colour.
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
