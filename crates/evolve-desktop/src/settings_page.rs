//! Private-manager-inspired Settings: grouped Game Storage, Asset Library,
//! appearance and update panels. Advanced paths are hidden by default.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Background, Border, Color, Element, Fill, Length};

fn panel<'a>(child: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(child)
        .width(Fill)
        .padding(16)
        .style(design::panel)
}

fn heading(name: &str) -> iced::widget::Text<'_> {
    text(name).size(20)
}

fn health_entry<'a>(title: &'static str, value: &str) -> Element<'a, Message> {
    panel(
        column![
            text(title).size(design::BODY),
            text(value.to_owned())
                .size(design::LABEL)
                .style(design::muted),
        ]
        .spacing(8),
    )
    .into()
}

fn health_panel(state: &State) -> Element<'_, Message> {
    crate::ui_kit::disclosure(
        &state.ui,
        "settings:health",
        text("System Health   ·   On-demand diagnostics"),
        state.health_expanded,
        Message::ToggleSystemHealth,
        || health_body(state),
    )
}

/// Disk and memory levels as threshold-coloured meters.
fn health_meters(meters: &[crate::system_meters::Meter]) -> Element<'_, Message> {
    if meters.is_empty() {
        return design::caption("Drive and memory levels were not available.").into();
    }
    meters
        .iter()
        .fold(column![].spacing(10), |list, meter| {
            list.push(crate::chrome::meter(
                meter.label.clone(),
                meter.level(),
                meter.detail.clone(),
            ))
        })
        .into()
}

fn health_body(state: &State) -> Element<'_, Message> {
    let check = row![
        design::button(if state.health_checking {
            "Checking..."
        } else {
            "Check System Health"
        })
        .on_press_maybe((!state.health_checking).then_some(Message::CheckSystemHealth)),
        text("Read-only until you choose an explicit operation")
            .size(design::LABEL)
            .style(design::muted),
    ]
    .spacing(12);
    if let Some(result) = &state.health_report {
        column![
            text("Checks are opt-in. Firewall, registry, dumps and cleanup settings are never changed.")
                .size(design::LABEL).style(design::muted),
            check,
            health_meters(&state.health_meters),
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
            text("Nothing is scanned while this panel is closed. Select Check System Health to run read-only checks.")
                .size(design::LABEL).style(design::muted),
            check,
        ].spacing(9).into()
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}

fn game_client_setup(state: &State) -> Element<'_, Message> {
    let paths_ok = crate::play_setup::validate_paths(
        &state.preferences.game_installation,
        &state.preferences.client_executable,
    )
    .is_ok();
    panel(
        column![
            heading("Game & Client"),
            text("Game launch method").size(design::BODY),
            container(crate::ui_kit::segmented(
                &state.ui,
                "launch-mode",
                vec![
                    (
                        "Client Launch",
                        if state.preferences.game_launch_mode == "Direct Launch" {
                            // Moving to the full client is worth a moment's
                            // explanation; switching back is not.
                            crate::chrome::confirm(
                                "Switch to Client Launch?",
                                "Play buttons will start the installed Modded Evolve client, which signs in to your profile and connects to the relay and Shift+Tab friends. Modded play still needs a verified signing stage. You can switch back to Direct Launch at any time.",
                                "Use Client Launch",
                                crate::design::Tone::Info,
                                Message::PreferenceGameLaunchMode("Client Launch"),
                            )
                        } else {
                            Message::PreferenceGameLaunchMode("Client Launch")
                        },
                    ),
                    ("Direct Launch", Message::PreferenceGameLaunchMode("Direct Launch")),
                ],
                usize::from(state.preferences.game_launch_mode == "Direct Launch"),
                true,
            ))
            .max_width(320),
            crate::ui_kit::callout(crate::design::Tone::Info, "Recommended: Client Launch for authenticated profiles, relay and Shift+Tab friends. Direct Launch currently uses a Stage 2 smoke host and does not establish the full relay/overlay session. Explicit Direct selection is preserved."),
            crate::forms::path_field(
                "Game location (EvolveGame or bin64_SteamRetail)",
                r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail",
                crate::forms::PathKind::GameInstall,
                &state.preferences,
                &state.path_checks,
                Message::PickGameInstallation,
            ),
            crate::forms::path_field(
                "Installed client executable (also supplies Direct Launch services)",
                r"C:\Users\billn\AppData\Local\Programs\Modded Evolve\ModdedEvolveLauncher.exe",
                crate::forms::PathKind::ClientExe,
                &state.preferences,
                &state.path_checks,
                Message::PickClientExecutable,
            ),
            text(if paths_ok {
                if state.preferences.game_launch_mode == "Direct Launch" {
                    "Game and compatible client selected. Direct modded play also needs verified signing and recovery files."
                } else {
                    "Full client selected for relay, authenticated profile and friends overlay. Modded play also requires a verified signing stage and Frida."
                }
            } else if state.preferences.game_launch_mode == "Direct Launch" {
                "Select valid game and client paths for Steam/profile services, then save."
            } else {
                "Choose valid game and client paths, then save."
            })
            .size(design::LABEL)
            .style(design::muted),
            row![
                design::button(if state.preferences.game_launch_mode == "Direct Launch" { "Save Game Setup" } else { "Save Game & Client" })
                    .on_press(Message::SaveGameSetup)
                    .style(design::primary),
                design::button(super::play_label(state, true, "Play with Mods"))
                    .on_press_maybe(super::play_action(state, Message::PlayWithMods))
                    .style(design::secondary),
                design::button(super::play_label(state, false, "Play without Mods"))
                    .on_press_maybe(super::play_action(state, Message::PlayWithoutMods))
                    .style(design::secondary),
            ]
            .spacing(8),
            crate::ui_kit::callout(crate::design::Tone::Info, "Creates a lightweight signing inventory. Only PAKs edited by enabled mods are copied/signed into the stage. Original game PAKs remain in place."),
            row![
                design::button(if state.busy {
                    "Preparing Offline Setup…"
                } else {
                    "Create Offline Setup"
                })
                .on_press_maybe((!state.busy).then_some(Message::StartAutomaticSetup))
                .style(design::primary),
                text("Sparse setup saves disk space. Permanent full-game snapshots are optional and disabled by default.")
                    .size(design::CAPTION)
                    .style(design::muted),
            ]
            .spacing(9),
        ]
        .spacing(9),
    )
    .into()
}

fn test_environment_setup(state: &State) -> Element<'_, Message> {
    let enabled = state.preferences.play_through_test_client;
    let valid = crate::play_setup::validate_test_paths(
        &state.preferences.test_game_installation,
        &state.preferences.test_client_executable,
        &state.preferences.game_installation,
        &state.preferences.client_executable,
    )
    .is_ok();
    panel(
        column![
            heading("Test Environment"),
            row![
                crate::ui_kit::switch(
                    &state.ui,
                    "test-client",
                    "Use Test Client",
                    enabled,
                    Message::PreferenceUseTestClient(!enabled),
                ),
                text(if enabled {
                    "Both Play buttons use the Test Client and Test Game."
                } else {
                    "Both Play buttons use the normal client and game."
                })
                .size(design::LABEL).style(design::muted),
            ].spacing(10).align_y(iced::Center),
            crate::forms::path_field(
                "Test Client executable (separate .exe build)",
                "Select Test Client .exe",
                crate::forms::PathKind::TestClient,
                &state.preferences,
                &state.path_checks,
                Message::PickTestClient,
            ),
            crate::forms::path_field(
                "Test Game directory (separate EvolveGame build)",
                "Select Test Game folder",
                crate::forms::PathKind::TestGame,
                &state.preferences,
                &state.path_checks,
                Message::PickTestGame,
            ),
            text(if valid {
                "Separate Test Client and Test Game verified. The test client must be configured to launch this Test Game."
            } else {
                "Choose both test builds. The Test Game only needs its executable here; production PAK inventory checks are not used."
            }).size(design::CAPTION).style(design::muted),
            row![
                design::button("Save Test Environment")
                    .on_press(Message::SaveTestSetup)
                    .style(design::secondary),
                design::button(if state.busy { "Creating Test Setup…" } else { "Create Test Mod Setup" })
                    .on_press_maybe((!state.busy && valid).then_some(Message::StartTestAutomaticSetup))
                    .style(design::secondary),
            ].spacing(8),
            crate::ui_kit::callout(crate::design::Tone::Info, "Modded test launches use an independent Test signing stage and swap journal. Creating a Test Mod Setup uses the existing signed-PAK setup workflow; unsupported test archive formats will be reported, not silently replaced."),
            text("Test signing stage (separate from production)").size(design::LABEL),
            design::text_input("Test stage folder", &state.preferences.test_stage)
                .on_input(Message::PreferenceTestStage),
            text("Test recovery / swap folder").size(design::LABEL),
            design::text_input("Test swap folder", &state.preferences.test_swap)
                .on_input(Message::PreferenceTestSwap),
        ].spacing(9),
    ).into()
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
    // Bucket sizes count up to their new values when an analysis finishes.
    const TICKERS: [&str; 5] = [
        "storage_0",
        "storage_1",
        "storage_2",
        "storage_3",
        "storage_4",
    ];
    // Legend order follows the sort header (size or name).
    let mut order: Vec<usize> = (0..labels.len()).collect();
    if let Some((column, ascending)) = state.storage_sort {
        order.sort_by(|&a, &b| {
            let order = if column == "size" {
                buckets[a].cmp(&buckets[b])
            } else {
                labels[a].cmp(labels[b])
            };
            if ascending { order } else { order.reverse() }
        });
    }
    let mut legend = row![
        crate::ui_kit::sort_header(
            "NAME",
            "name",
            state.storage_sort,
            Message::SortBy("storage", "name"),
            Length::Shrink
        ),
        crate::ui_kit::sort_header(
            "SIZE",
            "size",
            state.storage_sort,
            Message::SortBy("storage", "size"),
            Length::Shrink
        ),
    ]
    .spacing(15);
    for i in order {
        let label = &labels[i];
        legend =
            legend.push(
                text(format!(
                    "{}: {}",
                    label,
                    state.storage_usage.as_ref().map_or("—".into(), |_| mib(
                        crate::ui_kit::ticked(&state.ui, TICKERS[i], buckets[i] as f64) as u64
                    ))
                ))
                .size(design::CAPTION)
                .color(colors[i]),
            );
    }
    let total_label = state.storage_usage.as_ref().map(|_| {
        crate::ui_kit::format_bytes(crate::ui_kit::ticked(
            &state.ui,
            "storage_total",
            total as f64,
        ))
    });
    panel(column![
        text("GAME STORAGE").size(design::HEADING),
        row![
            text("Mod Data Location").size(design::LABEL),
            crate::ui_kit::copy_button(&state.ui, "storage-root", root.to_owned()),
            container(text(root).size(design::LABEL)).width(Fill).padding(9).style(|theme| container::Style {
                background: Some(Background::Color(Color::from_rgb8(31, 31, 37))),
                border: Border { color: design::border(theme), width: 1.0, radius: design::radius(theme, design::Radius::Small).into() },
                ..container::Style::default()
            }),
            design::button("Change…").on_press(Message::SettingsToggleAdvancedPaths).style(design::secondary)
        ].spacing(9).align_y(iced::Center),
        crate::ui_kit::callout(crate::design::Tone::Warning, "Changing this location affects FUTURE setups only. Existing signing and recovery files are not moved."),
        text(format!("Active stage: {}", if state.preferences.stage.is_empty() { "Not configured" } else { &state.preferences.stage })).size(design::CAPTION),
        container(column![
            row![
                text("STORAGE BREAKDOWN").size(design::LABEL),
                text(total_label.clone().map(|total| format!("{total} total")).unwrap_or_default()).size(design::LABEL),
                space().width(Fill),
                text(if state.storage_analyzing { "Checking storage…" } else if state.storage_usage.is_some() { "Scan complete" } else { "Not yet analyzed" }).size(design::CAPTION).style(design::muted)
            ],
            row![
                iced::widget::canvas(crate::fx::RingChart {
                    values: buckets.to_vec(),
                    colors: colors.to_vec(),
                    sweep: state.ui.ring_sweep(std::time::Instant::now()),
                    center: total_label.clone().unwrap_or_else(|| "—".into()),
                })
                .width(Length::Fixed(150.0))
                .height(Length::Fixed(150.0)),
                column![strip, legend.wrap()].spacing(10).width(Fill),
            ]
            .spacing(16)
            .align_y(iced::Center),
            row![
                design::button("Analyze Storage").on_press_maybe((!state.storage_analyzing && !state.busy).then_some(Message::SettingsAnalyzeStorage)).style(design::secondary),
                text(state.storage_usage.as_ref().map_or("No storage analysis yet.".to_owned(), |r| format!("{} files checked{}", r.files, if r.truncated { " • Scan limit reached; totals incomplete" } else { "" }))).size(design::CAPTION).style(design::muted)
            ].spacing(8)
        ].spacing(8)).padding(10).width(Fill).style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            ..container::Style::default()
        }),
        crate::ui_kit::disclosure(
            &state.ui,
            "settings:advanced-paths",
            text("Advanced folder paths").size(design::BODY),
            state.settings_advanced_paths,
            Message::SettingsToggleAdvancedPaths,
            || -> Element<'_, Message> {
            column![
                text("Future GameSets location").size(design::LABEL),
                design::text_input("GameSets folder", &state.preferences.game_storage_root).on_input(Message::PreferenceGameStorage),
                text("Leave blank to have Set Up Manager use its own GameSets folder under your Windows user profile. It creates new stage and recovery folders automatically; no private manager data is needed.").size(design::CAPTION).style(design::muted),
                text("Signing stage • existing location").size(design::LABEL),
                design::text_input("Stage folder", &state.preferences.stage).on_input(Message::PreferenceStage),
                text("Launch restoration journal").size(design::LABEL),
                design::text_input("Swap journal folder", &state.preferences.swap).on_input(Message::PreferenceSwap),
                crate::forms::path_field(
                    "Mod projects",
                    "Projects folder",
                    crate::forms::PathKind::Projects,
                    &state.preferences,
                    &state.path_checks,
                    Message::BrowsePath(crate::forms::PathKind::Projects),
                ),
                text("Current editor workspace").size(design::LABEL),
                design::text_input("Current workspace", &state.preferences.current_workspace).on_input(Message::PreferenceWorkspace),
                text("Paths are saved only when you select Save Settings; no data is automatically moved.").size(design::CAPTION).style(design::muted),
            ].spacing(6).into()
            },
        ),
        row![
            design::button("Set Up Manager").on_press(Message::StartAutomaticSetup).style(design::primary).width(Fill),
            design::button("Check Setup").on_press(Message::StartStageStatus).style(design::secondary).width(Fill),
            design::button("Open Projects").on_press(Message::Navigate(Page::Files)).style(design::secondary).width(Fill),
            design::button("Save Settings").on_press(Message::SavePreferences).style(design::secondary).width(Fill),
        ].spacing(8),
        design::button("Update Staged / Original PAKs after Game Update")
            .on_press_maybe((!state.busy).then_some(Message::RefreshGamePakBaseline))
            .style(design::secondary),
        crate::ui_kit::callout(crate::design::Tone::Info, "Set Up Manager creates a fresh signing stage and recovery folder from your installed game; you do not need to provide private-manager Stage or Swap paths."),
        crate::ui_kit::callout(crate::design::Tone::Warning, "Creates a new verified signing baseline from the updated game. Existing stages and mod projects are preserved; edits must be reviewed against the new game files before deployment."),
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
    panel(
        column![
            text("EXTRACTED PAK LIBRARY").size(design::HEADING),
            text(format!(
                "{} imported archives • {} stored",
                archive_count,
                if state.library_catalog.is_some() {
                    mib(total)
                } else {
                    "Not indexed yet".into()
                }
            ))
            .size(design::LABEL),
            text(reference_counts.map_or(
                "Shared references: —  •  Animation references: —".to_string(),
                |(shared, animations)| format!(
                    "Shared references: {shared}/6  •  Animation references: {animations}/3"
                )
            ))
            .size(design::LABEL),
            text(if state.library_root.is_empty() {
                "Default Asset Library location could not be detected"
            } else {
                state.library_root.as_str()
            })
            .size(design::CAPTION)
            .style(design::muted),
            crate::forms::path_field(
                "Asset Library folder",
                "Existing extracted AssetLibrary folder...",
                crate::forms::PathKind::AssetLibrary,
                &state.preferences,
                &state.path_checks,
                Message::BrowsePath(crate::forms::PathKind::AssetLibrary),
            ),
            design::button("Load Library")
                .on_press_maybe((!state.busy).then_some(Message::LibraryRefresh)),
            design::button("Manage Asset Library / Add PAKs…")
                .on_press(Message::Navigate(Page::AssetLibrary))
                .width(Fill)
                .style(design::secondary),
        ]
        .spacing(10),
    )
    .into()
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
    pub(crate) const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Storage,
        Self::Integration,
        Self::Updates,
        Self::SystemHealth,
        Self::Advanced,
    ];
    fn icon(self) -> iced::widget::svg::Handle {
        match self {
            Self::Appearance => crate::icons::palette(),
            Self::Storage => crate::icons::drive(),
            Self::Integration => crate::icons::plug(),
            Self::Updates => crate::icons::download(),
            Self::SystemHealth => crate::icons::pulse(),
            Self::Advanced => crate::icons::sliders(),
        }
    }

    /// A dot on the tab: an update waiting, or a path that fails its check.
    fn status(self, state: &State) -> Option<crate::design::Tone> {
        use crate::forms::PathKind;
        let failing = |kinds: &[PathKind]| {
            kinds
                .iter()
                .any(|kind| state.path_checks.get(*kind).is_some_and(|check| !check.ok))
        };
        match self {
            Self::Updates => (!state.launch_updates.is_empty() || state.available_update.is_some())
                .then_some(crate::design::Tone::Info),
            Self::Storage => failing(&[
                PathKind::GameInstall,
                PathKind::ClientExe,
                PathKind::TestGame,
                PathKind::TestClient,
                PathKind::Projects,
                PathKind::AssetLibrary,
            ])
            .then_some(crate::design::Tone::Danger),
            Self::Integration => failing(&[
                PathKind::Blender,
                PathKind::Ffdec,
                PathKind::Ruffle,
                PathKind::RuffleWeb,
                PathKind::Ffmpeg,
                PathKind::Vgmstream,
                PathKind::OggRemux,
                PathKind::FmodDesigner,
            ])
            .then_some(crate::design::Tone::Danger),
            Self::Appearance | Self::SystemHealth | Self::Advanced => None,
        }
    }

    pub(crate) fn title(self) -> &'static str {
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
    let tabs = crate::ui_kit::segmented_tabs(
        &state.ui,
        "settings-tabs",
        Tab::ALL
            .into_iter()
            .map(|tab| crate::ui_kit::TabItem {
                icon: tab.icon(),
                label: tab.title(),
                dot: tab.status(state),
                message: Message::SettingsTab(tab),
            })
            .collect(),
        Tab::ALL
            .iter()
            .position(|tab| *tab == state.settings_tab)
            .unwrap_or(0),
        138.0,
    );
    let content: Element<'_, Message> = match state.settings_tab {
        Tab::Appearance => column![
        heading("Appearance"),
        panel(column![
            text("INTERFACE THEME").size(design::LABEL),
            container(crate::ui_kit::segmented(
                &state.ui,
                "ui-theme",
                vec![
                    ("Modern", Message::PreferenceUiTheme("Modern")),
                    ("Evolve Legacy", Message::PreferenceUiTheme("Evolve Legacy")),
                ],
                usize::from(state.preferences.ui_theme == "Evolve Legacy"),
                true,
            ))
            .max_width(320),
            text(if state.preferences.ui_theme == "Evolve Legacy" {
                "Legacy theme: charcoal panels, red selection, white hover and sharper controls. The original supplied UI_Assets.pak is not bundled or modified; its textures require separate verified extraction."
            } else {
                "Modern theme: original color choices and rounder panels. Choose Evolve Legacy to preview the alternative instantly; switching back preserves your Modern colors."
            }).size(design::CAPTION).style(design::muted),
            row![
                crate::ui_kit::segmented(
                    &state.ui,
                    "appearance-mode",
                    vec![
                        ("Dark", Message::PreferenceAppearanceSet("Dark")),
                        ("Light", Message::PreferenceAppearanceSet("Light")),
                    ],
                    usize::from(state.preferences.appearance_mode == "Light"),
                    state.preferences.ui_theme == "Modern",
                ),
                design::button(text(format!("Density: {}", state.preferences.ui_density))).on_press(Message::PreferenceDensity).style(design::secondary),
                design::button(text(format!("Explorer: {}", state.preferences.file_explorer_side))).on_press(Message::PreferenceExplorerSide).style(design::secondary)
            ].spacing(8),
            crate::ui_kit::switch(&state.ui, "reduce-motion", "Reduce motion", state.preferences.reduce_motion, Message::PreferenceReduceMotion),
            crate::ui_kit::switch(&state.ui, "sidebar-peek", "Open the sidebar while hovering its icons", state.preferences.sidebar_hover_peek, Message::PreferenceSidebarPeek),
            text("Off: only the button at the top of the sidebar (or Ctrl+B) opens it. It always opens over the page.").size(design::CAPTION).style(design::muted),
            text("HOME SCREEN ARTWORK").size(design::LABEL),
            crate::ui_kit::switch(&state.ui, "home-background", "Show Evolve artwork on Home", state.preferences.show_home_background, Message::PreferenceHomeBackground),
            text("Off hides the Evolve artwork and restores opaque Home panels. This only affects Home, keeps other themes and pages unchanged, and is saved automatically.")
                .size(design::CAPTION).style(design::muted),
            text("THEME PRESETS").size(design::LABEL),
            theme_presets(state),
            row![
                text("Corners").size(design::LABEL),
                crate::ui_kit::segmented(
                    &state.ui,
                    "corner-style",
                    vec![
                        ("Sharp", Message::PreferenceCornerStyle("Sharp")),
                        ("Rounded", Message::PreferenceCornerStyle("Rounded")),
                        ("Soft", Message::PreferenceCornerStyle("Soft")),
                    ],
                    match state.preferences.corner_style.as_str() {
                        "Sharp" => 0,
                        "Soft" => 2,
                        _ => 1,
                    },
                    state.preferences.ui_theme == "Modern",
                ),
            ]
            .spacing(12)
            .align_y(iced::Center),
            text("CUSTOM COLOURS").size(design::LABEL),
            crate::color_picker::view(state),
            text("MODERN THEME ACCENT COLOR").size(design::LABEL),
            row![
                design::button("Default")
                    .on_press_maybe((state.preferences.ui_theme == "Modern").then_some(Message::PreferenceAccentPreset("Default")))
                    .style(if !state.preferences.appearance_colors.contains_key("accent") { design::primary } else { design::secondary }),
                design::button("Classic Red")
                    .on_press_maybe((state.preferences.ui_theme == "Modern").then_some(Message::PreferenceAccentPreset("Classic Red")))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#c9384a") { design::primary } else { design::secondary }),
                design::button("Evolve Red")
                    .on_press_maybe((state.preferences.ui_theme == "Modern").then_some(Message::PreferenceAccentPreset("Evolve Red")))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#e0354f") { design::primary } else { design::secondary }),
                design::button("Deep Red")
                    .on_press_maybe((state.preferences.ui_theme == "Modern").then_some(Message::PreferenceAccentPreset("Deep Red")))
                    .style(if state.preferences.appearance_colors.get("accent").is_some_and(|c| c == "#a91f38") { design::primary } else { design::secondary }),
            ].spacing(8),
            text("Modern accent presets and Light/Dark modes are retained separately; selecting Legacy never overwrites those preferences. Original Scaleform assets are not executed by the native theme.")
                .size(design::CAPTION).style(design::muted),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Updates => column![
        heading("Updates"),
        panel(column![
            row![
                text(format!("Installed version: {}", env!("CARGO_PKG_VERSION"))).size(design::LABEL),
                crate::ui_kit::copy_button(&state.ui, "version", env!("CARGO_PKG_VERSION").to_owned()),
            ]
            .push(env!("CARGO_PKG_VERSION").contains("-nightly.").then(|| {
                crate::ui_kit::badge(crate::ui_kit::BadgeKind::Nightly, "Nightly")
            }))
            .push(env!("CARGO_PKG_VERSION").contains("-beta.").then(|| {
                crate::ui_kit::badge(crate::ui_kit::BadgeKind::Modified, "Beta")
            }))
            .spacing(8)
            .align_y(iced::Center),
            row![
                design::button("Check for Updates").on_press(Message::CheckUpdates).style(design::primary),
                crate::ui_kit::segmented(
                    &state.ui,
                    "update-channel",
                    vec![
                        ("Stable", Message::PreferenceUpdateChannelSet("Stable")),
                        ("Beta", Message::PreferenceUpdateChannelSet("Beta")),
                        ("Nightly", Message::PreferenceUpdateChannelSet("Nightly")),
                    ],
                    if state.preferences.nightly_updates { 2 } else if state.preferences.experimental_updates { 1 } else { 0 },
                    true,
                ),
            ].spacing(8),
            text("Beta and Nightly builds are prereleases. Every channel is checked automatically when the manager starts.").size(design::CAPTION).style(design::muted),
            text(state.update_status.as_str()).size(design::LABEL).style(design::muted),
            if let Some(error) = &state.update_worker_failure {
                text(format!(
                    "Previous update helper failure: {error}. See LocalAppData/EvolveModManager/Updates/last-update-error.txt"
                )).size(design::LABEL).style(design::muted)
            } else {
                text("").size(design::LABEL)
            },
            if state.update_worker_failure.is_some() {
                design::button("Retry update")
                    .on_press_maybe((!state.busy).then_some(Message::RetryUpdate))
                    .style(design::primary)
            } else {
                design::button("Retry update")
                    .style(design::secondary)
            },
            super::download_action(state),
            super::launch_update_actions(state),
            crate::ui_kit::callout(crate::design::Tone::Success, "Updates are downloaded, SHA-256 verified and installed when you select Update and restart. Portable downloads remain portable; your game files and preferences are preserved."),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Integration => {
            let ffdec = crate::gfx_bundled_tools::valid_ffdec(&state.preferences.ffdec_executable);
            let ruffle = crate::gfx_bundled_tools::valid_ruffle(&state.preferences.ruffle_executable);
            let ruffle_web = crate::gfx_bundled_tools::valid_ruffle_web(&state.preferences.ruffle_web_runtime);
            column![
                heading("Integrations"),
                panel(column![
                    text("DISCORD RICH PRESENCE · LOCAL ONLY").size(design::HEADING),
                    text("No account linking, OAuth, bot, or token. Uses the desktop Discord app through local IPC. Discord's own Activity Privacy settings also control visibility.")
                        .size(design::LABEL).style(design::muted),
                    crate::ui_kit::switch(&state.ui, "discord-enabled", "Rich Presence", state.preferences.discord_presence_enabled, Message::PreferenceDiscordToggle("enabled")),
                    text("Manager: 1563784115785048114 (Evolve Mod Manager). Game: 1565093139616763987 (Evolve). Automatically switches when Evolve starts/closes. Override below applies only to the manager.")
                        .size(design::CAPTION).style(design::muted),
                    design::text_input("Manager application ID (optional developer override)", &state.preferences.discord_application_id)
                        .on_input(Message::PreferenceDiscordAppId),
                    row![
                        crate::ui_kit::switch(&state.ui, "discord-game", "Gameplay", state.preferences.discord_show_game, Message::PreferenceDiscordToggle("game")),
                        crate::ui_kit::switch(&state.ui, "discord-manager", "Manager", state.preferences.discord_show_manager, Message::PreferenceDiscordToggle("manager")),
                    ].spacing(8),
                    row![
                        crate::ui_kit::switch(&state.ui, "discord-character", "Character / Stage", state.preferences.discord_show_character, Message::PreferenceDiscordToggle("character")),
                        crate::ui_kit::switch(&state.ui, "discord-stats", "Health / Armor", state.preferences.discord_show_stats, Message::PreferenceDiscordToggle("stats")),
                    ].spacing(8),
                    row![
                        crate::ui_kit::switch(&state.ui, "discord-profile", "Mod Profile", state.preferences.discord_show_profile, Message::PreferenceDiscordToggle("profile")),
                        crate::ui_kit::switch(&state.ui, "discord-minimal", "Minimal privacy", state.preferences.discord_minimal, Message::PreferenceDiscordToggle("minimal")),
                    ].spacing(8),
                    text("Character names use the read-only local IEntity resolver and known aliases; unknown names are hidden. Health is available when validated. Armor and evolution are monster-only. Maps, modes and game art still need verified sources. Discord may prioritize the game's own presence over the manager's activity.")
                        .size(design::CAPTION).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("BLENDER · FBX EXPORT").size(design::HEADING),
                    if crate::blender_install::valid_blender(&state.blender_path).is_some() {
                        Element::from(text(format!("Installed: {}", state.blender_path))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Not detected. Install Blender 4.2+ or choose an existing blender.exe.")
                                .size(design::LABEL).style(design::muted),
                            crate::forms::path_field(
                                "Blender executable",
                                "Path to blender.exe",
                                crate::forms::PathKind::Blender,
                                &state.preferences,
                                &state.path_checks,
                                Message::BrowsePath(crate::forms::PathKind::Blender),
                            ),
                        ].spacing(6))
                    },
                ].spacing(8)),
                panel(column![
                    text("FFDEC · GFX / ACTIONSCRIPT EDITOR").size(design::HEADING),
                    if let Some(path) = &ffdec {
                        Element::from(text(format!("Installed: {}", path.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Not detected. Install the verified FFDec 26.3.0 portable distribution (~19 MiB download).")
                                .size(design::LABEL).style(design::muted),
                            design::button("Install FFDec")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Ffdec))),
                        ].spacing(6))
                    },
                    crate::forms::path_field(
                        "FFDec executable",
                        "Absolute path to ffdec-cli.exe or ffdec.jar",
                        crate::forms::PathKind::Ffdec,
                        &state.preferences,
                        &state.path_checks,
                        Message::GfxPickBackend,
                    ),
                    text("FFDec may require a compatible Java runtime. No Java installer is bundled.")
                        .size(design::CAPTION).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("AUDIO TOOLS · FMOD / FSB SOUND BANKS").size(design::HEADING),
                    text("vgmstream decodes original FSB5 samples to WAV for previews and exports. Native rebuilding supports replacement of existing FSB5 Vorbis samples. FMOD Designer remains optional for advanced FDP/FEV authoring.")
                        .size(design::LABEL).style(design::muted),
                    if let Some(decoder) = crate::audio_tools::valid_decoder(&state.preferences.vgmstream_executable) {
                        Element::from(text(format!("vgmstream ready: {}", decoder.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("vgmstream r2117 not configured (verified ~4.3 MiB Windows x64 download).")
                                .size(design::LABEL).style(design::muted),
                            design::button("Install vgmstream")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Vgmstream))),
                        ].spacing(6))
                    },
                    crate::forms::path_field(
                        "vgmstream decoder",
                        "Absolute path to vgmstream-cli.exe",
                        crate::forms::PathKind::Vgmstream,
                        &state.preferences,
                        &state.path_checks,
                        Message::AudioPickDecoder,
                    ),
                    text("NATIVE FSB5 COMPILATION · VORBIS REPLACEMENTS").size(design::LABEL),
                    text("For WAV replacements, a configured FFmpeg encoder and the MIT-licensed oggvorbis2fsb5 remuxer feed the manager's native multi-sample FSB5 writer. No FMOD Designer required.")
                        .size(design::CAPTION).style(design::muted),
                    if let Some(remuxer) = crate::audio_tools::valid_remuxer(&state.preferences.oggvorbis_remux_executable) {
                        Element::from(text(format!("Remuxer ready: {}", remuxer.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("oggvorbis2fsb5 is not configured.")
                                .size(design::LABEL).style(design::muted),
                            design::button("Install Vorbis FSB5 remuxer")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::OggRemux))),
                        ].spacing(6))
                    },
                    crate::forms::path_field(
                        "Vorbis FSB5 remuxer",
                        "Absolute path to oggvorbis2fsb5.exe",
                        crate::forms::PathKind::OggRemux,
                        &state.preferences,
                        &state.path_checks,
                        Message::AudioPickRemux,
                    ),
                    text("OGG inputs can bypass FFmpeg; WAV inputs use the FFmpeg configured below. IMA ADPCM bank writing and in-game compatibility remain unverified.")
                        .size(design::CAPTION).style(design::muted),
                    if let Some(designer) = crate::audio_tools::valid_designer(&state.preferences.fmod_designer_executable) {
                        Element::from(text(format!("Legacy FMOD Designer detected: {}", designer.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(text("Legacy FMOD Designer not configured. Install a licensed copy separately. Modern FMOD Studio does not rebuild legacy FDP projects.")
                            .size(design::LABEL).style(design::muted))
                    },
                    crate::forms::path_field(
                        "FMOD Designer",
                        "Absolute path to fmod_designer.exe",
                        crate::forms::PathKind::FmodDesigner,
                        &state.preferences,
                        &state.path_checks,
                        Message::AudioPickDesigner,
                    ),
                    text("Inspect audio in Asset Library → File Explorer. Only editable Modding projects can stage altered FDP projects or rebuilt FSB/FEV files; playback is preview-only.")
                        .size(design::CAPTION).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("ORIGINAL MAIN-MENU VIDEO · OPTIONAL FFMPEG").size(design::HEADING),
                    text("The Stage 2 Goliath background is a CRI USM in videos.pak. Select a trusted compatible ffmpeg.exe to convert the verified source in scratch storage without a console window. No game files are modified. If no decoder is configured, an adjacent mainmenu_goliath.preview.mp4 or .webm can be used.")
                        .size(design::LABEL).style(design::muted),
                    crate::forms::path_field(
                        "FFmpeg executable",
                        "Absolute path to ffmpeg.exe",
                        crate::forms::PathKind::Ffmpeg,
                        &state.preferences,
                        &state.path_checks,
                        Message::GfxPickFfmpeg,
                    ),
                    text("FFmpeg must be installed separately and support your original USM codec; this does not download or launch an unselected executable.")
                        .size(design::CAPTION).style(design::muted),
                ].spacing(8)),
                panel(column![
                    text("RUFFLE · GFX PREVIEW INTEGRATIONS").size(design::HEADING),
                    text("RUFFLE WEB · REQUIRED FOR LIVE BUTTONS").size(design::LABEL),
                    if let Some(path) = &ruffle_web {
                        Element::from(text(format!("Ready: {} (JavaScript + WebAssembly detected)", path.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Missing Ruffle Web. Live GFX controls will not open with desktop ruffle.exe alone. Install the self-hosted Ruffle Web 0.7.1 runtime, or select its ruffle.js file with the adjacent .wasm runtime.")
                                .size(design::LABEL).style(design::muted),
                            design::button("Install Ruffle Web")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::RuffleWeb))),
                        ].spacing(6))
                    },
                    crate::forms::path_field(
                        "Ruffle Web runtime",
                        "Absolute path to Ruffle Web ruffle.js",
                        crate::forms::PathKind::RuffleWeb,
                        &state.preferences,
                        &state.path_checks,
                        Message::GfxPickRuffleWeb,
                    ),
                    text("Live mode uses a private localhost browser player. Ruffle Web's ruffle.js and WebAssembly files must remain in the same folder. Blank path auto-detects the bundled third_party/RuffleWeb directory.")
                        .size(design::CAPTION).style(design::muted),
                    text("RUFFLE DESKTOP · STANDALONE SWF VIEWING").size(design::LABEL),
                    if let Some(path) = &ruffle {
                        Element::from(text(format!("Installed: {}", path.display()))
                            .size(design::LABEL).style(design::muted))
                    } else {
                        Element::from(column![
                            text("Optional desktop Ruffle player not detected (~20.5 MiB verified download). Not required for live browser GFX controls.")
                                .size(design::LABEL).style(design::muted),
                            design::button("Install Ruffle Desktop")
                                .on_press_maybe((!state.gfx_install_busy)
                                    .then_some(Message::GfxInstallTool(crate::gfx_tool_install::Tool::Ruffle))),
                        ].spacing(6))
                    },
                    crate::forms::path_field(
                        "Ruffle Desktop",
                        "Absolute path to desktop ruffle.exe",
                        crate::forms::PathKind::Ruffle,
                        &state.preferences,
                        &state.path_checks,
                        Message::GfxPickRuffle,
                    ),
                    crate::ui_kit::callout(crate::design::Tone::Warning, "An installed manager under Program Files may require administrator permission to install missing integrations. You can also rerun the installer with optional GFX Tools enabled. Native Evolve game callbacks remain unavailable in Ruffle."),
                ].spacing(8)),
                if state.gfx_install_busy {
                    Element::from(text("Downloading and verifying GFX integration files…")
                        .size(design::LABEL).style(design::muted))
                } else {
                    Element::from(text(state.gfx_install_status.as_str())
                        .size(design::LABEL).style(design::muted))
                },
                text("Tools included with the optional Windows installer component are detected automatically. Existing custom paths take priority; the portable manager can install missing tools on demand.")
                    .size(design::CAPTION).style(design::muted),
            ].spacing(12).into()
        },
        Tab::SystemHealth => column![
            heading("System Health"),
            panel(health_panel(state)),
        ]
        .spacing(12)
        .into(),
        Tab::Advanced => column![
        heading("Backups & Recovery"),
        text("Recovery copies are needed only for modified PAKs being swapped.").size(design::LABEL).style(design::muted),
        panel(column![
            crate::ui_kit::switch(&state.ui, "snapshots", "Keep full-game snapshots (extra disk use)", state.preferences.keep_original_snapshots, Message::PreferenceSnapshots),
            text("Crash reports").size(design::HEADING),
            text("Local diagnostics for model loading, GPU uploads, and Rust crashes. Enabled by default; logs may include local asset names and file paths.")
                .size(design::LABEL).style(design::muted),
            crate::ui_kit::switch(&state.ui, "crash-logging", "Crash logging", state.preferences.crash_logging_enabled, Message::PreferenceCrashLogging),
            row![
                design::button("Open crash log").on_press(Message::OpenCrashLogs).style(design::secondary),
                design::button("Open logs folder").on_press(Message::OpenCrashLogsFolder).style(design::secondary),
            ].spacing(8),
            crate::ui_kit::callout(crate::design::Tone::Warning, "Disabling logging stops new entries but does not delete existing logs. A native Windows fail-fast may not produce a Rust panic entry."),
            text("Recovery settings do not imply that every private-manager swap operation has been ported.").size(design::CAPTION).style(design::muted),
        ].spacing(8)),
        ].spacing(12).into(),
        Tab::Storage => column![
        game_client_setup(state),
        test_environment_setup(state),
        storage(state),
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
                .size(design::LABEL)
                .style(design::muted)
        ]
        .spacing(12),
    ]
    .spacing(16)
    .height(Fill)
    .into()
}

/// Swatch cards: Default, the curated presets, and any colours a preset
/// parked. Each card previews background, surface, accent and text.
fn theme_presets(state: &State) -> Element<'_, Message> {
    let light = state.preferences.appearance_mode == "Light";
    let modern = state.preferences.ui_theme == "Modern";
    let active = state.preferences.theme_preset.as_str();
    let mut cards = row![].spacing(10);
    cards = cards.push(swatch_card(
        "Default",
        "Built-in Evolve palette.",
        design::default_swatch(light),
        active.is_empty() && state.preferences.appearance_colors.is_empty(),
        modern.then_some(Message::PreferenceThemePreset("Default")),
    ));
    for preset in design::THEME_PRESETS {
        cards = cards.push(swatch_card(
            preset.name,
            preset.blurb,
            preset.swatch(light),
            active == preset.name,
            modern.then_some(Message::PreferenceThemePreset(preset.name)),
        ));
    }
    if let Some(saved) = state.preferences.theme_presets.get("Saved custom colours") {
        let pick = |role: &str, fallback: Color| {
            saved
                .get(role)
                .and_then(|value| design::hex(value))
                .unwrap_or(fallback)
        };
        let base = design::default_swatch(light);
        cards = cards.push(swatch_card(
            "Saved custom colours",
            "Your colours from before a preset.",
            [
                pick("background", base[0]),
                pick("surface", base[1]),
                pick("accent", base[2]),
                pick("text", base[3]),
            ],
            false,
            modern.then_some(Message::PreferenceThemePreset("Saved custom colours")),
        ));
    }
    iced::widget::scrollable(cards)
        .direction(iced::widget::scrollable::Direction::Horizontal(
            iced::widget::scrollable::Scrollbar::new()
                .width(4)
                .scroller_width(4),
        ))
        .into()
}

fn swatch_card<'a>(
    name: &'a str,
    blurb: &'a str,
    colors: [Color; 4],
    selected: bool,
    message: Option<Message>,
) -> Element<'a, Message> {
    let [background, surface, accent, foreground] = colors;
    let preview = container(
        column![
            container(space().height(6).width(Length::Fixed(46.0))).style(move |theme| {
                container::Style {
                    background: Some(Background::Color(accent)),
                    border: Border {
                        radius: design::radius(theme, design::Radius::Small).into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            }),
            container(text("Aa").size(design::BODY).color(foreground))
                .padding([4, 8])
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(Background::Color(surface)),
                    border: Border {
                        radius: design::radius(theme, design::Radius::Small).into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(6),
    )
    .padding(8)
    .width(Fill)
    .style(move |theme| container::Style {
        background: Some(Background::Color(background)),
        border: Border {
            radius: design::radius(theme, design::Radius::Medium).into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    iced::widget::button(
        column![
            preview,
            text(name).size(design::LABEL),
            text(blurb).size(10).style(design::muted),
        ]
        .spacing(6),
    )
    .width(Length::Fixed(150.0))
    .padding(8)
    .style(move |theme, status| {
        let mut style = design::secondary(theme, status);
        if selected {
            style.border.color = theme.palette().primary;
            style.border.width = 2.0;
        }
        style
    })
    .on_press_maybe(message)
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
