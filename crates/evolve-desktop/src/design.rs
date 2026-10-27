//! Semantic palette shared with the reference Qt appearance settings.
use crate::settings::Preferences;
use iced::theme::{
    Base, Palette,
    palette::{Extended, Pair},
};
use iced::widget::{button as buttons, text};

pub fn button<'a>(
    content: impl Into<iced::Element<'a, crate::Message>>,
) -> iced::widget::Button<'a, crate::Message> {
    iced::widget::button(content)
        .padding([7, 11])
        .style(secondary)
}

/// The native Legacy skin keeps the original menu's crisp rectangular
/// silhouette and white-hover contrast, without running Flash/Scaleform.
pub fn is_legacy(theme: &Theme) -> bool {
    theme.name() == "Evolve Legacy"
}

pub fn primary(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::primary(theme, status);
    if is_legacy(theme) {
        style.border = Border {
            color: theme.palette().primary,
            width: 1.0,
            radius: 1.0.into(),
        };
        if matches!(status, buttons::Status::Hovered | buttons::Status::Pressed) {
            style.background = Some(Background::Color(Color::WHITE));
            style.text_color = Color::from_rgb8(15, 17, 19);
        } else {
            style.background = Some(Background::Color(theme.palette().primary));
            style.text_color = Color::WHITE;
        }
    } else {
        style.border.radius = 8.0.into();
    }
    style
}

pub fn secondary(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::secondary(theme, status);
    style.border = Border {
        color: border(theme),
        width: 1.0,
        radius: if is_legacy(theme) { 1.0 } else { 8.0 }.into(),
    };
    if is_legacy(theme) {
        if matches!(status, buttons::Status::Hovered | buttons::Status::Pressed) {
            style.background = Some(Background::Color(Color::WHITE));
            style.text_color = Color::from_rgb8(15, 17, 19);
            style.border.color = Color::WHITE;
        } else {
            style.background = Some(Background::Color(field(theme)));
            style.text_color = theme.palette().text;
        }
    }
    style
}

pub fn text_button(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::text(theme, status);
    style.text_color = theme.palette().text;
    style.border.radius = if is_legacy(theme) { 0.0 } else { 8.0 }.into();
    if is_legacy(theme) && matches!(status, buttons::Status::Hovered | buttons::Status::Pressed) {
        style.text_color = Color::WHITE;
    }
    style
}

pub fn text_input<'a>(
    placeholder: &str,
    value: &str,
) -> iced::widget::TextInput<'a, crate::Message> {
    iced::widget::text_input(placeholder, value)
        .padding([7, 11])
        .style(|theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Background::Color(field(theme));
            style.border.radius = if is_legacy(theme) { 1.0 } else { 8.0 }.into();
            style.placeholder = theme.extended_palette().background.strongest.text;
            style
        })
}

pub fn icon(theme: &Theme, _: iced::widget::svg::Status) -> iced::widget::svg::Style {
    iced::widget::svg::Style {
        color: Some(theme.palette().text),
    }
}
use iced::{Background, Border, Color, Theme};

fn hex(value: &str) -> Option<Color> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(value, 16).ok()?;
    Some(Color::from_rgb8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    ))
}

pub fn theme(settings: &Preferences) -> Theme {
    let legacy = settings.ui_theme == "Evolve Legacy";
    // Legacy is intentionally a dark menu; Modern retains the Dark/Light
    // preference and each user's existing custom color overrides.
    let light = !legacy && settings.appearance_mode == "Light";
    let role = |name: &str, dark: &str, pale: &str, original: &str| {
        if legacy {
            return hex(original).unwrap();
        }
        settings
            .appearance_colors
            .get(name)
            .and_then(|v| hex(v))
            .unwrap_or_else(|| hex(if light { pale } else { dark }).unwrap())
    };
    let foreground = role("text", "#f2f3f5", "#242731", "#e9e9e6");
    let background = role("background", "#111318", "#f5f6f8", "#0a0c0e");
    let accent = role("accent", "#c9384a", "#b72b41", "#b62b3d");
    let surface = role("surface", "#1c1e24", "#ffffff", "#141719");
    let field = role("field", "#24262e", "#eff1f5", "#202325");
    let border = role("border", "#393e49", "#d6dbe3", "#50575a");
    let muted = role("muted", "#adb1bd", "#606775", "#adb2b3");
    let button = role("button", "#292c34", "#e9ecf1", "#191c1e");
    let selection = role("selection", "#48303b", "#f1dce2", "#4b1720");
    let palette = Palette {
        background,
        text: foreground,
        primary: accent,
        success: Color::from_rgb8(70, 170, 130),
        warning: Color::from_rgb8(233, 170, 80),
        danger: accent,
    };
    Theme::custom_with_fn(
        if legacy { "Evolve Legacy" } else { "Evolve" },
        palette,
        |base| {
            let mut extended = Extended::generate(base);
            extended.background.weak = Pair {
                color: surface,
                text: foreground,
            };
            extended.background.neutral = Pair {
                color: field,
                text: foreground,
            };
            extended.background.strong = Pair {
                color: border,
                text: foreground,
            };
            extended.background.strongest.text = muted;
            let light_background =
                background.r * 0.2126 + background.g * 0.7152 + background.b * 0.0722 > 0.55;
            extended.background.weaker = Pair {
                color: hex(if legacy {
                    "#0e1012"
                } else if light_background {
                    "#f0f2f5"
                } else {
                    "#17191f"
                })
                .unwrap(),
                text: foreground,
            };
            extended.secondary.base = Pair {
                color: button,
                text: foreground,
            };
            extended.primary.weak = Pair {
                color: selection,
                text: foreground,
            };
            extended
        },
    )
}

pub fn surface(theme: &Theme) -> Color {
    theme.extended_palette().background.weak.color
}
pub fn field(theme: &Theme) -> Color {
    theme.extended_palette().background.neutral.color
}
pub fn border(theme: &Theme) -> Color {
    theme.extended_palette().background.strong.color
}
pub fn sidebar(theme: &Theme) -> Color {
    theme.extended_palette().background.weaker.color
}
pub fn selection(theme: &Theme) -> Color {
    theme.extended_palette().primary.weak.color
}
pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().background.strongest.text),
    }
}
pub fn accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().primary),
    }
}

pub fn rail(theme: &Theme, status: buttons::Status, active: bool) -> buttons::Style {
    let mut style = buttons::text(theme, status);
    style.text_color = theme.palette().text;
    style.border = Border {
        radius: if is_legacy(theme) { 1.0 } else { 9.0 }.into(),
        color: if is_legacy(theme) && active {
            theme.palette().primary
        } else {
            Color::TRANSPARENT
        },
        width: if is_legacy(theme) && active { 1.0 } else { 0.0 },
    };
    if is_legacy(theme) && matches!(status, buttons::Status::Hovered | buttons::Status::Pressed) {
        style.background = Some(Background::Color(Color::WHITE));
        style.text_color = Color::from_rgb8(15, 17, 19);
    } else if active {
        style.background = Some(Background::Color(selection(theme)));
    } else if matches!(status, buttons::Status::Hovered | buttons::Status::Pressed) {
        style.background = Some(Background::Color(field(theme)));
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_theme_is_distinct_and_uses_white_menu_hover() {
        let mut prefs = Preferences {
            ui_theme: "Evolve Legacy".into(),
            ..Preferences::default()
        };
        let original = theme(&prefs);
        assert!(is_legacy(&original));
        assert_eq!(original.palette().background, hex("#0a0c0e").unwrap());
        let hover = secondary(&original, buttons::Status::Hovered);
        assert_eq!(hover.background, Some(Background::Color(Color::WHITE)));
        assert_eq!(hover.border.radius, 1.0.into());
        assert_eq!(
            primary(&original, buttons::Status::Hovered).text_color,
            Color::from_rgb8(15, 17, 19)
        );
        prefs.appearance_mode = "Light".into();
        prefs
            .appearance_colors
            .insert("background".into(), "#abcdef".into());
        assert_eq!(theme(&prefs).palette().background, hex("#0a0c0e").unwrap());
        prefs.ui_theme = "Modern".into();
        assert_eq!(theme(&prefs).palette().background, hex("#abcdef").unwrap());
    }

    #[test]
    fn reference_palettes_and_overrides_reach_widget_roles() {
        let mut prefs = Preferences::default();
        let dark = theme(&prefs);
        assert_eq!(dark.palette().background, hex("#111318").unwrap());
        assert_eq!(surface(&dark), hex("#1c1e24").unwrap());
        prefs.appearance_mode = "Light".into();
        let light = theme(&prefs);
        assert_eq!(surface(&light), Color::WHITE);
        assert_eq!(muted(&light).color, hex("#606775"));
        prefs
            .appearance_colors
            .insert("surface".into(), "#abcdef".into());
        prefs
            .appearance_colors
            .insert("accent".into(), "#123456".into());
        let custom = theme(&prefs);
        assert_eq!(surface(&custom), hex("#abcdef").unwrap());
        assert_eq!(custom.palette().primary, hex("#123456").unwrap());
        assert_eq!(
            rail(&custom, buttons::Status::Active, true).background,
            Some(Background::Color(selection(&custom)))
        );
    }
}
