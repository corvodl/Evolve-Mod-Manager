//! Semantic palette shared with the reference Qt appearance settings.
use crate::settings::Preferences;
use iced::theme::{
    Palette,
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

pub fn primary(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::primary(theme, status);
    style.border.radius = 8.0.into();
    style
}

pub fn secondary(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::secondary(theme, status);
    style.border = Border {
        color: border(theme),
        width: 1.0,
        radius: 8.0.into(),
    };
    style
}

pub fn text_button(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::text(theme, status);
    style.text_color = theme.palette().text;
    style.border.radius = 8.0.into();
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
            style.border.radius = 8.0.into();
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
    let light = settings.appearance_mode == "Light";
    let role = |name: &str, dark: &str, pale: &str| {
        settings
            .appearance_colors
            .get(name)
            .and_then(|v| hex(v))
            .unwrap_or_else(|| hex(if light { pale } else { dark }).unwrap())
    };
    let foreground = role("text", "#f2f3f5", "#242731");
    let background = role("background", "#111318", "#f5f6f8");
    let accent = role("accent", "#c9384a", "#b72b41");
    let surface = role("surface", "#1c1e24", "#ffffff");
    let field = role("field", "#24262e", "#eff1f5");
    let border = role("border", "#393e49", "#d6dbe3");
    let muted = role("muted", "#adb1bd", "#606775");
    let button = role("button", "#292c34", "#e9ecf1");
    let selection = role("selection", "#48303b", "#f1dce2");
    let palette = Palette {
        background,
        text: foreground,
        primary: accent,
        success: Color::from_rgb8(70, 170, 130),
        warning: Color::from_rgb8(233, 170, 80),
        danger: accent,
    };
    Theme::custom_with_fn("Evolve", palette, |base| {
        let mut extended = Extended::generate(base);
        // Store semantic roles in the extended palette so every window and
        // style closure follows the same live preferences without global state.
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
            color: hex(if light_background {
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
    })
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
        radius: 9.0.into(),
        ..Border::default()
    };
    if active {
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
