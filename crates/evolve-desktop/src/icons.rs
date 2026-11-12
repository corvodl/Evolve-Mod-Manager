//! Small stroke icons for dock toolbars, drawn in the same 24×24,
//! 1.65-stroke style as `assets/nav`. `design::icon` recolours them to the
//! theme's text colour.

use iced::widget::svg;

fn icon(body: &str) -> svg::Handle {
    svg::Handle::from_memory(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#D6454D" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
    ).into_bytes())
}

pub fn frame_all() -> svg::Handle {
    icon(r#"<path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4"/><circle cx="12" cy="12" r="2.5"/>"#)
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
    icon(r#"<path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 3v18M4 7.5l16 9M20 7.5l-16 9"/>"#)
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
    icon(r#"<circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="1.5"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/>"#)
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
