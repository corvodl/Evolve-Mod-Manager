//! Private-manager-aligned landing, documentation and informational pages.
//! Home deliberately does not enumerate PAKs or scan the user's installation.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{button, column, container, image, row, scrollable, svg, text};
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

fn status_card<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    surface(
        column![
            text(label).size(13),
            text(value).size(12).style(design::muted),
        ]
        .spacing(8),
    )
    .into()
}

fn latest_announcement(state: &State) -> Element<'_, Message> {
    let news = &state.news;
    let mut content = column![
        row![
            text("Announcements").size(19),
            design::button("More in What's New")
                .on_press(Message::Navigate(Page::WhatsNew))
                .style(design::secondary),
        ]
        .spacing(12)
        .align_y(iced::Center),
    ]
    .spacing(10);
    if let Some(post) = news.cache.announcements.first() {
        if !post.title.is_empty() {
            content = content.push(text(&post.title).size(18));
        }
        content = content.push(
            text(format!("{}  ·  {}", post.author, post.timestamp))
                .size(12)
                .style(design::muted),
        );
        for body in [&post.content, &post.description] {
            if !body.is_empty() {
                let excerpt: String = body.chars().take(700).collect();
                content = content.push(text(excerpt).size(13));
            }
        }
        for url in post.image_urls.iter().take(2) {
            if !crate::whats_new::allowed_image_url(url) {
                continue;
            }
            if let Some(handle) = news.image_handles.get(url) {
                content = content.push(
                    image(handle.clone())
                        .width(Fill)
                        .height(Length::Fixed(260.0))
                        .content_fit(iced::ContentFit::Contain),
                );
            } else if news.image_failures.contains(url) {
                content = content.push(
                    text("Image preview unavailable")
                        .size(11)
                        .style(design::muted),
                );
            } else {
                content = content.push(text("Loading image…").size(11).style(design::muted));
            }
        }
    } else {
        content = content.push(
            text(if news.loading {
                "Loading the latest announcement…"
            } else {
                "No announcements available yet."
            })
            .size(12)
            .style(design::muted),
        );
    }
    surface(content).into()
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
    let base = std::env::current_exe().ok()?.parent()?.to_path_buf();
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
    None
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
                        rgba[out + channel] = (palette[index][channel] as u16 * 52 / 100) as u8;
                    }
                    rgba[out + 3] = if c0 <= c1 && index == 3 { 0 } else { 255 };
                }
            }
        }
    }
    Some((width, height, rgba))
}

pub(super) fn home(state: &State) -> Element<'_, Message> {
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
        surface(
            row![
                column![
                    legacy_section_label("YOUR WORKBENCH", state.preferences.ui_theme == "Evolve Legacy"),
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
        )
        .padding([20, 24])
    } else {
        surface(
            row![
                column![
                    legacy_section_label("GETTING STARTED", state.preferences.ui_theme == "Evolve Legacy"),
                text("Ready to make your first mod?").size(22),
                text("Start by importing a PAK into the Asset Library, or follow Docs for a guided setup. Your last selected project will appear here once you have one.")
                    .size(13)
                    .style(design::muted),
                row![
                    design::button("Getting Started")
                        .on_press(Message::Navigate(Page::Docs))
                        .style(design::primary),
                    design::button("Open Asset Library")
                        .on_press(Message::Navigate(Page::AssetLibrary)),
                ]
                .spacing(9),
                ]
                .spacing(12)
                .width(Fill),
            ]
            .spacing(12)
            .align_y(iced::Center),
        )
        .padding([20, 24])
    };
    let hero: Element<'_, Message> = if let Some(handle) = home_banner() {
        // Full-width 16:9 hero, darkened so labels remain readable.
        let picture = image(handle)
            .width(Fill)
            .height(Length::Fixed(280.0))
            .content_fit(iced::ContentFit::Cover);
        column![container(picture).width(Fill), hero,]
            .spacing(0)
            .into()
    } else {
        hero.into()
    };
    let quick = column![
        legacy_section_label("PLAY EVOLVE", state.preferences.ui_theme == "Evolve Legacy"),
        text(if state.developer_unlocked && state.developer_online_mods {
            "Developer online testing is armed. All enabled mods are eligible, including gameplay changes. Actual server acceptance is not guaranteed."
        } else {
            "Choose an offline modded session or launch with original game files."
        })
            .size(12)
            .style(design::muted),
        row![
            design::button(if state.developer_unlocked && state.developer_online_mods { "Play with Mods Online · Dev Test" } else { "Play with Mods" }).on_press(Message::PlayWithMods).style(design::primary),
            design::button("Play without Mods").on_press(Message::PlayWithoutMods).style(design::secondary),
        ]
        .spacing(8),
        text(format!("Launch method: {} (change in Settings). Both Play buttons follow this choice. Direct mode starts Evolve.exe without the client, but client profile services are not yet reproduced.", state.preferences.game_launch_mode))
            .size(12)
            .style(design::muted),
    ]
    .spacing(9);
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
            }
        ),
        status_card(
            "Modded Evolve client",
            if state.preferences.client_executable.is_empty() {
                "Select ModdedEvolveLauncher.exe in Settings".to_owned()
            } else {
                state.preferences.client_executable.clone()
            }
        ),
        status_card(
            "Client Version",
            state.client_version.as_ref().map_or_else(
                || "Not detected".to_owned(),
                |version| format!("v{version}")
            ),
        ),
        status_card(
            "Signing stage",
            if state.preferences.stage.is_empty() {
                "Not prepared".to_owned()
            } else {
                "Isolated offline stage configured".to_owned()
            }
        ),
        status_card(
            "Editing project",
            if state.preferences.current_workspace.is_empty() {
                "None selected".to_owned()
            } else {
                state.preferences.current_workspace.clone()
            }
        ),
    ]
    .spacing(9)
    .width(Length::FillPortion(2));
    scrollable(
        column![
            hero,
            quick,
            latest_announcement(state),
            row![entries, status].spacing(14),
        ]
        .spacing(20)
        .padding(12),
    )
    .height(Fill)
    .into()
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
                    text(entry.display.clone()).size(13).width(90),
                    design::button("Edit").on_press_maybe((!state.actor_busy).then_some(
                        Message::ActorEdit(actor.id, actor.address, field, entry.bits)
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
                            .label("Confirm isolated offline match (required to write)")
                            .on_toggle(Message::ArmorOfflineChanged),
                        iced::widget::checkbox(state.live_edit_enabled)
                            .label("Enable one-shot actor field writes (experimental)")
                            .on_toggle(Message::LiveEditEnabled),
                        design::button(if state.actor_busy {
                            "Reading actors..."
                        } else {
                            "Refresh all actors"
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
                        actor_rows(state),
                        text("Field editor").size(19),
                        text(
                            state
                                .actor_edit
                                .as_ref()
                                .map(|(_, _, field, _)| field.title())
                                .unwrap_or("Click Edit on an actor field above")
                        )
                        .size(12)
                        .style(design::muted),
                        design::text_input("New field value", &state.actor_edit_value)
                            .on_input(Message::ActorEditValue),
                        design::button("Write selected field once").on_press_maybe(
                            (state.actor_edit.is_some()
                                && !state.actor_busy
                                && state.live_edit_enabled
                                && state.armor_offline_confirmed
                                && !state.developer_online_mods
                                && !state.actor_edit_value.trim().is_empty())
                            .then_some(Message::ActorWrite)
                        ),
                        text("One-shot writes · Offline only · Value checked before write")
                            .size(12)
                            .style(design::muted),
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
