//! Private-manager-inspired Settings: grouped Game Storage, Asset Library,
//! appearance and update panels. Advanced paths are hidden by default.
use super::{Message, Page, State};
use iced::widget::{button, column, container, row, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Length};

const PANEL: Color = Color::from_rgb8(39, 40, 46);
const EDGE: Color = Color::from_rgb8(57, 57, 66);
const MUTED: Color = Color::from_rgb8(173, 178, 190);

fn panel<'a>(child: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(child)
        .width(Fill)
        .padding(14)
        .style(|_| container::Style {
            background: Some(Background::Color(PANEL)),
            border: Border {
                color: EDGE,
                width: 1.0,
                radius: 7.0.into(),
            },
            ..container::Style::default()
        })
}

fn heading<'a>(name: &'a str, description: &'a str) -> iced::widget::Column<'a, Message> {
    column![text(name).size(17), text(description).size(12).color(MUTED)].spacing(7)
}

fn mib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
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
        text("One location for future setups, with storage breakdown and advanced paths.").size(12).color(MUTED),
        row![
            text("Mod Data Location").size(12),
            container(text(root).size(12)).width(Fill).padding(9).style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb8(31, 31, 37))),
                border: Border { color: EDGE, width: 1.0, radius: 5.0.into() },
                ..container::Style::default()
            }),
            button("Change…").on_press(Message::SettingsToggleAdvancedPaths).style(button::secondary)
        ].spacing(9).align_y(iced::Center),
        text("Changing this location affects FUTURE setups only. Existing signing and recovery files are not moved.").size(11).color(MUTED),
        text(format!("Active stage: {}", if state.preferences.stage.is_empty() { "Not configured" } else { &state.preferences.stage })).size(11),
        container(column![
            row![
                text("STORAGE BREAKDOWN").size(12),
                space().width(Fill),
                text(if state.storage_analyzing { "Checking storage…" } else if state.storage_usage.is_some() { "Scan complete" } else { "Not yet analyzed" }).size(11).color(MUTED)
            ],
            strip,
            legend,
            row![
                button("Analyze Storage").on_press_maybe((!state.storage_analyzing && !state.busy).then_some(Message::SettingsAnalyzeStorage)).style(button::secondary),
                text(state.storage_usage.as_ref().map_or("No storage analysis yet.".to_owned(), |r| format!("{} files checked{}", r.files, if r.truncated { " • Scan limit reached; totals incomplete" } else { "" }))).size(11).color(MUTED)
            ].spacing(8)
        ].spacing(8)).padding(10).width(Fill).style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgb8(31, 32, 37))),
            ..container::Style::default()
        }),
        button(text(if state.settings_advanced_paths { "▾ Advanced folder paths" } else { "▸ Advanced folder paths" }))
            .on_press(Message::SettingsToggleAdvancedPaths).style(button::text),
        if state.settings_advanced_paths {
            column![
                text("Future GameSets location").size(12),
                text_input("GameSets folder", &state.preferences.game_storage_root).on_input(Message::PreferenceGameStorage),
                text("Signing stage • existing location").size(12),
                text_input("Stage folder", &state.preferences.stage).on_input(Message::PreferenceStage),
                text("Launch restoration journal").size(12),
                text_input("Swap journal folder", &state.preferences.swap).on_input(Message::PreferenceSwap),
                text("Mod projects").size(12),
                text_input("Projects folder", &state.preferences.projects).on_input(Message::PreferenceProjects),
                text("Asset Library").size(12),
                text_input("Asset Library folder", &state.preferences.asset_library_root).on_input(Message::PreferenceAssets),
                text("Current editor workspace").size(12),
                text_input("Current workspace", &state.preferences.current_workspace).on_input(Message::PreferenceWorkspace),
                text("Paths are saved only when you select Save Settings; no data is automatically moved.").size(11).color(MUTED),
            ].spacing(6).into()
        } else {
            Element::<Message>::from(space().height(Length::Fixed(1.0)))
        },
        row![
            button("Set Up Manager").on_press(Message::Navigate(Page::Files)).style(button::primary).width(Fill),
            button("Check Setup").on_press(Message::StartStageStatus).style(button::secondary).width(Fill),
            button("Open Projects").on_press(Message::Navigate(Page::Files)).style(button::secondary).width(Fill),
            button("Save Settings").on_press(Message::SavePreferences).style(button::secondary).width(Fill),
        ].spacing(8),
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
        text("Reference PAKs for characters, textures, animations and levels. The location is detected automatically from the saved manager profile.").size(12).color(MUTED),
        text(format!("{} imported archives • {} stored", archive_count, if state.library_catalog.is_some() { mib(total) } else { "Not indexed yet".into() })).size(12),
        text(reference_counts.map_or("Shared references: —  •  Animation references: —".to_string(), |(shared, animations)| format!("Shared references: {shared}/6  •  Animation references: {animations}/3"))).size(12),
        text(if state.library_root.is_empty() { "Default Asset Library location could not be detected" } else { state.library_root.as_str() }).size(11).color(MUTED),
        button("Manage Asset Library / Add PAKs…").on_press(Message::Navigate(Page::AssetLibrary)).width(Fill).style(button::secondary),
    ].spacing(10)).into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    column![
        text("Settings").size(24),
        heading("GAME STORAGE", "Review disk usage and configure future setups without changing active recovery files."),
        storage(state),
        heading("ASSET LIBRARY", "Keep extracted reference PAKs available across Modding, models and levels."),
        library(state),
        heading("GENERAL & APPEARANCE", "Compact controls with the same sidebar and theme behavior as the private manager."),
        panel(column![
            row![
                button(text(format!("Appearance: {}", state.preferences.appearance_mode))).on_press(Message::PreferenceAppearance).style(button::secondary),
                button(text(format!("Density: {}", state.preferences.ui_density))).on_press(Message::PreferenceDensity).style(button::secondary),
                button(text(format!("Explorer: {}", state.preferences.file_explorer_side))).on_press(Message::PreferenceExplorerSide).style(button::secondary)
            ].spacing(8),
            button(text(format!("Reduce menu animation: {}", if state.preferences.reduce_motion { "On" } else { "Off" }))).on_press(Message::PreferenceReduceMotion).style(button::secondary),
            text("Color presets and font preferences are preserved in settings.json; visual pickers are not implemented yet.").size(11).color(MUTED),
        ].spacing(8)),
        heading("UPDATES", "Stable by default, with a verified experimental channel available."),
        panel(column![
            text(format!("Installed version: {}", env!("CARGO_PKG_VERSION"))).size(12),
            row![
                button("Check for Updates").on_press(Message::CheckUpdates).style(button::primary),
                button(text(format!("Experimental: {}", if state.preferences.experimental_updates { "On" } else { "Off" }))).on_press(Message::PreferenceExperimental).style(button::secondary),
                button(text(format!("Auto-check: {}", if state.preferences.automatic_update_checks { "On" } else { "Off" }))).on_press(Message::PreferenceAutoUpdates).style(button::secondary)
            ].spacing(8),
            text(state.update_status.as_str()).size(12).color(MUTED),
            super::download_action(state),
            super::run_installer_action(state),
        ].spacing(8)),
        heading("BLENDER INTEGRATION", "Blender is used for verified native FBX export."),
        panel(column![
            text("Blender executable").size(12),
            text_input("Path to Blender executable", &state.preferences.blender_executable).on_input(Message::PreferenceBlender),
        ].spacing(7)),
        heading("BACKUPS & RECOVERY", "Preserve source archives and restoration history."),
        panel(column![
            button(text(format!("Keep permanent original snapshots: {}", if state.preferences.keep_original_snapshots { "On" } else { "Off" }))).on_press(Message::PreferenceSnapshots).style(button::secondary),
            button(text(format!("Show activity during tasks: {}", if state.preferences.show_activity_on_tasks { "On" } else { "Off" }))).on_press(Message::PreferenceActivity).style(button::secondary),
            text("Recovery settings do not imply that every private-manager swap operation has been ported.").size(11).color(MUTED),
        ].spacing(8)),
        row![
            text(state.settings_status.as_str()).size(12).color(MUTED).width(Fill),
            button("Save Settings").on_press(Message::SavePreferences).style(button::primary)
        ].spacing(8)
    ].spacing(12).into()
}

#[cfg(test)]
mod tests {
    use super::*;
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
