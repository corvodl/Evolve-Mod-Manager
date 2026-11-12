//! Modding workspace, aligned with the private Qt project's page structure.
//! Projects and changed assets are central. Specialist archive actions are
//! contextual and collapsed by default. Cryptographic keys are never entered.
use super::{Message, State};
use crate::design;
use crate::modding_projects::{PakExportMode, ProjectRow};
use evolve_core::mod_safety::{ModKind, classify};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Alignment, Background, Element, Fill, Length};
use std::collections::HashMap;

pub(super) const PAGE: usize = 90;

/// A full project manifest can contain up to 60,000 changed members. The
/// inspector is a live view, so cap the work per frame and say so rather than
/// presenting a partial scan as complete.
const MAX_SCAN_MEMBERS: usize = 200_000;
/// Keep the inspector usable for very large conflict sets.
const MAX_INSPECTOR_GROUPS: usize = 40;

/// One shared changed-member path across two or more enabled projects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberCollision {
    path: String,
    /// Project indices in the effective Play order.
    owners: Vec<usize>,
}

/// Two enabled sources targeting the same PAK. Play with Mods refuses this,
/// but cached state can still describe an inconsistent set.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetConflict {
    target: String,
    owners: Vec<usize>,
}

/// Derived, read-only load-order and collision view of the project registry.
///
/// The effective order is exactly the project-list order: `discover_projects`
/// sorts projects, and Play with Mods walks that list and keeps enabled rows.
/// No manual priority is persisted yet, so this view never invents one.
#[derive(Debug, Clone, Default)]
struct LoadOrderScan {
    /// Enabled project indices in effective Play order.
    order: Vec<usize>,
    /// 1-based Play position for each project index (`None` when disabled).
    positions: Vec<Option<usize>>,
    /// Number of shared changed-member paths per project index.
    shared_paths_by_project: Vec<usize>,
    /// Only groups with more than one owner.
    member_collisions: Vec<MemberCollision>,
    target_conflicts: Vec<TargetConflict>,
    scanned_members: usize,
    truncated: bool,
    incomplete_enabled: usize,
}

impl LoadOrderScan {
    fn enabled_count(&self) -> usize {
        self.order.len()
    }

    fn conflict_count(&self) -> usize {
        self.member_collisions.len() + self.target_conflicts.len()
    }

    fn position(&self, index: usize) -> Option<usize> {
        self.positions.get(index).copied().flatten()
    }

    fn shared_paths(&self, index: usize) -> usize {
        self.shared_paths_by_project
            .get(index)
            .copied()
            .unwrap_or(0)
    }

    fn has_target_conflict(&self, index: usize) -> bool {
        self.target_conflicts
            .iter()
            .any(|conflict| conflict.owners.contains(&index))
    }
}

/// Scan only enabled projects. Disabled projects cannot collide at runtime,
/// even if their manifests declare the same paths or PAK target.
fn scan_projects(projects: &[ProjectRow]) -> LoadOrderScan {
    let mut scan = LoadOrderScan {
        positions: vec![None; projects.len()],
        shared_paths_by_project: vec![0; projects.len()],
        ..LoadOrderScan::default()
    };

    for (index, project) in projects.iter().enumerate() {
        if project.enabled {
            scan.order.push(index);
            scan.positions[index] = Some(scan.order.len());
            if !project.changed_known {
                scan.incomplete_enabled += 1;
            }
        }
    }

    let mut targets: HashMap<String, (String, Vec<usize>)> = HashMap::new();
    for &index in &scan.order {
        let project = &projects[index];
        let key = project.target.replace('\\', "/").to_ascii_lowercase();
        targets
            .entry(key)
            .or_insert_with(|| (project.target.clone(), Vec::new()))
            .1
            .push(index);
    }
    scan.target_conflicts = targets
        .into_values()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(target, owners)| TargetConflict { target, owners })
        .collect();
    scan.target_conflicts.sort_by(|left, right| {
        left.owners
            .first()
            .cmp(&right.owners.first())
            .then_with(|| {
                left.target
                    .to_ascii_lowercase()
                    .cmp(&right.target.to_ascii_lowercase())
            })
    });

    // Windows PAK/VFS names are case-insensitive; lower-case keys prevent a
    // cosmetic case difference from hiding a real shared member path.
    let mut members: HashMap<String, (String, Vec<usize>)> = HashMap::new();
    'projects: for &index in &scan.order {
        for path in &projects[index].modified {
            if scan.scanned_members >= MAX_SCAN_MEMBERS {
                scan.truncated = true;
                break 'projects;
            }
            if path.is_empty() {
                continue;
            }
            scan.scanned_members += 1;
            let entry = members
                .entry(path.to_ascii_lowercase())
                .or_insert_with(|| (path.clone(), Vec::new()));
            // Projects are scanned in order, so a repeated path inside one
            // project never masquerades as another project's ownership.
            if entry.1.last().copied() != Some(index) {
                entry.1.push(index);
            }
        }
    }
    scan.member_collisions = members
        .into_values()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(path, owners)| MemberCollision { path, owners })
        .collect();
    scan.member_collisions.sort_by(|left, right| {
        left.owners
            .first()
            .cmp(&right.owners.first())
            .then_with(|| {
                left.path
                    .to_ascii_lowercase()
                    .cmp(&right.path.to_ascii_lowercase())
            })
    });
    for collision in &scan.member_collisions {
        for &index in &collision.owners {
            if let Some(count) = scan.shared_paths_by_project.get_mut(index) {
                *count += 1;
            }
        }
    }

    scan
}

pub(super) fn matching_projects(state: &State) -> Vec<usize> {
    let query = state.modding_filter.to_lowercase();
    let mut matches = state
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
        .filter(|&i| {
            let p = &state.modding_projects.as_deref().unwrap_or(&[])[i];
            state.modding_chips.iter().all(|chip| match chip.as_str() {
                "Modified only" => !p.modified.is_empty(),
                "Unmodified only" => p.modified.is_empty(),
                "Enabled only" => p.enabled,
                "Disabled only" => !p.enabled,
                _ => chip
                    .strip_prefix("PAK: ")
                    .is_none_or(|target| p.target == target),
            })
        })
        .collect::<Vec<_>>();
    sort_projects(state, &mut matches);
    matches
}

/// Apply the project list's header sort (display order only; the load
/// order number shown in each row is unchanged).
fn sort_projects(state: &State, indices: &mut [usize]) {
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    if let Some((column, ascending)) = state.modding_sort {
        indices.sort_by(|&a, &b| {
            let (a, b) = (&projects[a], &projects[b]);
            let order = match column {
                "pak" => a.target.to_lowercase().cmp(&b.target.to_lowercase()),
                "status" => a.status.cmp(&b.status),
                "enabled" => b.enabled.cmp(&a.enabled),
                _ => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
            };
            if ascending { order } else { order.reverse() }
        });
    }
}

/// Options for the project chip filter.
pub(super) fn modding_chip_options(projects: &[ProjectRow]) -> Vec<String> {
    let mut options = vec![
        "Modified only".to_owned(),
        "Unmodified only".to_owned(),
        "Enabled only".to_owned(),
        "Disabled only".to_owned(),
    ];
    let targets: std::collections::BTreeSet<_> = projects
        .iter()
        .map(|p| format!("PAK: {}", p.target))
        .collect();
    options.extend(targets);
    options
}

pub(super) fn page_range(count: usize, page: usize) -> std::ops::Range<usize> {
    let start = page.min(count.saturating_sub(1) / PAGE) * PAGE;
    start..(start + PAGE).min(count)
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(16)
        .style(design::panel)
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
            design::button("Open XML / Lua / TXT editor")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingOpenProject))),
            design::button("Open Level Editor").on_press(Message::MenuCommand(Box::new(
                Message::LevelOpenSelectedModProject
            ))),
            design::button("Review Changes").on_press(Message::MenuCommand(Box::new(
                Message::ModdingReviewChanges
            ))),
            design::button("Build Mod")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingBuild))),
            design::button("Export Mod…")
                .on_press(Message::MenuCommand(Box::new(Message::ModdingExportPak))),
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
            design::button("Audit installed original PAKs (read-only)")
                .on_press(Message::MenuCommand(Box::new(Message::StartLivePakAudit))),
            design::button("Play with mods (existing private helper)").on_press(
                Message::MenuCommand(Box::new(Message::StartOfflineLauncherLaunch))
            ),
            crate::ui_kit::hold_button(
                &state.ui,
                "restore-original-files-menu",
                "Hold: restore original PAKs + hook",
                Message::MenuCommand(Box::new(Message::StartOfflineLauncherRestore)),
                !state.busy,
                state.preferences.reduce_motion,
            ),
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

fn project_header(advanced: bool, sort: crate::ui_kit::Sort) -> Element<'static, Message> {
    let sortable = |label: &'static str, column: &'static str, width: Length| {
        crate::ui_kit::sort_header(
            label,
            column,
            sort,
            Message::SortBy("modding", column),
            width,
        )
    };
    let mut header = row![
        text("ORDER")
            .size(10)
            .style(design::muted)
            .width(Length::Fixed(48.0)),
        sortable("ENABLED", "enabled", Length::FillPortion(2)),
        sortable("MOD NAME / SOURCE", "name", Length::FillPortion(7)),
        sortable("PAK", "pak", Length::FillPortion(5)),
        text("DECLARED COLLISION")
            .size(10)
            .style(design::muted)
            .width(Length::Fixed(132.0)),
        sortable("MANIFEST", "status", Length::FillPortion(2)),
    ];
    if advanced {
        header = header
            .push(
                text("SOURCE BUILD")
                    .size(10)
                    .style(design::muted)
                    .width(Length::FillPortion(2)),
            )
            .push(
                text("MOD TYPE")
                    .size(10)
                    .style(design::muted)
                    .width(Length::FillPortion(2)),
            )
            .push(
                text("DETAILS")
                    .size(10)
                    .style(design::muted)
                    .width(Length::FillPortion(3)),
            );
    }
    header.spacing(8).align_y(Alignment::Center).into()
}

#[derive(Clone, Copy)]
enum BadgeTone {
    Clean,
    Shared,
    Target,
    Disabled,
}

/// Project badges use the shared theme-aware `ui_kit::badge`.
fn badge(label: impl Into<String>, tone: BadgeTone) -> Element<'static, Message> {
    use crate::ui_kit::BadgeKind;
    let kind = match tone {
        BadgeTone::Clean => BadgeKind::Enabled,
        BadgeTone::Shared => BadgeKind::Modified,
        BadgeTone::Target => BadgeKind::Conflict,
        BadgeTone::Disabled => BadgeKind::Neutral,
    };
    crate::ui_kit::badge(kind, label.into())
}

/// Lightweight, vector-sharp sliding switch. No images or theme-specific
/// assets are bundled; thumb and red track interpolate on the Iced Tick.
fn mod_toggle_switch(position: f32) -> Element<'static, Message> {
    // Same themed track/knob as `ui_kit::switch`; position eases via anim.
    iced::widget::canvas(crate::fx::SwitchKnob {
        position: position.clamp(0.0, 1.0),
    })
    .width(Length::Fixed(40.0))
    .height(Length::Fixed(22.0))
    .into()
}

fn projects<'a>(state: &'a State, scan: &LoadOrderScan) -> Element<'a, Message> {
    let mut rows = column![project_header(state.modding_advanced, state.modding_sort)].spacing(4);
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    let matches = matching_projects(state);
    let count = matches.len();
    let range = page_range(count, state.modding_page);
    for &index in &matches[range.clone()] {
        let record = &projects[index];
        let selected = state.project_selection.indices.contains(&index);
        let order = scan
            .position(index)
            .map(|position| format!("#{position}"))
            .unwrap_or_else(|| "—".into());
        let source = if record.source_build.trim().is_empty() {
            if record.details.trim().is_empty() {
                "Local project"
            } else {
                record.details.as_str()
            }
        } else {
            record.source_build.as_str()
        };
        let shared = scan.shared_paths(index);
        let collision = if !record.enabled {
            badge("Disabled", BadgeTone::Disabled)
        } else if scan.has_target_conflict(index) {
            badge("PAK target clash", BadgeTone::Target)
        } else if shared > 0 {
            badge(format!("{shared} shared file(s)"), BadgeTone::Shared)
        } else {
            badge("Clean", BadgeTone::Clean)
        };
        let mut cells = row![
            column![
                row![text(record.label.as_str()).size(12)]
                    .push((!record.modified.is_empty()).then(|| {
                        crate::ui_kit::badge(
                            crate::ui_kit::BadgeKind::Modified,
                            format!("{} modified", record.modified.len()),
                        )
                    }))
                    .spacing(6)
                    .align_y(Alignment::Center),
                text(source).size(11).style(design::muted),
            ]
            .width(Length::FillPortion(7)),
            text(record.target.as_str())
                .size(12)
                .width(Length::FillPortion(5)),
            container(collision)
                .width(Length::Fixed(132.0))
                .align_x(Alignment::Start),
            text(record.status.as_str())
                .size(11)
                .style(design::muted)
                .width(Length::FillPortion(2)),
        ];
        if state.modding_advanced {
            cells = cells
                .push(
                    text(record.source_build.as_str())
                        .size(12)
                        .width(Length::FillPortion(2)),
                )
                .push(
                    text(
                        classify(
                            record.modified.iter().map(String::as_str),
                            record.changed_known,
                        )
                        .label(),
                    )
                    .size(12)
                    .width(Length::FillPortion(2)),
                )
                .push(
                    text(record.details.as_str())
                        .size(12)
                        .width(Length::FillPortion(3)),
                );
        }
        let cells = cells.spacing(8).align_y(Alignment::Center);
        let position = state
            .mod_toggle_animations
            .iter()
            .find(|anim| anim.workspace == record.workspace)
            .map(|anim| anim.position(std::time::Instant::now()))
            .unwrap_or(if record.enabled { 1.0 } else { 0.0 });
        rows = rows.push(
            container(
                row![
                    text(order)
                        .size(12)
                        .style(design::muted)
                        .width(Length::Fixed(48.0)),
                    design::button(
                        row![
                            mod_toggle_switch(position),
                            text(if record.enabled { "On" } else { "Off" }).size(11),
                        ]
                        .spacing(3)
                        .align_y(Alignment::Center),
                    )
                    .style(design::text_button)
                    .padding([0, 3])
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
                .spacing(8)
                .align_y(Alignment::Center),
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
    if projects.is_empty() {
        // Empty states point straight at the fix instead of bare text.
        let state_view = if state.preferences.projects.trim().is_empty() {
            crate::ui_kit::empty_state(
                Some(super::Page::Files.icon()),
                "No projects folder yet",
                "Choose where mod projects are stored, then refresh this list.",
                Some((
                    "Open Settings › Storage",
                    Message::OpenSettingsTab(crate::settings_page::Tab::Storage),
                )),
            )
        } else {
            crate::ui_kit::empty_state(
                Some(super::Page::Files.icon()),
                "No mod projects",
                "The Asset Library stores originals, not editing projects. Start a project from gameplay values or a model.",
                Some((
                    "Create a mod…",
                    Message::GuidedEdit(crate::guided_modding::Action::Open(false)),
                )),
            )
        };
        return panel(state_view).into();
    }
    let empty: Element<'_, Message> = if count == 0 {
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
            scrollable(rows).height(Length::Fixed(300.0)),
            row![
                empty,
                space().width(Fill),
                design::button("Previous")
                    .on_press_maybe((range.start > 0).then_some(Message::ModdingPage(false))),
                design::button("Next")
                    .on_press_maybe((range.end < count).then_some(Message::ModdingPage(true))),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(8),
    )
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
        rows = rows.push(
            row![
                design::text_input(
                    "New relative path under project files/…",
                    &state.modding_added_member
                )
                .on_input(Message::ModdingAddedMemberChanged)
                .width(Fill),
                design::button("Register PAK addition")
                    .style(design::secondary)
                    .on_press(Message::ModdingRegisterAddedMember),
                design::button("Unregister")
                    .style(design::secondary)
                    .on_press(Message::ModdingUnregisterAddedMember),
            ]
            .spacing(8),
        );
        rows = rows.push(text("Opt-in: place the file under this project's files/ directory, then register its exact archive path. Unregister leaves your file untouched. Existing members remain strict replacements; unregistered files are ignored. Build Mod signs up to 64 combined changes.")
            .size(11).style(design::muted));
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
    panel(scrollable(rows).height(Length::Fixed(180.0))).into()
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
    if matching == 0 && state.library_catalog.is_some() {
        rows = rows.push(
            text("No imported PAKs match the current search.")
                .size(12)
                .style(design::muted),
        );
    }
    let description = if state.library_startup_loading {
        "Loading saved PAK manifests; you do not need to reimport."
    } else if state.library_catalog.is_none() {
        "No saved Asset Library loaded. Reload existing manifests, or use Browse staged PAKs to edit your prepared stage directly."
    } else {
        "Select a PAK to create an editable copy; originals stay unchanged."
    };
    panel(
        column![
            row![
                text("IMPORTED PAKS").size(13),
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
                design::button("Create Editable Project")
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

/// Registered signed PAKs are not editable Mod Projects. This list lets the
/// user opt them into the *same* Play with Mods pipeline without rebuilding.
fn signed_paks(state: &State) -> Element<'_, Message> {
    let mut entries = column![].spacing(6);
    if state.preferences.stage.trim().is_empty() {
        // Without a signing stage nothing here can work; go straight to setup.
        entries = entries.push(crate::ui_kit::empty_state(
            Some(crate::icons::view_iso()),
            "No signing stage",
            "Signed PAKs are verified and staged from a protected signing stage. Run game setup to create one.",
            Some((
                "Open Settings › Storage",
                Message::OpenSettingsTab(crate::settings_page::Tab::Storage),
            )),
        ));
    } else if let Some(ui) = &state.signed_ui {
        let mut count = 0;
        for profile in &ui.catalog.profiles {
            if profile.mods.is_empty() {
                continue;
            }
            entries = entries.push(text(profile.name.as_str()).size(13));
            for item in &profile.mods {
                count += 1;
                let choice = crate::mod_play::SignedChoice {
                    profile_id: profile.id.clone(),
                    mod_id: item.id.clone(),
                };
                let enabled = ui.selected.contains(&choice);
                let target = item.target.clone();
                entries = entries.push(
                    row![
                        design::button(if enabled { "On" } else { "Off" })
                            .style(if enabled {
                                design::primary
                            } else {
                                design::secondary
                            })
                            .on_press_maybe((!state.signed_busy && !state.busy).then_some(
                                Message::SignedPakToggle(
                                    profile.id.clone(),
                                    item.id.clone(),
                                    !enabled
                                )
                            )),
                        text(target).size(12).width(Fill),
                        crate::ui_kit::badge(crate::ui_kit::BadgeKind::Signed, "Signed"),
                        text(if enabled { "Selected" } else { "Not selected" })
                            .size(11)
                            .style(design::muted),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                );
            }
        }
        if count == 0 {
            entries = entries.push(crate::ui_kit::empty_state(
                None,
                "No signed PAKs yet",
                "Build a mod project or register a signed PAK to choose it for play.",
                Some(("Refresh", Message::SignedProfilesReload)),
            ));
        }
    } else {
        entries = entries.push(
            text("Refresh Mods to load registered signed PAKs.")
                .size(12)
                .style(design::muted),
        );
    }
    panel(
        column![
            row![
                text("SIGNED PAKS FOR PLAY").size(13),
                space().width(Fill),
                design::button("Refresh")
                    .style(design::secondary)
                    .on_press(Message::SignedProfilesReload),
            ]
            .spacing(8)
            .align_y(iced::Center),
            text(
                "Selected archives are verified and staged when you play; originals stay untouched."
            )
            .size(11)
            .style(design::muted),
            scrollable(entries).height(Length::Fixed(205.0)),
            text(state.signed_status.as_str())
                .size(11)
                .style(design::muted),
        ]
        .spacing(7),
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
                crate::ui_kit::popover_form(
                    &state.ui,
                    "new-profile",
                    "New profile…",
                    "New profile name…",
                    &state.profile_name,
                    Message::ProfileNameChanged,
                    (!state.profile_name.trim().is_empty()).then_some(Message::StartProfileCreate),
                ),
                space().width(Fill),
                design::button("Refresh").on_press(Message::StartProfileList)
            ]
            .spacing(8),
            scrollable(catalog).height(Length::Fixed(110.0)),
            profile_mod_order(state),
            text(format!(
                "Staged target: {}",
                if state.input.is_empty() {
                    "Select a PAK in the archive browser"
                } else {
                    &state.input
                }
            ))
                .size(12)
                .style(design::muted),
            design::text_input(
                "Signed PAK file to add to selected staged target…",
                &state.profile_source,
            )
            .on_input(Message::ProfileSourceChanged)
            .width(Fill),
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
            text("A signed PAK with added members can be registered here. Verify and apply it to the isolated stage, then use Tools → Play with mods (existing private helper). The main Play with Mods button builds enabled projects instead. The launcher still requires authenticated original game PAKs; never overwrite a live PAK to make this check pass.")
                .size(12)
                .style(design::muted),
            text(state.profile_feedback.as_str())
                .size(12)
                .style(design::muted)
        ]
        .spacing(7),
    )
    .into()
}

/// The selected profile's mods in apply order, reorderable by drag,
/// ↑/↓ buttons or Alt+↑/↓.
fn profile_mod_order(state: &State) -> Element<'_, Message> {
    let Some(profile) = state
        .profile_catalog
        .as_ref()
        .and_then(|catalog| catalog.profiles.iter().find(|p| p.id == state.profile_id))
    else {
        return space().height(0).into();
    };
    if profile.mods.is_empty() {
        return text("This profile has no mods yet.")
            .size(12)
            .style(design::muted)
            .into();
    }
    let rows = profile
        .mods
        .iter()
        .enumerate()
        .map(|(index, item)| {
            row![
                text(format!("{}.", index + 1))
                    .size(11)
                    .style(design::muted)
                    .width(Length::Fixed(24.0)),
                text(item.target.as_str()).size(12).width(Fill),
                if item.enabled {
                    crate::ui_kit::badge(crate::ui_kit::BadgeKind::Enabled, "Enabled")
                } else {
                    crate::ui_kit::badge(crate::ui_kit::BadgeKind::Neutral, "Off")
                },
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        })
        .collect();
    column![
        text(format!("APPLY ORDER · {}", profile.name)).size(11).style(design::muted),
        crate::ui_kit::reorder_list(&state.ui, "profile-mods", rows, Message::ProfileModMove),
        text("Mods apply top to bottom. Enabled mods never share a target PAK, so order does not change which file wins.")
            .size(11)
            .style(design::muted),
    ]
    .spacing(6)
    .into()
}

fn create_mod_card(state: &State) -> Element<'_, Message> {
    let card = panel(
        column![
            text("CREATE A MOD").size(10).style(design::muted),
            text("Author New Project").size(16),
            text("Create isolated workspace overrides for gameplay values or visual assets.")
                .size(11)
                .style(design::muted),
            space().height(Length::Fixed(6.0)),
            row![
                design::button("Gameplay values…")
                    .style(design::secondary)
                    .on_press(Message::GuidedEdit(crate::guided_modding::Action::Open(
                        false
                    ))),
                design::button("Models & Skins…")
                    .style(design::secondary)
                    .on_press(Message::GuidedEdit(crate::guided_modding::Action::Open(
                        true
                    ))),
            ]
            .spacing(8),
        ]
        .spacing(6),
    );
    crate::ui_kit::spotlight(
        &state.ui,
        "card:create".into(),
        card,
        state.preferences.reduce_motion,
    )
}

fn play_mods_card<'a>(state: &'a State, scan: &LoadOrderScan) -> Element<'a, Message> {
    let signed = state.signed_ui.as_ref().map_or(0, |ui| ui.selected.len());
    // Eases when mods are toggled; falls back to the scan before first sync.
    let enabled = crate::ui_kit::ticked(&state.ui, "enabled_mods", scan.enabled_count() as f64)
        .round() as usize;
    let working = super::footer_is_working(state);
    let card = panel(
        column![
            text("PLAY WITH MODS").size(10).style(design::accent),
            text("Stage 2 Sandbox Launch").size(16),
            text(format!(
                "Offline only · {enabled} enabled project(s) and {signed} selected signed PAK(s)."
            ))
            .size(11)
            .style(design::muted),
            space().height(Length::Fixed(6.0)),
            row![
                // A beam circles the button while a build or launch runs.
                crate::ui_kit::beam(
                    &state.ui,
                    design::button("Play with Mods")
                        .style(design::primary)
                        .on_press(Message::PlayWithMods),
                    working,
                    state.preferences.reduce_motion,
                ),
                design::button("Refresh Mods")
                    .style(design::secondary)
                    .on_press(Message::ModdingRefresh),
            ]
            .spacing(8),
        ]
        .spacing(6),
    );
    crate::ui_kit::spotlight(
        &state.ui,
        "card:play".into(),
        card,
        state.preferences.reduce_motion,
    )
}

fn conflict_hub_card<'a>(state: &'a State, scan: &LoadOrderScan) -> Element<'a, Message> {
    let first_conflict = scan
        .target_conflicts
        .first()
        .and_then(|conflict| conflict.owners.first())
        .or_else(|| {
            scan.member_collisions
                .first()
                .and_then(|collision| collision.owners.first())
        })
        .copied();
    let tone = if scan.conflict_count() == 0 {
        BadgeTone::Clean
    } else {
        BadgeTone::Shared
    };
    let card = panel(
        column![
            row![
                text("LOAD ORDER & CONFLICT HUB").size(10).style(design::muted),
                space().width(Fill),
                badge(format!("{} group(s)", scan.conflict_count()), tone),
            ]
            .align_y(Alignment::Center),
            text("Asset Collision Guard").size(16),
            text(format!(
                "{} enabled · {} shared-member group(s) · {} PAK target clash(es).",
                scan.enabled_count(),
                scan.member_collisions.len(),
                scan.target_conflicts.len()
            ))
            .size(11)
            .style(design::muted),
            text("Derived from project manifests; this is not a live game VFS read and no winning copy is written.")
                .size(11)
                .style(design::muted),
            space().height(Length::Fixed(6.0)),
            row![
                design::button("Review first conflict")
                    .style(design::secondary)
                    .on_press_maybe(first_conflict.map(Message::ModdingSelectProject)),
                design::button("Refresh Mods")
                    .style(design::secondary)
                    .on_press(Message::ModdingRefresh),
            ]
            .spacing(8),
        ]
        .spacing(6),
    );
    crate::ui_kit::spotlight(
        &state.ui,
        "card:conflicts".into(),
        card,
        state.preferences.reduce_motion,
    )
}

fn collision_owner<'a>(projects: &'a [ProjectRow], index: usize) -> Element<'a, Message> {
    let Some(record) = projects.get(index) else {
        return text("").into();
    };
    container(
        column![
            text(record.label.as_str()).size(12),
            text(record.target.as_str()).size(10).style(design::muted),
            row![
                design::button("Open")
                    .style(design::text_button)
                    .padding([2, 5])
                    .on_press(Message::ModdingSelectProject(index)),
                design::button("Disable")
                    .style(design::secondary)
                    .padding([2, 6])
                    .on_press(Message::ModdingToggleEnabled(index)),
            ]
            .spacing(4),
        ]
        .spacing(2),
    )
    .width(Fill)
    .padding(8)
    .style(design::panel)
    .into()
}

fn target_conflict_group<'a>(
    projects: &'a [ProjectRow],
    target: &str,
    owners: &[usize],
) -> Element<'a, Message> {
    let mut cards = row![].spacing(6);
    for &index in owners {
        cards = cards.push(collision_owner(projects, index));
    }
    container(
        column![
            row![
                text(target.to_owned()).size(11),
                space().width(Fill),
                badge("PAK target clash", BadgeTone::Target),
            ]
            .align_y(Alignment::Center),
            cards,
            text("Play with Mods refuses two enabled sources with the same target PAK. Disable one source.")
                .size(10)
                .style(design::muted),
        ]
        .spacing(6),
    )
    .padding(10)
    .style(design::panel)
    .into()
}

fn member_collision_group<'a>(
    projects: &'a [ProjectRow],
    path: &str,
    owners: &[usize],
) -> Element<'a, Message> {
    let mut cards = row![].spacing(6);
    for &index in owners {
        cards = cards.push(collision_owner(projects, index));
    }
    container(
        column![
            row![
                text(path.to_owned()).size(11),
                space().width(Fill),
                badge(format!("{} sources", owners.len()), BadgeTone::Shared),
            ]
            .align_y(Alignment::Center),
            cards,
            text("Same declared changed-member path. The manager cannot select or apply a VFS winner yet; disable a source or consolidate the file.")
                .size(10)
                .style(design::muted),
        ]
        .spacing(6),
    )
    .padding(10)
    .style(design::panel)
    .into()
}

fn conflict_inspector<'a>(state: &'a State, scan: &LoadOrderScan) -> Element<'a, Message> {
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    let mut body = column![].spacing(8);
    if scan.enabled_count() == 0 {
        body = body.push(
            text("No enabled projects. Enable a project to add it to the effective Play order.")
                .size(12)
                .style(design::muted),
        );
    }
    if scan.incomplete_enabled > 0 {
        body = body.push(badge(
            format!(
                "{} enabled project(s) have incomplete change scans",
                scan.incomplete_enabled
            ),
            BadgeTone::Target,
        ));
    }
    if scan.truncated {
        body = body.push(badge(
            format!("Scan stopped after {MAX_SCAN_MEMBERS} changed-member entries",),
            BadgeTone::Target,
        ));
    }
    if scan.conflict_count() == 0 && scan.enabled_count() > 0 {
        body = body.push(
            text("No declared shared changed-member paths or duplicate PAK targets among enabled projects.")
                .size(12)
                .style(design::muted),
        );
    }
    let mut shown = 0usize;
    for conflict in &scan.target_conflicts {
        if shown >= MAX_INSPECTOR_GROUPS {
            break;
        }
        body = body.push(target_conflict_group(
            projects,
            conflict.target.as_str(),
            &conflict.owners,
        ));
        shown += 1;
    }
    for collision in &scan.member_collisions {
        if shown >= MAX_INSPECTOR_GROUPS {
            break;
        }
        body = body.push(member_collision_group(
            projects,
            collision.path.as_str(),
            &collision.owners,
        ));
        shown += 1;
    }
    if scan.conflict_count() > shown {
        body = body.push(
            text(format!(
                "{} more collision group(s) are not shown; use the project filter or disable a source to reduce the scan.",
                scan.conflict_count() - shown
            ))
            .size(10)
            .style(design::muted),
        );
    }
    let tone = if scan.conflict_count() == 0 {
        BadgeTone::Clean
    } else {
        BadgeTone::Shared
    };
    panel(
        column![
            row![
                text("COLLISION INSPECTOR").size(13),
                space().width(Fill),
                badge(format!("{} group(s)", scan.conflict_count()), tone),
            ]
            .align_y(Alignment::Center),
            text("Effective Play order is this project-list order: Play with Mods walks the registry and keeps enabled projects. Disabled projects are not loaded.")
                .size(11)
                .style(design::muted),
            scrollable(body).height(Length::Fixed(300.0)),
            text(format!(
                "Scanned {} declared changed-member entries across enabled projects.",
                scan.scanned_members
            ))
            .size(10)
            .style(design::muted),
            text("Shared paths are declared in each project manifest. Manual priority, CryEngine PAK mount order and a winning copy are not controlled or verified here.")
                .size(10)
                .style(design::muted),
            row![
                design::button("Review Changes")
                    .style(design::secondary)
                    .on_press(Message::ModdingReviewChanges),
                design::button("Refresh Mods")
                    .style(design::secondary)
                    .on_press(Message::ModdingRefresh),
            ]
            .spacing(8),
        ]
        .spacing(7),
    )
    .into()
}

fn mod_kind_count(projects: &[ProjectRow], kind: ModKind) -> usize {
    projects
        .iter()
        .filter(|project| {
            classify(
                project.modified.iter().map(String::as_str),
                project.changed_known,
            ) == kind
        })
        .count()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let chosen = state.modding_selected.and_then(|index| {
        state
            .modding_projects
            .as_ref()
            .and_then(|list| list.get(index))
    });
    if state.guided_editor.open {
        return column![text("Modding").size(22), crate::guided_modding::view(state)]
            .spacing(10)
            .into();
    }

    let selected_actions = row![
        design::button("Continue Editing")
            .style(design::secondary)
            .on_press(Message::ModdingOpenProject),
        design::button("Edit Level")
            .style(design::secondary)
            .on_press_maybe(
                state
                    .modding_selected
                    .map(|_| Message::LevelOpenSelectedModProject)
            ),
    ]
    .spacing(7)
    .align_y(iced::Center);
    let mut selected_content = column![
        row![
            text("SELECTED PROJECT").size(13),
            space().width(Fill),
            text(
                chosen
                    .map(|p| p.target.as_str())
                    .unwrap_or("Select a project above")
            )
            .size(12)
            .style(design::muted),
        ]
        .spacing(8)
        .align_y(iced::Center),
        row![crate::ui_kit::popover_form(
            &state.ui,
            "rename-mod",
            "Rename mod…",
            "Mod name…",
            &state.modding_name_input,
            Message::ModdingNameChanged,
            state.modding_selected.map(|_| Message::ModdingSaveName),
        ),]
        .spacing(8)
        .align_y(iced::Center),
        selected_actions,
        row![
            design::button("Export Mod…")
                .style(design::primary)
                .on_press_maybe(state.modding_selected.map(|_| Message::ModdingExportPak)),
            text("Choose an export format, then Save As to any folder.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(8)
        .align_y(iced::Center),
    ]
    .spacing(10);
    if state.modding_export_options_open && chosen.is_some() {
        selected_content = selected_content.push(
            column![
                text("EXPORT FORMAT").size(12).style(design::muted),
                design::button("zz_[modname].pak — changed files only")
                    .style(design::secondary)
                    .on_press(Message::ModdingExportChoose(PakExportMode::ChangedZz)),
                design::button("[OriginalPakName].pak — changed files only")
                    .style(design::secondary)
                    .on_press(Message::ModdingExportChoose(PakExportMode::ChangedOriginal)),
                design::button("[OriginalPakName].pak — entire original PAK")
                    .style(design::secondary)
                    .on_press(Message::ModdingExportChoose(PakExportMode::FullOriginal)),
            ]
            .spacing(5),
        );
    }
    if state.modding_advanced {
        selected_content = selected_content.push(
            row![
                design::button("Advanced Profiles…")
                    .style(design::secondary)
                    .on_press(Message::ModdingToggleProfiles),
                design::button("Build Selected Mod")
                    .style(design::secondary)
                    .on_press(Message::ModdingBuild),
                design::button("Prepare Selected Mod for Test")
                    .style(design::secondary)
                    .on_press(Message::ModdingBuildForOfflineTest),
            ]
            .spacing(8),
        );
    }
    let project_list = state.modding_projects.as_deref().unwrap_or(&[]);
    let scan = scan_projects(project_list);
    let all_count = project_list.len();
    let cosmetic_count = mod_kind_count(project_list, ModKind::CosmeticCandidate);
    let gameplay_count = mod_kind_count(project_list, ModKind::Gameplay);
    let review_count = mod_kind_count(project_list, ModKind::NeedsReview);
    let selected_project = panel(selected_content);
    let top_cards = row![
        container(create_mod_card(state)).width(Length::FillPortion(1)),
        container(play_mods_card(state, &scan)).width(Length::FillPortion(1)),
        container(conflict_hub_card(state, &scan)).width(Length::FillPortion(1)),
    ]
    .spacing(12);
    let filter_bar = panel(
        column![
            row![
                text("YOUR PROJECTS").size(15),
                space().width(Fill),
                text(format!("{} matching", matching_projects(state).len()))
                    .size(11)
                    .style(design::muted),
                badge(
                    format!("{} enabled", scan.enabled_count()),
                    if scan.enabled_count() == 0 {
                        BadgeTone::Disabled
                    } else {
                        BadgeTone::Clean
                    },
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                design::text_input(
                    "Find project, target PAK or imported source…",
                    &state.modding_filter
                )
                .on_input(Message::ModdingFilterChanged)
                .width(Fill),
                crate::ui_kit::chips(
                    &state.modding_chips,
                    &state.modding_chip_options,
                    &state.modding_chip_query,
                    Message::ModdingChipPicked,
                    Message::ModdingChipInput,
                    Message::ModdingChipRemove,
                ),
                design::button(text(format!("All mods ({all_count})")).size(11))
                    .style(if state.modding_kind.is_none() {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press(Message::ModdingKind(None)),
                design::button(text(format!("Cosmetic ({cosmetic_count})")).size(11))
                    .style(if state.modding_kind == Some(ModKind::CosmeticCandidate) {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press(Message::ModdingKind(Some(ModKind::CosmeticCandidate))),
                design::button(text(format!("Gameplay ({gameplay_count})")).size(11))
                    .style(if state.modding_kind == Some(ModKind::Gameplay) {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press(Message::ModdingKind(Some(ModKind::Gameplay))),
                design::button(text(format!("Needs review ({review_count})")).size(11))
                    .style(if state.modding_kind == Some(ModKind::NeedsReview) {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press(Message::ModdingKind(Some(ModKind::NeedsReview))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(8),
    );
    let order_split = row![
        container(projects(state, &scan)).width(Length::FillPortion(3)),
        container(conflict_inspector(state, &scan)).width(Length::FillPortion(2)),
    ]
    .spacing(12)
    .align_y(Alignment::Start);
    let mut main = column![
        text("Modding").size(24),
        menu(state),
        top_cards,
        filter_bar,
        order_split,
        selected_project,
        modifications(state),
        text("Cosmetic candidates require content verification before online use. Gameplay, mixed and unverified mods stay offline.")
            .size(11)
            .style(design::muted),
        text("OTHER MOD SOURCES").size(15),
        row![
            imported_paks(state),
            signed_paks(state),
        ]
        .spacing(12),
        panel(column![
            text("RECOVERY").size(13),
            row![
                crate::ui_kit::hold_button(
                    &state.ui,
                    "restore-original-files",
                    "Hold to Restore Original Files",
                    Message::StartOfflineLauncherRestore,
                    !state.busy,
                    state.preferences.reduce_motion,
                ),
                design::button("Restore staged profile")
                    .style(design::secondary)
                    .on_press(Message::StartProfileRecover),
            ]
            .spacing(8),
        ]
        .spacing(9)),
        row![
            design::button(if state.modding_advanced {
                "Hide Advanced Archive Tools"
            } else {
                "Show Advanced Archive Tools"
            })
            .style(design::secondary)
            .on_press(Message::ModdingToggleAdvanced),
        ]
        .spacing(8),
    ]
    .spacing(12);
    if state.modding_advanced {
        main = main.push(archive_browser(state));
    }
    if state.modding_profiles {
        main = main.push(profiles(state));
    }
    if let Some(report) = &state.live_pak_audit {
        let mut lines = column![text(format!(
            "INSTALLED ORIGINAL PAK AUDIT — {} verified, {} failed, {} verified but different from baseline (no files changed)",
            report.verified, report.failed, report.baseline_differences
        )).size(12)];
        for issue in &report.issues {
            lines = lines.push(text(issue.as_str()).size(11));
        }
        main = main.push(scrollable(lines.spacing(4)).height(Length::Fixed(145.0)));
    }
    crate::menu_popup::overlay(
        main,
        dropdown(state),
        dropdown_left(state.modding_menu),
        115.0,
        310.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project(
        label: &str,
        target: &str,
        enabled: bool,
        changed_known: bool,
        modified: &[&str],
    ) -> ProjectRow {
        ProjectRow {
            label: label.into(),
            target: target.into(),
            source_build: "Test build".into(),
            status: "Modified".into(),
            details: String::new(),
            workspace: PathBuf::from(label),
            modified: modified.iter().map(|path| (*path).to_owned()).collect(),
            changed_known,
            enabled,
        }
    }

    #[test]
    fn load_order_is_the_enabled_project_list_order() {
        let projects = vec![
            project("Beta", "b.pak", true, true, &["textures/b.dds"]),
            project("Alpha", "a.pak", false, true, &[]),
            project("Gamma", "c.pak", true, true, &[]),
        ];
        let scan = scan_projects(&projects);
        assert_eq!(scan.order, vec![0, 2]);
        assert_eq!(scan.position(0), Some(1));
        assert_eq!(scan.position(1), None);
        assert_eq!(scan.position(2), Some(2));
        assert_eq!(scan.enabled_count(), 2);
    }

    #[test]
    fn shared_members_are_case_insensitive_and_ignore_disabled_projects() {
        let projects = vec![
            project(
                "A",
                "a.pak",
                true,
                true,
                &["Textures/A.dds", "textures/b.dds"],
            ),
            project("B", "b.pak", true, true, &["textures/a.dds"]),
            project("C", "c.pak", false, true, &["textures/b.dds"]),
        ];
        let scan = scan_projects(&projects);
        assert_eq!(
            scan.member_collisions,
            vec![MemberCollision {
                path: "Textures/A.dds".into(),
                owners: vec![0, 1],
            }]
        );
        assert_eq!(scan.shared_paths(0), 1);
        assert_eq!(scan.shared_paths(1), 1);
        assert_eq!(scan.shared_paths(2), 0);
    }

    #[test]
    fn duplicate_paths_inside_one_project_count_one_owner() {
        let projects = vec![
            project("A", "a.pak", true, true, &["same.dds", "SAME.dds"]),
            project("B", "b.pak", true, true, &["same.dds"]),
        ];
        let scan = scan_projects(&projects);
        assert_eq!(
            scan.member_collisions,
            vec![MemberCollision {
                path: "same.dds".into(),
                owners: vec![0, 1],
            }]
        );
        assert_eq!(scan.shared_paths(0), 1);
    }

    #[test]
    fn duplicate_target_paks_are_reported_only_for_enabled_sources() {
        let projects = vec![
            project("A", "Textures.pak", true, true, &[]),
            project("B", "textures.PAK", true, true, &[]),
            project("C", "TEXTURES.pak", false, true, &[]),
        ];
        let scan = scan_projects(&projects);
        assert_eq!(
            scan.target_conflicts,
            vec![TargetConflict {
                target: "Textures.pak".into(),
                owners: vec![0, 1],
            }]
        );
        assert!(scan.has_target_conflict(0));
        assert!(!scan.has_target_conflict(2));
    }

    #[test]
    fn incomplete_enabled_scans_are_flagged() {
        let projects = vec![
            project("A", "a.pak", true, false, &["objects/a.dds"]),
            project("B", "b.pak", false, false, &[]),
        ];
        let scan = scan_projects(&projects);
        assert_eq!(scan.incomplete_enabled, 1);
        assert_eq!(scan.enabled_count(), 1);
    }

    #[test]
    fn empty_projects_have_no_conflicts_or_positions() {
        let scan = scan_projects(&[]);
        assert!(scan.order.is_empty());
        assert!(scan.positions.is_empty());
        assert_eq!(scan.conflict_count(), 0);
        assert_eq!(scan.position(0), None);
    }
}
