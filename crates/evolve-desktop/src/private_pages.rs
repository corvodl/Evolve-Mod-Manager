//! Private-manager-aligned landing, documentation and informational pages.
//! Home deliberately does not enumerate PAKs or scan the user's installation.
use super::{Message, Page, State};
use crate::{design, developer_hotkeys, runtime_teleport};
use iced::widget::{button, column, container, image, row, scrollable, stack, svg, text, tooltip};
use iced::{Background, Border, Element, Fill, Length};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum DocsSection {
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

fn surface<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(15)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: if design::is_legacy(theme) { 1.0 } else { 10.0 }.into(),
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
        backdrop.a = if wallpaper { 0.88 } else { 1.0 };
        container::Style {
            background: Some(Background::Color(backdrop)),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: if design::is_legacy(theme) { 1.0 } else { 10.0 }.into(),
            },
            ..container::Style::default()
        }
    })
}

/// The launch actions are the most important Home controls. Keep the panel
/// compact, but distinguish it from news and onboarding with a restrained
/// Evolve-red border and a very slight tint, without sacrificing contrast.
fn home_play_surface<'a>(
    content: impl Into<Element<'a, Message>>,
    wallpaper: bool,
) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding([13, 18])
        .style(move |theme| {
            let mut backdrop = design::surface(theme);
            let accent = theme.palette().primary;
            backdrop.r = backdrop.r * 0.94 + accent.r * 0.06;
            backdrop.g = backdrop.g * 0.94 + accent.g * 0.06;
            backdrop.b = backdrop.b * 0.94 + accent.b * 0.06;
            backdrop.a = if wallpaper { 0.91 } else { 1.0 };
            container::Style {
                background: Some(Background::Color(backdrop)),
                border: Border {
                    color: accent,
                    width: 1.25,
                    radius: if design::is_legacy(theme) { 1.0 } else { 10.0 }.into(),
                },
                ..container::Style::default()
            }
        })
}

/// Original Evolve red corner atlas sprite, cropped from the uploaded
/// button_I8.dds.0. Only rendered when the optional Legacy theme is selected.
fn legacy_section_label(label: &'static str, legacy: bool) -> Element<'static, Message> {
    let title = text(label).size(12).style(design::accent);
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

fn tile(name: &'static str, description: &'static str, page: Page) -> Element<'static, Message> {
    design::button(
        row![
            svg(page.icon()).style(design::icon).width(24).height(24),
            column![
                text(name).size(16),
                text(description).size(12).style(design::muted),
            ]
            .spacing(5),
        ]
        .spacing(14)
        .align_y(iced::Center),
    )
    .style(|theme, status| {
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
                radius: 12.0.into(),
            };
        }
        style
    })
    .width(Fill)
    .height(Length::Fixed(106.0))
    .padding(15)
    .on_press(Message::Navigate(page))
    .into()
}

fn status_card<'a>(label: &'static str, value: String, wallpaper: bool) -> Element<'a, Message> {
    home_surface(
        column![
            text(label).size(13),
            text(value).size(12).style(design::muted),
        ]
        .spacing(8),
        wallpaper,
    )
    .into()
}

/// The long +build identifier remains available on hover, but never
/// dominates the narrow At a Glance column.
fn short_client_version(version: &str) -> String {
    if let Some((release, build)) = version.split_once('+') {
        let prefix: String = build.chars().take(8).collect();
        if !release.is_empty() && !prefix.is_empty() {
            return format!("v{release} · {prefix}");
        }
    }
    if version.chars().count() > 28 {
        return format!("v{}…", version.chars().take(24).collect::<String>());
    }
    format!("v{version}")
}

fn client_version_card<'a>(version: Option<&str>, wallpaper: bool) -> Element<'a, Message> {
    let value: Element<'a, Message> = if let Some(version) = version {
        tooltip(
            text(short_client_version(version))
                .size(12)
                .style(design::muted),
            text(format!("Full client version: v{version}")).size(11),
            tooltip::Position::Top,
        )
        .into()
    } else {
        text("Not detected").size(12).style(design::muted).into()
    };
    home_surface(
        column![text("Client Version").size(13), value].spacing(8),
        wallpaper,
    )
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
            .style(design::secondary),
    ]
    .spacing(12)
    .align_y(iced::Center);

    let mut details = column![].spacing(6).width(Fill);
    let mut preview: Option<Element<'_, Message>> = None;
    if let Some(post) = news.cache.announcements.first() {
        if !post.title.is_empty() {
            details = details.push(text(&post.title).size(16));
        }
        details = details.push(
            text(format!("{}  ·  {}", post.author, post.timestamp))
                .size(11)
                .style(design::muted),
        );
        for body in [&post.content, &post.description] {
            if !body.is_empty() {
                let excerpt: String = body.chars().take(260).collect();
                details = details.push(text(excerpt).size(12));
            }
        }
        if let Some(url) = post
            .image_urls
            .iter()
            .find(|url| crate::whats_new::allowed_image_url(url))
        {
            if let Some(handle) = news.image_handles.get(url) {
                preview = Some(
                    container(
                        image(handle.clone())
                            .width(Length::Fixed(340.0))
                            .height(Length::Fixed(125.0))
                            .content_fit(iced::ContentFit::Contain),
                    )
                    .width(Length::Fixed(340.0))
                    .into(),
                );
            } else if news.image_failures.contains(url) {
                details = details.push(
                    text("Image preview unavailable")
                        .size(11)
                        .style(design::muted),
                );
            } else {
                details = details.push(text("Loading image…").size(11).style(design::muted));
            }
        }
    } else {
        details = details.push(
            text(if news.loading {
                "Loading the latest announcement…"
            } else {
                "No announcements available yet."
            })
            .size(12)
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

pub(super) fn home(state: &State) -> Element<'_, Message> {
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
                    text("Continue where you left off").size(22),
                    row![
                        svg(Page::Files.icon())
                            .style(design::icon)
                            .width(30)
                            .height(30),
                        column![
                            text(label).size(16),
                            text(details).size(12).style(design::muted),
                        ]
                        .spacing(5),
                    ]
                    .align_y(iced::Center)
                    .spacing(13),
                    row![
                        design::button("Continue Editing")
                            .on_press_maybe((!state.busy).then_some(Message::HomeContinueEditing))
                            .style(design::primary),
                        design::button("Open Project")
                            .on_press_maybe((!state.busy).then_some(Message::HomeOpenProject)),
                    ]
                    .spacing(9),
                ]
                .spacing(12)
                .width(Fill),
            ]
            .spacing(12)
            .align_y(iced::Center),
            wallpaper,
        )
        .padding([13, 18])
    } else {
        let paths_selected = !state.preferences.game_installation.is_empty()
            && !state.preferences.client_executable.is_empty();
        let primary_action = if paths_selected {
            design::button("Open Asset Library")
                .on_press(Message::Navigate(Page::AssetLibrary))
                .style(design::primary)
        } else {
            design::button("Getting Started")
                .on_press(Message::Navigate(Page::Docs))
                .style(design::primary)
        };
        let secondary_action = if paths_selected {
            design::button("Getting Started").on_press(Message::Navigate(Page::Docs))
        } else {
            design::button("Open Asset Library").on_press(Message::Navigate(Page::AssetLibrary))
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
                        "Ready to make your first mod?"
                    })
                    .size(18),
                    text(if paths_selected {
                        "Import a PAK into the Asset Library to begin editing."
                    } else {
                        "Configure your paths in Settings, then import a PAK to begin."
                    })
                    .size(12)
                    .style(design::muted),
                    row![primary_action, secondary_action].spacing(8),
                ]
                .spacing(7)
                .width(Fill),
            ]
            .spacing(12)
            .align_y(iced::Center),
            wallpaper,
        )
        .padding([12, 18])
    };
    let quick = column![
        row![
            legacy_section_label("PLAY EVOLVE", state.preferences.ui_theme == "Evolve Legacy"),
            text("Launch Evolve").size(20),
        ]
        .spacing(12)
        .align_y(iced::Center),
        text(if state.developer_unlocked && state.developer_online_mods {
            "Developer online testing is armed. All enabled mods are eligible, including gameplay changes. Actual server acceptance is not guaranteed."
        } else {
            "Choose an offline modded session or launch with original game files."
        })
            .size(12)
            .style(design::muted),
        row![
            design::button(if state.developer_unlocked && state.developer_online_mods { "Play with Mods Online · Dev Test" } else { "Play with Mods" })
                .on_press(Message::PlayWithMods)
                .padding([10, 17])
                .style(design::primary),
            design::button("Play without Mods")
                .on_press(Message::PlayWithoutMods)
                .padding([10, 15])
                .style(design::secondary),
        ]
        .spacing(8),
        text(format!("Launch method: {} (change in Settings). Client Launch uses the original relay, profiles and friends overlay. Direct Launch is experimental and does not yet reproduce the full client session.", state.preferences.game_launch_mode))
            .size(12)
            .style(design::muted),
    ]
    .spacing(8);
    let entries = column![
        text("Your workspace").size(19),
        text("Everything in the manager, one click away.")
            .size(12)
            .style(design::muted),
        row![
            tile(
                "Modding",
                "Open a project, edit assets and sign a PAK.",
                Page::Files
            ),
            tile(
                "Asset Library",
                "Browse textures, models, animation and GFX.",
                Page::AssetLibrary
            ),
        ]
        .spacing(10),
        row![
            tile(
                "Docs",
                "Getting started guides and step-by-step How To topics.",
                Page::Docs
            ),
            tile(
                "Settings",
                "Appearance, paths, updates and maintenance.",
                Page::Settings
            ),
        ]
        .spacing(10),
        row![
            tile(
                "What's New",
                "Changes included in the latest release.",
                Page::WhatsNew
            ),
            tile(
                "Credits",
                "Project information and acknowledgments.",
                Page::Credits
            ),
        ]
        .spacing(10),
    ]
    .spacing(9)
    .width(Length::FillPortion(5));
    let status = column![
        text("At a glance").size(19),
        text("Installed game and client status; launch actions are separate.")
            .size(12)
            .style(design::muted),
        status_card(
            "Game installation",
            if state.preferences.game_installation.is_empty() {
                "Select EvolveGame in Settings".to_owned()
            } else {
                state.preferences.game_installation.clone()
            },
            wallpaper,
        ),
        status_card(
            "Modded Evolve client",
            if state.preferences.client_executable.is_empty() {
                "Select ModdedEvolveLauncher.exe in Settings".to_owned()
            } else {
                state.preferences.client_executable.clone()
            },
            wallpaper,
        ),
        client_version_card(state.client_version.as_deref(), wallpaper),
        status_card(
            "Signing stage",
            if state.preferences.stage.is_empty() {
                "Not prepared".to_owned()
            } else {
                "Isolated offline stage configured".to_owned()
            },
            wallpaper,
        ),
        status_card(
            "Editing project",
            if state.preferences.current_workspace.is_empty() {
                "None selected".to_owned()
            } else {
                state.preferences.current_workspace.clone()
            },
            wallpaper,
        ),
    ]
    .spacing(9)
    .width(Length::FillPortion(2));
    // Scroll the Home controls above a fixed, viewport-sized background.
    // The art has no layout height of its own, so it never pushes the page
    // below the fold; the background stays in place while cards scroll.
    let content = scrollable(
        column![
            hero,
            home_play_surface(quick, wallpaper),
            latest_announcement(state, wallpaper),
            row![entries, status].spacing(14),
        ]
        .spacing(13)
        .padding(12),
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
        stack![background, shade, content]
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        content.into()
    }
}

fn docs_spec(
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

fn docs_section(section: DocsSection, expanded: bool) -> Element<'static, Message> {
    let (title, summary, steps) = docs_spec(section);
    let arrow = if expanded { "▾" } else { "▸" };
    let header = design::button(
        row![text(arrow).size(17), text(title).size(17)]
            .spacing(10)
            .align_y(iced::Center),
    )
    .on_press(Message::ToggleDocsSection(section))
    .width(Fill)
    .style(design::secondary);
    let mut content = column![header].spacing(11);
    if expanded {
        content = content.push(text(summary).size(13).style(design::muted));
        for (index, (step, details)) in steps.iter().enumerate() {
            content = content.push(
                column![
                    text(format!("{}  {}", index + 1, step)).size(14),
                    text(*details).size(12).style(design::muted),
                ]
                .spacing(4),
            );
        }
        let destination = match section {
            DocsSection::GettingStarted
            | DocsSection::Paths
            | DocsSection::Updates
            | DocsSection::Install => Some(("Open Settings", Page::Settings)),
            DocsSection::Import | DocsSection::Browse | DocsSection::Export => {
                Some(("Open Asset Library", Page::AssetLibrary))
            }
            DocsSection::Projects | DocsSection::Edit | DocsSection::Gfx => {
                Some(("Open Modding", Page::Files))
            }
            DocsSection::LaunchMods | DocsSection::LaunchClean | DocsSection::Recovery => {
                Some(("Go to Home", Page::Home))
            }
            DocsSection::Troubleshoot | DocsSection::Faq => None,
        };
        if let Some((label, page)) = destination {
            content = content.push(design::button(label).on_press(Message::Navigate(page)));
        }
    }
    surface(content).into()
}

pub(super) fn docs(state: &State) -> Element<'_, Message> {
    let mut sections = column![
        text("Docs").size(24),
        text("Easy guides for getting started and finding your way around Evolve Mod Manager.")
            .size(13).style(design::muted),
        row![
            design::button("Expand all").on_press(Message::DocsExpandAll(true)),
            design::button("Collapse all").on_press(Message::DocsExpandAll(false)),
            design::button("Go to Settings").on_press(Message::Navigate(Page::Settings)),
        ].spacing(8),
        text("Getting Started").size(19),
        docs_section(DocsSection::GettingStarted,
            state.docs_expanded.contains(&DocsSection::GettingStarted)),
        text("How To").size(19),
        text("Choose a topic to open its step-by-step guide. You can keep several topics open at once.")
            .size(12).style(design::muted),
    ].spacing(11);
    for section in DocsSection::ALL.into_iter().skip(1) {
        sections = sections.push(docs_section(
            section,
            state.docs_expanded.contains(&section),
        ));
    }
    scrollable(sections.padding(12)).height(Fill).into()
}

pub(super) fn whats_new(state: &State) -> Element<'_, Message> {
    let news = &state.news;
    let mut cards = column![
        text("What's New").size(24),
        row![
            text("Latest published version and community announcements")
                .size(12)
                .style(design::muted),
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
                .size(12)
                .style(design::muted),
                text(if release.notes.is_empty() {
                    "No release description was published.".to_owned()
                } else {
                    release.notes.clone()
                })
                .size(13),
                design::button("View full release notes on GitHub")
                    .on_press(Message::OpenNewsLink(release.url.clone())),
            ]
            .spacing(9),
        )
        .into()
    } else {
        surface(
            text(if news.loading {
                "Loading release notes from GitHub Releases..."
            } else {
                "No published release notes available yet."
            })
            .size(13),
        )
        .into()
    };
    cards = cards.push(release_card);
    if !news.release_status.is_empty() {
        cards = cards.push(text(&news.release_status).size(11).style(design::muted));
    }
    cards = cards.push(text("DISCORD ANNOUNCEMENTS").size(14).style(design::accent));

    if news.cache.announcements.is_empty() {
        cards = cards.push(surface(text(if news.loading {
            "Loading Discord channel messages..."
        } else {
            "No Discord channel messages available. Post an update in the configured channel and verify the GitHub Pages feed."
        }).size(13)));
    } else {
        let displayable_images = crate::whats_new::image_urls(&news.cache.announcements);
        for post in news.cache.announcements.iter().take(20) {
            let mut details = column![].spacing(8);
            if !post.title.is_empty() {
                details = details.push(text(&post.title).size(17));
            }
            details = details.push(
                text(format!("{}  ·  {}", post.author, post.timestamp))
                    .size(12)
                    .style(design::muted),
            );
            if !post.content.is_empty() {
                details = details.push(text(&post.content).size(13));
            }
            if !post.description.is_empty() {
                details = details.push(text(&post.description).size(13));
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
                            .size(11)
                            .style(design::muted),
                    );
                } else {
                    details = details.push(text("Loading image…").size(11).style(design::muted));
                }
            }
            cards = cards.push(surface(details));
        }
    }
    if !news.discord_status.is_empty() {
        cards = cards.push(text(&news.discord_status).size(11).style(design::muted));
    }
    scrollable(cards.padding(12)).height(Fill).into()
}

pub(super) fn credits() -> Element<'static, Message> {
    scrollable(column![
        text("Credits").size(24),
        surface(column![
            text("@CorvoDL on Discord").size(16),
            text("Project by @CorvoDL").size(13).style(design::muted),
            text("PROJECT REPOSITORY").size(12),
            text("github.com/corvodl/Evolve-Mod-Manager").size(13),
            text("OFFICIAL GAME INSTALLER").size(12),
            text("modded-evolve.com").size(13),
        ].spacing(10)),
        surface(column![
            text("Interface icons").size(16),
            text("Private manager navigation icon design. Flaticon Uicons attribution retained in the original project.").size(12).style(design::muted),
        ].spacing(8)),
        text("Install Evolve Stage 2 via modded-evolve.com before using the manager.").size(12).style(design::muted),
    ].spacing(12).padding(12)).height(Fill).into()
}

fn actor_rows(state: &State) -> Element<'_, Message> {
    let Some(listing) = &state.actor_listing else {
        return text("No actor snapshot. Detect Evolve and refresh actors.")
            .size(12)
            .style(design::muted)
            .into();
    };
    let mut list = column![
        text(format!(
            "{} actor(s), PID {}",
            listing.actors.len(),
            listing.pid
        ))
        .size(13)
    ]
    .spacing(10);
    for actor in &listing.actors {
        let mut rows = column![
            text(format!(
                "{}{}  |  ID 0x{:X}  |  Actor 0x{:X}",
                actor.name,
                if actor.local { " (Local)" } else { "" },
                actor.id,
                actor.address
            ))
            .size(14)
        ]
        .spacing(5);
        for entry in &actor.fields {
            let field = entry.field;
            let selected = state.actor_edit.is_some_and(|(id, addr, f, _)| {
                id == actor.id && addr == actor.address && f == field
            });
            let key = (actor.id, actor.address, field);
            let frozen = state.actor_frozen.get(&key);
            let editable = state.developer_unlocked && !state.actor_busy;
            let value_cell: Element<'_, Message> = if selected {
                design::text_input("Value", &state.actor_edit_value)
                    .on_input(Message::ActorEditValue)
                    .on_submit(Message::ActorWrite)
                    .width(90)
                    .into()
            } else {
                design::button(entry.display.as_str())
                    .on_press_maybe(editable.then_some(Message::ActorEdit(
                        actor.id,
                        actor.address,
                        field,
                        entry.bits,
                    )))
                    .width(90)
                    .into()
            };
            rows = rows.push(
                row![
                    text(format!(
                        "{}  +0x{:X}  ({})",
                        field.title(),
                        field.offset(),
                        field.kind()
                    ))
                    .size(12)
                    .width(Fill),
                    value_cell,
                    iced::widget::checkbox(frozen.is_some())
                        .label("Freeze")
                        .on_toggle(move |checked| Message::ActorFreeze(
                            actor.id,
                            actor.address,
                            field,
                            checked
                        )),
                ]
                .align_y(iced::Alignment::Center)
                .spacing(8),
            );
        }
        list = list.push(surface(rows.padding(10)));
    }
    list.into()
}

fn match_perk_rows(state: &State) -> Element<'_, Message> {
    let mut rows = column![
        text("Current match · character perks").size(19),
        text(if state.match_perks_status.is_empty() {
            "Refresh all actors to verify the round-start log against live perk entities."
        } else {
            state.match_perks_status.as_str()
        })
        .size(12)
        .style(design::muted),
    ]
    .spacing(7);
    if let Some(listing) = &state.match_perks {
        for player in &listing.players {
            let actor = match player.actor_id {
                Some(id) => format!(
                    " · Actor ID 0x{id:X}{}",
                    if player.local { " (Local)" } else { "" }
                ),
                None => " · Actor not identified".to_owned(),
            };
            let perks = if player.perks.is_empty() {
                "No selected perks".to_owned()
            } else {
                player.perks.join(" · ")
            };
            rows = rows.push(surface(
                column![
                    text(format!(
                        "{} · {} ({}){actor}",
                        player.class, player.display, player.character
                    ))
                    .size(13),
                    text(format!("{}: {perks}", player.slot))
                        .size(12)
                        .style(design::muted),
                ]
                .spacing(4)
                .padding(8),
            ));
        }
    }
    rows.into()
}

fn developer_keybind_controls(state: &State) -> Element<'_, Message> {
    let assigned = state
        .preferences
        .developer_hotkeys
        .iter()
        .filter(|&&k| k != 0)
        .count();
    let mut rows = column![
        text("Developer shortcuts").size(17),
        text(format!("{assigned}/9 shortcuts assigned. Shortcuts require the overlay and enabled actor writes to teleport."))
            .size(12)
            .style(design::muted),
        design::button("Load suggested shortcuts (F6–F12, Ctrl+F6/F7)")
            .on_press(Message::DeveloperKeybindDefaults),
        text(if assigned == 0 {
            "No shortcuts assigned yet. Click the button above or select keys below."
        } else {
            "Custom bindings are preserved unless you select Load suggested shortcuts."
        })
            .size(12)
            .style(design::muted),
    ]
    .spacing(7);
    for action in developer_hotkeys::Action::ALL {
        let current =
            developer_hotkeys::Keybind(state.preferences.developer_hotkeys[action.index()]);
        rows = rows.push(
            row![
                text(action.label()).width(Length::Fixed(185.0)),
                iced::widget::pick_list(
                    developer_hotkeys::Keybind::OPTIONS,
                    Some(current),
                    move |key| Message::DeveloperKeybindChanged(action, key),
                )
                .width(Length::Fixed(145.0)),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
        );
    }
    rows = rows.push(
        text(state.developer_hotkey_status.as_str())
            .size(12)
            .style(design::muted),
    );
    rows.into()
}

fn ai_pause_controls(state: &State) -> Element<'_, Message> {
    let ready = state.developer_unlocked
        && state.live_edit_enabled
        && !state.developer_online_mods
        && !state.ai_pause_busy
        && state
            .runtime_snapshot
            .as_ref()
            .and_then(|s| s.pid)
            .is_some();
    let can_pause = ready
        && !state.ai_pause_owned
        && state.ai_pause_probe.is_some_and(|probe| probe.value == 0);
    let can_resume = ready && state.ai_pause_owned;

    surface(
        column![
            text("Global AI pause").size(19),
            text("Experimental · For isolated testing · ai_NoUpdate Int32")
                .size(12).style(design::muted),
            text("Enter a separately verified CVar VALUE address. A readable 0/1 alone does not prove the address is ai_NoUpdate. No automatic detection yet.")
                .size(12).style(design::muted),
            row![
                design::text_input("0x... or Evolve.exe+0x...", &state.ai_pause_address)
                    .on_input(Message::AiAddressChanged),
                design::button("Read Int32").on_press_maybe(
                    (state.developer_unlocked && !state.developer_online_mods
                        && !state.ai_pause_busy && !state.ai_pause_owned)
                        .then_some(Message::AiInspect)
                ),
            ].spacing(8),
            row![
                design::button("Pause all AI").on_press_maybe(
                    can_pause.then_some(Message::AiChange(true))
                ),
                design::button("Resume AI").on_press_maybe(
                    can_resume.then_some(Message::AiChange(false))
                ),
                design::button("Clear ended session").on_press_maybe(
                    (!state.ai_pause_busy && state.ai_pause_probe.is_some())
                        .then_some(Message::AiForget)
                ),
            ].spacing(8),
            text(if state.ai_pause_busy {
                "Reading or writing AI update flag..."
            } else if state.ai_pause_status.is_empty() {
                "Read an externally verified address to enable pause; original 0 is restored on Resume."
            } else {
                state.ai_pause_status.as_str()
            }).size(12).style(design::muted),
        ].spacing(8).padding(10),
    ).into()
}

fn teleport_controls(state: &State) -> Element<'_, Message> {
    // Guided armor scanning is unrelated to teleporting. Process detection
    // happens automatically on the first action if no snapshot is available.
    let can_read = state.developer_unlocked
        && !state.teleport_busy
        && !state.teleport_detect_pending
        && state.teleport_hold_cancel.is_none();
    let enabled = can_read && state.live_edit_enabled;

    surface(
        column![
            text("Teleport").size(19),
            text("Teleport & freeze · read position without arming writes; enable actor memory writes to move")
                .size(12)
                .style(design::muted),
            text(if state.teleport_detect_pending {
                "Detecting Evolve.exe before the requested action..."
            } else if state.runtime_snapshot.as_ref().and_then(|s| s.pid).is_none() {
                "Running Evolve.exe will be detected automatically on the first teleport action."
            } else {
                "Evolve.exe detected · action status and errors appear below."
            })
            .size(12)
            .style(design::muted),
            row![
                design::text_input("X", &state.teleport_x)
                    .on_input(|v| Message::TeleportXYZChanged(0, v)),
                design::text_input("Y", &state.teleport_y)
                    .on_input(|v| Message::TeleportXYZChanged(1, v)),
                design::text_input("Z", &state.teleport_z)
                    .on_input(|v| Message::TeleportXYZChanged(2, v)),
            ]
            .spacing(8),
            row![
                design::button("Read position")
                    .on_press_maybe(can_read.then_some(Message::TeleportCapture(false))),
                design::button("Save position")
                    .on_press_maybe(can_read.then_some(Message::TeleportCapture(true))),
                design::button("Teleport XYZ")
                    .on_press_maybe(enabled.then_some(Message::TeleportAbsolute)),
                design::button("Return to saved").on_press_maybe(
                    (enabled && state.teleport_saved.is_some())
                        .then_some(Message::TeleportReturnSaved)
                ),
            ]
            .spacing(8),
            text(if state.teleport_busy {
                "Teleport operation running..."
            } else if state.teleport_status.is_empty() {
                "Position and teleport errors appear here."
            } else {
                state.teleport_status.as_str()
            })
            .size(12)
            .style(design::muted),
            text("Coordinate freeze · hover and move horizontally in the air").size(17),
            row![
                design::button(if state.teleport_hold_mode == Some(runtime_teleport::HoldMode::Z) {
                    "Holding Z"
                } else {
                    "Freeze Z (hover)"
                })
                .on_press_maybe(enabled.then_some(Message::TeleportHold(runtime_teleport::HoldMode::Z))),
                design::button(if state.teleport_hold_mode == Some(runtime_teleport::HoldMode::Xyz) {
                    "Holding XYZ"
                } else {
                    "Freeze XYZ"
                })
                .on_press_maybe(enabled.then_some(Message::TeleportHold(runtime_teleport::HoldMode::Xyz))),
                design::button(if state.teleport_hold_releasing {
                    "Releasing..."
                } else {
                    "Stop hold / Fly"
                })
                .on_press_maybe(
                    (state.teleport_hold_cancel.is_some() && !state.teleport_hold_releasing)
                        .then_some(Message::TeleportHoldRelease)
                ),
            ]
            .spacing(8),
            text("Fly Mode · WASD moves relative to actor facing; Space rises, Ctrl descends.")
                .size(15),
            row![
                text("Speed").width(Length::Fixed(65.0)),
                design::text_input("8", &state.teleport_fly_speed)
                    .on_input(Message::TeleportFlySpeedChanged)
                    .width(Length::Fixed(95.0)),
                text("hover / fly units per second (1–40)").size(12).style(design::muted),
                design::button(if state.teleport_hold_mode == Some(runtime_teleport::HoldMode::Fly) {
                    "Flying..."
                } else {
                    "Enable Fly Mode"
                })
                    .on_press_maybe(enabled.then_some(
                        Message::TeleportHold(runtime_teleport::HoldMode::Fly)
                    )),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
            text(if state.teleport_hold_status.is_empty() {
                "Freeze Z actively moves X/Y with WASD (no ground movement needed). Fly Mode adds Space/Ctrl for vertical movement. Freeze XYZ locks all axes."
            } else {
                state.teleport_hold_status.as_str()
            })
            .size(12)
            .style(design::muted),
            text("Position-based hover/flight writes XYZ, not gravity or velocity. If the game overrides movement, the hold stops and reports the problem.")
                .size(12)
                .style(design::muted),
            text("Relative move · 20 units per click (world axes)")
                .size(12)
                .style(design::muted),
            row![
                text("X").width(Length::Fixed(24.0)),
                design::button("−20 X")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(-20.0, 0.0, 0.0))),
                design::button("+20 X")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(20.0, 0.0, 0.0))),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
            row![
                text("Y").width(Length::Fixed(24.0)),
                design::button("−20 Y")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(0.0, -20.0, 0.0))),
                design::button("+20 Y")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(0.0, 20.0, 0.0))),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
            row![
                text("Z").width(Length::Fixed(24.0)),
                design::button("−20 Z")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(0.0, 0.0, -20.0))),
                design::button("+20 Z")
                    .on_press_maybe(enabled.then_some(Message::TeleportRelative(0.0, 0.0, 20.0))),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
            text("Approximate crosshair teleport").size(17),
            text("Uses actor facing (+Y), not the camera ray. No geometry collision test.")
                .size(12)
                .style(design::muted),
            row![
                text("Distance").width(Length::Fixed(65.0)),
                design::text_input("30", &state.teleport_aim_distance)
                    .on_input(Message::TeleportAimDistanceChanged)
                    .width(Length::Fixed(90.0)),
                design::button("Preview coordinates").on_press_maybe(
                    (can_read && !state.teleport_aim_preview_busy)
                        .then_some(Message::TeleportPreviewAim)
                ),
                design::button("Teleport to Crosshair")
                    .on_press_maybe(enabled.then_some(Message::TeleportAim)),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
            iced::widget::checkbox(
                state.teleport_overlay.is_some() || state.teleport_overlay_pending
            )
                .label("Show external overlay and enable shortcuts (windowed fullscreen)")
                .on_toggle(Message::TeleportOverlayChanged),
            iced::widget::checkbox(state.teleport_overlay_experimental)
                .label("Experimental hooked input (Windows keyboard hook)")
                .on_toggle(Message::TeleportOverlayExperimentalChanged),
            text("Experimental mode captures F6–F12 hotkeys using a Windows low-level keyboard hook while Evolve is focused. The visual overlay remains external. No game injection; exclusive fullscreen and protected input are not guaranteed.")
                .size(12)
                .style(design::muted),
            text(if state.teleport_overlay_experimental {
                if state.teleport_hook_ready {
                    "Experimental hook installed · shortcuts are captured only while Evolve has focus."
                } else if state.teleport_overlay.is_some() {
                    "Experimental hook starting · check shortcut status below."
                } else {
                    "Experimental mode selected · start the external overlay to enable the hook."
                }
            } else {
                "Standard mode · Windows RegisterHotKey shortcuts."
            }).size(12).style(design::muted),
            text(if state.teleport_overlay_pending {
                "Detecting running game..."
            } else if state.teleport_status.is_empty() {
                "Overlay preview needs only a running Evolve.exe; teleport shortcuts still require armed memory writes."
            } else {
                state.teleport_status.as_str()
            })
            .size(12)
            .style(design::muted),
            developer_keybind_controls(state),
            text("Overlay appears only over Evolve while the game is in focus. The center marker is not a raycast hit location.")
                .size(12)
                .style(design::muted),

        ]
        .spacing(9)
        .padding(10),
    )
    .into()
}

pub(super) fn developer(state: &State) -> Element<'_, Message> {
    if state.developer_unlocked {
        scrollable(
            column![
                text("Developer").size(24),
                surface(
                    column![
                        text("Network testing").size(19),
                        text("Developer-only · Firewall policy")
                            .size(12)
                            .style(design::muted),
                        iced::widget::checkbox(state.developer_online_mods)
                            .label("Allow modded online testing")
                            .on_toggle(Message::DeveloperOnlineMods),
                        text(if state.developer_network_busy {
                            "Applying network policy..."
                        } else if state.developer_online_mods {
                            "Online testing enabled · Server acceptance not guaranteed"
                        } else {
                            "Offline isolation selected"
                        })
                        .size(12)
                        .style(design::muted),
                        text(state.developer_network_status.as_str())
                            .size(12)
                            .style(design::muted),
                        text("Process privileges").size(19),
                        text("Manager: High · Client/Game: Medium")
                            .size(12)
                            .style(design::muted),
                        design::button(if state.developer_privilege_busy {
                            "Inspecting process tokens..."
                        } else {
                            "Inspect Client / Game Privileges"
                        })
                        .on_press_maybe(
                            (!state.developer_privilege_busy)
                                .then_some(Message::DeveloperInspectProcessPrivileges)
                        ),
                        text(state.developer_privilege_status.as_str())
                            .size(12)
                            .style(design::muted),
                        text("Runtime actors").size(19),
                        text("Offline memory · Validate fields before editing")
                            .size(12)
                            .style(design::muted),
                        design::button(if state.runtime_checking {
                            "Detecting..."
                        } else {
                            "Detect Evolve.exe"
                        })
                        .on_press_maybe(
                            (!state.runtime_checking).then_some(Message::RuntimeDetect)
                        ),
                        text(
                            state
                                .runtime_snapshot
                                .as_ref()
                                .map(|s| s.status.as_str())
                                .unwrap_or("Not detected")
                        )
                        .size(12)
                        .style(design::muted),
                        text(
                            state
                                .runtime_snapshot
                                .as_ref()
                                .map(|s| s.executable.as_str())
                                .unwrap_or("Executable not checked")
                        )
                        .size(12)
                        .style(design::muted),
                        iced::widget::checkbox(state.armor_offline_confirmed)
                            .label("Enable guided armor scanning (optional)")
                            .on_toggle(Message::ArmorOfflineChanged),
                        iced::widget::checkbox(state.live_edit_enabled)
                            .label("Enable actor memory writes / Freeze (experimental)")
                            .on_toggle(Message::LiveEditEnabled),
                        design::button(if state.actor_busy {
                            "Reading actors and perks..."
                        } else {
                            "Refresh actors and match perks"
                        })
                        .on_press_maybe(
                            (!state.actor_busy
                                && state
                                    .runtime_snapshot
                                    .as_ref()
                                    .and_then(|s| s.pid)
                                    .is_some())
                            .then_some(Message::ActorRefresh)
                        ),
                        text(state.actor_status.as_str())
                            .size(12)
                            .style(design::muted),
                        match_perk_rows(state),
                        actor_rows(state),
                        ai_pause_controls(state),
                        teleport_controls(state),
                        text("Click a value to edit it · Enter to write · Freeze holds the current value")
                            .size(12).style(design::muted),
                        design::button("Lock developer area").on_press(Message::LockDeveloper),
                    ]
                    .spacing(12)
                ),
            ]
            .spacing(13)
            .padding(12),
        )
        .height(Fill)
        .into()
    } else {
        column![
            text("Developer").size(24),
            surface(
                column![
                    text("Password protected").size(17),
                    text("Developer features are not available without authorized credentials.")
                        .size(13)
                        .style(design::muted),
                    design::text_input("Developer password", &state.developer_password)
                        .secure(true)
                        .on_input(Message::DeveloperPasswordChanged)
                        .on_submit(Message::UnlockDeveloper),
                    design::button("Unlock").on_press(Message::UnlockDeveloper),
                    text(state.developer_status.as_str())
                        .size(12)
                        .style(design::muted),
                ]
                .spacing(12)
            ),
        ]
        .spacing(13)
        .padding(12)
        .into()
    }
}

#[cfg(test)]
mod home_layout_tests {
    use super::short_client_version;

    #[test]
    fn client_build_is_short_but_release_is_preserved() {
        assert_eq!(
            short_client_version("0.1.95+b45fa23dbded9764d99cd3e021ba40fb8e6d3a93"),
            "v0.1.95 · b45fa23d"
        );
        assert_eq!(short_client_version("1.2.3"), "v1.2.3");
    }

    #[test]
    fn older_long_versions_are_bounded() {
        let displayed = short_client_version("012345678901234567890123456789");
        assert_eq!(displayed, "v012345678901234567890123…");
    }
}
