//! Private-manager-inspired Settings: grouped Game Storage, Asset Library,
//! appearance and update panels. Advanced paths are hidden by default.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Background, Border, Color, Element, Fill, Length};

fn panel<'a>(child: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(child)
        .width(Fill)
        .padding(14)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 7.0.into(),
            },
            ..container::Style::default()
        })
}

fn heading<'a>(name: &'a str, description: &'a str) -> iced::widget::Column<'a, Message> {
    column![
        text(name).size(17),
        text(description).size(12).style(design::muted)
    ]
    .spacing(7)
}

fn health_entry<'a>(title: &'static str, value: &str) -> Element<'a, Message> {
    panel(
        column![
            text(title).size(13),
            text(value.to_owned()).size(12).style(design::muted),
        ]
        .spacing(6),
    )
    .into()
}

fn health_panel(state: &State) -> Element<'_, Message> {
    let heading = design::button(if state.health_expanded {
        "⌄ System Health   ·   On-demand diagnostics"
    } else {
        "› System Health   ·   On-demand diagnostics"
    })
    .on_press(Message::ToggleSystemHealth)
    .style(design::secondary)
    .width(Fill);
    if !state.health_expanded {
        return heading.into();
    }
    let check = row![
        design::button(if state.health_checking {
            "Checking..."
        } else {
            "Check System Health"
        })
        .on_press_maybe((!state.health_checking).then_some(Message::CheckSystemHealth)),
        text("Read-only until you choose an explicit operation")
            .size(12)
            .style(design::muted),
    ]
    .spacing(12);
    if let Some(result) = &state.health_report {
        column![
            heading,
            text("Checks are opt-in. Firewall, registry, dumps and cleanup settings are never changed.")
                .size(12).style(design::muted),
            check,
            row![
                health_entry("Windows Firewall", &result.firewall),
                health_entry("Game Process", &result.game_process),
            ].spacing(10),
            row![
                health_entry("Crash Dumps & Support", &result.dumps),
                health_entry("Storage & Backups", &result.storage),
            ].spacing(10),
            health_entry("Game Environment", &result.environment),
            health_entry("Recovery & Projects", &result.recovery),
        ].spacing(10).into()
    } else {
        column![
            heading,
            text("Nothing is scanned while this panel is closed. Select Check System Health to run read-only checks.")
                .size(12).style(design::muted),
            check,
        ].spacing(9).into()
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}

fn game_client_setup(state: &State) -> Element<'_, Message> {
    let paths_ok = if state.preferences.game_launch_mode == "Direct Launch" {
        crate::play_setup::game_root(&state.preferences.game_installation).is_ok()
    } else {
        crate::play_setup::validate_paths(
            &state.preferences.game_installation,
            &state.preferences.client_executable,
        )
        .is_ok()
    };
    panel(
        column![
            heading(
                "SET UP MANAGER / INSTALLED GAME",
                "Direct Launch is the default and needs only the installed EvolveGame directory; Client Launch additionally needs ModdedEvolveLauncher.exe."
            ),
            text("GAME LAUNCH METHOD").size(12),
            row![
                design::button("Client Launch")
                    .on_press(Message::PreferenceGameLaunchMode("Client Launch"))
                    .style(if state.preferences.game_launch_mode != "Direct Launch" { design::primary } else { design::secondary }),
                design::button("Direct Launch")
                    .on_press(Message::PreferenceGameLaunchMode("Direct Launch"))
                    .style(if state.preferences.game_launch_mode == "Direct Launch" { design::primary } else { design::secondary }),
            ].spacing(8),
            text("Both Play buttons use the selected method. Client Launch retains the normal launcher and its profile services. Direct Launch creates Evolve.exe itself; modded play swaps verified PAKs before startup, and restores originals after the game exits. Client service / online parity is not guaranteed.")
                .size(11).style(design::muted),
            text("Game location (EvolveGame or bin64_SteamRetail)").size(12),
            row![
                design::text_input(
                    r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail",
                    &state.preferences.game_installation,
                )
                .on_input(Message::PreferenceGameInstall),
                design::button("Browse…")
                    .on_press(Message::PickGameInstallation)
                    .style(design::secondary),
            ]
            .spacing(8),
            text("Client executable (only required for Client Launch)").size(12),
            row![
                design::text_input(
                    r"C:\Users\billn\AppData\Local\Programs\Modded Evolve\ModdedEvolveLauncher.exe",
                    &state.preferences.client_executable,
                )
                .on_input(Message::PreferenceClientExecutable),
                design::button("Browse…")
                    .on_press(Message::PickClientExecutable)
                    .style(design::secondary),
            ]
            .spacing(8),
            text(if paths_ok {
                if state.preferences.game_launch_mode == "Direct Launch" {
                    "Installed game verified. Direct modded play also needs a prepared signing stage and recovery helper."
                } else {
                    "Installed game and client found. Modded play also requires a verified signing stage."
                }
            } else if state.preferences.game_launch_mode == "Direct Launch" {
                "Select a valid EvolveGame installation, then save."
            } else {
                "Choose valid game and client paths, then save."
            })
            .size(12)
            .style(design::muted),
            row![
                design::button(if state.preferences.game_launch_mode == "Direct Launch" { "Save Game Setup" } else { "Save Game & Client" })
                    .on_press(Message::SaveGameSetup)
                    .style(design::primary),
                design::button("Play with Mods")
                    .on_press(Message::PlayWithMods)
                    .style(design::secondary),
                design::button("Play without Mods")
                    .on_press(Message::PlayWithoutMods)
                    .style(design::secondary),
            ]
            .spacing(8),
            text("The manager can generate a fresh isolated signing stage, locally created RSA keys, signed PAK copies and private-compatible recovery helpers without modifying the installed game.")
                .size(11)
                .style(design::muted),
            row![
                design::button(if state.busy {
                    "Preparing Offline Setup…"
                } else {
                    "Create Offline Setup"
                })
                .on_press_maybe((!state.busy).then_some(Message::StartAutomaticSetup))
                .style(design::primary),
                text("Requires available Game Storage space; optional original snapshots follow Settings > Keep original snapshots.")
                    .size(11)
                    .style(design::muted),
            ]
            .spacing(9),
        ]
        .spacing(9),
    )
    .into()
}

fn storage(state: &State) -> Element<'_, Message> {
    let root = state.preferences.game_storage_root.as_str();
    let buckets = state
        .storage_usage
        .as_ref()
        .map(|r| r.buckets)
        .unwrap_or([0; 5]);
    let total = buckets.iter().sum::<u64>();
    let labels = [
        "Active setup",
        "Old setups",
        "Projects",
        "Asset Library",
        "Other",
    ];
    let colors = [
        Color::from_rgb8(222, 51, 79),
        Color::from_rgb8(122, 151, 202),
        Color::from_rgb8(200, 170, 103),
        Color::from_rgb8(88, 172, 166),
        Color::from_rgb8(126, 129, 146),
    ];
    let mut strip = row![].spacing(0).height(Length::Fixed(13.0));
    for (i, value) in buckets.iter().enumerate() {
        let amount = if total == 0 {
            1
        } else {
            ((*value as f64 / total as f64) * 1000.0).round() as u16
        };
        if amount == 0 {
            continue;
        }
        let color = colors[i];
        strip = strip.push(
            container(space().height(Length::Fixed(13.0)))
                .width(Length::FillPortion(amount))
                .style(move |_| container::Style {
                    background: Some(Background::Color(color)),
                    ..container::Style::default()
                }),
        );
    }
    let mut legend = row![].spacing(15);
    for (i, label) in labels.iter().enumerate() {
        legend = legend.push(
            text(format!(
                "{}: {}",
                label,
                state
                    .storage_usage
                    .as_ref()
                    .map_or("—".into(), |_| mib(buckets[i]))
            ))
            .size(11)
            .color(colors[i]),
        );
    }
    panel(column![
        text("GAME STORAGE").size(15),
        text("One location for future setups, with storage breakdown and advanced paths.").size(12).style(design::muted),
        row![
            text("Mod Data Location").size(12),
            container(text(root).size(12)).width(Fill).padding(9).style(|theme| container::Style {
                background: Some(Background::Color(Color::from_rgb8(31, 31, 37))),
                border: Border { color: design::border(theme), width: 1.0, radius: 5.0.into() },
                ..container::Style::default()
            }),
            design::button("Change…").on_press(Message::SettingsToggleAdvancedPaths).style(design::secondary)
        ].spacing(9).align_y(iced::Center),
        text("Changing this location affects FUTURE setups only. Existing signing and recovery files are not moved.").size(11).style(design::muted),
        text(format!("Active stage: {}", if state.preferences.stage.is_empty() { "Not configured" } else { &state.preferences.stage })).size(11),
        container(column![
            row![
                text("STORAGE BREAKDOWN").size(12),
                space().width(Fill),
                text(if state.storage_analyzing { "Checking storage…" } else if state.storage_usage.is_some() { "Scan complete" } else { "Not yet analyzed" }).size(11).style(design::muted)
            ],
            strip,
            legend,
            row![
                design::button("Analyze Storage").on_press_maybe((!state.storage_analyzing && !state.busy).then_some(Message::SettingsAnalyzeStorage)).style(design::secondary),
                text(state.storage_usage.as_ref().map_or("No storage analysis yet.".to_owned(), |r| format!("{} files checked{}", r.files, if r.truncated { " • Scan limit reached; totals incomplete" } else { "" }))).size(11).style(design::muted)
            ].spacing(8)
        ].spacing(8)).padding(10).width(Fill).style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            ..container::Style::default()
        }),
        design::button(text(if state.settings_advanced_paths { "▾ Advanced folder paths" } else { "▸ Advanced folder paths" }))
            .on_press(Message::SettingsToggleAdvancedPaths).style(design::text_button),
        if state.settings_advanced_paths {
            column![
                text("Future GameSets location").size(12),
                design::text_input("GameSets folder", &state.preferences.game_storage_root).on_input(Message::PreferenceGameStorage),
                text("Leave blank to have Set Up Manager use its own GameSets folder under your Windows user profile. It creates new stage and recovery folders automatically; no private manager data is needed.").size(11).style(design::muted),
                text("Signing stage • existing location").size(12),
                design::text_input("Stage folder", &state.preferences.stage).on_input(Message::PreferenceStage),
                text("Launch restoration journal").size(12),
                design::text_input("Swap journal folder", &state.preferences.swap).on_input(Message::PreferenceSwap),
                text("Mod projects").size(12),
                design::text_input("Projects folder", &state.preferences.projects).on_input(Message::PreferenceProjects),
                text("Current editor workspace").size(12),
                design::text_input("Current workspace", &state.preferences.current_workspace).on_input(Message::PreferenceWorkspace),
                text("Paths are saved only when you select Save Settings; no data is automatically moved.").size(11).style(design::muted),
            ].spacing(6).into()
        } else {
            Element::<Message>::from(space().height(Length::Fixed(1.0)))
        },
        row![
            design::button("Set Up Manager").on_press(Message::StartAutomaticSetup).style(design::primary).width(Fill),
            design::button("Check Setup").on_press(Message::StartStageStatus).style(design::secondary).width(Fill),
            design::button("Open Projects").on_press(Message::Navigate(Page::Files)).style(design::secondary).width(Fill),
            design::button("Save Settings").on_press(Message::SavePreferences).style(design::secondary).width(Fill),
        ].spacing(8),
        design::button("Update Staged / Original PAKs after Game Update")
            .on_press_maybe((!state.busy).then_some(Message::RefreshGamePakBaseline))
            .style(design::secondary),
        text("Set Up Manager creates a fresh signing stage and recovery folder from your installed game; you do not need to provide private-manager Stage or Swap paths.")
            .size(11).style(design::muted),
        text("Creates a new verified signing baseline from the updated game. Existing stages and mod projects are preserved; edits must be reviewed against the new game files before deployment.")
            .size(11).style(design::muted),
    ].spacing(11))
    .into()
}

const REQUIRED_SHARED: [&str; 6] = [
    "characters_hunters_shared.pak",
    "characters_hunters_shared_ts.pak",
    "characters_monsters_shared.pak",
    "characters_monsters_shared_ts.pak",
    "characters_creatures_shared.pak",
    "characters_creatures_shared_ts.pak",
];
const REQUIRED_ANIMATIONS: [&str; 3] = [
    "animationscaf.pak",
    "animationsconfig.pak",
    "animationsdba.pak",
];
fn required_reference_counts(names: impl IntoIterator<Item = String>) -> (usize, usize) {
    let present = names
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    (
        REQUIRED_SHARED
            .iter()
            .filter(|name| present.contains(**name))
            .count(),
        REQUIRED_ANIMATIONS
            .iter()
            .filter(|name| present.contains(**name))
            .count(),
    )
}

fn library(state: &State) -> Element<'_, Message> {
    let archive_count = state
        .library_catalog
        .as_ref()
        .map_or(0, |c| c.archives.len());
    let total = state.library_catalog.as_ref().map_or(0, |c| c.total_bytes);
    let reference_counts = state.library_catalog.as_ref().map(|catalog| {
        required_reference_counts(catalog.archives.iter().map(|archive| archive.name.clone()))
    });
    panel(column![
        text("EXTRACTED PAK LIBRARY").size(15),
        text("Reference PAKs for characters, textures, animations and levels. The location is detected automatically from the saved manager profile.").size(12).style(design::muted),
        text(format!("{} imported archives • {} stored", archive_count, if state.library_catalog.is_some() { mib(total) } else { "Not indexed yet".into() })).size(12),
        text(reference_counts.map_or("Shared references: —  •  Animation references: —".to_string(), |(shared, animations)| format!("Shared references: {shared}/6  •  Animation references: {animations}/3"))).size(12),
        text(if state.library_root.is_empty() { "Default Asset Library location could not be detected" } else { state.library_root.as_str() }).size(11).style(design::muted),
        design::text_input("Existing extracted AssetLibrary folder...", &state.preferences.asset_library_root).on_input(Message::PreferenceAssets),
        design::button("Load Library").on_press_maybe((!state.busy).then_some(Message::LibraryRefresh)),
        design::button("Manage Asset Library / Add PAKs…").on_press(Message::Navigate(Page::AssetLibrary)).width(Fill).style(design::secondary),
    ].spacing(10)).into()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Appearance,
    Storage,
    Integration,
    Updates,
    SystemHealth,
    Advanced,
}

impl Tab {
    const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Storage,
        Self::Integration,
        Self::Updates,
        Self::SystemHealth,
        Self::Advanced,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Storage => "Storage",
            Self::Integration => "Integrations",
            Self::Updates => "Updates",
            Self::SystemHealth => "System Health",
            Self::Advanced => "Advanced",
        }
    }
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let tabs = Tab::ALL.into_iter().fold(row![].spacing(8), |tabs, tab| {
        tabs.push(
            design::button(tab.title())
                .style(if state.settings_tab == tab {
                    design::primary
                } else {
                    design::secondary
                })
                .on_press(Message::SettingsTab(tab)),
        )
    });
    let content: Element<'_, Message> = match state.settings_tab {
        Tab::Appearance => column![
        heading("GENERAL & APPEARANCE", "Compact controls with the same sidebar and theme behavior as the private manager."),
        panel(column![
            row![
                design::button(text(format!("Appearance: {}", state.preferences.appearance_mode))).on_press(Message::PreferenceAppearance).style(design::secondary),
                design::button(text(format!("Density: {}", state.preferences.ui_density))).on_press(Message::PreferenceDensity).style(design::secondary),
                design::button(text(format!("Explorer: {}", state.preferences.file_explorer_side))).on_press(Message::PreferenceExplorerSide).style(design::secondary)
            ].spacing(8),
            design::button(text(format!("Reduce menu animation: {}", if state.preferences.reduce_motion { "On" } else { "Off" }))).on_press(Message::PreferenceReduceMotion).style(design::secondary),
            text("ACCENT COLOR").size(12),
            row![
                design::button("Default")
                    .on_press(Message::PreferenceAccentPreset("Default"))
                    .style(if !state.preferences.appearance_colors.contains_key("accent") { design::primary } else { design::secondary }),
                design::button("Classic Red")
                    .on_press(Message::PreferenceAccentPreset("Classic Red"))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#c9384a") { design::primary } else { design::secondary }),
                design::button("Evolve Red")
                    .on_press(Message::PreferenceAccentPreset("Evolve Red"))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#e0354f") { design::primary } else { design::secondary }),
                design::button("Deep Red")
                    .on_press(Message::PreferenceAccentPreset("Deep Red"))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#a91f38") { design::primary } else { design::secondary }),
            ].spacing(8),
            text("Accent presets update the live theme without overwriting custom background, surface or font settings. Arbitrary color and font pickers remain planned.")
                .size(11).style(design::muted),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Updates => column![
        heading("UPDATES", "Select Stable, Beta or Nightly. Nightly builds are prerelease versions."),
        panel(column![
            text(format!("Installed version: {}", env!("CARGO_PKG_VERSION"))).size(12),
            row![
                design::button("Check for Updates").on_press(Message::CheckUpdates).style(design::primary),
                design::button(text(format!("Channel: {}", if state.preferences.nightly_updates { "Nightly" } else if state.preferences.experimental_updates { "Beta" } else { "Stable" }))).on_press(Message::PreferenceUpdateChannel).style(design::secondary),
                design::button(text(format!("Auto-check: {}", if state.preferences.automatic_update_checks { "On" } else { "Off" }))).on_press(Message::PreferenceAutoUpdates).style(design::secondary)
            ].spacing(8),
            text(state.update_status.as_str()).size(12).style(design::muted),
            if let Some(error) = &state.update_worker_failure {
                text(format!(
                    "Previous update helper failure: {error}. See LocalAppData/EvolveModManager/Updates/last-update-error.txt"
                )).size(12).style(design::muted)
            } else {
                text("").size(12)
            },
            super::download_action(state),
            text("Updates are downloaded, SHA-256 verified and installed when you select Update and restart. Portable downloads remain portable; your game files and preferences are preserved.").size(12).style(design::muted),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Integration => {
            let ffdec = crate::gfx_bundled_tools::valid_ffdec(&state.preferences.ffdec_executable);
            let ruffle = crate::gfx_bundled_tools::valid_ruffle(&state.preferences.ruffle_executable);
            let ruffle_web = crate::gfx_bundled_tools::valid_ruffle_web(&state.preferences.ruffle_web_runtime);
            column![
                heading("INTEGRATIONS", "Configure external tools for exporting models and editing game GFX."),
                panel(column![
                    text("BLENDER · FBX EXPORT").size(15),
                    if crate::blender_install::valid_blender(&state.blender_path).is_some() {
                        Element::from(text(format!("Installed: {}", state.blender_path))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Not detected. Install Blender 4.2+ or choose an existing blender.exe.")
                                .size(12).style(design::muted),
                            design::text_input("Path to Blender executable", &state.preferences.blender_executable)
                                .on_input(Message::PreferenceBlender),
                        ].spacing(6))
                    },
                ].spacing(8)),
                panel(column![
                    text("FFDEC · GFX / ACTIONSCRIPT EDITOR").size(15),
                    if let Some(path) = &ffdec {
                        Element::from(text(format!("Installed: {}", path.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Not detected. Install the verified FFDec 26.3.0 portable distribution (~19 MiB download).")
                                .size(12).style(design::muted),
                            design::button("Install FFDec")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Ffdec))),
                        ].spacing(6))
                    },
                    row![
                        design::text_input("Absolute path to ffdec-cli.exe or ffdec.jar", &state.preferences.ffdec_executable)
                            .on_input(Message::GfxConfigChanged),
                        design::button("Select existing…")
                            .on_press(Message::GfxPickBackend)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("FFDec may require a compatible Java runtime. No Java installer is bundled.")
                        .size(11).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("AUDIO TOOLS · FMOD / FSB SOUND BANKS").size(15),
                    text("vgmstream decodes original FSB5 samples to WAV for previews and exports. Native rebuilding supports replacement of existing FSB5 Vorbis samples. FMOD Designer remains optional for advanced FDP/FEV authoring.")
                        .size(12).style(design::muted),
                    if let Some(decoder) = crate::audio_tools::valid_decoder(&state.preferences.vgmstream_executable) {
                        Element::from(text(format!("vgmstream ready: {}", decoder.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("vgmstream r2117 not configured (verified ~4.3 MiB Windows x64 download).")
                                .size(12).style(design::muted),
                            design::button("Install vgmstream")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Vgmstream))),
                        ].spacing(6))
                    },
                    row![
                        design::text_input("Absolute path to vgmstream-cli.exe", &state.preferences.vgmstream_executable)
                            .on_input(Message::AudioDecoderPathChanged),
                        design::button("Select existing…").on_press(Message::AudioPickDecoder)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("NATIVE FSB5 COMPILATION · VORBIS REPLACEMENTS").size(12),
                    text("For WAV replacements, a configured FFmpeg encoder and the MIT-licensed oggvorbis2fsb5 remuxer feed the manager's native multi-sample FSB5 writer. No FMOD Designer required.")
                        .size(11).style(design::muted),
                    if let Some(remuxer) = crate::audio_tools::valid_remuxer(&state.preferences.oggvorbis_remux_executable) {
                        Element::from(text(format!("Remuxer ready: {}", remuxer.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("oggvorbis2fsb5 is not configured.")
                                .size(12).style(design::muted),
                            design::button("Install Vorbis FSB5 remuxer")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::OggRemux))),
                        ].spacing(6))
                    },
                    row![
                        design::text_input("Absolute path to oggvorbis2fsb5.exe", &state.preferences.oggvorbis_remux_executable)
                            .on_input(Message::AudioRemuxPathChanged),
                        design::button("Select existing…").on_press(Message::AudioPickRemux)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("OGG inputs can bypass FFmpeg; WAV inputs use the FFmpeg configured below. IMA ADPCM bank writing and in-game compatibility remain unverified.")
                        .size(11).style(design::muted),
                    if let Some(designer) = crate::audio_tools::valid_designer(&state.preferences.fmod_designer_executable) {
                        Element::from(text(format!("Legacy FMOD Designer detected: {}", designer.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(text("Legacy FMOD Designer not configured. Install a licensed copy separately. Modern FMOD Studio does not rebuild legacy FDP projects.")
                            .size(12).style(design::muted))
                    },
                    row![
                        design::text_input("Absolute path to fmod_designer.exe", &state.preferences.fmod_designer_executable)
                            .on_input(Message::AudioDesignerPathChanged),
                        design::button("Select existing…").on_press(Message::AudioPickDesigner)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("Inspect audio in Asset Library → File Explorer. Only editable Modding projects can stage altered FDP projects or rebuilt FSB/FEV files; playback is preview-only.")
                        .size(11).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("ORIGINAL MAIN-MENU VIDEO · OPTIONAL FFMPEG").size(15),
                    text("The Stage 2 Goliath background is a CRI USM in videos.pak. Select a trusted compatible ffmpeg.exe to convert the verified source in scratch storage without a console window. No game files are modified. If no decoder is configured, an adjacent mainmenu_goliath.preview.mp4 or .webm can be used.")
                        .size(12).style(design::muted),
                    row![
                        design::text_input("Absolute path to ffmpeg.exe", &state.preferences.gfx_ffmpeg_executable)
                            .on_input(Message::GfxFfmpegConfigChanged),
                        design::button("Select existing…")
                            .on_press(Message::GfxPickFfmpeg)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("FFmpeg must be installed separately and support your original USM codec; this does not download or launch an unselected executable.")
                        .size(11).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("RUFFLE · GFX PREVIEW INTEGRATIONS").size(15),
                    text("RUFFLE WEB · REQUIRED FOR LIVE BUTTONS").size(12),
                    if let Some(path) = &ruffle_web {
                        Element::from(text(format!("Ready: {} (JavaScript + WebAssembly detected)", path.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Missing Ruffle Web. Live GFX controls will not open with desktop ruffle.exe alone. Install the self-hosted Ruffle Web 0.7.1 runtime, or select its ruffle.js file with the adjacent .wasm runtime.")
                                .size(12).style(design::muted),
                            design::button("Install Ruffle Web")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::RuffleWeb))),
                        ].spacing(6))
                    },
                    row![
                        design::text_input("Absolute path to Ruffle Web ruffle.js", &state.preferences.ruffle_web_runtime)
                            .on_input(Message::GfxRuffleWebConfigChanged),
                        design::button("Select existing…")
                            .on_press(Message::GfxPickRuffleWeb)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("Live mode uses a private localhost browser player. Ruffle Web's ruffle.js and WebAssembly files must remain in the same folder. Blank path auto-detects the bundled third_party/RuffleWeb directory.")
                        .size(11).style(design::muted),
                    text("RUFFLE DESKTOP · STANDALONE SWF VIEWING").size(12),
                    if let Some(path) = &ruffle {
                        Element::from(text(format!("Installed: {}", path.display()))
                            .size(12).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Optional desktop Ruffle player not detected (~20.5 MiB verified download). Not required for live browser GFX controls.")
                                .size(12).style(design::muted),
                            design::button("Install Ruffle Desktop")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Ruffle))),
                        ].spacing(6))
                    },
                    row![
                        design::text_input("Absolute path to desktop ruffle.exe", &state.preferences.ruffle_executable)
                            .on_input(Message::GfxRuffleConfigChanged),
                        design::button("Select existing…")
                            .on_press(Message::GfxPickRuffle)
                            .style(design::secondary),
                    ].spacing(8).align_y(iced::Center),
                    text("An installed manager under Program Files may require administrator permission to install missing integrations. You can also rerun the installer with optional GFX Tools enabled. Native Evolve game callbacks remain unavailable in Ruffle.")
                        .size(11).style(design::muted),
                ].spacing(8)),
                if state.gfx_install_busy {
                    Element::from(text("Downloading and verifying GFX integration files…")
                        .size(12).style(design::muted))
                } else {
                    Element::from(text(state.gfx_install_status.as_str())
                        .size(12).style(design::muted))
                },
                text("Tools included with the optional Windows installer component are detected automatically. Existing custom paths take priority; the portable manager can install missing tools on demand.")
                    .size(11).style(design::muted),
            ].spacing(12).into()
        },
        Tab::SystemHealth => column![
            heading(
                "SYSTEM HEALTH",
                "Read-only, on-demand diagnostics for your local Evolve installation.",
            ),
            panel(health_panel(state)),
        ]
        .spacing(12)
        .into(),
        Tab::Advanced => column![
        heading("BACKUPS & RECOVERY", "Preserve source archives and restoration history."),
        panel(column![
            design::button(text(format!("Keep permanent original snapshots: {}", if state.preferences.keep_original_snapshots { "On" } else { "Off" }))).on_press(Message::PreferenceSnapshots).style(design::secondary),
            design::button(text(format!("Show activity during tasks: {}", if state.preferences.show_activity_on_tasks { "On" } else { "Off" }))).on_press(Message::PreferenceActivity).style(design::secondary),
            text("Crash reports").size(15),
            text("Local diagnostics for model loading, GPU uploads, and Rust crashes. Enabled by default; logs may include local asset names and file paths.")
                .size(12).style(design::muted),
            design::button(text(format!("Crash logging: {}", if state.preferences.crash_logging_enabled { "On" } else { "Off" })))
                .on_press(Message::PreferenceCrashLogging).style(design::secondary),
            row![
                design::button("Open crash log").on_press(Message::OpenCrashLogs).style(design::secondary),
                design::button("Open logs folder").on_press(Message::OpenCrashLogsFolder).style(design::secondary),
            ].spacing(8),
            text("Disabling logging stops new entries but does not delete existing logs. A native Windows fail-fast may not produce a Rust panic entry.")
                .size(11).style(design::muted),
            text("Recovery settings do not imply that every private-manager swap operation has been ported.").size(11).style(design::muted),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Storage => column![
        game_client_setup(state),
        heading("GAME STORAGE", "Review disk usage and configure future setups without changing active recovery files."),
        storage(state),
        heading("ASSET LIBRARY", "Keep extracted reference PAKs available across Modding, models and levels."),
        library(state),
        ].spacing(12).into(),
    };
    column![
        text("Settings").size(24),
        tabs,
        scrollable(content).height(Fill),
        row![
            design::button("Save Settings")
                .style(design::primary)
                .on_press(Message::SavePreferences),
            text(state.settings_status.as_str())
                .size(12)
                .style(design::muted)
        ]
        .spacing(12),
    ]
    .spacing(16)
    .height(Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_health_is_a_selectable_settings_tab() {
        assert!(Tab::ALL.contains(&Tab::SystemHealth));
        assert_eq!(Tab::SystemHealth.title(), "System Health");
    }

    #[test]
    fn shared_and_animation_archive_readiness_counts_unique_exact_names() {
        let (shared, animations) = required_reference_counts([
            "characters_monsters_shared.pak".to_owned(),
            "CHARACTERS_MONSTERS_SHARED.PAK".to_owned(),
            "animationscaf.pak".to_owned(),
            "animationsconfig.pak".to_owned(),
            "fake_characters_monsters_shared.pak".to_owned(),
        ]);
        assert_eq!((shared, animations), (1, 2));
    }
}
