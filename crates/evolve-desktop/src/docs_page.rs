//! Docs as two panes: a contents rail (Getting Started / How To / Help)
//! with search on the left, the selected section in full on the right.
//! Getting Started is a live checklist driven by the app's own state.

use crate::forms::PathKind;
use crate::private_pages::{DocsSection, docs_spec};
use crate::settings_page::Tab;
use crate::{Message, Page, State, design, icons};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Element, Fill, Length};

/// The rail's groups, in reading order.
pub const GROUPS: &[(&str, &[DocsSection])] = &[
    (
        "Getting Started",
        &[
            DocsSection::GettingStarted,
            DocsSection::Install,
            DocsSection::Paths,
        ],
    ),
    (
        "How To",
        &[
            DocsSection::Import,
            DocsSection::Browse,
            DocsSection::Projects,
            DocsSection::Edit,
            DocsSection::Gfx,
            DocsSection::Export,
            DocsSection::LaunchMods,
            DocsSection::LaunchClean,
            DocsSection::Updates,
        ],
    ),
    (
        "Help",
        &[
            DocsSection::Recovery,
            DocsSection::Troubleshoot,
            DocsSection::Faq,
        ],
    ),
];

/// The section after `section` in rail order.
pub fn next_section(section: DocsSection) -> Option<DocsSection> {
    let order: Vec<DocsSection> = GROUPS
        .iter()
        .flat_map(|(_, items)| items.iter().copied())
        .collect();
    let at = order.iter().position(|item| *item == section)?;
    order.get(at + 1).copied()
}

pub fn scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("docs-body")
}

/// One search hit: a section, and the matching step if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub section: DocsSection,
    pub step: Option<usize>,
    pub title: &'static str,
    pub snippet: &'static str,
}

/// Titles, summaries and step text that contain `query`.
pub fn search(query: &str) -> Vec<Hit> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let has = |text: &str| text.to_ascii_lowercase().contains(&needle);
    let mut hits = Vec::new();
    for section in &DocsSection::ALL {
        {
            let (title, summary, steps) = docs_spec(*section);
            if has(title) || has(summary) {
                hits.push(Hit {
                    section: *section,
                    step: None,
                    title,
                    snippet: summary,
                });
            }
            for (index, (step, details)) in steps.iter().enumerate() {
                if has(step) || has(details) {
                    hits.push(Hit {
                        section: *section,
                        step: Some(index),
                        title: step,
                        snippet: details,
                    });
                }
            }
        }
    }
    hits
}

/// Where a section's "Open …" button goes: the exact Settings tab, not
/// just the Settings page.
fn destination(section: DocsSection) -> Option<(&'static str, Message)> {
    Some(match section {
        DocsSection::Paths | DocsSection::GettingStarted => (
            "Open Settings › Storage (paths)",
            Message::OpenSettingsTab(Tab::Storage),
        ),
        DocsSection::Install | DocsSection::Updates => (
            "Open Settings › Updates",
            Message::OpenSettingsTab(Tab::Updates),
        ),
        DocsSection::Import | DocsSection::Browse | DocsSection::Export => {
            ("Open Asset Library", Message::Navigate(Page::AssetLibrary))
        }
        DocsSection::Projects | DocsSection::Edit | DocsSection::Gfx => {
            ("Open Modding", Message::Navigate(Page::Files))
        }
        DocsSection::LaunchMods | DocsSection::LaunchClean | DocsSection::Recovery => {
            ("Go to Home", Message::Navigate(Page::Home))
        }
        DocsSection::Troubleshoot => (
            "Open System Health",
            Message::OpenSettingsTab(Tab::SystemHealth),
        ),
        DocsSection::Faq => return None,
    })
}

/// Safety notes read as warnings rather than ordinary steps.
fn is_safety_note(details: &str) -> bool {
    let lower = details.to_ascii_lowercase();
    ["do not ", "don't ", "never ", "avoid "]
        .iter()
        .any(|lead| lower.starts_with(lead) || lower.contains(&format!(". {lead}")))
}

/// A Getting Started step that ticks itself from state.
pub struct CheckStep {
    pub label: &'static str,
    pub done: bool,
    pub action: (&'static str, Message),
}

pub fn checklist(state: &State) -> Vec<CheckStep> {
    let path_ok = |kind| state.path_checks.get(kind).is_some_and(|check| check.ok);
    let stage = state.preferences.stage.trim();
    let projects = state.modding_projects.as_deref().unwrap_or(&[]);
    vec![
        CheckStep {
            label: "Game folder found",
            done: path_ok(PathKind::GameInstall),
            action: ("Set path", Message::OpenSettingsTab(Tab::Storage)),
        },
        CheckStep {
            label: "Client executable found",
            done: path_ok(PathKind::ClientExe),
            action: ("Set path", Message::OpenSettingsTab(Tab::Storage)),
        },
        CheckStep {
            label: "Offline setup created",
            done: !stage.is_empty() && std::path::Path::new(stage).is_dir(),
            action: ("Create setup", Message::OpenSettingsTab(Tab::Storage)),
        },
        CheckStep {
            label: "Asset Library imported",
            done: state
                .library_catalog
                .as_ref()
                .is_some_and(|catalog| !catalog.archives.is_empty()),
            action: ("Import PAKs", Message::Navigate(Page::AssetLibrary)),
        },
        CheckStep {
            label: "A Modding project exists",
            done: !projects.is_empty(),
            action: ("Open Modding", Message::Navigate(Page::Files)),
        },
        CheckStep {
            label: "A mod is enabled",
            done: projects.iter().any(|project| project.enabled),
            action: ("Enable a mod", Message::Navigate(Page::Files)),
        },
        CheckStep {
            label: "Played with mods once",
            done: state.preferences.played_with_mods,
            action: ("Play with mods", Message::PlayWithMods),
        },
    ]
}

/// Plain text for a bug report: versions, paths and their checks, the last
/// error and the System Health summary.
pub fn diagnostics_text(state: &State) -> String {
    let mut lines = vec![
        format!("Evolve Mod Manager {}", env!("CARGO_PKG_VERSION")),
        format!(
            "Client version: {}",
            state.client_version.as_deref().unwrap_or("unknown")
        ),
        format!("Launch mode: {}", state.preferences.game_launch_mode),
        String::from("Paths:"),
    ];
    for kind in PathKind::ALL {
        let value = kind.value(&state.preferences);
        let check = state.path_checks.get(kind).map_or_else(
            || "not set".to_owned(),
            |check| {
                format!(
                    "{} {}",
                    if check.ok { "OK" } else { "PROBLEM" },
                    check.message
                )
            },
        );
        lines.push(format!("  {kind:?}: {value} [{check}]"));
    }
    if let Some(error) = &state.update_worker_failure {
        lines.push(format!("Last update error: {error}"));
    }
    if !state.status.is_empty() {
        lines.push(format!("Last status: {}", state.status));
    }
    match &state.health_report {
        Some(report) => {
            lines.push("System Health:".into());
            lines.push(format!("  Firewall: {}", report.firewall));
            lines.push(format!("  Game process: {}", report.game_process));
            lines.push(format!("  Storage: {}", report.storage));
            lines.push(format!("  Environment: {}", report.environment));
            lines.push(format!("  Recovery: {}", report.recovery));
            for meter in &state.health_meters {
                lines.push(format!("  {}: {}", meter.label, meter.detail));
            }
        }
        None => lines.push("System Health: not run this session".into()),
    }
    lines.join("\n")
}

fn rail(state: &State) -> Element<'_, Message> {
    let mut rail = column![crate::forms::search_field(
        "Search the docs…",
        &state.docs_search,
        (!state.docs_search.trim().is_empty()).then(|| search(&state.docs_search).len()),
        Message::DocsSearch,
    )]
    .spacing(10)
    .width(Length::Fixed(250.0));
    if !state.docs_search.trim().is_empty() {
        let accent = crate::forms::highlight_color(&state.preferences);
        let hits = search(&state.docs_search);
        if hits.is_empty() {
            rail = rail.push(design::caption("No matches."));
        }
        for (index, hit) in hits.into_iter().take(30).enumerate() {
            rail = rail.push(
                design::button(
                    column![
                        crate::forms::highlight(
                            hit.title,
                            &state.docs_search,
                            design::LABEL,
                            accent
                        ),
                        text(docs_spec(hit.section).0)
                            .size(design::CAPTION)
                            .style(design::muted),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding([5, 8])
                .style(design::text_button)
                .on_press(Message::DocsOpenResult(index)),
            );
        }
        rail = rail.push(design::caption("Enter opens the first result."));
        return scrollable(rail).height(Fill).into();
    }
    for (group, sections) in GROUPS {
        rail = rail.push(text(*group).size(design::CAPTION).style(design::accent));
        for section in *sections {
            let selected = state.docs_selected == *section;
            rail = rail.push(
                design::button(text(docs_spec(*section).0).size(design::LABEL))
                    .width(Fill)
                    .padding([6, 10])
                    .style(move |theme, status| design::rail(theme, status, selected))
                    .on_press(Message::DocsSelect(*section)),
            );
        }
    }
    scrollable(rail).height(Fill).into()
}

fn checklist_view(state: &State) -> Element<'_, Message> {
    let steps = checklist(state);
    let done = steps.iter().filter(|step| step.done).count();
    let mut list = column![
        row![
            design::heading("Your setup"),
            space().width(Fill),
            text(format!("{done} of {}", steps.len())).size(design::LABEL),
        ]
        .align_y(iced::Center),
        iced::widget::progress_bar(0.0..=steps.len() as f32, done as f32).girth(6),
    ]
    .spacing(8);
    for step in steps {
        let done = step.done;
        list = list.push(
            row![
                icons::glyph(if done { icons::check() } else { icons::info() }, 16.0).style(
                    move |theme: &iced::Theme, _| iced::widget::svg::Style {
                        color: Some(if done {
                            theme.palette().success
                        } else {
                            design::with_alpha(theme.palette().text, 0.5)
                        }),
                    }
                ),
                text(step.label).size(design::BODY).width(Fill),
            ]
            .push((!done).then(|| {
                design::button(text(step.action.0).size(design::LABEL))
                    .padding([4, 10])
                    .style(design::secondary)
                    .on_press(step.action.1)
            }))
            .spacing(10)
            .align_y(iced::Center),
        );
    }
    container(list).padding(14).style(design::panel).into()
}

fn section_view(state: &State, section: DocsSection) -> Element<'_, Message> {
    let (title, summary, steps) = docs_spec(section);
    let mut body = column![
        design::title(title),
        design::body(summary).style(design::muted)
    ]
    .spacing(14);
    if section == DocsSection::GettingStarted {
        body = body.push(checklist_view(state));
    }
    for (index, (step, details)) in steps.iter().enumerate() {
        body = body.push(if is_safety_note(details) {
            column![
                text(format!("{}  {}", index + 1, step)).size(14),
                crate::ui_kit::callout(design::Tone::Warning, *details),
            ]
            .spacing(5)
        } else {
            column![
                text(format!("{}  {}", index + 1, step)).size(14),
                text(*details).size(design::LABEL).style(design::muted),
            ]
            .spacing(4)
        });
    }
    if section == DocsSection::Troubleshoot {
        body = body.push(
            container(
                row![
                    column![
                        design::heading("Copy diagnostics"),
                        design::caption(
                            "Copies versions, configured paths and their checks, the last error and the System Health summary as plain text for a bug report."
                        ),
                    ]
                    .spacing(4)
                    .width(Fill),
                    crate::ui_kit::copy_button(&state.ui, "docs-diagnostics", diagnostics_text(state)),
                    design::button("System Health")
                        .style(design::secondary)
                        .on_press(Message::OpenSettingsTab(Tab::SystemHealth)),
                ]
                .spacing(10)
                .align_y(iced::Center),
            )
            .padding(14)
            .style(design::panel),
        );
    }
    let mut footer = row![].spacing(10).align_y(iced::Center);
    if let Some((label, message)) = destination(section) {
        footer = footer.push(
            design::button(label)
                .style(design::secondary)
                .on_press(message),
        );
    }
    footer = footer.push(space().width(Fill));
    if let Some(next) = next_section(section) {
        footer = footer.push(
            design::button(text(format!("Next: {} →", docs_spec(next).0)).size(design::LABEL))
                .style(design::primary)
                .on_press(Message::DocsSelect(next)),
        );
    }
    body = body.push(footer);
    scrollable(container(body).padding([4, 18]).max_width(860))
        .id(scroll_id())
        .height(Fill)
        .into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    column![
        design::title("Docs"),
        row![
            container(rail(state))
                .padding(12)
                .height(Fill)
                .style(design::panel),
            section_view(state, state.docs_selected),
        ]
        .spacing(16)
        .height(Fill),
    ]
    .spacing(12)
    .padding(12)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_is_in_the_rail_once() {
        let listed: Vec<DocsSection> = GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        assert_eq!(listed.len(), DocsSection::ALL.len());
        for section in DocsSection::ALL {
            assert_eq!(listed.iter().filter(|s| **s == section).count(), 1);
        }
    }

    #[test]
    fn next_follows_rail_order_and_ends() {
        assert_eq!(
            next_section(DocsSection::GettingStarted),
            Some(DocsSection::Install)
        );
        assert_eq!(next_section(DocsSection::Faq), None);
    }

    #[test]
    fn search_finds_steps_case_insensitively() {
        let hits = search("MODDEDEVOLVELAUNCHER");
        assert!(hits.iter().any(|hit| hit.step.is_some()));
        assert!(search("   ").is_empty());
    }

    #[test]
    fn safety_notes_are_detected() {
        assert!(is_safety_note("Do not delete your data."));
        assert!(is_safety_note("Keep it. Never move files."));
        assert!(!is_safety_note("Open Settings."));
    }

    #[test]
    fn checklist_starts_incomplete_and_diagnostics_list_paths() {
        let state = State::default();
        let steps = checklist(&state);
        assert_eq!(steps.len(), 7);
        assert!(steps.iter().all(|step| !step.done));
        let text = diagnostics_text(&state);
        assert!(text.contains("GameInstall"));
        assert!(text.contains("System Health: not run"));
    }
}
