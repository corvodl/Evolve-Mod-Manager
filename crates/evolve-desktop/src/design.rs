//! Semantic palette shared with the reference Qt appearance settings.
use crate::settings::Preferences;
use iced::theme::{
    Base, Palette,
    palette::{Extended, Pair},
};
use iced::widget::{button as buttons, container, text};
use std::sync::atomic::{AtomicU32, Ordering};

/// Corner sizes used across the app. Every hand-written radius goes through
/// [`radius`] so the Sharp / Rounded / Soft setting and presets apply
/// everywhere at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radius {
    /// Markers, chips, checkbox-sized details.
    Small,
    /// Buttons, inputs, rail items.
    Medium,
    /// Panels, cards, toasts.
    Large,
    /// Fully rounded pills.
    Pill,
}

/// Corner scale chosen in Settings; stored as f32 bits because iced style
/// functions only receive `&Theme`. Updated whenever [`theme`] is built.
static CORNER_SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

pub fn corner_scale(style: &str) -> f32 {
    match style {
        "Sharp" => 0.25,
        "Soft" => 1.6,
        _ => 1.0,
    }
}

pub fn radius(theme: &Theme, size: Radius) -> f32 {
    if is_legacy(theme) {
        // Legacy keeps the original crisp menu silhouette.
        return match size {
            Radius::Small => 0.0,
            Radius::Pill => 2.0,
            _ => 1.0,
        };
    }
    let scale = f32::from_bits(CORNER_SCALE.load(Ordering::Relaxed));
    match size {
        Radius::Small => 4.0 * scale,
        Radius::Medium => 8.0 * scale,
        Radius::Large => 12.0 * scale,
        Radius::Pill => 999.0,
    }
}

/// A curated colour set. Roles match `appearance_colors` keys.
#[derive(Debug, Clone, Copy)]
pub struct ThemePreset {
    pub name: &'static str,
    pub blurb: &'static str,
    pub dark: &'static [(&'static str, &'static str)],
    pub light: &'static [(&'static str, &'static str)],
    pub corners: &'static str,
}

impl ThemePreset {
    pub fn role(&self, light: bool, role: &str) -> Option<&'static str> {
        let set = if light { self.light } else { self.dark };
        set.iter()
            .find(|(key, _)| *key == role)
            .map(|(_, value)| *value)
    }

    /// Preview colours for a swatch card: background, surface, accent, text.
    pub fn swatch(&self, light: bool) -> [Color; 4] {
        ["background", "surface", "accent", "text"]
            .map(|role| self.role(light, role).and_then(hex).unwrap_or(Color::BLACK))
    }
}

/// Curated presets. Catppuccin colours are from the MIT-licensed
/// catppuccin/palette (Mocha for dark, Latte for light).
pub const THEME_PRESETS: &[ThemePreset] = &[
    ThemePreset {
        name: "Evolve Ember",
        blurb: "Charred dark with an ember-orange accent.",
        dark: &[
            ("text", "#f0e9e2"),
            ("background", "#120e0c"),
            ("accent", "#ff6a2b"),
            ("surface", "#1b1512"),
            ("field", "#261d18"),
            ("border", "#3a2c24"),
            ("muted", "#b3a294"),
            ("button", "#261d18"),
            ("selection", "#4a2416"),
        ],
        light: &[
            ("text", "#2b201a"),
            ("background", "#f8f3ef"),
            ("accent", "#d4501a"),
            ("surface", "#fffaf6"),
            ("field", "#f0e6de"),
            ("border", "#dccbbf"),
            ("muted", "#7a6658"),
            ("button", "#efe3d9"),
            ("selection", "#fbd9c6"),
        ],
        corners: "Rounded",
    },
    ThemePreset {
        name: "Graphite Mono",
        blurb: "Neutral greys, monochrome accent, sharp corners.",
        dark: &[
            ("text", "#e6e6e6"),
            ("background", "#121212"),
            ("accent", "#d4d4d4"),
            ("surface", "#1a1a1a"),
            ("field", "#242424"),
            ("border", "#333333"),
            ("muted", "#9a9a9a"),
            ("button", "#242424"),
            ("selection", "#2e2e2e"),
        ],
        light: &[
            ("text", "#1a1a1a"),
            ("background", "#f4f4f4"),
            ("accent", "#2a2a2a"),
            ("surface", "#ffffff"),
            ("field", "#ececec"),
            ("border", "#d4d4d4"),
            ("muted", "#666666"),
            ("button", "#e8e8e8"),
            ("selection", "#e0e0e0"),
        ],
        corners: "Sharp",
    },
    ThemePreset {
        name: "Amber Slate",
        blurb: "Blue-grey slate with a warm amber accent.",
        dark: &[
            ("text", "#e8eaed"),
            ("background", "#0f141a"),
            ("accent", "#f0a83a"),
            ("surface", "#161d25"),
            ("field", "#1f2832"),
            ("border", "#2c3844"),
            ("muted", "#93a1b0"),
            ("button", "#1f2832"),
            ("selection", "#3a2f17"),
        ],
        light: &[
            ("text", "#1d2733"),
            ("background", "#f3f5f7"),
            ("accent", "#c77d0a"),
            ("surface", "#ffffff"),
            ("field", "#e8ecf0"),
            ("border", "#d0d7de"),
            ("muted", "#5b6b7b"),
            ("button", "#e6eaee"),
            ("selection", "#f8e6c4"),
        ],
        corners: "Soft",
    },
    ThemePreset {
        name: "Catppuccin",
        blurb: "Pastel Mocha (dark) and Latte (light).",
        dark: &[
            ("text", "#cdd6f4"),
            ("background", "#181825"),
            ("accent", "#cba6f7"),
            ("surface", "#1e1e2e"),
            ("field", "#313244"),
            ("border", "#45475a"),
            ("muted", "#a6adc8"),
            ("button", "#313244"),
            ("selection", "#45475a"),
        ],
        light: &[
            ("text", "#4c4f69"),
            ("background", "#e6e9ef"),
            ("accent", "#8839ef"),
            ("surface", "#eff1f5"),
            ("field", "#ccd0da"),
            ("border", "#bcc0cc"),
            ("muted", "#6c6f85"),
            ("button", "#ccd0da"),
            ("selection", "#bcc0cc"),
        ],
        corners: "Soft",
    },
];

pub fn theme_preset(name: &str) -> Option<&'static ThemePreset> {
    THEME_PRESETS.iter().find(|preset| preset.name == name)
}

/// Built-in Modern defaults, for the swatch of the "Default" card.
pub fn default_swatch(light: bool) -> [Color; 4] {
    let pick = |dark: &str, pale: &str| hex(if light { pale } else { dark }).unwrap();
    [
        pick("#151719", "#f7f7f6"),
        pick("#1c1f21", "#ffffff"),
        pick("#c9384a", "#b72b41"),
        pick("#eceef0", "#25282b"),
    ]
}

pub fn button<'a>(
    content: impl Into<iced::Element<'a, crate::Message>>,
) -> iced::widget::Button<'a, crate::Message> {
    iced::widget::button(content)
        .padding([8, 12])
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
        style.border.radius = radius(theme, Radius::Medium).into();
    }
    style
}

pub fn secondary(theme: &Theme, status: buttons::Status) -> buttons::Style {
    let mut style = buttons::secondary(theme, status);
    style.border = Border {
        color: border(theme),
        width: 1.0,
        radius: radius(theme, Radius::Medium).into(),
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
    style.border.radius = if is_legacy(theme) {
        0.0
    } else {
        radius(theme, Radius::Medium)
    }
    .into();
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
        .padding([8, 12])
        .style(|theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Background::Color(field(theme));
            style.border.radius = radius(theme, Radius::Medium).into();
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

pub fn hex(value: &str) -> Option<Color> {
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
    CORNER_SCALE.store(
        corner_scale(&settings.corner_style).to_bits(),
        Ordering::Relaxed,
    );
    let preset = theme_preset(&settings.theme_preset);
    let role = |name: &str, dark: &str, pale: &str, original: &str| {
        if legacy {
            return hex(original).unwrap();
        }
        // Custom colours win over the preset, which wins over the built-in
        // Modern palette for the current Dark/Light mode.
        settings
            .appearance_colors
            .get(name)
            .and_then(|v| hex(v))
            .or_else(|| preset.and_then(|p| p.role(light, name)).and_then(hex))
            .unwrap_or_else(|| hex(if light { pale } else { dark }).unwrap())
    };
    let foreground = role("text", "#eceef0", "#25282b", "#e9e9e6");
    let background = role("background", "#151719", "#f7f7f6", "#0a0c0e");
    let accent = role("accent", "#c9384a", "#b72b41", "#b62b3d");
    let surface = role("surface", "#1c1f21", "#ffffff", "#141719");
    let field = role("field", "#292c2f", "#eff0f0", "#202325");
    let border = role("border", "#363a3d", "#daddde", "#50575a");
    let muted = role("muted", "#aab0b4", "#61686c", "#adb2b3");
    let button = role("button", "#292c2f", "#edeef0", "#191c1e");
    let selection = role("selection", "#292d30", "#eceeef", "#4b1720");
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
                    "#f3f4f3"
                } else {
                    "#111315"
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
/// Shared opaque workspace panel; wallpaper-specific translucency stays on Home.
pub fn panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface(theme))),
        border: Border {
            color: border(theme),
            width: 1.0,
            radius: radius(theme, Radius::Large).into(),
        },
        ..container::Style::default()
    }
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
        radius: radius(theme, Radius::Medium).into(),
        color: if active && is_legacy(theme) {
            theme.palette().primary
        } else {
            Color::TRANSPARENT
        },
        width: if active && is_legacy(theme) { 1.0 } else { 0.0 },
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

/// Semantic tone for callouts, steps and status pills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Danger,
}

pub fn tone_color(theme: &Theme, tone: Tone) -> Color {
    let palette = theme.palette();
    match tone {
        Tone::Info => palette.primary,
        Tone::Success => palette.success,
        Tone::Warning => palette.warning,
        Tone::Danger => palette.danger,
    }
}

pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

/// Linear blend from `a` to `b`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// Tinted panel used by callouts: faint tone wash with a tone border.
pub fn tinted(theme: &Theme, tone: Tone) -> container::Style {
    let color = tone_color(theme, tone);
    container::Style {
        background: Some(Background::Color(mix(surface(theme), color, 0.08))),
        border: Border {
            color: with_alpha(color, 0.45),
            width: 1.0,
            radius: radius(theme, Radius::Medium).into(),
        },
        ..container::Style::default()
    }
}

/// Small rounded pill (save status, island, chips).
pub fn pill(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(field(theme))),
        border: Border {
            color: border(theme),
            width: 1.0,
            radius: radius(theme, Radius::Pill).into(),
        },
        ..container::Style::default()
    }
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
    fn modern_palette_preserves_brand_accent_and_custom_overrides() {
        let mut prefs = Preferences::default();
        let default = theme(&prefs);
        assert_eq!(default.palette().primary, hex("#c9384a").unwrap());
        assert_eq!(default.palette().background, hex("#151719").unwrap());
        prefs
            .appearance_colors
            .insert("surface".into(), "#313437".into());
        assert_eq!(surface(&theme(&prefs)), hex("#313437").unwrap());
    }

    #[test]
    fn reference_palettes_and_overrides_reach_widget_roles() {
        let mut prefs = Preferences::default();
        let dark = theme(&prefs);
        assert_eq!(dark.palette().background, hex("#151719").unwrap());
        assert_eq!(surface(&dark), hex("#1c1f21").unwrap());
        prefs.appearance_mode = "Light".into();
        let light = theme(&prefs);
        assert_eq!(surface(&light), Color::WHITE);
        assert_eq!(muted(&light).color, hex("#61686c"));
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

    #[test]
    fn presets_are_complete_valid_and_custom_colours_still_win() {
        for preset in THEME_PRESETS {
            for set in [preset.dark, preset.light] {
                for role in [
                    "text",
                    "background",
                    "accent",
                    "surface",
                    "field",
                    "border",
                    "muted",
                    "button",
                    "selection",
                ] {
                    let value = set.iter().find(|(key, _)| *key == role);
                    assert!(
                        value.and_then(|(_, v)| hex(v)).is_some(),
                        "{} {role}",
                        preset.name
                    );
                }
            }
            assert!(["Sharp", "Rounded", "Soft"].contains(&preset.corners));
        }
        let mut prefs = Preferences {
            theme_preset: "Catppuccin".into(),
            ..Preferences::default()
        };
        assert_eq!(theme(&prefs).palette().primary, hex("#cba6f7").unwrap());
        prefs.appearance_mode = "Light".into();
        assert_eq!(theme(&prefs).palette().primary, hex("#8839ef").unwrap());
        prefs
            .appearance_colors
            .insert("accent".into(), "#123456".into());
        assert_eq!(theme(&prefs).palette().primary, hex("#123456").unwrap());
    }

    #[test]
    fn corner_scale_and_legacy_radius() {
        assert_eq!(corner_scale("Sharp"), 0.25);
        assert_eq!(corner_scale("Soft"), 1.6);
        assert_eq!(corner_scale("anything"), 1.0);
        let legacy = theme(&Preferences {
            ui_theme: "Evolve Legacy".into(),
            ..Preferences::default()
        });
        assert_eq!(radius(&legacy, Radius::Large), 1.0);
        assert_eq!(radius(&legacy, Radius::Small), 0.0);
    }
}
