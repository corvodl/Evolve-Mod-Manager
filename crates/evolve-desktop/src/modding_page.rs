//! Modding workspace, aligned with the private Qt project's page structure.
//! Projects and changed assets are central. Specialist archive actions are
//! contextual and collapsed by default. Cryptographic keys are never entered.
use super::{Message, State};
use crate::design;
use evolve_core::mod_safety::{ModKind, classify};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Background, Border, Element, Fill, Length};

pub(super) const PAGE: usize = 90;

pub(super) fn matching_projects(state: &State) -> Vec<usize> {
    let query = state.modding_filter.to_lowercase();
    state
        .modding_projects
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            (state.modding_kind.is_none_or(|kind| {
                kind == classify(p.modified.iter().map(String::as_str), p.changed_known)
            }) && (query.is_empty()
                || [&p.label, &p.target, &p.source_build]
                    .iter()
                    .any(|value| value.to_lowercase().contains(&query))))
            .then_some(i)
        })
        .collect()
}

pub(super) fn page_range(count: usize, page: usize) -> std::ops::Range<usize> {
    let start = page.min(count.saturating_sub(1) / PAGE) * PAGE;
    start..(start + PAGE).min(count)
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(10)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 12.0.into(),
            },
            ..container::Style::default()
        })
}

fn menu(_state: &State) -> Element<'_, Message> {
    row![
        design::button("File")
            .style(design::text_button)
            .on_press(Message::ModdingMenu("File")),
        design::button("Edit")
            .style(design::text_button)
            .on_press(Message::ModdingMenu("Edit")),
        design::button("View")
            .style(design::text_button)
            .on_press(Message::ModdingMenu("View")),
        design::button("Tools")
            .style(design::text_button)
            .on_press(Message::ModdingMenu("Tools")),
        space().width(Fill),
        text("Workspace actions").size(12).style(design::muted)
    ]
    .spacing(15)
    .align_y(iced::Center)
    .into()
}

fn dropdown(state: &State) -> Option<Element<'_, Message>> {
    let commands: Element<'_, Message> = match state.modding_menu? {
        "File" => column![
            design::button("Refresh Projects")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingRefresh))),
            design::button("Browse staged PAKs")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingBrowsePaks))),
            design::button("Installed Mods / Profiles").on_press(Message::MenuCommand(Box::new(
                Message::ModdingToggleProfiles
            ))),
        ]
        .spacing(3)
        .into(),
        "Edit" => column![
            design::button("Continue Editing")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingOpenProject))),
            design::button("Review Changes").on_press(Message::MenuCommand(Box::new(
                Message::ModdingReviewChanges
            ))),
            design::button("Build Mod")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingBuild))),
        ]
        .spacing(3)
        .into(),
        "View" => column![
            design::button(if state.modding_advanced {
                "Hide archive browser"
            } else {
                "Show archive browser"
            })
            .on_press(Message::MenuCommand(Box::new(
                Message::ModdingToggleAdvanced
            ))),
            design::button("Open Asset Viewer")
                .on_press(Message::MenuCommand(Box::new(Message::OpenViewer))),
        ]
        .spacing(3)
        .into(),
        "Tools" => column![
            design::button("Verify selected signed PAK").on_press(Message::MenuCommand(Box::new(
                Message::StartSignedInspection
            ))),
            design::button("Initialize isolated stage")
                .on_press(Message::MenuCommand(Box::new(Message::StartStageInit))),
            design::button("Check stage")
                .on_press(Message::MenuCommand(Box::new(Message::StartStageStatus))),
            design::button("Restore backup")
                .on_press(Message::MenuCommand(Box::new(Message::StartStageRestore))),
            design::button("Import signed original")
                .on_press(Message::MenuCommand(Box::new(Message::StartStageImport))),
            design::button("Apply signed replacement")
                .on_press(Message::MenuCommand(Box::new(Message::StartStageApply))),
            design::button("Installed Mods / Profiles").on_press(Message::MenuCommand(Box::new(
                Message::ModdingToggleProfiles
            ))),
            design::button("Asset Library").on_press(Message::MenuCommand(Box::new(
                Message::Navigate(super::Page::AssetLibrary)
            ))),
            design::button("Verify offline launcher + signed PAKs").on_press(Message::MenuCommand(
                Box::new(Message::StartOfflineLauncherCheck)
            )),
            design::button("Play with mods (existing private helper)").on_press(
                Message::MenuCommand(Box::new(Message::StartOfflineLauncherLaunch))
            ),
            design::button("Restore original PAKs + hook").on_press(Message::MenuCommand(
                Box::new(Message::StartOfflineLauncherRestore)
            )),
            text("Offline only. Launcher and backup checks are verified before use.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(3)
        .into(),
        _ => return None,
    };
    Some(commands)
}

fn dropdown_left(name: Option<&str>) -> f32 {
    match name {
        Some("File") => 8.0,
        Some("Edit") => 72.0,
        Some("View") => 138.0,
        Some("Tools") => 203.0,
        _ => 8.0,
    }
}

fn project_header() -> Element<'static, Message> {
    row![
        text("ENABLED").width(Length::FillPortion(2)),
        text("MOD").width(Length::FillPortion(4)),
        text("TARGET PAK").width(Length::FillPortion(4)),
        text("CHANGED").width(Length::FillPortion(1)),
        text("SOURCE BUILD").width(Length::FillPortion(2)),
        text("MOD TYPE").width(Length::FillPortion(2)),
        text("DETAILS").width(Length::FillPortion(3)),
    ]
    .spacing(8)
    .into()
}

fn projects(state: &State) -> Element<'_, Message> {
    let mut rows = column![project_header()].spacing(4);
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    let matches = matching_projects(state);
    let count = matches.len();
    let range = page_range(count, state.modding_page);
    for &index in &matches[range.clone()] {
        let record = &projects[index];
        let selected = state.project_selection.indices.contains(&index);
        let changed = if record.changed_known {
            record.modified.len().to_string()
        } else {
            "Review".into()
        };
        let cells = row![
            text(record.label.as_str())
                .size(12)
                .width(Length::FillPortion(4)),
            text(record.target.as_str())
                .size(12)
                .width(Length::FillPortion(4)),
            text(changed).size(12).width(Length::FillPortion(1)),
            text(record.source_build.as_str())
                .size(12)
                .width(Length::FillPortion(2)),
            text(
                classify(
                    record.modified.iter().map(String::as_str),
                    record.changed_known
                )
                .label()
            )
            .size(12)
            .width(Length::FillPortion(2)),
            text(record.details.as_str())
                .size(12)
                .width(Length::FillPortion(3)),
        ]
        .spacing(8)
        .align_y(iced::Center);
        rows = rows.push(
            container(
                row![
                    design::button(if record.enabled { "On" } else { "Off" })
                        .style(if record.enabled {
                            design::primary
                        } else {
                            design::secondary
                        })
                        .on_press(Message::ModdingToggleEnabled(index))
                        .width(Length::FillPortion(2)),
                    design::button(cells)
                        .width(Length::FillPortion(20))
                        .padding([
                            if state.preferences.ui_density == "Compact" {
                                2
                            } else {
                                5
                            },
                            7,
                        ])
                        .style(design::text_button)
                        .on_press(Message::ModdingSelectProject(index)),
                ]
                .spacing(8),
            )
            .width(Fill)
            .style(move |theme| container::Style {
                background: Some(Background::Color(if selected {
                    design::selection(theme)
                } else {
                    design::surface(theme)
                })),
                ..container::Style::default()
            }),
        );
    }
    let empty: Element<'_, Message> =
        if projects.is_empty() {
            text(if state.preferences.projects.trim().is_empty() {
            "No projects folder configured. Set Projects in Settings, then refresh."
        } else {
            "No editable projects found. The Asset Library stores originals, not editing projects."
        }).size(13).style(design::muted).into()
        } else if count == 0 {
            text("No projects match this filter.")
                .size(13)
                .style(design::muted)
                .into()
        } else {
            text(format!(
                "{}–{} of {count} matching projects",
                range.start + 1,
                range.end
            ))
            .size(11)
            .style(design::muted)
            .into()
        };
    panel(
        column![
            scrollable(rows).height(Length::FillPortion(3)),
            row![
                empty,
                space().width(Fill),
                design::button("Previous")
                    .on_press_maybe((range.start > 0).then_some(Message::ModdingPage(false))),
                design::button("Next")
                    .on_press_maybe((range.end < count).then_some(Message::ModdingPage(true))),
            ]
            .spacing(8)
            .align_y(iced::Center)
        ]
        .spacing(8)
        .height(Fill),
    )
    .height(Length::FillPortion(3))
    .into()
}

fn modifications(state: &State) -> Element<'_, Message> {
    let record = state.modding_selected.and_then(|index| {
        state
            .modding_projects
            .as_ref()
            .and_then(|list| list.get(index))
    });
    let mut rows = column![
        row![
            text("MODIFIED FILES IN SELECTED PROJECT")
                .size(12)
                .style(design::muted),
            space().width(Fill),
            design::button("Review Changes")
                .style(design::secondary)
                .on_press(Message::ModdingReviewChanges),
        ]
        .align_y(iced::Center)
    ]
    .spacing(5);
    if let Some(record) = record {
        let range = page_range(record.modified.len(), state.modding_changes_page);
        for changed in &record.modified[range.clone()] {
            rows = rows.push(
                row![
                    text(changed.as_str())
                        .width(Length::FillPortion(3))
                        .size(12),
                    text(evolve_core::mod_safety::member_kind(changed).label())
                        .width(Length::FillPortion(2))
                        .size(11)
                        .style(design::muted),
                ]
                .spacing(12),
            );
        }
        if !record.modified.is_empty() {
            rows = rows.push(
                row![
                    text(format!(
                        "{}–{} of {} changed files",
                        range.start + 1,
                        range.end,
                        record.modified.len()
                    ))
                    .size(12),
                    space().width(Fill),
                    design::button("Previous").on_press_maybe(
                        (range.start > 0).then_some(Message::ModdingChangesPage(false))
                    ),
                    design::button("Next").on_press_maybe(
                        (range.end < record.modified.len())
                            .then_some(Message::ModdingChangesPage(true))
                    ),
                ]
                .spacing(8),
            );
        } else {
            rows = rows.push(text(if record.changed_known {
                "No modified files detected in this project."
            } else {
                "Some files could not be checked; review the source and project before building."
            }).size(12).style(design::muted));
        }
    } else {
        rows = rows.push(
            text("Select an editable project above to see changed files.")
                .size(12)
                .style(design::muted),
        );
    }
    panel(scrollable(rows).height(Fill))
        .height(Length::FillPortion(2))
        .into()
}

fn archive_browser(state: &State) -> Element<'_, Message> {
    let mut list = column![].spacing(2);
    let mut count = 0usize;
    if let Some(inventory) = &state.inventory {
        for (index, file) in inventory.entries.iter().enumerate() {
            let is_pak = file.relative_path.to_ascii_lowercase().ends_with(".pak");
            if !is_pak
                || !state.search.is_empty()
                    && !file
                        .relative_path
                        .to_ascii_lowercase()
                        .contains(&state.search.to_ascii_lowercase())
            {
                continue;
            }
            count += 1;
            if count <= PAGE {
                let selected = state.file_selection.indices.contains(&index);
                list = list.push(
                    design::button(row![
                        text(file.relative_path.as_str()).size(12).width(Fill),
                        text(format!("{} KiB", file.byte_size / 1024))
                            .size(12)
                            .style(design::muted)
                    ])
                    .width(Fill)
                    .padding([5, 8])
                    .style(if selected {
                        design::primary
                    } else {
                        design::text_button
                    })
                    .on_press(Message::ModdingSelectArchive(index)),
                );
            }
        }
    }
    panel(column![
        row![
            design::text_input("Search PAKs, maps and dependency groups…", &state.search)
                .on_input(Message::SearchChanged).width(Fill),
            design::button("Refresh").style(design::secondary).on_press(Message::ModdingBrowsePaks)
        ].spacing(8),
        scrollable(list).height(Length::Fixed(180.0)),
        text(format!("{count} matching staged PAK(s)")).size(12).style(design::muted),
        row![
            design::button("Verify PAK").style(design::secondary)
                .on_press(Message::StartSignedInspection),
            design::button("Unpack PAK").style(design::secondary)
                .on_press(Message::ModdingUnpackPak),
            design::button("Edit Files").style(design::primary)
                .on_press(Message::ModdingOpenProject),
            design::button("Level Editor").style(design::secondary)
                .on_press(Message::Navigate(super::Page::Levels)),
        ].spacing(8),
        text("SPECIALIST · REBUILD ONE SIGNED MEMBER").size(12).style(design::muted),
        row![
            design::text_input("Exact archive member path…", &state.signed_entry)
                .on_input(Message::SignedMemberChanged).width(Fill),
            design::text_input("Modified replacement file…", &state.signed_replacement)
                .on_input(Message::SignedReplacementChanged).width(Fill),
        ].spacing(7),
        row![
            design::text_input("NEW output path ending in original .pak basename…", &state.output)
                .on_input(Message::OutputChanged).width(Fill),
            design::button("Rebuild one member").style(design::secondary)
                .on_press(Message::StartSignedRebuild),
        ].spacing(7),
        text("Signing keys and the signed basename are resolved from your prepared stage and selected archive.")
            .size(12).style(design::muted),
    ].spacing(8)).into()
}

/// One-click source PAK editing from the persisted Asset Library. This
/// shares the Asset Library selection and preparation action, but does not
/// require visiting that page or starting a new import on every launch.
fn imported_paks(state: &State) -> Element<'_, Message> {
    let mut rows = column![].spacing(2);
    let query = state.modding_filter.to_ascii_lowercase();
    let mut matching = 0usize;
    if let Some(library) = &state.library_catalog {
        for (index, archive) in library.archives.iter().enumerate() {
            if !query.is_empty()
                && !archive.name.to_ascii_lowercase().contains(&query)
                && !archive.group.to_ascii_lowercase().contains(&query)
                && !archive.source.to_ascii_lowercase().contains(&query)
            {
                continue;
            }
            matching += 1;
            if matching > 40 {
                continue;
            }
            rows = rows.push(
                design::button(
                    row![
                        text(archive.name.as_str()).size(12).width(Fill),
                        text(archive.group.as_str()).size(11).style(design::muted)
                    ]
                    .spacing(8),
                )
                .width(Fill)
                .style(if state.library_selected == Some(index) {
                    design::secondary
                } else {
                    design::text_button
                })
                .on_press(Message::ModdingSelectImported(index)),
            );
        }
    }
    let selected = state
        .library_selected
        .and_then(|i| state.library_catalog.as_ref()?.archives.get(i))
        .is_some();
    let description = if state.library_startup_loading {
        "Loading saved PAK manifests; you do not need to reimport."
    } else if state.library_catalog.is_some() {
        "Previously imported PAKs are available for editing. Select one to create a staged copy; source files stay unchanged."
    } else {
        "No saved Asset Library loaded. Reload existing manifests, or use Browse staged PAKs to edit your prepared stage directly."
    };
    panel(
        column![
            row![
                text("AVAILABLE IMPORTED PAKS").size(13),
                space().width(Fill),
                text(format!("{matching} match(es)"))
                    .size(11)
                    .style(design::muted),
                design::button("Reload saved library")
                    .on_press_maybe(
                        (!state.library_startup_loading).then_some(Message::ModdingReloadLibrary)
                    )
                    .style(design::text_button),
            ]
            .spacing(8)
            .align_y(iced::Center),
            text(description).size(11).style(design::muted),
            scrollable(rows).height(Length::Fixed(120.0)),
            row![
                design::button("Edit selected PAK / Add to Project")
                    .style(design::primary)
                    .on_press_maybe(selected.then_some(Message::LibraryMakeModProject)),
                design::button("Browse staged PAKs")
                    .style(design::secondary)
                    .on_press(Message::ModdingBrowsePaks),
            ]
            .spacing(8),
        ]
        .spacing(5),
    )
    .into()
}

fn profiles(state: &State) -> Element<'_, Message> {
    let mut catalog = column![].spacing(3);
    if let Some(data) = &state.profile_catalog {
        for profile in data.profiles.iter().take(40) {
            catalog = catalog.push(
                design::button(
                    text(format!("{} · {} mod(s)", profile.name, profile.mods.len())).size(12),
                )
                .style(design::text_button)
                .on_press(Message::SelectProfile(profile.id.clone())),
            );
        }
    } else {
        catalog = catalog.push(
            text("Select Refresh to browse offline profiles.")
                .size(12)
                .style(design::muted),
        );
    }
    panel(
        column![
            text("INSTALLED MODS / PROFILES").size(14),
            row![
                design::text_input("New profile name…", &state.profile_name)
                    .on_input(Message::ProfileNameChanged)
                    .width(Fill),
                design::button("Create profile").on_press(Message::StartProfileCreate),
                design::button("Refresh").on_press(Message::StartProfileList)
            ]
            .spacing(8),
            scrollable(catalog).height(Length::Fixed(110.0)),
            row![
                design::button("Add signed PAK")
                    .style(design::secondary)
                    .on_press(Message::StartProfileAdd),
                design::button("Verify selected")
                    .style(design::secondary)
                    .on_press(Message::StartProfilePlan),
                design::button("Apply to offline stage")
                    .style(design::secondary)
                    .on_press(Message::StartProfileApply),
                design::button("Check recovery")
                    .style(design::secondary)
                    .on_press(Message::StartProfileStatus),
                design::button("Restore staged profile")
                    .style(design::secondary)
                    .on_press(Message::StartProfileRecover),
            ]
            .spacing(8),
            text(state.profile_feedback.as_str())
                .size(12)
                .style(design::muted)
        ]
        .spacing(7),
    )
    .into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let chosen = state.modding_selected.and_then(|index| {
        state
            .modding_projects
            .as_ref()
            .and_then(|list| list.get(index))
    });
    let mut actions = row![
        design::button("Refresh Mods")
            .style(design::secondary)
            .on_press(Message::ModdingRefresh),
        design::button("Continue Editing")
            .style(design::secondary)
            .on_press(Message::ModdingOpenProject),
    ]
    .spacing(7)
    .align_y(iced::Center);
    if state.modding_advanced {
        actions = actions
            .push(design::button("Advanced Profiles…")
                .style(design::secondary)
                .on_press(Message::ModdingToggleProfiles))
            .push(design::button("Build Selected Mod")
                .style(design::secondary)
                .on_press(Message::ModdingBuild))
            .push(design::button("Prepare Selected Mod for Test")
                .style(design::secondary)
                .on_press(Message::ModdingBuildForOfflineTest));
    }
    let mut main = column![
        text("Modding").size(22),
        text("Name your mods, switch them On or Off, then choose Play with Mods. Enabled mods are built and loaded automatically for offline play.")
            .size(12)
            .style(design::muted),
        menu(state),
        row![
            design::text_input(
                "Find workspace, target PAK or build…",
                &state.modding_filter
            )
            .on_input(Message::ModdingFilterChanged)
            .width(Fill),
            text(
                chosen
                    .map(|p| p.target.as_str())
                    .unwrap_or("Select a project")
            )
            .size(12)
            .style(design::muted),
        ]
        .spacing(10)
        .align_y(iced::Center),
        row![
            design::button("All mods").on_press(Message::ModdingKind(None)),
            design::button("Cosmetic candidates").on_press(Message::ModdingKind(Some(ModKind::CosmeticCandidate))),
            design::button("Gameplay").on_press(Message::ModdingKind(Some(ModKind::Gameplay))),
            design::button("Needs review").on_press(Message::ModdingKind(Some(ModKind::NeedsReview))),
        ].spacing(6),
        text("Cosmetic candidates need content verification before online use. Gameplay, mixed and unverified mods stay offline. Current mod launching is offline only.").size(12).style(design::muted),
        row![
            design::button("Gameplay values…").on_press(Message::GuidedEdit(crate::guided_modding::Action::Open(false))),
            design::button("Models…").on_press(Message::GuidedEdit(crate::guided_modding::Action::Open(true))),
        ].spacing(6),
        imported_paks(state),
        projects(state),
        row![
            text("Mod name").size(12),
            design::text_input("Select a mod to name it…", &state.modding_name_input)
                .on_input(Message::ModdingNameChanged)
                .width(Length::FillPortion(4)),
            design::button("Save Name")
                .style(design::secondary)
                .on_press_maybe(state.modding_selected.map(|_| Message::ModdingSaveName)),
        ]
        .spacing(8)
        .align_y(iced::Center),
        modifications(state),
        actions,
        design::button(if state.modding_advanced {
            "Hide Advanced Archive Tools"
        } else {
            "Show Advanced Archive Tools"
        })
        .style(design::secondary)
        .on_press(Message::ModdingToggleAdvanced),
    ]
    .spacing(9)
    .height(Fill);
    if state.guided_editor.open {
        return column![text("Modding").size(22), crate::guided_modding::view(state)]
            .spacing(10)
            .into();
    }
    if state.modding_advanced {
        main = main.push(archive_browser(state));
    }
    if state.modding_profiles {
        main = main.push(profiles(state));
    }
    crate::menu_popup::overlay(
        main,
        dropdown(state),
        dropdown_left(state.modding_menu),
        115.0,
        310.0,
    )
}
