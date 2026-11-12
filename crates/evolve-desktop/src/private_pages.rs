//! Private-manager-aligned landing, documentation and informational pages.
//! Home deliberately does not enumerate PAKs or scan the user's installation.
use super::{Message, Page, State};
use crate::{design, developer_hotkeys, runtime_teleport};
use iced::widget::{button, column, container, image, row, scrollable, stack, svg, text, tooltip};
use iced::{Background, Border, Element, Fill, Length};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(super) enum DocsSection {
    #[default]
    GettingStarted,
    Install,
    Paths,
    Import,
    Browse,
    Projects,
    Edit,
    Gfx,
    Export,
    LaunchMods,
    LaunchClean,
    Recovery,
    Updates,
    Troubleshoot,
    Faq,
}
impl DocsSection {
    pub(super) const ALL: [Self; 15] = [
        Self::GettingStarted,
        Self::Install,
        Self::Paths,
        Self::Import,
        Self::Browse,
        Self::Projects,
        Self::Edit,
        Self::Gfx,
        Self::Export,
        Self::LaunchMods,
        Self::LaunchClean,
        Self::Recovery,
        Self::Updates,
        Self::Troubleshoot,
        Self::Faq,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum DeveloperSection {
    Network,
    Actors,
    Ai,
    Teleport,
    Overlay,
    Keybinds,
}
impl DeveloperSection {
    pub(super) const ALL: [Self; 6] = [
        Self::Network,
        Self::Actors,
        Self::Ai,
        Self::Teleport,
        Self::Overlay,
        Self::Keybinds,
    ];

    fn icon(self) -> iced::widget::svg::Handle {
        match self {
            Self::Network => crate::icons::network(),
            Self::Actors => crate::icons::actors(),
            Self::Ai => crate::icons::chip(),
            Self::Teleport => crate::icons::teleport(),
            Self::Overlay => crate::icons::layers(),
            Self::Keybinds => crate::icons::keyboard(),
        }
    }

    /// The remembered tab, Network by default.
    fn from_title(title: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|section| section.title() == title)
            .unwrap_or(Self::Network)
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Network => "Network & Process",
            Self::Actors => "Actors & Memory",
            Self::Ai => "AI Controls",
            Self::Teleport => "Teleport & Fly",
            Self::Overlay => "External Overlay",
            Self::Keybinds => "Keybinds",
        }
    }
}

fn surface<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(16)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, design::Radius::Large).into(),
            },
            ..container::Style::default()
        })
}

/// Translucent surfaces belong to Home only. Other pages retain opaque
/// panels because the artwork is exclusive to the Home view.
fn home_surface<'a>(
    content: impl Into<Element<'a, Message>>,
    wallpaper: bool,
) -> iced::widget::Container<'a, Message> {
    surface(content).style(move |theme| {
        let mut backdrop = design::surface(theme);
        backdrop.a = if wallpaper { 0.96 } else { 1.0 };
        container::Style {
            background: Some(Background::Color(backdrop)),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, design::Radius::Large).into(),
            },
            ..container::Style::default()
        }
    })
}

/// Keep the launch controls in a calm panel; the primary action and active
/// workspace carry the accent instead of tinting another large surface.
fn home_play_surface<'a>(
    content: impl Into<Element<'a, Message>>,
    wallpaper: bool,
) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding([16, 20])
        .style(move |theme| {
            let mut backdrop = design::surface(theme);
            backdrop.a = if wallpaper { 0.94 } else { 1.0 };
            container::Style {
                background: Some(Background::Color(backdrop)),
                border: Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: design::radius(theme, design::Radius::Large).into(),
                },
                ..container::Style::default()
            }
        })
}

/// Original Evolve red corner atlas sprite, cropped from the uploaded
/// button_I8.dds.0. Only rendered when the optional Legacy theme is selected.
fn legacy_section_label(label: &'static str, legacy: bool) -> Element<'static, Message> {
    let title = text(if legacy {
        label
    } else {
        match label {
            "YOUR WORKBENCH" => "Your workbench",
            "GETTING STARTED" => "Getting started",
            "PLAY EVOLVE" => "Play Evolve",
            _ => label,
        }
    })
    .size(design::LABEL)
    .style(if legacy {
        design::accent
    } else {
        design::muted
    });
    if legacy {
        row![
            image(image::Handle::from_bytes(
                include_bytes!("../../../assets/legacy/legacy-menu-corner.png").to_vec(),
            ))
            .width(20)
            .height(20),
            title
        ]
        .spacing(4)
        .align_y(iced::Center)
        .into()
    } else {
        title.into()
    }
}

/// Keep Home actions readable without painting opaque rectangles over the
/// optional Evolve artwork. Other pages retain the ordinary theme buttons.
fn home_button_style(
    theme: &iced::Theme,
    status: button::Status,
    wallpaper: bool,
    primary: bool,
) -> button::Style {
    let mut style = if primary {
        design::primary(theme, status)
    } else {
        design::secondary(theme, status)
    };
    if wallpaper {
        let mut tint = if primary {
            theme.palette().primary
        } else {
            design::field(theme)
        };
        tint.a = match status {
            button::Status::Hovered => 0.42,
            button::Status::Pressed => 0.54,
            button::Status::Disabled => 0.09,
            button::Status::Active => 0.18,
        };
        style.background = Some(Background::Color(tint));
        if primary {
            style.text_color = iced::Color::WHITE;
        }
    }
    style
}

fn tile(name: &'static str, page: Page, wallpaper: bool) -> Element<'static, Message> {
    design::button(
        row![
            svg(page.icon()).style(design::icon).width(18).height(18),
            text(name).size(14),
        ]
        .spacing(12)
        .align_y(iced::Center),
    )
    .style(move |theme, status| {
        let mut style = design::secondary(theme, status);
        if design::is_legacy(theme) {
            // Legacy's iconic white hover must not be overridden by the
            // Modern workspace-tile styling.
            if !matches!(status, button::Status::Hovered | button::Status::Pressed) {
                style.background = Some(Background::Color(design::surface(theme)));
            }
            style.border.radius = 1.0.into();
        } else {
            style.background = Some(Background::Color(match status {
                button::Status::Hovered => design::field(theme),
                button::Status::Pressed => design::selection(theme),
                _ => design::surface(theme),
            }));
            style.text_color = theme.palette().text;
            style.border = Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, design::Radius::Medium).into(),
            };
        }
        if wallpaper {
            let mut backdrop = match status {
                button::Status::Hovered => design::field(theme),
                button::Status::Pressed => design::selection(theme),
                _ => design::surface(theme),
            };
            backdrop.a = if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                0.96
            } else {
                0.90
            };
            style.background = Some(Background::Color(backdrop));
            style.text_color = theme.palette().text;
        }
        style
    })
    .width(Fill)
    .height(Length::Fixed(44.0))
    .padding([8, 12])
    .on_press(Message::Navigate(page))
    .into()
}

fn status_value_preview(value: &str) -> String {
    if value.chars().count() > 24 {
        format!(
            "…{}",
            value
                .chars()
                .rev()
                .take(23)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    } else {
        value.to_owned()
    }
}

fn status_card<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    let short = status_value_preview(&value);
    row![
        text(label).size(design::BODY).width(Length::FillPortion(2)),
        container(tooltip(
            text(short).size(design::BODY).style(design::muted),
            text(value).size(design::LABEL),
            tooltip::Position::Top,
        ))
        .width(Length::FillPortion(3)),
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}

/// The release part only; the +build identifier stays available on hover.
fn short_client_version(version: &str) -> String {
    if let Some((release, _)) = version.split_once('+')
        && !release.is_empty()
    {
        return format!("v{release}");
    }
    if version.chars().count() > 28 {
        return format!("v{}…", version.chars().take(24).collect::<String>());
    }
    format!("v{version}")
}

fn client_version_card<'a>(version: Option<&str>) -> Element<'a, Message> {
    let value: Element<'a, Message> = if let Some(version) = version {
        tooltip(
            text(short_client_version(version))
                .size(design::LABEL)
                .style(design::muted),
            text(format!("Full client version: v{version}")).size(design::CAPTION),
            tooltip::Position::Top,
        )
        .into()
    } else {
        text("Not detected")
            .size(design::LABEL)
            .style(design::muted)
            .into()
    };
    row![
        text("Client version")
            .size(design::BODY)
            .width(Length::FillPortion(2)),
        container(value).width(Length::FillPortion(3)),
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}

/// A horizontal announcement preview uses the wide panel efficiently, with
/// one small uncropped attachment alongside the excerpt. Full images and
/// additional embeds remain available on What's New.
fn latest_announcement(state: &State, wallpaper: bool) -> Element<'_, Message> {
    let news = &state.news;
    let heading = row![
        text("Announcements").size(19),
        design::button("More in What's New")
            .on_press(Message::Navigate(Page::WhatsNew))
            .style(move |theme, status| home_button_style(theme, status, wallpaper, false)),
    ]
    .spacing(12)
    .align_y(iced::Center);

    let mut details = column![].spacing(6).width(Length::FillPortion(4));
    let mut preview: Option<Element<'_, Message>> = None;
    if let Some(post) = news.cache.announcements.first() {
        if !post.title.is_empty() {
            details = details.push(text(&post.title).size(16));
        }
        details = details.push(
            text(format!("{}  ·  {}", post.author, post.timestamp))
                .size(design::CAPTION)
                .style(design::muted),
        );
        for body in [&post.content, &post.description] {
            if !body.is_empty() {
                let excerpt: String = body.chars().take(260).collect();
                details = details.push(text(excerpt).size(design::LABEL));
            }
        }
        if let Some(url) = post
            .image_urls
            .iter()
            .find(|url| crate::whats_new::allowed_image_url(url))
        {
            if let Some(handle) = news.image_handles.get(url) {
                // A compact filled thumbnail keeps the workspace visible on
                // Home; clicking opens the complete original attachment.
                preview = Some(
                    button(
                        image(handle.clone())
                            .width(Fill)
                            .height(Length::Fixed(112.0))
                            .content_fit(iced::ContentFit::Cover),
                    )
                    .width(Length::FillPortion(3))
                    .padding(0)
                    .style(design::text_button)
                    .on_press(Message::OpenNewsLink(url.clone()))
                    .into(),
                );
            } else if news.image_failures.contains(url) {
                details = details.push(
                    text("Image preview unavailable")
                        .size(design::CAPTION)
                        .style(design::muted),
                );
            } else {
                details = details.push(
                    text("Loading image…")
                        .size(design::CAPTION)
                        .style(design::muted),
                );
            }
        }
    } else if news.loading {
        // Pulsing placeholders instead of a bare "Loading…" line.
        let alpha = crate::ui_kit::pulse(
            state.ui.clock(std::time::Instant::now()),
            state.preferences.reduce_motion,
        );
        details = details
            .push(crate::ui_kit::skeleton(Length::FillPortion(3), 16.0, alpha))
            .push(crate::ui_kit::skeleton(Length::Fixed(160.0), 10.0, alpha))
            .push(crate::ui_kit::skeleton(Fill, 11.0, alpha))
            .push(crate::ui_kit::skeleton(Fill, 11.0, alpha))
            .push(crate::ui_kit::skeleton(Length::FillPortion(2), 11.0, alpha));
        preview = Some(crate::ui_kit::skeleton(
            Length::FillPortion(3),
            112.0,
            alpha,
        ));
    } else {
        details = details.push(
            text("No announcements available yet.")
                .size(design::LABEL)
                .style(design::muted),
        );
    }

    let body: Element<'_, Message> = if let Some(preview) = preview {
        row![details, preview]
            .spacing(16)
            .align_y(iced::Center)
            .into()
    } else {
        details.into()
    };
    home_surface(column![heading, body].spacing(7), wallpaper)
        .padding([12, 15])
        .into()
}

/// The optional Home art is distinct from animated progress indicators.
/// A supplied 1920x1080 default_loading.dds is loaded as home art; if
/// unavailable the Workbench remains completely usable without a placeholder.
fn home_banner() -> Option<iced::widget::image::Handle> {
    static CACHE: std::sync::OnceLock<Option<iced::widget::image::Handle>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(load_home_banner).clone()
}

fn load_home_banner() -> Option<iced::widget::image::Handle> {
    // Let installers or portable users override the bundled image. A missing
    // or corrupt external DDS should not make the Home artwork disappear.
    if let Some(base) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    {
        let candidates = [
            base.join("assets").join("default_loading.dds"),
            base.join("default_loading.dds"),
        ];
        for path in candidates {
            if let Ok(data) = std::fs::read(&path)
                && let Some((width, height, rgba)) = decode_home_dds(&data)
            {
                return Some(iced::widget::image::Handle::from_rgba(width, height, rgba));
            }
        }
    }
    // Some updater paths only replace the executable. Embed the verified
    // 1920×1080 original so the hero art survives those updates as well.
    let bundled = include_bytes!("../../../assets/default_loading.dds");
    let (width, height, rgba) = decode_home_dds(bundled)?;
    Some(iced::widget::image::Handle::from_rgba(width, height, rgba))
}

/// Strictly bounded DXT1 (BC1) DDS support for the provided Home banner.
fn decode_home_dds(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if data.len() < 128 || &data[..4] != b"DDS " || &data[84..88] != b"DXT1" {
        return None;
    }
    let width = u32::from_le_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_le_bytes(data[12..16].try_into().ok()?);
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let needed = blocks_x
        .checked_mul(blocks_y)?
        .checked_mul(8)?
        .checked_add(128)?;
    if data.len() < needed {
        return None;
    }
    let mut rgba = vec![
        0u8;
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?
    ];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let start = 128 + (by * blocks_x + bx) * 8;
            let c0 = u16::from_le_bytes([data[start], data[start + 1]]);
            let c1 = u16::from_le_bytes([data[start + 2], data[start + 3]]);
            let color = |c: u16| -> [u8; 3] {
                let r = ((c >> 11) & 31) as u32;
                let g = ((c >> 5) & 63) as u32;
                let b = (c & 31) as u32;
                [
                    ((r * 255 + 15) / 31) as u8,
                    ((g * 255 + 31) / 63) as u8,
                    ((b * 255 + 15) / 31) as u8,
                ]
            };
            let a = color(c0);
            let b = color(c1);
            let blend = |x: [u8; 3], y: [u8; 3], mx: u16, my: u16, d: u16| -> [u8; 3] {
                [
                    ((x[0] as u16 * mx + y[0] as u16 * my) / d) as u8,
                    ((x[1] as u16 * mx + y[1] as u16 * my) / d) as u8,
                    ((x[2] as u16 * mx + y[2] as u16 * my) / d) as u8,
                ]
            };
            let palette = [
                a,
                b,
                if c0 > c1 {
                    blend(a, b, 2, 1, 3)
                } else {
                    blend(a, b, 1, 1, 2)
                },
                if c0 > c1 {
                    blend(a, b, 1, 2, 3)
                } else {
                    [0, 0, 0]
                },
            ];
            let bits = u32::from_le_bytes(data[start + 4..start + 8].try_into().ok()?);
            for dy in 0..4usize {
                for dx in 0..4usize {
                    let x = bx * 4 + dx;
                    let y = by * 4 + dy;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let index = ((bits >> (2 * (dy * 4 + dx))) & 3) as usize;
                    let out = (y * width as usize + x) * 4;
                    // Subdued artwork lets the controls and workbench remain legible.
                    for channel in 0..3 {
                        rgba[out + channel] = (palette[index][channel] as u16 * 59 / 100) as u8;
                    }
                    rgba[out + 3] = if c0 <= c1 && index == 3 { 0 } else { 255 };
                }
            }
        }
    }
    Some((width, height, rgba))
}

fn home_launch_card<'a>(
    label: &'static str,
    title: &'static str,
    description: &'static str,
    buttons: Element<'a, Message>,
    wallpaper: bool,
) -> container::Container<'a, Message> {
    home_surface(
        column![
            text(label).size(10).style(design::accent),
            text(title).size(17),
            text(description).size(design::LABEL).style(design::muted),
            iced::widget::space().height(Fill),
            buttons,
        ]
        .spacing(9),
        wallpaper,
    )
    .padding(16)
    .width(Fill)
    .height(Length::Fixed(210.0))
}

pub(super) fn home(state: &State) -> Element<'_, Message> {
    if state.preferences.ui_theme == "Evolve Legacy" {
        return home_legacy(state);
    }
    let wallpaper = state.preferences.show_home_background;
    let hero = home_surface(
        row![
            image(image::Handle::from_bytes(include_bytes!("../../../assets/hunt.png").to_vec()))
                .width(64).height(64),
            column![
                text("Evolve Mod Manager").size(24),
                text("Your workspace for CryPAK mods, original assets, model playback and level editing.")
                    .size(design::BODY).style(design::muted),
            ].spacing(5).width(Fill),
            column![
                text("INSTALLATION").size(10).style(design::muted),
                text(if state.preferences.game_installation.is_empty() { "Not configured" } else { "Configured" }).size(design::LABEL),
            ].spacing(4),
            column![
                text("CLIENT").size(10).style(design::muted),
                text(if state.preferences.client_executable.is_empty() { "Not configured" } else { "Configured" }).size(design::LABEL),
            ].spacing(4),
        ].spacing(18).align_y(iced::Center), wallpaper,
    ).padding([16, 20]);

    let project = super::home_recent_workspace(state)
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "No project selected".into());
    let launchpads = row![
        home_launch_card(
            "INSTANT PLAY",
            "Launch Evolve",
            "Launch with enabled mod projects and staged overrides.",
            design::button("Play with Mods")
                .on_press(Message::PlayWithMods)
                .style(move |theme, status| home_button_style(theme, status, wallpaper, true))
                .width(Fill)
                .into(),
            wallpaper
        ),
        home_surface(
            column![
                text("ACTIVE PROJECT").size(10).style(design::accent),
                text(project).size(17),
                text("Continue with your selected, isolated Modding workspace.")
                    .size(design::LABEL)
                    .style(design::muted),
                iced::widget::space().height(Fill),
                row![
                    design::button("Open Project")
                        .on_press(Message::HomeContinueEditing)
                        .width(Fill),
                    design::button("Package Mod")
                        .on_press(Message::HomeOpenProject)
                        .width(Fill),
                ]
                .spacing(8),
            ]
            .spacing(9),
            wallpaper
        )
        .padding(16)
        .width(Fill)
        .height(Length::Fixed(210.0)),
        home_launch_card(
            "AUTHORING SUITE",
            "Create New Mod Project",
            "Import source assets, edit gameplay data and preview models.",
            row![
                design::button("Gameplay Values…")
                    .on_press(Message::Navigate(Page::Files))
                    .width(Fill),
                design::button("Models & Skins…")
                    .on_press(Message::Navigate(Page::AssetLibrary))
                    .width(Fill),
            ]
            .spacing(8)
            .into(),
            wallpaper
        ),
    ]
    .spacing(14);
    let dashboard = home_bento(state);
    let tools = column![
        text("QUICK TOOL ACCESS")
            .size(design::CAPTION)
            .style(design::muted),
        design::button("CryPAK Archive Explorer  →")
            .on_press(Message::Navigate(Page::Files))
            .width(Fill),
        design::button("Open Level Editor window  ↗")
            .on_press(Message::OpenLevelEditorWindow)
            .width(Fill),
        design::button("Open Asset Viewer  ↗")
            .on_press(Message::OpenViewer)
            .width(Fill),
        design::button("Developer Tools  →")
            .on_press(Message::Navigate(Page::Developer))
            .width(Fill),
    ]
    .spacing(9)
    .width(Length::FillPortion(1));
    let content = scrollable(
        column![
            whats_new_intro(state),
            hero,
            dashboard,
            launchpads,
            row![
                container(latest_announcement(state, wallpaper)).width(Length::FillPortion(2)),
                tools
            ]
            .spacing(16),
        ]
        .spacing(18)
        .padding(24),
    )
    .width(Fill)
    .height(Fill);
    with_embers(state, content.into())
}

/// Slow embers drift up behind Home's translucent panels. Only drawn with
/// the wallpaper on and motion allowed; the frame clock stops off Home.
fn with_embers<'a>(state: &State, content: Element<'a, Message>) -> Element<'a, Message> {
    if !state.preferences.show_home_background || state.preferences.reduce_motion {
        return content;
    }
    iced::widget::stack![
        iced::widget::canvas(crate::fx::Embers {
            time: state.ui.clock(std::time::Instant::now()),
        })
        .width(Fill)
        .height(Fill),
        content,
    ]
    .into()
}

/// Bento dashboard: small at-a-glance tiles laid out with FillPortion.
fn home_bento(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{bento_tile, ticked};
    let client = column![
        match state.client_version.as_deref() {
            Some(version) => design::tooltip(
                text(short_client_version(version)).size(18),
                format!("Full client version: v{version}"),
                None,
                iced::widget::tooltip::Position::Bottom,
            ),
            None => text("Unknown").size(18).into(),
        },
        text(if state.preferences.client_executable.is_empty() {
            "Client not configured"
        } else {
            "Installed client"
        })
        .size(design::CAPTION)
        .style(design::muted),
    ]
    .spacing(4);
    let enabled = state
        .modding_projects
        .as_ref()
        .map(|projects| projects.iter().filter(|p| p.enabled).count())
        .unwrap_or(0);
    let total = state.modding_projects.as_ref().map_or(0, Vec::len);
    let mods = column![
        text(format!(
            "{} enabled",
            ticked(&state.ui, "enabled_mods", enabled as f64).round() as usize
        ))
        .size(18),
        text(format!("of {total} mod project(s)"))
            .size(design::CAPTION)
            .style(design::muted),
    ]
    .spacing(4);
    let storage_total = state
        .storage_usage
        .as_ref()
        .map(|usage| usage.buckets.iter().sum::<u64>() as f64);
    let storage = column![
        text(storage_total.map_or_else(
            || "Not analyzed".to_owned(),
            |bytes| crate::ui_kit::format_bytes(ticked(&state.ui, "storage_total", bytes)),
        ))
        .size(18),
        design::button(text("Analyze").size(design::CAPTION))
            .padding([3, 8])
            .on_press_maybe(
                (!state.busy && !state.storage_analyzing)
                    .then_some(Message::OpenSettingsTab(crate::settings_page::Tab::Storage),)
            ),
    ]
    .spacing(4);
    let update = column![
        text(format!("v{}", env!("CARGO_PKG_VERSION"))).size(18),
        text(if state.available_update.is_some() {
            "Update available"
        } else {
            "Manager version"
        })
        .size(design::CAPTION)
        .style(design::muted),
    ]
    .spacing(4);
    row![
        bento_tile("GAME CLIENT", client, 2),
        bento_tile("MODS", mods, 2),
        bento_tile("STORAGE", storage, 2),
        bento_tile("MANAGER", update, 1),
    ]
    .spacing(14)
    .height(Length::Fixed(104.0))
    .into()
}

/// Shown once after the manager updates: highlights from this version's
/// release notes, dismissed into `last_seen_version`.
pub(super) fn whats_new_intro(state: &State) -> Element<'_, Message> {
    let installed = env!("CARGO_PKG_VERSION");
    if !super::intro_due(&state.preferences.last_seen_version, installed) {
        return iced::widget::space().height(0).into();
    }
    let release = state
        .news
        .cache
        .release
        .as_ref()
        .filter(|release| release.version.trim_start_matches('v') == installed);
    let mut highlights = column![].spacing(4);
    match release {
        Some(release) if !release.notes.trim().is_empty() => {
            highlights = highlights.push(
                container(super::release_notes_view(state))
                    .max_height(260.0)
                    .clip(true),
            );
        }
        Some(_) => {
            highlights = highlights.push(
                text("This release has no description.")
                    .size(design::LABEL)
                    .style(design::muted),
            );
        }
        None => {
            highlights = highlights.push(
                text("Release notes for this version will appear in What's New.")
                    .size(design::LABEL)
                    .style(design::muted),
            );
        }
    }
    container(
        column![
            row![
                text(format!("What's new in v{installed}"))
                    .size(17)
                    .width(Fill),
                design::button("Read all")
                    .style(design::secondary)
                    .on_press(Message::Navigate(Page::WhatsNew)),
                design::button("Dismiss")
                    .style(design::primary)
                    .on_press(Message::DismissIntro),
            ]
            .spacing(8)
            .align_y(iced::Center),
            highlights,
        ]
        .spacing(10),
    )
    .padding([14, 18])
    .width(Fill)
    .style(|theme| design::tinted(theme, design::Tone::Info))
    .into()
}

fn home_legacy(state: &State) -> Element<'_, Message> {
    let wallpaper = state.preferences.show_home_background;
    let hero = if let Some(workspace) = super::home_recent_workspace(state) {
        let project = state.modding_projects.as_ref().and_then(|projects| {
            projects.iter().find(|project| {
                project.workspace == workspace
                    || project.workspace.as_path()
                        == std::path::Path::new(&state.preferences.current_workspace)
            })
        });
        let label = project
            .map(|project| project.label.clone())
            .unwrap_or_else(|| {
                workspace
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Recent project".into())
            });
        let details = project
            .map(|project| {
                format!(
                    "{} · {} changed file(s)",
                    project.status,
                    project.modified.len()
                )
            })
            .unwrap_or_else(|| "Last selected editable project".into());
        home_surface(
            row![
                column![
                    legacy_section_label(
                        "YOUR WORKBENCH",
                        state.preferences.ui_theme == "Evolve Legacy"
                    ),
                    text("Continue editing").size(22),
                    row![
                        svg(Page::Files.icon())
                            .style(design::icon)
                            .width(24)
                            .height(24),
                        column![
                            text(label).size(16),
                            text(details).size(design::LABEL).style(design::muted),
                        ]
                        .spacing(4),
                    ]
                    .align_y(iced::Center)
                    .spacing(12),
                    row![
                        design::button("Continue Editing")
                            .on_press_maybe((!state.busy).then_some(Message::HomeContinueEditing))
                            .style(move |theme, status| home_button_style(
                                theme, status, wallpaper, true
                            )),
                        design::button("Open Project")
                            .on_press_maybe((!state.busy).then_some(Message::HomeOpenProject))
                            .style(move |theme, status| home_button_style(
                                theme, status, wallpaper, false
                            )),
                    ]
                    .spacing(8),
                ]
                .spacing(8)
                .width(Fill),
            ]
            .spacing(16)
            .align_y(iced::Center),
            wallpaper,
        )
        .padding([12, 20])
    } else {
        let paths_selected = !state.preferences.game_installation.is_empty()
            && !state.preferences.client_executable.is_empty();
        let primary_action = if paths_selected {
            design::button("Open Asset Library")
                .on_press(Message::Navigate(Page::AssetLibrary))
                .style(move |theme, status| home_button_style(theme, status, wallpaper, true))
        } else {
            design::button("Getting Started")
                .on_press(Message::Navigate(Page::Docs))
                .style(move |theme, status| home_button_style(theme, status, wallpaper, true))
        };
        let secondary_action = if paths_selected {
            design::button("Getting Started")
                .on_press(Message::Navigate(Page::Docs))
                .style(move |theme, status| home_button_style(theme, status, wallpaper, false))
        } else {
            design::button("Open Asset Library")
                .on_press(Message::Navigate(Page::AssetLibrary))
                .style(move |theme, status| home_button_style(theme, status, wallpaper, false))
        };
        home_surface(
            row![
                column![
                    legacy_section_label(
                        "GETTING STARTED",
                        state.preferences.ui_theme == "Evolve Legacy"
                    ),
                    text(if paths_selected {
                        "Create your first mod"
                    } else {
                        "Set up your modding workspace"
                    })
                    .size(24),
                    if paths_selected {
                        Element::from(
                            text("Import a PAK to get started.")
                                .size(design::LABEL)
                                .style(design::muted),
                        )
                    } else {
                        Element::from(
                            design::button("Set up paths")
                                .on_press(Message::Navigate(Page::Settings))
                                .style(move |theme, status| {
                                    home_button_style(theme, status, wallpaper, false)
                                }),
                        )
                    },
                    row![primary_action, secondary_action].spacing(8),
                ]
                .spacing(12)
                .width(Fill),
            ]
            .spacing(16)
            .align_y(iced::Center),
            wallpaper,
        )
        .padding([12, 20])
    };
    let mut quick = column![
        legacy_section_label("PLAY EVOLVE", state.preferences.ui_theme == "Evolve Legacy"),
        text("Launch Evolve").size(20),
    ]
    .spacing(8);
    if state.developer_unlocked && state.developer_online_mods {
        quick = quick.push(
            text("Developer online testing is armed. Server acceptance is not guaranteed.")
                .size(design::LABEL)
                .style(design::muted),
        );
    }
    quick = quick.push(
        row![
            design::button(if state.developer_unlocked && state.developer_online_mods {
                "Play with Mods Online · Dev Test"
            } else {
                "Play with Mods"
            })
            .on_press(Message::PlayWithMods)
            .padding([12, 20])
            .style(move |theme, status| home_button_style(theme, status, wallpaper, true)),
            design::button("Play without Mods")
                .on_press(Message::PlayWithoutMods)
                .padding([12, 20])
                .style(move |theme, status| home_button_style(theme, status, wallpaper, false)),
        ]
        .spacing(8),
    );
    quick = quick.push(
        text(format!(
            "Launch method: {} · Change in Settings",
            state.preferences.game_launch_mode
        ))
        .size(design::LABEL)
        .style(design::muted),
    );
    if state.preferences.game_launch_mode.contains("Direct") {
        quick = quick.push(
            text("Direct Launch is experimental and does not provide the full client session.")
                .size(design::LABEL)
                .style(design::muted),
        );
    }
    let entries = column![
        text("Your workspace").size(19),
        row![
            tile("Modding", Page::Files, wallpaper),
            tile("Asset Library", Page::AssetLibrary, wallpaper),
        ]
        .spacing(10),
        row![
            tile("Docs", Page::Docs, wallpaper),
            tile("Settings", Page::Settings, wallpaper),
        ]
        .spacing(10),
        row![
            tile("What's New", Page::WhatsNew, wallpaper),
            tile("Credits", Page::Credits, wallpaper),
        ]
        .spacing(12),
    ]
    .spacing(12)
    .width(Length::FillPortion(3));
    let status = column![
        text("At a glance").size(19),
        home_surface(
            column![
                status_card(
                    "Game installation",
                    if state.preferences.game_installation.is_empty() {
                        "Select EvolveGame in Settings".to_owned()
                    } else {
                        state.preferences.game_installation.clone()
                    },
                ),
                status_card(
                    "Modded Evolve client",
                    if state.preferences.client_executable.is_empty() {
                        "Select ModdedEvolveLauncher.exe in Settings".to_owned()
                    } else {
                        state.preferences.client_executable.clone()
                    },
                ),
                client_version_card(state.client_version.as_deref()),
                status_card(
                    "Signing stage",
                    if state.preferences.stage.is_empty() {
                        "Not prepared".to_owned()
                    } else {
                        "Isolated offline stage configured".to_owned()
                    },
                ),
                status_card(
                    "Editing project",
                    if state.preferences.current_workspace.is_empty() {
                        "None selected".to_owned()
                    } else {
                        state.preferences.current_workspace.clone()
                    },
                ),
            ]
            .spacing(16),
            wallpaper
        ),
    ]
    .spacing(12)
    .width(Length::FillPortion(2));
    // Scroll the Home controls above a fixed, viewport-sized background.
    // The art has no layout height of its own, so it never pushes the page
    // below the fold; the background stays in place while cards scroll.
    let content = scrollable(
        column![
            whats_new_intro(state),
            hero,
            home_play_surface(quick, wallpaper),
            latest_announcement(state, wallpaper),
            row![entries, status].spacing(24),
        ]
        .spacing(16)
        .padding(20),
    )
    .width(Fill)
    .height(Fill);

    if let Some(handle) = wallpaper.then(home_banner).flatten() {
        // Cover the available viewport, not a 280px banner. The DDS decoder
        // already darkens the image; a subtle shade ensures text over the
        // artwork stays readable even on bright regions.
        let background = image(handle)
            .width(Fill)
            .height(Fill)
            .content_fit(iced::ContentFit::Cover);
        let shade = container(iced::widget::space().width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(iced::Color::from_rgba(
                    0.015, 0.018, 0.025, 0.18,
                ))),
                ..container::Style::default()
            });
        // Embers drift between the artwork and the controls.
        let content = with_embers(state, content.into());
        stack![background, shade, content]
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        content.into()
    }
}

pub(super) fn docs_spec(
    section: DocsSection,
) -> (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
) {
    match section {
        DocsSection::GettingStarted => (
            "Getting Started",
            "New to Evolve Mod Manager? Follow these steps in order. You do not need to know Rust, CryEngine, or how PAK files work.",
            &[
                (
                    "Install Evolve Stage 2",
                    "Install and launch Evolve Stage 2 using your normal Modded Evolve client first. The manager does not include the game or client.",
                ),
                (
                    "Install the manager",
                    "Download the installer or portable ZIP from the project’s GitHub Releases page. The installer can create shortcuts; the portable version runs from its extracted folder.",
                ),
                (
                    "Set your paths",
                    "Open Settings and choose your EvolveGame folder and ModdedEvolveLauncher.exe. Set a safe workspace location for imported assets, projects, and backups.",
                ),
                (
                    "Save Settings",
                    "Save your paths. If the manager offers an initial setup or verification step, complete it before trying to launch modified game files.",
                ),
                (
                    "Explore before editing",
                    "Open Asset Library and import a PAK or a folder of PAKs. Preview a model or texture to learn how the library works; imports are not edits to the installed game.",
                ),
                (
                    "Make your first mod",
                    "Choose an imported asset and create an editable Mod Project. Work inside the project copy rather than changing an original PAK by hand.",
                ),
                (
                    "Test offline",
                    "Enable your Mod Project in Modding, then choose Play with Mods. Use Play without Mods as a clean comparison. Check the launch popup for readiness, warnings, and errors.",
                ),
            ],
        ),
        DocsSection::Install => (
            "Install or update the manager",
            "Choose the build that fits your needs. Stable is intended for wider use; Nightly contains newer experimental changes.",
            &[
                (
                    "Download a build",
                    "Use the repository's Releases page, not an unknown mirror. Select the Windows Setup executable or Portable ZIP.",
                ),
                (
                    "Setup vs. Portable",
                    "Setup installs the manager and optionally adds desktop and Start Menu shortcuts. Portable is extracted to a folder and run directly.",
                ),
                (
                    "Start the manager",
                    "Windows may request administrator permission for protected game-file preparation and recovery. The game and its launcher should start with normal user privileges.",
                ),
                (
                    "Update later",
                    "Open Settings > Updates, choose your channel, and use Check for Updates. When offered, Update and restart verifies and installs the matching update.",
                ),
                (
                    "Keep your data",
                    "Do not delete your manager data directory, projects, original snapshots, or recovery files when updating.",
                ),
            ],
        ),
        DocsSection::Paths => (
            "Set up game and workspace paths",
            "Correct paths are the most common first-run requirement. Keep original game files separate from editable project copies.",
            &[
                (
                    "Find your game directory",
                    "In Settings, select the EvolveGame installation directory containing the game's bin64_SteamRetail folder.",
                ),
                (
                    "Choose the normal client",
                    "Point the client setting to ModdedEvolveLauncher.exe from your existing Modded Evolve installation—not to Evolve.exe itself.",
                ),
                (
                    "Configure storage",
                    "Use Settings > Storage to inspect or change where the Asset Library, working projects, prepared files and original snapshots are kept.",
                ),
                (
                    "Save and verify",
                    "Save Settings, then follow any first-run setup or health warning. Do not treat a missing path or failed preparation as a completed mod installation.",
                ),
                (
                    "Avoid moving active files",
                    "Close the game and finish restore/recovery before changing storage locations or manually moving a signing workspace.",
                ),
            ],
        ),
        DocsSection::Import => (
            "Import PAKs into Asset Library",
            "A PAK is a container of game assets. Importing makes its contents browseable; it does not automatically activate a mod.",
            &[
                (
                    "Open Asset Library",
                    "Choose Add PAK files for one or several archives, or Import ALL PAKs from folder for a full group.",
                ),
                (
                    "Select the correct build",
                    "Do not mix unrelated Stage 2 and legacy/2012 archives. Use the library's build selection and follow any format warnings.",
                ),
                (
                    "Watch import progress",
                    "Large groups can take time. Keep the originals intact and let the import complete before creating projects from them.",
                ),
                (
                    "Find an asset",
                    "Use the library explorer, search, and tabs to inspect file paths and dependencies. Some signed/legacy formats remain experimental.",
                ),
                (
                    "Create an editable copy",
                    "Select an imported asset or archive and use the available Create editable copy / project action. If the manager asks for a signing stage, configure it first.",
                ),
            ],
        ),
        DocsSection::Browse => (
            "Preview assets and inspect dependencies",
            "The viewer helps you understand files before changing them; a successful preview does not guarantee a game-compatible export.",
            &[
                (
                    "Pick a supported file",
                    "Double-click a supported DDS, CGF, CHR, SKIN, CDF, or other indexed asset in Asset Library to inspect it.",
                ),
                (
                    "Use the viewer controls",
                    "Orbit and pan around a model, inspect the materials and model parts, and switch tabs for information such as Inspector and Dependency Graph.",
                ),
                (
                    "Check missing pieces",
                    "A character may use models, textures, skeletons and attachments from multiple PAKs. Import dependencies when the viewer reports missing pieces.",
                ),
                (
                    "Try animations carefully",
                    "CAF/DBA inspection and playback are experimental. If a clip has no compatible skeleton or source data, the viewer should report the limitation.",
                ),
                (
                    "Return to the library",
                    "Close the asset preview when finished. The source PAK remains unchanged unless you deliberately make an editable project.",
                ),
            ],
        ),
        DocsSection::Projects => (
            "Create and manage Mod Projects",
            "A Mod Project is the manager's editable copy of changed assets. Only enabled projects are included in a modded launch.",
            &[
                (
                    "Start from an import",
                    "In Asset Library, select the imported source and create an editable copy, or use the Modding project workflow.",
                ),
                (
                    "Give it a clear name",
                    "Use an easy-to-recognize project name. The name helps organize projects; it does not determine whether changes affect gameplay.",
                ),
                (
                    "Edit and save changes",
                    "Work in the project copy, then use the supported build/stage actions. Do not hand-edit the installed game's original files.",
                ),
                (
                    "Enable or disable",
                    "Use the Modding project's enable switch to decide whether it participates in Play with Mods. Disabled projects remain in your workspace.",
                ),
                (
                    "Watch for conflicts",
                    "Two enabled projects targeting the same game archive can conflict. Resolve target overlap or warnings before a launch.",
                ),
            ],
        ),
        DocsSection::Edit => (
            "Edit and rebuild PAK contents",
            "Make small, reversible changes first. Archive inspection, rebuilding and signing compatibility depend on the source build.",
            &[
                (
                    "Open a project",
                    "Go to Modding, select your editable project, and inspect its files. Keep the original imported source unchanged.",
                ),
                (
                    "Choose a supported edit",
                    "Modify a texture, model-related asset, GFX resource or other supported file. Not every binary format has a full editor yet.",
                ),
                (
                    "Check dependencies",
                    "Keep the original file path and format requirements in mind. If an asset references another PAK, make sure it is available.",
                ),
                (
                    "Prepare your build",
                    "Use the project build/staging controls rather than replacing installed game PAKs manually. Signed Stage 2 archives require the manager's verified signing setup.",
                ),
                (
                    "Fix warnings before play",
                    "If validation reports a signing, archive or dependency error, correct it first. Do not bypass an original-backup or recovery warning.",
                ),
            ],
        ),
        DocsSection::Gfx => (
            "Work with menu and GFX assets",
            "GFX/Scaleform menus contain both artwork and behavior. Cosmetic changes can still affect navigation and runtime callbacks.",
            &[
                (
                    "Open the relevant GFX",
                    "Import the correct UI assets, then open a GFX file in the library viewer or Modding project.",
                ),
                (
                    "Use GFX tools",
                    "Preview supported frames, inspect text and artwork, and use the editor's exposed GFX controls where available.",
                ),
                (
                    "Expect external dependencies",
                    "Original menus can require DDS images, fonts, localization strings, videos and native game callbacks. A blank preview is not always a missing button.",
                ),
                (
                    "Edit in a project copy",
                    "Save the changed GFX and any supporting DDS resources together, preserving their intended paths and names.",
                ),
                (
                    "Verify in-game",
                    "The preview cannot fully reproduce all game initialization and service callbacks. Test an offline modded launch and compare against clean gameplay.",
                ),
            ],
        ),
        DocsSection::Export => (
            "Export models, textures and animations",
            "Exports are for inspection and creative work. Their quality depends on source meshes, rigs, materials and animation dependencies.",
            &[
                (
                    "Select the asset",
                    "Open a model or character assembly in Asset Library and choose the supported export action.",
                ),
                (
                    "Review the export options",
                    "For FBX exports, select rigging, texture embedding and dependencies when available. A character assembly may need multiple source files.",
                ),
                (
                    "Textures and normal maps",
                    "DDS files can be inspected and supported images exported to PNG. Keep material assignments and normal maps with models when needed.",
                ),
                (
                    "Open in Blender",
                    "Inspect UVs, materials, rigging, and normals after export. Experimental exports may require corrections and are not guaranteed to match in-game rendering.",
                ),
                (
                    "Level exports",
                    "Terrain and scene exports are also experimental. Use source/import warnings to identify missing textures or geometry.",
                ),
            ],
        ),
        DocsSection::LaunchMods => (
            "Play with Mods (offline)",
            "The normal modded workflow stages enabled projects and launches through the Modded Evolve client with recoverable game-file swaps.",
            &[
                (
                    "Finish editing",
                    "Save outstanding changes and build your enabled projects in Modding before starting a match.",
                ),
                (
                    "Check configuration",
                    "Make sure game/client paths, signing stage and original backups are ready. Follow any setup prompts.",
                ),
                (
                    "Click Play with Mods",
                    "Use the Home button. The Game Launch popup shows preparation, progress, errors and when the game has been released.",
                ),
                (
                    "Wait for the actual result",
                    "Hook ready, preparing and verified swap are different states. Do not assume the game is running with mods until the popup confirms the relevant step.",
                ),
                (
                    "Keep the session isolated",
                    "Use this as an offline test unless you have an explicit and separately authorized online testing configuration. Regular users should not assume online compatibility.",
                ),
            ],
        ),
        DocsSection::LaunchClean => (
            "Play without Mods",
            "Use a clean launch when you need to compare behavior or confirm that an issue comes from the modified files.",
            &[
                (
                    "Return to Home",
                    "Select Play without Mods instead of Play with Mods.",
                ),
                (
                    "Use the normal client",
                    "The manager starts the configured Modded Evolve client without applying enabled Mod Projects.",
                ),
                (
                    "Keep original files safe",
                    "If a previous modded launch was interrupted, resolve any restore/recovery warning before relying on a clean comparison.",
                ),
                (
                    "Compare the result",
                    "If an issue only occurs when playing with mods, check recently edited projects and the Game Launch error details.",
                ),
            ],
        ),
        DocsSection::Recovery => (
            "Restore originals and recover after an interruption",
            "The manager uses original-file backups and a recovery journal. Never delete these files just to silence an error.",
            &[
                (
                    "Close the game and launcher",
                    "Leave the game before attempting manual recovery, so original files are not in use.",
                ),
                (
                    "Look for the restore status",
                    "The modded launch helper normally restores original PAKs when the game and launcher exit. An interrupted session may require a separate restore action.",
                ),
                (
                    "Follow the recovery prompt",
                    "Use the manager's available restore/recovery control instead of manually swapping .pak, .customkey-original or .ready files.",
                ),
                (
                    "Keep backups intact",
                    "If validation refuses to proceed, retain the source files and logs. Do not delete a partial or stale cache until the actual failure is understood.",
                ),
                (
                    "Try the clean launch",
                    "After recovery reports success, use Play without Mods to confirm the original game still loads.",
                ),
            ],
        ),
        DocsSection::Updates => (
            "Updates, settings and logs",
            "The Settings tabs provide updates, storage, integrations, health checks and crash diagnostics.",
            &[
                (
                    "Choose an update channel",
                    "Settings > Updates lets you choose Stable, Beta or Nightly; Nightly is the fastest-moving experimental channel.",
                ),
                (
                    "Update and restart",
                    "Use the built-in updater when available. It checks and verifies the release before restarting the manager.",
                ),
                (
                    "Check integrations",
                    "Settings > Integrations contains optional tools such as Blender and GFX-related helpers where supported.",
                ),
                (
                    "Check health and storage",
                    "Use System Health and Storage to find missing paths, available space or configuration problems.",
                ),
                (
                    "Find diagnostic files",
                    "Settings > Advanced includes Open logs folder and Open crash log. Use the Game Launch popup for the immediate failure reason.",
                ),
            ],
        ),
        DocsSection::Troubleshoot => (
            "Fix common problems",
            "Start with the simplest checks and avoid changing original PAKs until you know what failed.",
            &[
                (
                    "Game does not start",
                    "Verify EvolveGame and ModdedEvolveLauncher.exe paths in Settings. Try Play without Mods and inspect the Game Launch report.",
                ),
                (
                    "A modded launch fails early",
                    "Read the popup/error log: preparation, signing, swap, restore and launcher failures have different causes.",
                ),
                (
                    "An asset is missing",
                    "Check whether its PAK and dependencies were imported from the same build. Reopen the library after importing missing archives.",
                ),
                (
                    "The model looks wrong",
                    "Missing materials, high-resolution DDS streams, skeleton data or attachments can affect the viewer and Blender export.",
                ),
                (
                    "The interface appears unresponsive",
                    "Check the footer progress or Activity window for long imports, builds, and exports. Avoid starting competing operations.",
                ),
                (
                    "An update or crash fails",
                    "Open crash logs and updater errors from Settings. Keep the full error text and reproduce the action before reporting a bug.",
                ),
            ],
        ),
        DocsSection::Faq => (
            "Frequently asked questions",
            "Short answers to questions that often come up during a first mod.",
            &[
                (
                    "Does importing a PAK change my game?",
                    "No. Importing into Asset Library creates an inspection workspace. Playing with mods is a separate launch workflow.",
                ),
                (
                    "Do I need Rust or Visual Studio?",
                    "No, not to run the downloaded installer or portable manager.",
                ),
                (
                    "Can I play online with mods?",
                    "Normal Play with Mods is designed for offline testing. Developer online testing is separate, opt-in, and does not guarantee server acceptance.",
                ),
                (
                    "Will the manager restore my originals?",
                    "The normal modded workflow is designed to restore backed-up original PAKs on exit. Interrupted launches may still require manual recovery.",
                ),
                (
                    "Why is a model or animation incomplete?",
                    "Supported CryEngine assets often reference other archives or formats the preview/export pipeline does not fully implement yet.",
                ),
                (
                    "Should I edit original game PAKs directly?",
                    "No. Use an editable Mod Project and let the manager handle verified staging, backups, and recovery.",
                ),
            ],
        ),
    }
}

pub(super) fn docs(state: &State) -> Element<'_, Message> {
    crate::docs_page::view(state)
}

pub(super) fn whats_new(state: &State) -> Element<'_, Message> {
    let news = &state.news;
    let mut cards = column![
        text("What's New").size(24),
        row![
            design::button(if news.loading {
                "Refreshing..."
            } else {
                "Refresh"
            })
            .on_press_maybe((!news.loading).then_some(Message::RefreshNews)),
        ]
        .spacing(15)
        .align_y(iced::Center),
        text("LATEST VERSION").size(14).style(design::accent),
    ]
    .spacing(12);

    let release_card: Element<'_, Message> = if let Some(release) = &news.cache.release {
        surface(
            column![
                text(&release.title).size(18),
                text(format!(
                    "{}  ·  Published {}",
                    release.version, release.published_at
                ))
                .size(design::LABEL)
                .style(design::muted),
                if release.notes.is_empty() {
                    Element::from(text("No release description was published.").size(design::BODY))
                } else {
                    super::release_notes_view(state)
                },
                design::button("View full release notes on GitHub")
                    .on_press(Message::OpenNewsLink(release.url.clone())),
            ]
            .spacing(9),
        )
        .into()
    } else if news.loading {
        news_skeleton(state, 4)
    } else {
        surface(text("No published release notes available yet.").size(design::BODY)).into()
    };
    cards = cards.push(release_card);
    if !news.release_status.is_empty() {
        cards = cards.push(
            text(&news.release_status)
                .size(design::CAPTION)
                .style(design::muted),
        );
    }
    cards = cards.push(text("DISCORD ANNOUNCEMENTS").size(14).style(design::accent));

    if news.cache.announcements.is_empty() && news.loading {
        for _ in 0..3 {
            cards = cards.push(news_skeleton(state, 3));
        }
    } else if news.cache.announcements.is_empty() {
        cards = cards.push(crate::ui_kit::empty_state(
            Some(Page::WhatsNew.icon()),
            "No announcements yet",
            "Discord channel messages appear here once the GitHub Pages feed publishes them.",
            Some(("Refresh", Message::RefreshNews)),
        ));
    } else {
        // Announcements read as a timeline: newest first, one dot per post.
        let mut items = Vec::new();
        let displayable_images = crate::whats_new::image_urls(&news.cache.announcements);
        for post in news.cache.announcements.iter().take(20) {
            let mut details = column![].spacing(8);
            if !post.content.is_empty() {
                details = details.push(text(&post.content).size(design::BODY));
            }
            if !post.description.is_empty() {
                details = details.push(text(&post.description).size(design::BODY));
            }
            // Never display a link to a private Discord server or open
            // attachments using Windows Explorer.
            for url in post.image_urls.iter().take(2) {
                if !displayable_images.contains(url) {
                    continue;
                }
                if let Some(handle) = news.image_handles.get(url) {
                    details = details.push(
                        image(handle.clone())
                            .width(Fill)
                            .height(Length::Fixed(290.0))
                            .content_fit(iced::ContentFit::Contain),
                    );
                } else if news.image_failures.contains(url) {
                    details = details.push(
                        text("Image preview unavailable")
                            .size(design::CAPTION)
                            .style(design::muted),
                    );
                } else {
                    details = details.push(
                        text("Loading image…")
                            .size(design::CAPTION)
                            .style(design::muted),
                    );
                }
            }
            items.push(crate::ui_kit::TimelineItem {
                title: if post.title.is_empty() {
                    post.author.clone()
                } else {
                    post.title.clone()
                },
                meta: format!("{}  ·  {}", post.author, post.timestamp),
                body: Some(surface(details).into()),
                tone: None,
            });
        }
        cards = cards.push(crate::ui_kit::timeline(items));
    }
    if !news.discord_status.is_empty() {
        cards = cards.push(
            text(&news.discord_status)
                .size(design::CAPTION)
                .style(design::muted),
        );
    }
    scrollable(cards.padding(12)).height(Fill).into()
}

/// Placeholder card with pulsing lines while news loads.
fn news_skeleton(state: &State, lines: usize) -> Element<'_, Message> {
    let alpha = crate::ui_kit::pulse(
        state.ui.clock(std::time::Instant::now()),
        state.preferences.reduce_motion,
    );
    let mut column = column![
        crate::ui_kit::skeleton(Length::FillPortion(2), 16.0, alpha),
        crate::ui_kit::skeleton(Length::Fixed(180.0), 10.0, alpha),
    ]
    .spacing(9);
    for index in 0..lines {
        column = column.push(if index + 1 == lines {
            crate::ui_kit::skeleton(Length::FillPortion(3), 11.0, alpha)
        } else {
            crate::ui_kit::skeleton(Fill, 11.0, alpha)
        });
    }
    surface(column).into()
}

pub(super) fn credits() -> Element<'static, Message> {
    scrollable(column![
        text("Credits").size(24),
        surface(column![
            text("@CorvoDL on Discord").size(16),
            text("Project by @CorvoDL").size(design::BODY).style(design::muted),
            text("PROJECT REPOSITORY").size(design::LABEL),
            text("github.com/corvodl/Evolve-Mod-Manager").size(design::BODY),
            text("OFFICIAL GAME INSTALLER").size(design::LABEL),
            text("modded-evolve.com").size(design::BODY),
        ].spacing(10)),
        surface(column![
            text("Interface icons").size(16),
            text("Private manager navigation icon design. Flaticon Uicons attribution retained in the original project.").size(design::LABEL).style(design::muted),
        ].spacing(8)),
        text("Install Evolve Stage 2 via modded-evolve.com before using the manager.").size(design::LABEL).style(design::muted),
    ].spacing(12).padding(12)).height(Fill).into()
}

// --- Developer tab building blocks -----------------------------------------

/// A titled card inside a Developer tab, with an optional ⓘ note.
fn dev_card<'a>(
    title: &'static str,
    note: Option<&'static str>,
    trailing: Option<Element<'a, Message>>,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut head = row![design::heading(title)]
        .spacing(8)
        .align_y(iced::Center);
    if let Some(note) = note {
        head = head.push(design::tooltip(
            crate::icons::glyph(crate::icons::info(), 15.0),
            note,
            None,
            iced::widget::tooltip::Position::Top,
        ));
    }
    head = head.push(iced::widget::space().width(Fill));
    if let Some(trailing) = trailing {
        head = head.push(trailing);
    }
    container(column![head, body.into()].spacing(10))
        .padding(12)
        .width(Fill)
        .style(|theme: &iced::Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, design::Radius::Medium).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A labelled button that explains itself while disabled.
fn dev_action<'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<Message>,
    reason: &'a str,
) -> Element<'a, Message> {
    let disabled = message.is_none();
    crate::ui_kit::why_disabled(
        design::button(text(label)).on_press_maybe(message),
        disabled,
        reason,
    )
}

/// Key / value line; values are monospaced and optionally copyable.
fn dev_value<'a>(
    state: &'a State,
    key: &'static str,
    value: String,
    copy: Option<&str>,
) -> Element<'a, Message> {
    let mut line = row![
        design::caption(key).width(Length::Fixed(170.0)),
        design::mono(value.clone()).width(Fill),
    ]
    .spacing(8)
    .align_y(iced::Center);
    if let Some(id) = copy {
        line = line.push(crate::ui_kit::copy_button(&state.ui, id, value));
    }
    line.into()
}

fn muted_line<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    text(content)
        .size(design::LABEL)
        .style(design::muted)
        .into()
}

/// Why teleport actions are unavailable, or `""` when they are available.
fn teleport_reason(state: &State) -> &'static str {
    if !state.developer_unlocked {
        "Unlock the developer area first"
    } else if state.teleport_busy {
        "A teleport action is running"
    } else if state.teleport_detect_pending {
        "Detecting Evolve.exe…"
    } else if state.teleport_hold_cancel.is_some() {
        "Stop the current hold first"
    } else if !state.live_edit_enabled {
        "Enable actor memory writes on Actors & Memory first"
    } else {
        ""
    }
}

/// `0x1234ABCD` or `Evolve.exe+0x1234`.
pub(super) fn ai_address_valid(value: &str) -> bool {
    let value = value.trim();
    let offset = match value.split_once('+') {
        Some((module, offset)) => {
            if !module.trim().eq_ignore_ascii_case("evolve.exe") {
                return false;
            }
            offset.trim()
        }
        None => value,
    };
    offset
        .strip_prefix("0x")
        .or_else(|| offset.strip_prefix("0X"))
        .is_some_and(|hex| !hex.is_empty() && usize::from_str_radix(hex, 16).is_ok())
}

// --- Actors & Memory ---------------------------------------------------------

fn research_rows(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge, disclosure};
    let mut body = column![
        row![
            design::heading("Runtime memory research"),
            badge(BadgeKind::ReadOnly, "Read-only"),
        ]
        .spacing(8)
        .align_y(iced::Center),
        muted_line(if state.runtime_research_status.is_empty() {
            "Refresh actors to validate the current process and research layouts."
        } else {
            state.runtime_research_status.as_str()
        }),
    ]
    .spacing(8);
    let Some(report) = &state.runtime_research else {
        return body.into();
    };
    let open = |key: &str| state.developer_open.contains(key);
    let roster = state.match_perks.as_ref().map(|perks| {
        perks
            .players
            .iter()
            .filter(|player| player.slot.starts_with("PlayerSlot_monster_"))
            .map(|player| player.slot.as_str())
            .collect::<Vec<_>>()
    });
    body = body.push(disclosure(
        &state.ui,
        "research-process",
        design::heading("Process"),
        open("research-process"),
        Message::DeveloperDisclose("research-process"),
        move || {
            let mut list = column![
                dev_value(state, "PID", report.pid.to_string(), None),
                dev_value(
                    state,
                    "Module base",
                    format!("0x{:X}", report.base),
                    Some("research-base")
                ),
                dev_value(
                    state,
                    "Local actor ID",
                    format!("0x{:X}", report.local_id),
                    Some("research-local")
                ),
                dev_value(
                    state,
                    "Executable SHA-256",
                    report.image_sha256.clone(),
                    Some("research-sha")
                ),
                text(&report.slots).size(design::LABEL),
            ]
            .spacing(6);
            if let Some(slots) = roster {
                list = list.push(muted_line(format!(
                    "Logged roster: {} monster slot(s) [{}]. Roster evidence, not a live allocation API.",
                    slots.len(),
                    slots.join(", ")
                )));
            }
            list.into()
        },
    ));
    body = body.push(disclosure(
        &state.ui,
        "research-resources",
        design::heading("Monster resources"),
        open("research-resources"),
        Message::DeveloperDisclose("research-resources"),
        move || {
            column![
                design::mono(report.evolution_energy.as_str()),
                design::mono(report.traversal_energy.as_str()),
                muted_line("Decoded with a fresh process key and verified actor, entity and movement links. Snapshot only; write safety is not established."),
            ]
            .spacing(6)
            .into()
        },
    ));
    body = body.push(disclosure(
        &state.ui,
        "research-gravity",
        design::heading("Gravity & jetpack"),
        open("research-gravity"),
        Message::DeveloperDisclose("research-gravity"),
        move || {
            column![
                design::mono(report.character_gravity.as_str()),
                design::mono(report.world_gravity.as_str()),
                design::mono(report.jetpack.as_str()),
            ]
            .spacing(6)
            .into()
        },
    ));
    let errors = report.inventory_errors.len();
    body = body.push(disclosure(
        &state.ui,
        "research-inventory",
        row![
            design::heading("Inventory"),
            badge(BadgeKind::Neutral, report.inventory.len().to_string()),
        ]
        .spacing(8)
        .align_y(iced::Center),
        open("research-inventory"),
        Message::DeveloperDisclose("research-inventory"),
        move || {
            let mut list = column![].spacing(8);
            for (index, inv) in report.inventory.iter().enumerate() {
                let items = inv
                    .item_ids
                    .iter()
                    .map(|id| format!("0x{id:X}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let occupied = inv.loadout.iter().filter(|(id, _)| *id != 0).count();
                let flags = inv
                    .loadout
                    .iter()
                    .enumerate()
                    .filter(|(_, (id, _))| *id != 0)
                    .map(|(slot, (id, flag))| format!("{slot}:0x{id:X}/0x{flag:X}"))
                    .collect::<Vec<_>>()
                    .join(" · ");
                list = list.push(
                    container(
                        column![
                            text(inv.actor_name.as_str()).size(design::BODY),
                            dev_value(
                                state,
                                "Actor ID",
                                format!("0x{:X}", inv.actor_id),
                                Some(&format!("inv-actor-{index}")),
                            ),
                            dev_value(
                                state,
                                "Inventory",
                                format!("0x{:X}", inv.inventory_address),
                                Some(&format!("inv-addr-{index}")),
                            ),
                            dev_value(
                                state,
                                "Items",
                                format!(
                                    "{} ({} registered) [{items}]",
                                    inv.item_ids.len(),
                                    inv.registered_ids
                                ),
                                None,
                            ),
                            dev_value(
                                state,
                                "Selected / previous",
                                format!(
                                    "0x{:X} / 0x{:X} · in list: {} · mirrors match: {}",
                                    inv.current,
                                    inv.previous,
                                    inv.selected_in_list,
                                    inv.actor_mirrors_match
                                ),
                                None,
                            ),
                            dev_value(state, "Loadout", format!("{occupied}/16 · {flags}"), None),
                        ]
                        .spacing(4),
                    )
                    .padding(8)
                    .width(Fill)
                    .style(|theme: &iced::Theme| container::Style {
                        background: Some(Background::Color(design::surface(theme))),
                        ..container::Style::default()
                    }),
                );
            }
            if errors > 0 {
                let detail = report
                    .inventory_errors
                    .iter()
                    .take(16)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                list = list.push(crate::ui_kit::callout(
                    design::Tone::Danger,
                    format!("{errors} inventory record(s) not verified:\n{detail}"),
                ));
            }
            list.into()
        },
    ));
    body = body.push(disclosure(
        &state.ui,
        "research-projectiles",
        design::heading("Projectiles"),
        open("research-projectiles"),
        Message::DeveloperDisclose("research-projectiles"),
        move || {
            let mut list = column![].spacing(5);
            for note in &report.projectiles {
                list = list.push(muted_line(note.as_str()));
            }
            list.push(muted_line(report.feature_status.as_str())).into()
        },
    ));
    body.into()
}

fn actor_rows(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let Some(listing) = &state.actor_listing else {
        return crate::ui_kit::empty_state(
            Some(crate::icons::actors()),
            "No actor snapshot",
            "Detect Evolve.exe, then refresh actors.",
            None,
        );
    };
    let query = state.actor_field_search.trim().to_ascii_lowercase();
    let field_count: usize = listing
        .actors
        .iter()
        .map(|actor| {
            actor
                .fields
                .iter()
                .filter(|entry| {
                    query.is_empty() || entry.field.title().to_ascii_lowercase().contains(&query)
                })
                .count()
        })
        .sum();
    let mut list = column![
        row![
            design::heading(format!("{} actor(s)", listing.actors.len())),
            iced::widget::space().width(Fill),
            container(crate::forms::search_field(
                "Filter fields…",
                &state.actor_field_search,
                Some(field_count),
                Message::ActorFieldSearch,
            ))
            .width(Length::Fixed(260.0)),
        ]
        .spacing(8)
        .align_y(iced::Center)
    ]
    .spacing(10);
    let editable = state.developer_unlocked && !state.actor_busy;
    for actor in &listing.actors {
        let collapsed = state.developer_collapsed_actors.contains(&actor.id);
        let mut head = row![
            crate::icons::glyph(
                if collapsed {
                    crate::icons::chevron()
                } else {
                    crate::icons::chevron_down()
                },
                14.0
            ),
            text(actor.name.as_str()).size(14),
        ]
        .spacing(8)
        .align_y(iced::Center);
        if actor.local {
            head = head.push(badge(BadgeKind::Enabled, "Local"));
        }
        let header = row![
            design::button(head)
                .style(design::text_button)
                .padding([4, 6])
                .on_press(Message::DeveloperToggleActor(actor.id)),
            design::mono(format!("ID 0x{:X}", actor.id)),
            crate::ui_kit::copy_button(
                &state.ui,
                &format!("actor-id-{}-{}", actor.id, actor.address),
                format!("0x{:X}", actor.id),
            ),
            iced::widget::space().width(Fill),
            design::caption(format!("Actor 0x{:X}", actor.address)),
        ]
        .spacing(8)
        .align_y(iced::Center);
        let mut table = column![header].spacing(2);
        if !collapsed {
            table = table.push(
                row![
                    design::caption("Field").width(Length::FillPortion(4)),
                    design::caption("Offset").width(Length::FillPortion(2)),
                    design::caption("Type").width(Length::FillPortion(2)),
                    design::caption("Value").width(Length::Fixed(110.0)),
                    design::caption("Freeze").width(Length::Fixed(52.0)),
                ]
                .spacing(8)
                .padding([4, 6]),
            );
            for entry in &actor.fields {
                let field = entry.field;
                if !query.is_empty() && !field.title().to_ascii_lowercase().contains(&query) {
                    continue;
                }
                let selected = state.actor_edit.is_some_and(|(id, addr, f, _)| {
                    id == actor.id && addr == actor.address && f == field
                });
                let frozen = state
                    .actor_frozen
                    .contains_key(&(actor.id, actor.address, field));
                let value_cell: Element<'_, Message> = if selected {
                    design::text_input("Value", &state.actor_edit_value)
                        .on_input(Message::ActorEditValue)
                        .on_submit(Message::ActorWrite)
                        .width(Length::Fixed(110.0))
                        .into()
                } else {
                    dev_action(
                        entry.display.as_str(),
                        editable.then_some(Message::ActorEdit(
                            actor.id,
                            actor.address,
                            field,
                            entry.bits,
                        )),
                        "Reading actors…",
                    )
                };
                let pin = design::tooltip(
                    iced::widget::button(crate::icons::glyph(crate::icons::pin(), 15.0))
                        .padding(5)
                        .style(if frozen {
                            design::primary
                        } else {
                            design::text_button
                        })
                        .on_press(Message::ActorFreeze(
                            actor.id,
                            actor.address,
                            field,
                            !frozen,
                        )),
                    if frozen {
                        "Unfreeze"
                    } else {
                        "Freeze the current value"
                    },
                    None,
                    iced::widget::tooltip::Position::Left,
                );
                table = table.push(
                    container(
                        row![
                            text(field.title())
                                .size(design::LABEL)
                                .width(Length::FillPortion(4)),
                            design::mono(format!("+0x{:X}", field.offset()))
                                .width(Length::FillPortion(2)),
                            design::caption(field.kind().to_string()).width(Length::FillPortion(2)),
                            container(value_cell).width(Length::Fixed(110.0)),
                            container(pin).width(Length::Fixed(52.0)),
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    )
                    .padding([2, 6])
                    .style(move |theme: &iced::Theme| container::Style {
                        background: frozen.then(|| Background::Color(design::selection(theme))),
                        ..container::Style::default()
                    }),
                );
            }
        }
        list = list.push(surface(table).padding(10));
    }
    list.push(muted_line(
        "Click a value to edit it · Enter writes · the pin holds the current value",
    ))
    .into()
}

fn match_perk_rows(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let mut rows = column![
        design::heading("Current match · character perks"),
        muted_line(if state.match_perks_status.is_empty() {
            "Refresh actors to verify the round-start log against live perk entities."
        } else {
            state.match_perks_status.as_str()
        }),
    ]
    .spacing(8);
    if let Some(listing) = &state.match_perks {
        let mut grid = row![].spacing(8);
        for player in &listing.players {
            let mut tags = row![badge(BadgeKind::Neutral, player.class.as_str())].spacing(5);
            match player.actor_id {
                Some(id) => {
                    if player.local {
                        tags = tags.push(badge(BadgeKind::Enabled, "Local"));
                    }
                    tags = tags.push(design::mono(format!("0x{id:X}")));
                }
                None => tags = tags.push(badge(BadgeKind::Conflict, "Actor not identified")),
            }
            let mut perks = row![].spacing(5);
            if player.perks.is_empty() {
                perks = perks.push(design::caption("No selected perks"));
            }
            for perk in &player.perks {
                perks = perks.push(badge(BadgeKind::Signed, perk.as_str()));
            }
            grid = grid.push(
                container(
                    column![
                        text(player.display.as_str()).size(design::BODY),
                        design::caption(format!("{} · {}", player.character, player.slot)),
                        tags.wrap(),
                        perks.wrap(),
                    ]
                    .spacing(6),
                )
                .padding(10)
                .width(Length::Fixed(270.0))
                .style(|theme: &iced::Theme| container::Style {
                    background: Some(Background::Color(design::field(theme))),
                    border: Border {
                        color: design::border(theme),
                        width: 1.0,
                        radius: design::radius(theme, design::Radius::Medium).into(),
                    },
                    ..container::Style::default()
                }),
            );
        }
        rows = rows.push(grid.wrap().vertical_spacing(8));
    }
    rows.into()
}

fn actor_controls(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge, switch};
    let detected = state
        .runtime_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.pid)
        .is_some();
    let process = dev_card(
        "Process",
        None,
        Some(crate::ui_kit::icon_button(
            crate::icons::refresh(),
            "Detect Evolve.exe",
            (!state.runtime_checking).then_some(Message::RuntimeDetect),
            Some("Detecting…"),
        )),
        column![
            row![badge(
                if detected {
                    BadgeKind::Enabled
                } else {
                    BadgeKind::Neutral
                },
                state
                    .runtime_snapshot
                    .as_ref()
                    .map_or("Not detected", |snapshot| snapshot.status.as_str()),
            ),],
            design::caption(
                state
                    .runtime_snapshot
                    .as_ref()
                    .map_or("Executable not checked", |snapshot| {
                        snapshot.executable.as_str()
                    }),
            ),
        ]
        .spacing(6),
    );
    let safety = dev_card(
        "Safety",
        Some("Offline memory only. Validate fields before editing."),
        state
            .live_edit_enabled
            .then(|| badge(BadgeKind::Conflict, "Writes armed")),
        column![
            switch(
                &state.ui,
                "dev-armor-scan",
                "Guided armor scanning",
                state.armor_offline_confirmed,
                Message::ArmorOfflineChanged(!state.armor_offline_confirmed),
            ),
            design::caption("Optional. Scans for armor values step by step."),
            switch(
                &state.ui,
                "dev-live-edit",
                "Actor memory writes and Freeze",
                state.live_edit_enabled,
                Message::LiveEditEnabled(!state.live_edit_enabled),
            ),
            design::caption("Experimental. Needed for editing values, teleporting and holds."),
        ]
        .spacing(6),
    );
    let refresh_reason = if state.actor_busy {
        "Reading actors…"
    } else {
        "Detect Evolve.exe first"
    };
    column![
        process,
        safety,
        row![
            dev_action(
                if state.actor_busy {
                    "Reading actors and perks…"
                } else {
                    "Refresh actors and match perks"
                },
                (!state.actor_busy && detected).then_some(Message::ActorRefresh),
                refresh_reason,
            ),
            muted_line(state.actor_status.as_str()),
        ]
        .spacing(10)
        .align_y(iced::Center),
        match_perk_rows(state),
        research_rows(state),
        actor_rows(state),
    ]
    .spacing(12)
    .into()
}

// --- Keybinds ---------------------------------------------------------------

fn developer_keybind_controls(state: &State) -> Element<'_, Message> {
    let assigned = state
        .preferences
        .developer_hotkeys
        .iter()
        .filter(|&&k| k != 0)
        .count();
    let mut rows = column![
        row![
            design::caption(format!("{assigned} / 10 shortcuts assigned"))
                .width(Length::Fixed(200.0)),
            iced::widget::progress_bar(0.0..=10.0, assigned as f32).girth(6),
        ]
        .spacing(10)
        .align_y(iced::Center),
        muted_line("Shortcuts need the external overlay, and actor memory writes for movement."),
        row![
            crate::ui_kit::hold_button(
                &state.ui,
                "developer-keybind-defaults",
                "Hold: reset to defaults (F6–F12, Ctrl+F6/F7/F8)",
                Message::DeveloperKeybindDefaults,
                true,
                state.preferences.reduce_motion,
            ),
            muted_line("Click a key to change it · Esc cancels · Backspace clears"),
        ]
        .spacing(10)
        .align_y(iced::Center),
    ]
    .spacing(8);
    let conflicts = keybind_conflicts(&state.preferences.developer_hotkeys);
    for action in developer_hotkeys::Action::ALL {
        let current =
            developer_hotkeys::Keybind(state.preferences.developer_hotkeys[action.index()]);
        let capturing = state.developer_capture == Some(action);
        let cap: Element<'_, Message> = if capturing {
            design::button(text("Press a key…").size(design::LABEL))
                .style(design::primary)
                .on_press(Message::DeveloperCaptureKey(None))
                .into()
        } else {
            design::button(if current.0 == 0 {
                Element::from(design::caption("Not set"))
            } else {
                design::key_hint(&current.to_string())
            })
            .style(design::text_button)
            .on_press(Message::DeveloperCaptureKey(Some(action)))
            .into()
        };
        let mut line = row![
            text(action.label()).width(Length::Fixed(200.0)),
            container(cap).width(Length::Fixed(150.0)),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8);
        if conflicts.contains(&action.index()) {
            line = line.push(crate::ui_kit::badge(
                crate::ui_kit::BadgeKind::Conflict,
                "Conflict",
            ));
        }
        rows = rows.push(line);
    }
    if !state.developer_hotkey_status.is_empty() {
        rows = rows.push(muted_line(state.developer_hotkey_status.as_str()));
    }
    rows.into()
}

// --- AI Controls ------------------------------------------------------------

fn ai_pause_controls(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let detected = state
        .runtime_snapshot
        .as_ref()
        .and_then(|s| s.pid)
        .is_some();
    let ready = state.developer_unlocked
        && state.live_edit_enabled
        && !state.developer_online_mods
        && !state.ai_pause_busy
        && detected;
    let can_pause = ready
        && !state.ai_pause_owned
        && state.ai_pause_probe.is_some_and(|probe| probe.value == 0);
    let can_resume = ready && state.ai_pause_owned;
    let reason = if state.developer_online_mods {
        "Turn off modded online testing first"
    } else if !detected {
        "Evolve.exe not detected"
    } else if !state.live_edit_enabled {
        "Enable actor memory writes on Actors & Memory first"
    } else if state.ai_pause_busy {
        "Reading or writing the AI flag…"
    } else {
        "Read a verified address first"
    };
    let address = state.ai_pause_address.trim();
    let invalid = !address.is_empty() && !ai_address_valid(address);
    let can_read = state.developer_unlocked
        && !state.developer_online_mods
        && !state.ai_pause_busy
        && !state.ai_pause_owned
        && !address.is_empty()
        && !invalid;
    let field = design::text_input("0x… or Evolve.exe+0x…", &state.ai_pause_address)
        .on_input(Message::AiAddressChanged)
        .style(move |theme: &iced::Theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Background::Color(design::field(theme));
            if invalid {
                style.border.color = theme.palette().danger;
                style.border.width = 1.0;
            }
            style
        });
    let mut address_block = column![
        row![
            field,
            dev_action(
                "Read Int32",
                can_read.then_some(Message::AiInspect),
                if invalid || address.is_empty() {
                    "Enter a valid address first"
                } else {
                    reason
                },
            ),
        ]
        .spacing(8)
        .align_y(iced::Center)
    ]
    .spacing(4);
    if invalid {
        address_block = address_block.push(
            text("Expected 0x… or Evolve.exe+0x…")
                .size(design::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(theme.palette().danger),
                }),
        );
    }
    let pill = state.ai_pause_probe.map(|probe| {
        if probe.value == 0 {
            badge(BadgeKind::Enabled, "Value 0 · Running")
        } else {
            badge(
                BadgeKind::Conflict,
                format!("Value {} · Paused", probe.value),
            )
        }
    });
    let toggle = if state.ai_pause_owned {
        crate::ui_kit::why_disabled(
            design::button(text("Resume AI").size(15))
                .padding([10, 22])
                .style(design::primary)
                .on_press_maybe(can_resume.then_some(Message::AiChange(false))),
            !can_resume,
            reason,
        )
    } else {
        crate::ui_kit::why_disabled(
            design::button(text("Pause all AI").size(15))
                .padding([10, 22])
                .style(design::primary)
                .on_press_maybe(can_pause.then_some(Message::AiChange(true))),
            !can_pause,
            reason,
        )
    };
    column![
        crate::ui_kit::callout(
            design::Tone::Warning,
            "Experimental, for isolated testing (ai_NoUpdate, Int32). Enter a separately verified CVar value address: a readable 0 or 1 alone does not prove it is ai_NoUpdate. The original 0 is restored on Resume.",
        ),
        dev_card(
            "Global AI pause",
            None,
            pill,
            column![
                address_block,
                row![
                    toggle,
                    iced::widget::space().width(Fill),
                    crate::ui_kit::icon_button(
                        crate::icons::discard(),
                        "Clear ended session",
                        (!state.ai_pause_busy && state.ai_pause_probe.is_some())
                            .then_some(Message::AiForget),
                        Some("Nothing to clear"),
                    ),
                ]
                .align_y(iced::Center),
                muted_line(if state.ai_pause_busy {
                    "Reading or writing the AI update flag…"
                } else if state.ai_pause_status.is_empty() {
                    "Read an externally verified address to enable pause."
                } else {
                    state.ai_pause_status.as_str()
                }),
            ]
            .spacing(10),
        ),
    ]
    .spacing(12)
    .into()
}

// --- Teleport & Fly -----------------------------------------------------------

fn teleport_controls(state: &State) -> Element<'_, Message> {
    use runtime_teleport::{HoldMode, MovementFrame};
    let can_read = state.developer_unlocked
        && !state.teleport_busy
        && !state.teleport_detect_pending
        && state.teleport_hold_cancel.is_none();
    let enabled = can_read && state.live_edit_enabled;
    let reason = teleport_reason(state);
    // Reading never needs armed writes, so it skips that last reason.
    let read_reason = if can_read { "" } else { reason };

    // Status: one island for every teleport message.
    let working = state.teleport_busy
        || state.teleport_detect_pending
        || state.teleport_aim_preview_busy
        || state.teleport_hold_releasing;
    let status = if state.teleport_detect_pending {
        "Detecting Evolve.exe before the requested action…".to_owned()
    } else if state.teleport_busy {
        "Teleport operation running…".to_owned()
    } else if !state.teleport_hold_status.is_empty() && state.teleport_hold_mode.is_some() {
        state.teleport_hold_status.clone()
    } else if !state.teleport_status.is_empty() {
        state.teleport_status.clone()
    } else if state
        .runtime_snapshot
        .as_ref()
        .and_then(|s| s.pid)
        .is_none()
    {
        "Evolve.exe is detected automatically on the first action".to_owned()
    } else {
        "Ready".to_owned()
    };
    let island = crate::ui_kit::status_island(status, None, state.loading_wheel_frame, working);

    const TELEPORT: crate::forms::NumberSpec =
        crate::forms::NumberSpec::new(-100_000.0, 100_000.0, 1.0, 2);
    let axis = |label: &'static str, index: usize, value: &str| {
        column![
            design::caption(label),
            crate::forms::number_field(
                &state.forms,
                state.keyboard_modifiers,
                format!("teleport.{index}"),
                value,
                TELEPORT,
                move |v| Message::TeleportXYZChanged(index, v),
            ),
        ]
        .spacing(3)
        .width(Fill)
    };
    let position = dev_card(
        "Position",
        Some("Reading needs no armed writes. Teleporting needs actor memory writes."),
        None,
        column![
            row![
                axis("X", 0, &state.teleport_x),
                axis("Y", 1, &state.teleport_y),
                axis("Z", 2, &state.teleport_z),
            ]
            .spacing(8),
            row![
                dev_action(
                    "Read position",
                    can_read.then_some(Message::TeleportCapture(false)),
                    read_reason,
                ),
                dev_action(
                    "Save position",
                    can_read.then_some(Message::TeleportCapture(true)),
                    read_reason,
                ),
                dev_action(
                    "Teleport XYZ",
                    enabled.then_some(Message::TeleportAbsolute),
                    reason,
                ),
                dev_action(
                    "Return to saved",
                    (enabled && state.teleport_saved.is_some())
                        .then_some(Message::TeleportReturnSaved),
                    if reason.is_empty() {
                        "Save a position first"
                    } else {
                        reason
                    },
                ),
            ]
            .spacing(8),
        ]
        .spacing(10),
    );

    let mut slots = column![].spacing(4);
    if state.preferences.teleport_slots.is_empty() {
        slots = slots.push(design::caption(
            "No saved positions. Enter or read X, Y and Z, then add them here.",
        ));
    }
    for (index, slot) in state.preferences.teleport_slots.iter().enumerate() {
        let coords = format!("{:.2}, {:.2}, {:.2}", slot.x, slot.y, slot.z);
        slots = slots.push(
            row![
                text(slot.name.as_str())
                    .size(design::BODY)
                    .width(Length::Fixed(140.0)),
                design::mono(coords.clone()).width(Fill),
                crate::ui_kit::copy_button(&state.ui, &format!("teleport-slot-{index}"), coords),
                crate::ui_kit::icon_button(
                    crate::icons::teleport(),
                    "Teleport here",
                    enabled.then_some(Message::TeleportSlotGo(index)),
                    (!reason.is_empty()).then_some(reason),
                ),
                crate::ui_kit::icon_button(
                    crate::icons::discard(),
                    "Delete position",
                    Some(Message::TeleportSlotDelete(index)),
                    None,
                ),
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
    }
    let saved = dev_card(
        "Saved positions",
        Some("Kept between sessions, up to 32."),
        Some(crate::ui_kit::icon_button(
            crate::icons::plus(),
            "Save the X, Y and Z fields",
            Some(Message::TeleportSlotSave),
            None,
        )),
        slots,
    );

    let holding = state.teleport_hold_cancel.is_some();
    let mode_index = match state.teleport_hold_mode {
        None => 0,
        Some(HoldMode::Z) => 1,
        Some(HoldMode::Xyz) => 2,
        Some(HoldMode::Fly) => 3,
    };
    let modes = crate::ui_kit::segmented(
        &state.ui,
        "teleport-hold-mode",
        vec![
            ("Off", Message::TeleportHoldRelease),
            ("Hover (Z)", Message::TeleportHold(HoldMode::Z)),
            ("Lock XYZ", Message::TeleportHold(HoldMode::Xyz)),
            ("Fly", Message::TeleportHold(HoldMode::Fly)),
        ],
        mode_index,
        (enabled || holding) && !state.teleport_hold_releasing,
    );
    let frame = crate::ui_kit::why_disabled(
        crate::ui_kit::segmented(
            &state.ui,
            "teleport-frame",
            vec![
                (
                    "Actor-facing",
                    Message::TeleportMovementFrameChanged(MovementFrame::Actor),
                ),
                (
                    "Camera-facing",
                    Message::TeleportMovementFrameChanged(MovementFrame::Camera),
                ),
            ],
            usize::from(state.teleport_movement_frame == MovementFrame::Camera),
            can_read,
        ),
        !can_read,
        if holding {
            "Stop the hold to change the reference"
        } else {
            read_reason
        },
    );
    let speed_value = state
        .teleport_fly_speed
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .unwrap_or(8.0)
        .clamp(1.0, 40.0);
    let mut hold_head = row![].spacing(6).align_y(iced::Center);
    if holding {
        hold_head = hold_head.push(design::indicator_dot(
            None,
            crate::ui_kit::pulse(
                state.ui.clock(std::time::Instant::now()),
                state.preferences.reduce_motion,
            ),
        ));
        hold_head = hold_head.push(design::caption(match state.teleport_hold_mode {
            Some(HoldMode::Z) => "Hovering",
            Some(HoldMode::Xyz) => "Locked",
            Some(HoldMode::Fly) => "Flying",
            None => "Releasing",
        }));
    }
    let hold = dev_card(
        "Hold & Fly",
        Some(
            "Hover (Z) moves X/Y with WASD. Fly adds Space and Ctrl for height and follows camera yaw. Lock XYZ holds every axis. Holds write position, not gravity or velocity; if the game overrides movement, the hold stops and says why.",
        ),
        Some(hold_head.into()),
        column![
            modes,
            row![design::caption("WASD reference"), frame]
                .spacing(10)
                .align_y(iced::Center),
            row![
                design::caption("Speed").width(Length::Fixed(60.0)),
                iced::widget::slider(1.0..=40.0, speed_value, |v| {
                    Message::TeleportFlySpeedChanged(format!("{v:.0}"))
                })
                .step(1.0_f32)
                .width(Fill),
                container(crate::forms::number_field(
                    &state.forms,
                    state.keyboard_modifiers,
                    "teleport.speed",
                    &state.teleport_fly_speed,
                    crate::forms::NumberSpec::new(1.0, 40.0, 1.0, 0),
                    Message::TeleportFlySpeedChanged,
                ))
                .width(Length::Fixed(110.0)),
                design::caption("units/s"),
            ]
            .spacing(8)
            .align_y(iced::Center),
        ]
        .spacing(10),
    );

    let step = state
        .teleport_nudge_step
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .unwrap_or(20.0)
        .clamp(0.1, 1000.0);
    let nudge = |icon, label: &'static str, dx: f32, dy: f32, dz: f32| {
        crate::ui_kit::icon_button(
            icon,
            label,
            enabled.then_some(Message::TeleportRelative(dx * step, dy * step, dz * step)),
            (!reason.is_empty()).then_some(reason),
        )
    };
    let blank = || iced::widget::space().width(30).height(30);
    let pad = column![
        row![
            blank(),
            nudge(crate::icons::chevron_up(), "+Y", 0.0, 1.0, 0.0),
            blank()
        ]
        .spacing(4),
        row![
            nudge(crate::icons::chevron_left(), "−X", -1.0, 0.0, 0.0),
            container(crate::forms::number_field(
                &state.forms,
                state.keyboard_modifiers,
                "teleport.nudge",
                &state.teleport_nudge_step,
                crate::forms::NumberSpec::new(0.1, 1000.0, 1.0, 1),
                Message::TeleportNudgeStep,
            ))
            .width(Length::Fixed(100.0)),
            nudge(crate::icons::chevron(), "+X", 1.0, 0.0, 0.0),
        ]
        .spacing(4)
        .align_y(iced::Center),
        row![
            blank(),
            nudge(crate::icons::chevron_down(), "−Y", 0.0, -1.0, 0.0),
            blank()
        ]
        .spacing(4),
    ]
    .spacing(4)
    .align_x(iced::Center);
    let vertical = column![
        nudge(crate::icons::chevron_up(), "+Z (up)", 0.0, 0.0, 1.0),
        design::caption("Z"),
        nudge(crate::icons::chevron_down(), "−Z (down)", 0.0, 0.0, -1.0),
    ]
    .spacing(4)
    .align_x(iced::Center);
    let nudge_card = dev_card(
        "Nudge",
        Some("Moves along world axes by the step in the middle."),
        None,
        row![pad, vertical].spacing(28).align_y(iced::Center),
    );

    let mut aim = column![
        row![
            design::caption("Distance").width(Length::Fixed(60.0)),
            container(crate::forms::number_field(
                &state.forms,
                state.keyboard_modifiers,
                "teleport.aim",
                &state.teleport_aim_distance,
                crate::forms::NumberSpec::new(1.0, 500.0, 1.0, 0),
                Message::TeleportAimDistanceChanged,
            ))
            .width(Length::Fixed(120.0)),
            dev_action(
                "Preview coordinates",
                (can_read && !state.teleport_aim_preview_busy)
                    .then_some(Message::TeleportPreviewAim),
                if state.teleport_aim_preview_busy {
                    "Preview running…"
                } else {
                    read_reason
                },
            ),
            dev_action(
                "Teleport to crosshair",
                enabled.then_some(Message::TeleportAim),
                reason,
            ),
        ]
        .spacing(8)
        .align_y(iced::Center)
    ]
    .spacing(8);
    if let Some(target) = state.teleport_aim_result {
        let coords = format!("{:.2}, {:.2}, {:.2}", target.x, target.y, target.z);
        aim = aim.push(
            container(
                row![
                    design::caption("Target"),
                    design::mono(coords.clone()).width(Fill),
                    crate::ui_kit::copy_button(&state.ui, "teleport-aim", coords),
                    design::button(text("Use as target").size(design::LABEL))
                        .style(design::secondary)
                        .on_press(Message::TeleportUseAim),
                ]
                .spacing(8)
                .align_y(iced::Center),
            )
            .padding([6, 10])
            .style(|theme: &iced::Theme| design::tinted(theme, design::Tone::Info)),
        );
    }
    let crosshair = dev_card(
        "Crosshair",
        Some(
            "Approximate: uses the live camera direction from the player's position. No raycast or collision test.",
        ),
        None,
        aim,
    );

    column![island, position, saved, hold, nudge_card, crosshair]
        .spacing(12)
        .into()
}

// --- External Overlay -----------------------------------------------------------

fn overlay_controls(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{TimelineItem, switch, timeline};
    let on = state.teleport_overlay.is_some() || state.teleport_overlay_pending;
    let detected = state
        .runtime_snapshot
        .as_ref()
        .and_then(|s| s.pid)
        .is_some();
    let step =
        |title: &str, done: bool, waiting: bool, meta_done: &str, meta_wait: &str| TimelineItem {
            title: title.to_owned(),
            meta: if done {
                meta_done.to_owned()
            } else if waiting {
                meta_wait.to_owned()
            } else {
                "Not yet".to_owned()
            },
            body: None,
            tone: Some(if done {
                design::Tone::Success
            } else if waiting {
                design::Tone::Warning
            } else {
                design::Tone::Info
            }),
        };
    let shortcuts = if state.teleport_overlay_experimental {
        step(
            "Keyboard hook installed",
            state.teleport_hook_ready,
            state.teleport_overlay.is_some(),
            "Shortcuts are captured while Evolve has focus",
            "Starting…",
        )
    } else {
        step(
            "Shortcuts registered",
            state.teleport_overlay.is_some(),
            state.teleport_overlay_pending,
            "Windows RegisterHotKey",
            "Waiting for the overlay…",
        )
    };
    let mut body = column![
        switch(
            &state.ui,
            "dev-overlay",
            "External overlay and shortcuts",
            on,
            Message::TeleportOverlayChanged(!on),
        ),
        design::caption("Shown only over Evolve while it has focus, in windowed fullscreen. The centre marker is not a raycast hit."),
        switch(
            &state.ui,
            "dev-hooked-input",
            "Experimental hooked input",
            state.teleport_overlay_experimental,
            Message::TeleportOverlayExperimentalChanged(!state.teleport_overlay_experimental),
        ),
        design::caption("Captures F6–F12 with a Windows low-level keyboard hook while Evolve is focused. No game injection; exclusive fullscreen and protected input are not guaranteed."),
        timeline(vec![
            step(
                "Game detected",
                detected,
                state.teleport_overlay_pending,
                "Evolve.exe is running",
                "Detecting…",
            ),
            step(
                "Overlay shown",
                state.teleport_overlay.is_some(),
                state.teleport_overlay_pending,
                "Over the game window",
                "Starting…",
            ),
            shortcuts,
        ]),
    ]
    .spacing(8);
    if !state.teleport_status.is_empty() {
        body = body.push(muted_line(state.teleport_status.as_str()));
    }
    body.into()
}

// --- Network & Process -------------------------------------------------------

fn network_controls(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let online = state.developer_online_mods;
    let mut firewall = column![crate::ui_kit::switch(
        &state.ui,
        "dev-online-mods",
        "Allow modded online testing",
        online,
        Message::DeveloperOnlineMods(!online),
    )]
    .spacing(8);
    if online {
        firewall = firewall.push(crate::ui_kit::callout(
            design::Tone::Warning,
            "Online testing is on. Server acceptance is not guaranteed.",
        ));
    }
    if state.developer_network_busy {
        firewall = firewall.push(muted_line("Applying network policy…"));
    } else if !state.developer_network_status.is_empty() {
        firewall = firewall.push(muted_line(state.developer_network_status.as_str()));
    }
    let mut privileges = column![design::caption(
        "The manager runs at High integrity; the client and game run at Medium."
    )]
    .spacing(6);
    if state.developer_privilege_busy {
        privileges = privileges.push(muted_line("Reading Windows process tokens…"));
    }
    for line in state
        .developer_privilege_status
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let level = ["System", "High", "Medium", "Low"]
            .into_iter()
            .find(|level| line.contains(level));
        let mut entry = row![design::mono(line).width(Fill)]
            .spacing(8)
            .align_y(iced::Center);
        if let Some(level) = level {
            entry = entry.push(badge(
                if level == "High" || level == "System" {
                    BadgeKind::Conflict
                } else {
                    BadgeKind::Neutral
                },
                level,
            ));
        }
        privileges = privileges.push(entry);
    }
    column![
        dev_card(
            "Firewall",
            Some("Developer-only firewall policy for the game client."),
            Some(if online {
                badge(BadgeKind::Conflict, "Online testing")
            } else {
                badge(BadgeKind::Signed, "Offline isolated")
            }),
            firewall,
        ),
        dev_card(
            "Process privileges",
            None,
            Some(crate::ui_kit::icon_button(
                crate::icons::refresh(),
                "Inspect client and game privileges",
                (!state.developer_privilege_busy)
                    .then_some(Message::DeveloperInspectProcessPrivileges),
                Some("Inspection running"),
            )),
            privileges,
        ),
    ]
    .spacing(12)
    .into()
}

fn developer_tab_status(state: &State, section: DeveloperSection) -> Option<design::Tone> {
    let failed = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("failed") || lower.contains("error")
    };
    match section {
        DeveloperSection::Network => state.developer_online_mods.then_some(design::Tone::Warning),
        DeveloperSection::Actors => {
            if failed(&state.actor_status) || failed(&state.runtime_result) {
                Some(design::Tone::Danger)
            } else {
                state.live_edit_enabled.then_some(design::Tone::Warning)
            }
        }
        DeveloperSection::Ai => {
            if failed(&state.ai_pause_status) {
                Some(design::Tone::Danger)
            } else {
                state.ai_pause_owned.then_some(design::Tone::Success)
            }
        }
        DeveloperSection::Teleport => {
            if failed(&state.teleport_hold_status) {
                Some(design::Tone::Danger)
            } else {
                state
                    .teleport_hold_mode
                    .is_some()
                    .then_some(design::Tone::Success)
            }
        }
        DeveloperSection::Overlay => state
            .teleport_overlay
            .is_some()
            .then_some(design::Tone::Success),
        DeveloperSection::Keybinds => {
            let conflicts = keybind_conflicts(&state.preferences.developer_hotkeys);
            (!conflicts.is_empty()).then_some(design::Tone::Warning)
        }
    }
}

fn tone_dot<'a>(tone: design::Tone) -> Element<'a, Message> {
    container(iced::widget::space().width(8).height(8))
        .style(move |theme: &iced::Theme| container::Style {
            background: Some(Background::Color(design::tone_color(theme, tone))),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// Actions whose key is also bound to another action (disabled keys never
/// conflict).
pub(super) fn keybind_conflicts(keys: &[u8]) -> std::collections::BTreeSet<usize> {
    let mut conflicts = std::collections::BTreeSet::new();
    for (a, key) in keys.iter().enumerate() {
        if *key == 0 {
            continue;
        }
        for (b, other) in keys.iter().enumerate().skip(a + 1) {
            if other == key {
                conflicts.insert(a);
                conflicts.insert(b);
            }
        }
    }
    conflicts
}

/// "Attached to Evolve.exe · PID 1234 · Online blocked", with a copy
/// button for the PID and the lock button.
fn developer_header(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let pid = state
        .runtime_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.pid);
    let mut strip = row![design::title("Developer")]
        .spacing(10)
        .align_y(iced::Center);
    strip = strip.push(match pid {
        Some(pid) => row![
            badge(
                BadgeKind::Enabled,
                format!("Attached to Evolve.exe · PID {pid}")
            ),
            crate::ui_kit::copy_button(&state.ui, "developer-pid", pid.to_string()),
        ]
        .spacing(4)
        .align_y(iced::Center),
        None => row![badge(BadgeKind::Neutral, "Evolve.exe not detected")],
    });
    strip = strip.push(if state.developer_online_mods {
        badge(BadgeKind::Conflict, "Modded online testing allowed")
    } else {
        badge(BadgeKind::Signed, "Online blocked")
    });
    if state.live_edit_enabled {
        strip = strip.push(badge(BadgeKind::Conflict, "Writes armed"));
    }
    strip
        .push(iced::widget::space().width(Fill))
        .push(crate::ui_kit::icon_button(
            crate::icons::lock(),
            "Lock developer area",
            Some(Message::LockDeveloper),
            None,
        ))
        .into()
}

pub(super) fn developer(state: &State) -> Element<'_, Message> {
    if state.developer_unlocked {
        let active = DeveloperSection::from_title(&state.preferences.developer_tab);
        let mut tabs = column![].spacing(4).width(Length::Fixed(200.0));
        for section in DeveloperSection::ALL {
            let selected = section == active;
            let mut label = row![
                crate::icons::glyph(section.icon(), 17.0),
                text(section.title()).size(design::BODY).width(Fill),
            ]
            .spacing(9)
            .align_y(iced::Center);
            if let Some(tone) = developer_tab_status(state, section) {
                label = label.push(tone_dot(tone));
            }
            tabs = tabs.push(design::tooltip(
                design::button(label)
                    .width(Fill)
                    .padding([8, 10])
                    .style(move |theme, status| design::rail(theme, status, selected))
                    .on_press(Message::DeveloperTab(section)),
                section.title(),
                crate::chrome::shortcut_for(section.title()),
                iced::widget::tooltip::Position::Right,
            ));
        }
        // Only the active tab is built, so hidden actor listings cost nothing.
        let controls = match active {
            DeveloperSection::Network => network_controls(state),
            DeveloperSection::Actors => actor_controls(state),
            DeveloperSection::Ai => ai_pause_controls(state),
            DeveloperSection::Teleport => teleport_controls(state),
            DeveloperSection::Overlay => overlay_controls(state),
            DeveloperSection::Keybinds => developer_keybind_controls(state),
        };
        column![
            developer_header(state),
            row![
                tabs,
                scrollable(
                    surface(column![design::heading(active.title()), controls].spacing(10))
                        .padding(16)
                )
                .height(Fill)
                .width(Fill),
            ]
            .spacing(14)
            .height(Fill),
        ]
        .spacing(12)
        .padding(12)
        .into()
    } else {
        let refused = state.developer_status.starts_with("Invalid")
            || state.developer_status.contains("locked");
        let show = state.developer_show_password;
        let field = row![
            crate::icons::glyph(crate::icons::lock(), 16.0),
            iced::widget::text_input("Developer password", &state.developer_password)
                .secure(!show)
                .on_input(Message::DeveloperPasswordChanged)
                .on_submit(Message::UnlockDeveloper)
                .padding([7, 4])
                .width(Fill)
                .style(|theme: &iced::Theme, status| {
                    let mut style = iced::widget::text_input::default(theme, status);
                    style.background = Background::Color(iced::Color::TRANSPARENT);
                    style.border = Border::default();
                    style
                }),
            design::tooltip(
                iced::widget::button(crate::icons::glyph(
                    if show {
                        crate::icons::eye_off()
                    } else {
                        crate::icons::eye()
                    },
                    16.0,
                ))
                .padding(4)
                .style(design::text_button)
                .on_press(Message::DeveloperShowPassword(!show)),
                if show {
                    "Hide password"
                } else {
                    "Show password"
                },
                None,
                iced::widget::tooltip::Position::Top,
            ),
        ]
        .spacing(8)
        .align_y(iced::Center);
        let mut body = column![
            design::heading("Password protected"),
            container(field)
                .padding([0, 10])
                .style(move |theme: &iced::Theme| container::Style {
                    background: Some(Background::Color(design::field(theme))),
                    border: Border {
                        color: if refused {
                            theme.palette().danger
                        } else {
                            design::border(theme)
                        },
                        width: 1.0,
                        radius: design::radius(theme, design::Radius::Medium).into(),
                    },
                    ..container::Style::default()
                }),
            design::button("Unlock")
                .style(design::primary)
                .on_press_maybe((!state.developer_checking).then_some(Message::UnlockDeveloper)),
        ]
        .spacing(12);
        if refused {
            body = body.push(crate::ui_kit::callout(
                design::Tone::Danger,
                state.developer_status.as_str(),
            ));
        } else if !state.developer_status.is_empty() {
            body = body.push(design::caption(state.developer_status.as_str()));
        }
        column![design::title("Developer"), surface(body).max_width(520)]
            .spacing(13)
            .padding(12)
            .into()
    }
}

#[cfg(test)]
mod home_layout_tests {
    use super::{ai_address_valid, keybind_conflicts, short_client_version, status_value_preview};

    #[test]
    fn ai_address_accepts_absolute_and_module_relative_hex_only() {
        assert!(ai_address_valid("0x1234ABCD"));
        assert!(ai_address_valid(" Evolve.exe+0x10 "));
        assert!(ai_address_valid("evolve.exe + 0X1f"));
        assert!(!ai_address_valid("1234"));
        assert!(!ai_address_valid("0x"));
        assert!(!ai_address_valid("other.dll+0x10"));
        assert!(!ai_address_valid("0xZZ"));
    }

    #[test]
    fn shared_keybinds_are_conflicts_and_disabled_keys_are_not() {
        let conflicts = keybind_conflicts(&[3, 0, 3, 0, 5]);
        assert_eq!(conflicts.into_iter().collect::<Vec<_>>(), vec![0, 2]);
        assert!(keybind_conflicts(&[0, 0, 1, 2]).is_empty());
    }

    #[test]
    fn compact_status_keeps_readable_path_tail_without_losing_tooltip_value() {
        let value = r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail";
        let preview = status_value_preview(value);
        assert!(preview.starts_with('…'));
        assert!(preview.ends_with("bin64_SteamRetail"));
        assert!(preview.chars().count() <= 24);
        assert_eq!(status_value_preview("Not prepared"), "Not prepared");
        assert_eq!(status_value_preview("é"), "é");
    }

    #[test]
    fn client_build_is_short_but_release_is_preserved() {
        assert_eq!(
            short_client_version("0.1.95+b45fa23dbded9764d99cd3e021ba40fb8e6d3a93"),
            "v0.1.95"
        );
        assert_eq!(short_client_version("1.2.3"), "v1.2.3");
    }

    #[test]
    fn older_long_versions_are_bounded() {
        let displayed = short_client_version("012345678901234567890123456789");
        assert_eq!(displayed, "v012345678901234567890123…");
    }
}
