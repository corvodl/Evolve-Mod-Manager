//! Modding workspace, aligned with the private Qt project's page structure.
//! Projects and changed assets are central. Specialist archive actions are
//! contextual and collapsed by default. Cryptographic keys are never entered.
use super::{Message, State};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Length};

const MUTED: Color = Color::from_rgb(0.69, 0.72, 0.77);
const SURFACE: Color = Color::from_rgb(0.17, 0.18, 0.20);
const SELECTED: Color = Color::from_rgb(0.24, 0.20, 0.22);
const LINE: Color = Color::from_rgb(0.26, 0.27, 0.30);
const PAGE: usize = 90;

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content).width(Fill).padding(10).style(|_| container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 7.0.into(),
        },
        ..container::Style::default()
    })
}

fn menu<'a>(state: &'a State) -> Element<'a, Message> {
    let items = row![
        button("File").style(button::text).on_press(Message::ModdingMenu("File")),
        button("Edit").style(button::text).on_press(Message::ModdingMenu("Edit")),
        button("View").style(button::text).on_press(Message::ModdingMenu("View")),
        button("Tools").style(button::text).on_press(Message::ModdingMenu("Tools")),
        space().width(Fill),
        text("Workspace actions").size(12).color(MUTED)
    ].spacing(15).align_y(iced::Center);
    let commands: Element<'a, Message> = match state.modding_menu {
        Some("File") => row![
            button("Refresh Projects").on_press(Message::ModdingRefresh),
            button("Browse staged PAKs").on_press(Message::ModdingBrowsePaks),
            button("Installed Mods / Profiles").on_press(Message::ModdingToggleProfiles),
        ].spacing(8).into(),
        Some("Edit") => row![
            button("Continue Editing").on_press(Message::ModdingOpenProject),
            button("Review Changes").on_press(Message::ModdingReviewChanges),
            button("Build Mod").style(button::primary).on_press(Message::ModdingBuild),
        ].spacing(8).into(),
        Some("View") => row![
            button(if state.modding_advanced { "Hide archive browser" } else { "Show archive browser" })
                .on_press(Message::ModdingToggleAdvanced),
            button("Open Asset Viewer").on_press(Message::OpenViewer),
        ].spacing(8).into(),
        Some("Tools") => row![
            button("Verify selected signed PAK").on_press(Message::StartSignedInspection),
            button("Check offline stage").on_press(Message::StartStageStatus),
            button("Restore offline stage").on_press(Message::StartStageRestore),
            button("Asset Library").on_press(Message::Navigate(super::Page::AssetLibrary)),
        ].spacing(8).into(),
        _ => text("").into(),
    };
    if state.modding_menu.is_some() {
        column![items, commands].spacing(7).into()
    } else {
        items.into()
    }
}

fn project_header() -> Element<'static, Message> {
    row![
        text("PROJECT").width(Length::FillPortion(4)),
        text("TARGET PAK").width(Length::FillPortion(4)),
        text("CHANGED").width(Length::FillPortion(1)),
        text("SOURCE BUILD").width(Length::FillPortion(2)),
        text("STATE").width(Length::FillPortion(2)),
        text("DETAILS").width(Length::FillPortion(3)),
    ].spacing(8).into()
}

fn projects(state: &State) -> Element<'_, Message> {
    let mut rows = column![project_header()].spacing(4);
    let query = state.modding_filter.to_ascii_lowercase();
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    let matches: Vec<usize> = projects.iter().enumerate().filter_map(|(index, project)| {
        if query.is_empty()
            || project.label.to_ascii_lowercase().contains(&query)
            || project.target.to_ascii_lowercase().contains(&query)
            || project.source_build.to_ascii_lowercase().contains(&query)
        {
            Some(index)
        } else {
            None
        }
    }).collect();
    let count = matches.len();
    for index in matches.into_iter().take(PAGE) {
        let record = &projects[index];
        let selected = state.modding_selected == Some(index);
        let changed = if record.changed_known {
            record.modified.len().to_string()
        } else {
            "Review".into()
        };
        let cells = row![
            text(record.label.as_str()).size(12).width(Length::FillPortion(4)),
            text(record.target.as_str()).size(12).width(Length::FillPortion(4)),
            text(changed).size(12).width(Length::FillPortion(1)),
            text(record.source_build.as_str()).size(12).width(Length::FillPortion(2)),
            text(record.status.as_str()).size(12).width(Length::FillPortion(2)),
            text(record.details.as_str()).size(12).width(Length::FillPortion(3)),
        ].spacing(8).align_y(iced::Center);
        rows = rows.push(container(
            button(cells).width(Fill).padding([6, 7])
                .style(button::text).on_press(Message::ModdingSelectProject(index))
        ).width(Fill).style(move |_| container::Style {
            background: Some(Background::Color(if selected { SELECTED } else { SURFACE })),
            ..container::Style::default()
        }));
    }
    let empty: Element<'_, Message> = if projects.is_empty() {
        text(if state.preferences.projects.trim().is_empty() {
            "No projects folder configured. Set Projects in Settings, then refresh."
        } else {
            "No editable projects found. The Asset Library stores originals, not editing projects."
        }).size(13).color(MUTED).into()
    } else if count == 0 {
        text("No projects match this filter.").size(13).color(MUTED).into()
    } else {
        text(format!("{count} matching project(s); showing first {PAGE}."))
            .size(11).color(MUTED).into()
    };
    panel(column![
        scrollable(rows).height(Length::FillPortion(3)),
        empty,
    ].spacing(8).height(Fill)).height(Length::FillPortion(3)).into()
}

fn modifications(state: &State) -> Element<'_, Message> {
    let record = state.modding_selected.and_then(|index|
        state.modding_projects.as_ref().and_then(|list| list.get(index))
    );
    let mut rows = column![row![
        text("MODIFIED FILES IN SELECTED PROJECT").size(12).color(MUTED),
        space().width(Fill),
        button("Review Changes").style(button::secondary)
            .on_press(Message::ModdingReviewChanges),
    ].align_y(iced::Center)].spacing(5);
    if let Some(record) = record {
        for changed in record.modified.iter().take(PAGE) {
            rows = rows.push(row![
                text(changed.as_str()).width(Length::FillPortion(3)).size(12),
                text(record.workspace.to_string_lossy().to_string())
                    .width(Length::FillPortion(2)).size(11).color(MUTED),
            ].spacing(12));
        }
        if record.modified.is_empty() {
            rows = rows.push(text(if record.changed_known {
                "No modified files detected in this project."
            } else {
                "Some files could not be checked; review the source and project before building."
            }).size(12).color(MUTED));
        }
    } else {
        rows = rows.push(text("Select an editable project above to see changed files.")
            .size(12).color(MUTED));
    }
    panel(scrollable(rows).height(Fill)).height(Length::FillPortion(2)).into()
}

fn archive_browser(state: &State) -> Element<'_, Message> {
    let mut list = column![].spacing(2);
    let mut count = 0usize;
    if let Some(inventory) = &state.inventory {
        for (index, file) in inventory.entries.iter().enumerate() {
            let is_pak = file.relative_path.to_ascii_lowercase().ends_with(".pak");
            if !is_pak || !state.search.is_empty()
                && !file.relative_path.to_ascii_lowercase().contains(&state.search.to_ascii_lowercase()) {
                continue;
            }
            count += 1;
            if count <= PAGE {
                let selected = state.selected == Some(index);
                list = list.push(button(row![
                    text(file.relative_path.as_str()).size(12).width(Fill),
                    text(format!("{} KiB", file.byte_size / 1024)).size(12).color(MUTED)
                ]).width(Fill).padding([5, 8])
                    .style(if selected { button::primary } else { button::text })
                    .on_press(Message::ModdingSelectArchive(index)));
            }
        }
    }
    panel(column![
        row![
            text_input("Search PAKs, maps and dependency groups…", &state.search)
                .on_input(Message::SearchChanged).width(Fill),
            button("Refresh").style(button::secondary).on_press(Message::ModdingBrowsePaks)
        ].spacing(8),
        scrollable(list).height(Length::Fixed(180.0)),
        text(format!("{count} matching staged PAK(s)")).size(12).color(MUTED),
        row![
            button("Verify PAK").style(button::secondary)
                .on_press(Message::StartSignedInspection),
            button("Unpack PAK").style(button::secondary)
                .on_press(Message::ModdingUnpackPak),
            button("Edit Files").style(button::primary)
                .on_press(Message::ModdingOpenProject),
            button("Level Editor").style(button::secondary)
                .on_press(Message::Navigate(super::Page::Levels)),
        ].spacing(8),
        text("Signing keys and the signed basename are resolved from your prepared stage and selected archive.")
            .size(12).color(MUTED),
    ].spacing(8)).into()
}

fn profiles(state: &State) -> Element<'_, Message> {
    let mut catalog = column![].spacing(3);
    if let Some(data) = &state.profile_catalog {
        for profile in data.profiles.iter().take(40) {
            catalog = catalog.push(
                button(text(format!("{} · {} mod(s)", profile.name, profile.mods.len())).size(12))
                    .style(button::text)
                    .on_press(Message::SelectProfile(profile.id.clone()))
            );
        }
    } else {
        catalog = catalog.push(text("Select Refresh to browse offline profiles.").size(12).color(MUTED));
    }
    panel(column![
        text("INSTALLED MODS / PROFILES").size(14),
        row![
            text_input("New profile name…", &state.profile_name)
                .on_input(Message::ProfileNameChanged).width(Fill),
            button("Create profile").on_press(Message::StartProfileCreate),
            button("Refresh").on_press(Message::StartProfileList)
        ].spacing(8),
        scrollable(catalog).height(Length::Fixed(110.0)),
        row![
            button("Verify selected").on_press(Message::StartProfilePlan),
            button("Restore staged profile").on_press(Message::StartProfileRecover),
        ].spacing(8),
        text(state.profile_feedback.as_str()).size(12).color(MUTED)
    ].spacing(7)).into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let chosen = state.modding_selected.and_then(|index|
        state.modding_projects.as_ref().and_then(|list| list.get(index))
    );
    let actions = row![
        button("Install Mod…").style(button::secondary)
            .on_press(Message::ModdingToggleProfiles),
        button("Installed Mods…").style(button::secondary)
            .on_press(Message::ModdingToggleProfiles),
        button("Refresh Projects").style(button::secondary)
            .on_press(Message::ModdingRefresh),
        button("Continue Editing").style(button::secondary)
            .on_press(Message::ModdingOpenProject),
        button("Build Mod").style(button::primary)
            .on_press(Message::ModdingBuild),
        space().width(Fill),
    ].spacing(7).align_y(iced::Center);
    let mut main = column![
        text("Modding").size(22),
        text("Projects contain editable copies. Original signed PAKs stay in the Asset Library.")
            .size(12).color(MUTED),
        menu(state),
        row![
            text_input("Find workspace, target PAK or build…", &state.modding_filter)
                .on_input(Message::ModdingFilterChanged).width(Fill),
            text(chosen.map(|p| p.target.as_str()).unwrap_or("Select a project"))
                .size(12).color(MUTED),
        ].spacing(10).align_y(iced::Center),
        projects(state),
        modifications(state),
        actions,
        button(if state.modding_advanced {
            "Hide Advanced Archive Tools"
        } else {
            "Show Advanced Archive Tools"
        }).style(button::secondary).on_press(Message::ModdingToggleAdvanced),
    ].spacing(9).height(Fill);
    if state.modding_advanced {
        main = main.push(archive_browser(state));
    }
    if state.modding_profiles {
        main = main.push(profiles(state));
    }
    main.into()
}
