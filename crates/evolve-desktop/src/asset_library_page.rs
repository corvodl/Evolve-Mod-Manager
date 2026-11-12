//! Private-manager-style imported PAK browser. Original archives and
//! extracted Asset Library files remain read-only; mod editing lives elsewhere.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{column, container, mouse_area, row, scrollable, space, stack, text};
use iced::{Background, Border, Element, Fill, Length};
use std::collections::BTreeMap;

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
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

fn sort_cell(
    label: &'static str,
    column: &'static str,
    width: u16,
    state: &State,
) -> Element<'static, Message> {
    crate::ui_kit::sort_header(
        label,
        column,
        state.library_sort,
        Message::SortBy("library", column),
        Length::FillPortion(width),
    )
}

fn header_cell(label: &'static str, width: u16) -> Element<'static, Message> {
    text(label)
        .size(design::BODY)
        .width(Length::FillPortion(width))
        .into()
}

// Human-scale labels in the main table; original paths are always available
// under Details. The source location logic lives in `level_archive` so the
// Level Editor's map chooser can label archives the same way.
fn archive_title(archive: &evolve_core::ImportedPak) -> String {
    crate::level_archive::display_title(archive)
}

fn archive_build_label(archive: &evolve_core::ImportedPak) -> String {
    match archive.builds.as_slice() {
        [] => "Not assigned".to_owned(),
        [name] => name.clone(),
        many => format!("{} builds", many.len()),
    }
}

// The source filename/path identifies one *display* group only, NOT equivalent
// bytes. Entries with different IDs or hashes remain individually selectable
// after expansion, even when the original filename is identical.
fn source_group_key(archive: &evolve_core::ImportedPak) -> String {
    format!(
        "{}|{}",
        archive.name.to_ascii_lowercase(),
        archive.source.replace('\\', "/").to_ascii_lowercase()
    )
}

#[derive(Debug)]
enum BrowserRow {
    Category {
        key: String,
        name: String,
        count: usize,
    },
    SourceGroup {
        key: String,
        name: String,
        source: String,
        count: usize,
    },
    /// An archive row and its nesting depth in the tree.
    Archive(usize, usize),
}

fn browser_rows(state: &State) -> Vec<BrowserRow> {
    let Some(catalog) = state.library_catalog.as_ref() else {
        return Vec::new();
    };
    if state.library_flat_list {
        return state
            .library_matches
            .iter()
            .copied()
            .map(|index| BrowserRow::Archive(index, 0))
            .collect();
    }
    // Deterministic grouping from the CURRENT filtered match set; do not
    // mutate catalog order, identities, build memberships or selected indices.
    let mut groups = BTreeMap::<String, (String, BTreeMap<String, Vec<usize>>)>::new();
    for &index in &state.library_matches {
        let Some(archive) = catalog.archives.get(index) else {
            continue;
        };
        let category = archive.group.to_ascii_lowercase();
        groups
            .entry(category)
            .or_insert_with(|| (archive.group.clone(), BTreeMap::new()))
            .1
            .entry(source_group_key(archive))
            .or_default()
            .push(index);
    }
    let mut result = Vec::new();
    for (key, (name, sources)) in groups {
        let count: usize = sources.values().map(Vec::len).sum();
        let expanded = !state.library_search.trim().is_empty()
            || state.library_expanded_categories.contains(&key);
        result.push(BrowserRow::Category { key, name, count });
        if !expanded {
            continue;
        }
        for (source_key, members) in sources {
            if members.len() == 1 {
                result.push(BrowserRow::Archive(members[0], 1));
                continue;
            }
            let first = &catalog.archives[members[0]];
            let label = archive_title(first);
            let source = crate::level_archive::source_label(&first.source);
            let expanded = !state.library_search.trim().is_empty()
                || state.library_expanded_sources.contains(&source_key);
            result.push(BrowserRow::SourceGroup {
                key: source_key,
                name: label,
                source,
                count: members.len(),
            });
            if expanded {
                result.extend(
                    members
                        .into_iter()
                        .map(|index| BrowserRow::Archive(index, 2)),
                );
            }
        }
    }
    result
}

/// Shift selection follows only the visible selectable archives, including
/// across table pages. A collapsed category must never invisibly select PAKs.
pub(super) fn visible_archive_indices(state: &State) -> Vec<usize> {
    browser_rows(state)
        .into_iter()
        .filter_map(|row| match row {
            BrowserRow::Archive(index, _) => Some(index),
            _ => None,
        })
        .collect()
}

/// Rows per page the user can pick in the footer.
pub(super) const PAGE_SIZES: [usize; 3] = [50, 110, 250];
const BUILDS_POPOVER: &str = "library-builds";
const IMPORT_POPOVER: &str = "library-import";

fn page_size(state: &State) -> usize {
    if PAGE_SIZES.contains(&state.library_page_size) {
        state.library_page_size
    } else {
        110
    }
}

/// A category icon from the archive group name.
fn category_icon(group: &str) -> iced::widget::svg::Handle {
    let group = group.to_ascii_lowercase();
    if group.contains("level") || group.contains("map") {
        crate::icons::map()
    } else if group.contains("anim") {
        crate::icons::pulse()
    } else if group.contains("texture") {
        crate::icons::palette()
    } else if group.contains("sound") || group.contains("audio") || group.contains("music") {
        crate::icons::sliders()
    } else if group.contains("script") {
        crate::icons::file()
    } else if group.contains("object") || group.contains("model") {
        crate::icons::unpack()
    } else {
        crate::icons::folder()
    }
}

/// Legacy / Stage 2 / DLC, read from the source path label.
fn edition_badge(source: &str) -> Option<(crate::ui_kit::BadgeKind, &'static str)> {
    let source = source.to_ascii_lowercase();
    if source.contains("dlc") {
        Some((crate::ui_kit::BadgeKind::Signed, "DLC"))
    } else if source.contains("legacy") {
        Some((crate::ui_kit::BadgeKind::Nightly, "Legacy"))
    } else if source.contains("stage2") || source.contains("stage 2") || source.contains("stage_2")
    {
        Some((crate::ui_kit::BadgeKind::Enabled, "Stage 2"))
    } else {
        None
    }
}

fn select_then(index: usize, next: Message) -> Message {
    Message::Sequence(vec![Message::LibrarySelect(index), next])
}

#[allow(clippy::too_many_arguments)]
fn archive_row<'a>(
    state: &State,
    index: usize,
    archive: &'a evolve_core::ImportedPak,
    selected: bool,
    compact: bool,
    depth: usize,
    accent: iced::Color,
) -> Element<'a, Message> {
    use crate::ui_kit::{BadgeKind, badge, icon_button};
    let title = archive_title(archive);
    let mut name_line = row![
        crate::icons::glyph(category_icon(&archive.group), 14.0),
        crate::forms::highlight(title.as_str(), &state.library_search, 13.0, accent),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if let Some((kind, label)) = edition_badge(&archive.source) {
        name_line = name_line.push(badge(kind, label));
    }
    let builds: Element<'a, Message> = if archive.builds.len() > 1 {
        design::tooltip(
            badge(
                BadgeKind::Neutral,
                format!("{} builds", archive.builds.len()),
            ),
            archive.builds.join("\n"),
            None,
            iced::widget::tooltip::Position::Top,
        )
    } else {
        text(archive_build_label(archive))
            .size(design::LABEL)
            .into()
    };
    let content = row![
        crate::ui_kit::indent_guides(depth),
        column![
            name_line,
            text(archive.group.as_str())
                .size(design::CAPTION)
                .style(design::muted),
        ]
        .spacing(2)
        .width(Length::FillPortion(6)),
        text(crate::level_archive::source_label(&archive.source))
            .size(design::LABEL)
            .width(Length::FillPortion(4)),
        container(builds).width(Length::FillPortion(3)),
        text(archive.entries.to_string())
            .size(design::LABEL)
            .width(Length::FillPortion(1)),
    ]
    .spacing(9)
    .align_y(iced::Center);
    // Icon actions show on hover or selection; Details uses the side panel.
    let hovered = state.library_hover == Some(index);
    let actions: Element<'a, Message> = if hovered || selected {
        let mut actions = row![icon_button(
            crate::icons::eye(),
            "Preview",
            Some(select_then(index, Message::LibraryPreview)),
            None,
        )]
        .spacing(4);
        if crate::level_archive::offers_as_map(archive) {
            actions = actions.push(icon_button(
                crate::icons::map(),
                "Open in Level Editor",
                Some(Message::LibraryOpenMap(index)),
                None,
            ));
        }
        actions
            .push(icon_button(
                crate::icons::plus(),
                "New project",
                Some(select_then(index, Message::LibraryMakeModProject)),
                None,
            ))
            .push(icon_button(
                crate::icons::details(),
                "Details",
                Some(Message::Chrome(crate::chrome::Msg::OpenSheet(
                    crate::chrome::Sheet::Pak(index),
                ))),
                None,
            ))
            .into()
    } else {
        space().width(Length::Fixed(128.0)).into()
    };
    let line = row![
        mouse_area(
            design::button(content)
                .padding([if compact { 2 } else { 5 }, 8])
                .width(Fill)
                .style(design::text_button)
                .on_press(Message::LibrarySelect(index)),
        )
        .on_double_click(select_then(index, Message::LibraryPreview)),
        container(actions).width(Length::Fixed(136.0)),
    ]
    .spacing(6)
    .align_y(iced::Center);
    let row_index = index;
    container(
        mouse_area(line)
            .on_enter(Message::LibraryHoverRow(Some(row_index)))
            .on_exit(Message::LibraryHoverRow(None)),
    )
    .width(Fill)
    .style(move |theme| container::Style {
        background: Some(Background::Color(if selected {
            design::selection(theme)
        } else if index.is_multiple_of(2) {
            design::surface(theme)
        } else {
            design::field(theme)
        })),
        ..container::Style::default()
    })
    .into()
}

/// A category or source group as a tree row: animated chevron, nesting
/// guides and the count as a badge on the right.
fn browser_group_row<'a>(
    state: &State,
    key: &str,
    label: String,
    count: String,
    open: bool,
    message: Option<Message>,
    depth: usize,
) -> Element<'a, Message> {
    let turn = state
        .ui
        .folder_turn(&format!("library:{key}"), open, std::time::Instant::now());
    let content = row![
        crate::ui_kit::indent_guides(depth),
        crate::ui_kit::chevron(turn),
        text(label)
            .size(if depth == 0 {
                design::BODY
            } else {
                design::LABEL
            })
            .width(Fill),
        crate::ui_kit::badge(crate::ui_kit::BadgeKind::Neutral, count),
    ]
    .spacing(8)
    .align_y(iced::Center);
    design::button(content)
        .style(design::text_button)
        .width(Fill)
        .padding([if depth == 0 { 6 } else { 3 }, 10])
        .on_press_maybe(message)
        .into()
}

/// Menu entries as `(menu, label, action, checked, reason)`. The command
/// palette lists the enabled ones too.
#[allow(clippy::type_complexity)]
pub(super) fn menu_commands(
    state: &State,
) -> Vec<(
    &'static str,
    &'static str,
    Option<Message>,
    Option<bool>,
    Option<&'static str>,
)> {
    let pak = state.library_selected.is_some();
    let no_pak = Some("Select a PAK first");
    let by_category = !state.library_by_build && !state.library_flat_list;
    let level = state
        .library_selected
        .and_then(|index| state.library_catalog.as_ref()?.archives.get(index))
        .is_some_and(crate::level_archive::offers_as_map);
    vec![
        (
            "File",
            "Refresh catalog",
            Some(Message::LibraryRefresh),
            None,
            None,
        ),
        (
            "File",
            "Import PAKs…",
            Some(Message::LibraryToggleConfiguration),
            None,
            None,
        ),
        (
            "File",
            "Export files and assemblies…",
            pak.then_some(Message::LibraryExport),
            None,
            no_pak,
        ),
        (
            "File",
            "Open Asset Library folder in Explorer",
            Some(Message::LibraryOpenFolder),
            None,
            None,
        ),
        (
            "File",
            "Storage settings",
            Some(Message::OpenLibrarySettings),
            None,
            None,
        ),
        (
            "View",
            "By Category",
            Some(Message::LibrarySetByBuild(false)),
            Some(by_category),
            None,
        ),
        (
            "View",
            "By Build",
            Some(Message::LibrarySetByBuild(true)),
            Some(state.library_by_build),
            None,
        ),
        (
            "View",
            "All PAKs",
            Some(Message::LibrarySetFlat(true)),
            Some(state.library_flat_list),
            None,
        ),
        (
            "View",
            "Expand all groups",
            Some(Message::LibraryExpandAll(true)),
            None,
            None,
        ),
        (
            "View",
            "Collapse all groups",
            Some(Message::LibraryExpandAll(false)),
            None,
            None,
        ),
        (
            "Tools",
            "Manage game builds…",
            (!state.busy).then_some(Message::LibraryManageBuildsToggle),
            None,
            Some("Wait for the current operation"),
        ),
        (
            "Tools",
            "Preview selected PAK",
            pak.then_some(Message::LibraryPreview),
            None,
            no_pak,
        ),
        (
            "Tools",
            "Open in Level Editor",
            level
                .then_some(state.library_selected)
                .flatten()
                .map(Message::LibraryOpenMap),
            None,
            Some("Select a level PAK first"),
        ),
        (
            "Tools",
            "Create editable project",
            pak.then_some(Message::LibraryMakeModProject),
            None,
            no_pak,
        ),
        (
            "Tools",
            "Open Modding",
            Some(Message::Navigate(Page::Files)),
            None,
            None,
        ),
    ]
}

fn menu_content(state: &State) -> Option<Element<'_, Message>> {
    let open = state.library_menu?;
    let mut items = column![].spacing(2);
    for (menu, label, action, checked, reason) in menu_commands(state) {
        if menu == open {
            items = items.push(crate::chrome::menu_entry(label, action, checked, reason));
        }
    }
    Some(items.into())
}

fn popover(id: &'static str, open: bool) -> Message {
    Message::Ui(crate::ui_kit::UiMsg::Popover((!open).then_some(id)))
}

/// "Build: X ▾" opens every build with its PAK count; the current one is
/// checked, and "Manage builds…" sits at the bottom.
fn build_dropdown(state: &State) -> Element<'_, Message> {
    let catalog = state.library_catalog.as_ref();
    let builds = catalog.map_or(&[][..], |catalog| catalog.builds.as_slice());
    let open = state.ui.popover == Some(BUILDS_POPOVER);
    let current = builds.get(state.library_build_index);
    let trigger = design::button(
        row![
            text(
                current.map_or("No build snapshots".to_owned(), |name| format!(
                    "Build: {name}"
                ))
            )
            .size(design::LABEL),
            crate::icons::glyph(crate::icons::chevron_down(), 13.0),
        ]
        .spacing(6)
        .align_y(iced::Center),
    )
    .style(design::secondary)
    .on_press(popover(BUILDS_POPOVER, open));
    let menu = open.then(|| {
        let mut items = column![].spacing(2);
        for (index, name) in builds.iter().enumerate() {
            let paks = catalog.map_or(0, |catalog| {
                catalog
                    .archives
                    .iter()
                    .filter(|archive| archive.builds.contains(name))
                    .count()
            });
            let current = index == state.library_build_index;
            items = items.push(
                design::button(
                    row![
                        container(if current {
                            Element::from(crate::icons::glyph(crate::icons::check(), 13.0))
                        } else {
                            space().width(13).into()
                        })
                        .width(14),
                        text(name.as_str()).size(design::LABEL).width(Fill),
                        crate::ui_kit::badge(
                            crate::ui_kit::BadgeKind::Neutral,
                            format!("{paks} PAKs")
                        ),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                )
                .width(Fill)
                .padding([6, 10])
                .style(design::text_button)
                .on_press(Message::LibrarySelectBuild(index)),
            );
        }
        items = items.push(iced::widget::rule::horizontal(1)).push(
            design::button(text("Manage builds…").size(design::LABEL))
                .width(Fill)
                .padding([6, 10])
                .style(design::text_button)
                .on_press(Message::Sequence(vec![
                    popover(BUILDS_POPOVER, true),
                    Message::LibraryManageBuildsToggle,
                ])),
        );
        crate::chrome::menu_panel(items.into())
    });
    crate::anchored::anchored(trigger, menu, popover(BUILDS_POPOVER, true)).into()
}

/// "Import ▾": the main part opens the import panel; ▾ offers the two
/// import paths directly.
fn import_split(state: &State) -> Element<'_, Message> {
    let open = state.ui.popover == Some(IMPORT_POPOVER);
    let disabled = state.busy || state.library_import_scanning;
    let main = crate::ui_kit::icon_label_button(crate::icons::download(), "Import")
        .style(design::primary)
        .on_press(Message::LibraryToggleConfiguration);
    let arrow = design::button(crate::icons::glyph(crate::icons::chevron_down(), 14.0))
        .padding([9, 8])
        .style(design::primary)
        .on_press(popover(IMPORT_POPOVER, open));
    let menu = open.then(|| {
        crate::chrome::menu_panel(
            column![
                crate::chrome::menu_entry(
                    "Add PAK files…",
                    (!disabled).then_some(Message::Sequence(vec![
                        popover(IMPORT_POPOVER, true),
                        Message::LibraryPickFiles
                    ])),
                    None,
                    Some("Wait for the current operation"),
                ),
                crate::chrome::menu_entry(
                    "Import or update a game folder…",
                    (!disabled).then_some(Message::Sequence(vec![
                        popover(IMPORT_POPOVER, true),
                        Message::LibraryUpdatePickFolder
                    ])),
                    None,
                    Some("Wait for the current operation"),
                ),
            ]
            .spacing(2)
            .into(),
        )
    });
    row![
        main,
        crate::anchored::anchored(arrow, menu, popover(IMPORT_POPOVER, true)),
    ]
    .spacing(2)
    .into()
}

/// Scan → Review → Import → Done for both import flows.
fn import_steps<'a>(state: &State, has_plan: bool, importing: bool) -> Element<'a, Message> {
    let step = if state.library_import_scanning {
        0
    } else if importing {
        2
    } else if has_plan {
        1
    } else {
        3
    };
    crate::ui_kit::stepper(
        &crate::ui_kit::Steps::new(&["Scan", "Review", "Import", "Done"], step),
        state.loading_wheel_frame,
    )
}

fn import_meter<'a>(done: usize, total: usize, bytes: u64) -> Element<'a, Message> {
    crate::chrome::meter(
        "Importing".into(),
        if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        },
        format!(
            "{done} of {total} PAKs · {}",
            crate::ui_kit::format_bytes(bytes as f64)
        ),
    )
}

/// The Import side sheet: a drop zone, the two import paths, then the
/// preview and progress of the current import.
pub(crate) fn import_sheet(state: &State) -> Element<'_, Message> {
    let picking = state.library_import_scanning;
    let importing = state.library_import_cancel.is_some() && state.busy;
    let disabled = picking || state.busy;
    let drop = crate::ui_kit::drop_zone(
        container(space().height(Length::Fixed(96.0)))
            .width(Fill)
            .into(),
        "Drop .pak files or a game folder here".into(),
        false,
    );
    let mut sheet = column![
        drop,
        row![
            crate::ui_kit::icon_label_button(crate::icons::file(), "Add files…")
                .style(design::secondary)
                .on_press_maybe((!disabled).then_some(Message::LibraryPickFiles)),
            crate::ui_kit::icon_label_button(crate::icons::folder(), "Choose game folder…")
                .style(design::secondary)
                .on_press_maybe((!disabled).then_some(Message::LibraryUpdatePickFolder)),
        ]
        .spacing(8),
        crate::ui_kit::callout(
            design::Tone::Info,
            "Each PAK is verified with the original and configured public keys. Copies already imported from another game folder are reused, a damaged archive is reported without losing the others, and original PAKs are never modified.",
        ),
    ]
    .spacing(12);
    if picking || state.library_import_plan.is_some() || importing {
        sheet = sheet.push(import_steps(
            state,
            state.library_import_plan.is_some(),
            importing,
        ));
    }
    if let Some(plan) = &state.library_import_plan {
        sheet = sheet.push(
            row![
                crate::ui_kit::badge(
                    crate::ui_kit::BadgeKind::Neutral,
                    format!("{} PAKs", plan.files.len())
                ),
                crate::ui_kit::badge(
                    crate::ui_kit::BadgeKind::Neutral,
                    crate::ui_kit::format_bytes(plan.packed_bytes as f64)
                ),
                crate::ui_kit::badge(
                    crate::ui_kit::BadgeKind::Enabled,
                    format!("{} already imported", plan.already_imported)
                ),
            ]
            .spacing(6),
        );
        let mut files = column![].spacing(3);
        for path in &plan.files {
            files = files.push(
                design::mono(evolve_core::library_paths::ordinary_path(
                    &path.to_string_lossy(),
                ))
                .size(design::CAPTION),
            );
        }
        sheet = sheet.push(
            container(scrollable(files).height(Length::Fixed(160.0)))
                .padding(8)
                .width(Fill)
                .style(|theme| container::Style {
                    background: Some(Background::Color(design::field(theme))),
                    ..container::Style::default()
                }),
        );
        if importing {
            let done = state
                .library_import_progress
                .as_ref()
                .map_or(0, |p| p.load(std::sync::atomic::Ordering::Relaxed));
            sheet = sheet.push(
                row![
                    container(import_meter(done, plan.files.len(), plan.packed_bytes)).width(Fill),
                    design::button("Cancel after current PAK")
                        .style(design::secondary)
                        .on_press(Message::LibraryImportCancel),
                ]
                .spacing(10)
                .align_y(iced::Center),
            );
        } else {
            sheet = sheet.push(
                design::button(text(format!("Import {} PAKs", plan.files.len())))
                    .style(design::primary)
                    .on_press_maybe((!disabled).then_some(Message::LibraryImportStart)),
            );
        }
    }
    sheet.into()
}

/// The update plan: stat badges that filter a scrollable change list.
fn update_review(
    state: &State,
    plan: &evolve_core::LibraryBuildUpdatePlan,
) -> Element<'static, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let importing = state.library_import_cancel.is_some() && state.busy;
    let filter = state.library_update_filter;
    let chip = |label: &'static str, count: usize, kind: BadgeKind| {
        let active = filter == Some(label);
        design::button(badge(kind, format!("{count} {label}")))
            .padding(2)
            .style(move |theme, status| design::rail(theme, status, active))
            .on_press(Message::LibraryUpdateFilter(if active {
                None
            } else {
                Some(label)
            }))
    };
    let stats = row![
        chip("New", plan.new_count, BadgeKind::Enabled),
        chip("Changed", plan.changed_count, BadgeKind::Modified),
        chip("Unchanged", plan.unchanged, BadgeKind::Neutral),
        chip("Removed", plan.removed.len(), BadgeKind::Conflict),
        chip("Reused", plan.reused_count, BadgeKind::Signed),
        text(format!(
            "{} to import",
            crate::ui_kit::format_bytes(plan.import_bytes as f64)
        ))
        .size(design::CAPTION)
        .style(design::muted),
    ]
    .spacing(6)
    .align_y(iced::Center);
    let mut list = column![].spacing(2);
    for change in &plan.changed {
        let (label, kind) = if change.reuse_archive_id.is_some() {
            ("Reused", BadgeKind::Signed)
        } else if change.was_present {
            ("Changed", BadgeKind::Modified)
        } else {
            ("New", BadgeKind::Enabled)
        };
        let shown = match filter {
            None => true,
            Some("Reused") => change.reuse_archive_id.is_some(),
            Some(other) => other == label,
        };
        if shown {
            list = list.push(
                row![
                    container(badge(kind, label)).width(Length::Fixed(80.0)),
                    design::mono(change.relative.clone()).size(design::CAPTION),
                ]
                .spacing(8)
                .align_y(iced::Center),
            );
        }
    }
    if filter.is_none_or(|f| f == "Removed") {
        for removed in &plan.removed {
            list = list.push(
                row![
                    container(badge(BadgeKind::Conflict, "Removed")).width(Length::Fixed(80.0)),
                    design::mono(removed.clone()).size(design::CAPTION),
                ]
                .spacing(8)
                .align_y(iced::Center),
            );
        }
    }
    let done = state.library_import_progress.as_ref().map_or(0, |counter| {
        counter.load(std::sync::atomic::Ordering::Relaxed)
    });
    let mut body = column![
        row![
            design::heading("Update imported build"),
            space().width(Fill),
            design::button("Close")
                .style(design::text_button)
                .on_press_maybe((!state.busy).then_some(Message::LibraryUpdateScanned(Err(
                    "Update preview dismissed".into()
                )))),
        ]
        .align_y(iced::Center),
        design::caption(format!(
            "Source installation: {}",
            evolve_core::library_paths::ordinary_path(&plan.source_root.to_string_lossy())
        )),
        import_steps(state, true, importing),
        stats,
        container(scrollable(list).height(Length::Fixed(170.0)))
            .padding(8)
            .width(Fill)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::field(theme))),
                ..container::Style::default()
            }),
        crate::ui_kit::disclosure(
            &state.ui,
            "library:update-help",
            text("How updates are imported").size(design::LABEL),
            state.library_update_help,
            Message::LibraryUpdateHelp,
            || {
                design::caption(
                    "Each PAK is identified by its SHA-256. Archives identical to ones imported from another installation reuse that extraction, so only new or changed bytes are copied. A changed encrypted PAK is verified and extracted again. Installed games and mod projects stay untouched.",
                )
                .into()
            },
        ),
    ]
    .spacing(10);
    body = body.push(if importing {
        Element::from(
            row![
                container(import_meter(done, plan.changed.len(), plan.import_bytes)).width(Fill),
                design::button("Cancel after current PAK")
                    .style(design::secondary)
                    .on_press(Message::LibraryImportCancel),
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
    } else {
        design::button(text(
            if plan.changed.is_empty() && plan.removed.is_empty() {
                "Register current build snapshot".to_owned()
            } else {
                format!(
                    "Apply update · {} new or changed, {} removed",
                    plan.changed.len(),
                    plan.removed.len()
                )
            },
        ))
        .style(design::primary)
        .on_press_maybe(
            (!state.busy && !state.library_import_scanning).then_some(Message::LibraryUpdateStart),
        )
        .into()
    });
    panel(container(body).padding(14)).into()
}

/// The bottom pill while archives are selected.
fn selection_bar(state: &State) -> Option<Element<'_, Message>> {
    let count = state.library_selection.indices.len();
    if count == 0 && state.library_selected.is_none() {
        return None;
    }
    let count = count.max(1);
    let pak = state.library_selected.is_some();
    let action = |icon, label: &'static str, message: Message| {
        crate::ui_kit::icon_label_button(icon, label)
            .style(design::text_button)
            .on_press_maybe(pak.then_some(message))
    };
    let bar = row![
        text(format!("{count} selected")).size(design::BODY),
        text("·").style(design::muted),
        action(crate::icons::eye(), "Preview", Message::LibraryPreview),
        action(
            crate::icons::plus(),
            "New project",
            Message::LibraryMakeModProject
        ),
        action(crate::icons::export(), "Export", Message::LibraryExport),
        crate::ui_kit::icon_label_button(crate::icons::close(), "Clear")
            .style(design::text_button)
            .on_press(Message::LibraryClearSelection),
        text("·").style(design::muted),
        design::key_hint::<Message>("Shift"),
        design::caption("range"),
        design::key_hint::<Message>("Ctrl"),
        design::caption("toggle"),
    ]
    .spacing(6)
    .align_y(iced::Center);
    Some(
        container(bar)
            .padding([6, 14])
            .style(|theme: &iced::Theme| container::Style {
                background: Some(Background::Color(design::surface(theme))),
                border: Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: design::radius(theme, design::Radius::Pill).into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                    offset: iced::Vector::new(0.0, 6.0),
                    blur_radius: 18.0,
                },
                ..container::Style::default()
            })
            .into(),
    )
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge, icon_button};
    let library = state.library_catalog.as_ref();
    let selected = state
        .library_selected
        .and_then(|i| library.and_then(|c| c.archives.get(i)));

    let menubar = crate::chrome::menubar(
        crate::chrome::MenuBar::Library,
        &["File", "View", "Tools"],
        state.library_menu,
        Message::LibraryMenu,
        menu_content(state),
    );
    let busy = state.busy || state.library_import_scanning;
    let toolbar = row![
        import_split(state),
        space().width(Fill),
        icon_button(
            crate::icons::layers(),
            "Manage game builds",
            (!busy).then_some(Message::LibraryManageBuildsToggle),
            Some("Wait for the current operation"),
        ),
        icon_button(
            crate::icons::drive(),
            "Storage settings",
            Some(Message::OpenLibrarySettings),
            None,
        ),
    ]
    .spacing(6)
    .align_y(iced::Center);
    let mut layout = column![
        row![
            design::title("Asset Library"),
            space().width(Length::Fixed(18.0)),
            menubar
        ]
        .align_y(iced::Center),
        toolbar,
    ]
    .spacing(12);

    if let Some(plan) = &state.library_update_plan {
        layout = layout.push(update_review(state, plan));
    } else if state.library_import_scanning && !state.library_show_configuration {
        layout = layout.push(design::caption(
            "Scanning this build and hashing original PAKs. No files are being imported.",
        ));
    }

    let by_build = state.library_by_build;
    let flat = state.library_flat_list;
    let build = library.and_then(|c| c.builds.get(state.library_build_index));
    let view_index = if flat {
        2
    } else if by_build {
        1
    } else {
        0
    };
    let mut views = row![crate::ui_kit::segmented(
        &state.ui,
        "library-view",
        vec![
            ("By Category", Message::LibrarySetByBuild(false)),
            ("By Build", Message::LibrarySetByBuild(true)),
            ("All PAKs", Message::LibrarySetFlat(true)),
        ],
        view_index,
        true,
    )]
    .spacing(12)
    .align_y(iced::Center);
    if by_build {
        views = views.push(build_dropdown(state));
    }
    layout = layout.push(views);
    layout = layout.push(
        row![
            crate::forms::search_field(
                "Search imported PAKs, names, folders...",
                &state.library_search,
                Some(state.library_matches.len()),
                Message::LibrarySearchChanged,
            ),
            icon_button(
                crate::icons::refresh(),
                "Refresh catalog",
                Some(Message::LibraryRefresh),
                None,
            ),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    layout = layout.push(crate::ui_kit::chips(
        &state.library_chips,
        &state.library_chip_options,
        &state.library_chip_query,
        Message::LibraryChipPicked,
        Message::LibraryChipInput,
        Message::LibraryChipRemove,
    ));
    // Library › Build › PAK for the current selection.
    if let Some(archive) = selected {
        let mut crumbs = vec![crate::chrome::Crumb {
            label: "Library".into(),
            message: Some(Message::Chrome(crate::chrome::Msg::LibraryRoot)),
        }];
        let build = library
            .filter(|_| state.library_by_build)
            .and_then(|catalog| catalog.builds.get(state.library_build_index))
            .or_else(|| archive.builds.first());
        if let Some(build) = build {
            crumbs.push(crate::chrome::Crumb {
                label: build.clone(),
                message: None,
            });
        }
        if !archive.group.is_empty() {
            crumbs.push(crate::chrome::Crumb {
                label: archive.group.clone(),
                message: None,
            });
        }
        crumbs.push(crate::chrome::Crumb {
            label: archive_title(archive),
            message: None,
        });
        layout = layout.push(crate::chrome::breadcrumbs(crumbs, state.chrome.crumb_menu));
    }

    let display_rows = browser_rows(state);
    let matching = state.library_matches.len();
    let accent = crate::forms::highlight_color(&state.preferences);
    let per_page = page_size(state);
    let pages = display_rows.len().div_ceil(per_page).max(1);
    let page = state.library_page.min(pages - 1);
    let compact = state.preferences.ui_density == "Compact";
    let searching = !state.library_search.trim().is_empty();
    let mut rows = column![].spacing(0);
    if let Some(catalog) = library {
        let begin = page.saturating_mul(per_page);
        for entry in display_rows.iter().skip(begin).take(per_page) {
            match entry {
                BrowserRow::Category { key, name, count } => {
                    let open = searching || state.library_expanded_categories.contains(key);
                    rows = rows.push(browser_group_row(
                        state,
                        key,
                        name.clone(),
                        format!("{count} PAKs"),
                        open,
                        (!searching).then(|| Message::LibraryToggleCategory(key.clone())),
                        0,
                    ));
                }
                BrowserRow::SourceGroup {
                    key,
                    name,
                    source,
                    count,
                } => {
                    let open = searching || state.library_expanded_sources.contains(key);
                    rows = rows.push(browser_group_row(
                        state,
                        key,
                        format!("{name}  ·  {source}"),
                        format!("{count} records"),
                        open,
                        (!searching).then(|| Message::LibraryToggleSource(key.clone())),
                        1,
                    ));
                }
                BrowserRow::Archive(index, depth) => {
                    if let Some(archive) = catalog.archives.get(*index) {
                        rows = rows.push(archive_row(
                            state,
                            *index,
                            archive,
                            state.library_selection.indices.contains(index),
                            compact,
                            *depth,
                            accent,
                        ));
                    }
                }
            }
        }
    }
    if matching == 0 {
        rows = rows.push(if state.library_startup_loading {
            // Skeleton rows pulse while manifests are indexed.
            let alpha = crate::ui_kit::pulse(
                state.ui.clock(std::time::Instant::now()),
                state.preferences.reduce_motion,
            );
            let mut skeleton = column![].spacing(10).padding(14);
            for _ in 0..8 {
                skeleton = skeleton.push(
                    row![
                        column![
                            crate::ui_kit::skeleton(Fill, 12.0, alpha),
                            crate::ui_kit::skeleton(Length::Fixed(120.0), 9.0, alpha),
                        ]
                        .spacing(5)
                        .width(Length::FillPortion(6)),
                        crate::ui_kit::skeleton(Length::FillPortion(4), 11.0, alpha),
                        crate::ui_kit::skeleton(Length::FillPortion(3), 11.0, alpha),
                        crate::ui_kit::skeleton(Length::FillPortion(1), 11.0, alpha),
                    ]
                    .spacing(9)
                    .align_y(iced::Center),
                );
            }
            Element::from(skeleton)
        } else if library.is_none_or(|catalog| catalog.archives.is_empty()) {
            crate::ui_kit::empty_state(
                Some(Page::AssetLibrary.icon()),
                "Your library is empty",
                "Import original game PAKs to browse, preview and start mod projects from them. Originals stay read-only.",
                Some(("Import PAKs…", Message::LibraryToggleConfiguration)),
            )
        } else if by_build && build.is_none() {
            container(
                text("No build snapshot is available. Use By Category or import snapshot metadata.")
                    .size(design::BODY)
                    .style(design::muted),
            )
            .padding(18)
            .into()
        } else {
            crate::ui_kit::empty_state(None, "No matches", "No imported PAKs match this search.", None)
        });
    }
    let table_heading = container(
        row![
            sort_cell("Archive / Category", "name", 6, state),
            sort_cell("Game", "game", 4, state),
            sort_cell("Build(s)", "builds", 3, state),
            sort_cell("Files", "files", 1, state),
            container(header_cell("Actions", 1)).width(Length::Fixed(136.0)),
        ]
        .spacing(9)
        .padding([10, 8]),
    )
    .style(|theme| container::Style {
        background: Some(Background::Color(design::field(theme))),
        ..container::Style::default()
    })
    .width(Fill);
    let table = panel(
        column![
            table_heading,
            crate::ui_kit::faded_scroll(&state.ui, "library-rows", rows, display_rows.len() > 12),
        ]
        .spacing(0)
        .height(Fill),
    );
    layout = layout.push(table.height(Fill));

    // Footer: stats on the left, pagination on the right.
    let (count, bytes, ignored) = library
        .map(|c| (c.archives.len(), c.total_bytes, c.ignored))
        .unwrap_or((0, 0, 0));
    let mut stats = row![
        badge(BadgeKind::Neutral, format!("{count} imported")),
        badge(BadgeKind::Neutral, format!("{matching} matching")),
        badge(
            BadgeKind::Neutral,
            crate::ui_kit::format_bytes(bytes as f64)
        ),
        badge(BadgeKind::ReadOnly, "Read-only previews"),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if ignored > 0 {
        stats = stats.push(design::tooltip(
            badge(BadgeKind::Conflict, format!("{ignored} invalid manifests skipped")),
            "These imported folders have a missing or damaged manifest and are left out of the library. Re-import their PAKs to restore them.",
            None,
            iced::widget::tooltip::Position::Top,
        ));
    }
    let mut sizes = row![design::caption("Rows")]
        .spacing(3)
        .align_y(iced::Center);
    for size in PAGE_SIZES {
        let active = per_page == size;
        sizes = sizes.push(
            design::button(text(size.to_string()).size(design::CAPTION))
                .padding([2, 6])
                .style(move |theme, status| design::rail(theme, status, active))
                .on_press(Message::LibraryPageSize(size)),
        );
    }
    layout = layout.push(
        row![
            stats.width(Fill),
            sizes,
            icon_button(
                crate::icons::chevron_left(),
                "Previous page",
                (page > 0).then_some(Message::LibraryPreviousPage),
                Some("Already on the first page"),
            ),
            text(format!("Page {} of {pages}", page + 1)).size(design::LABEL),
            icon_button(
                crate::icons::chevron(),
                "Next page",
                (page + 1 < pages).then_some(Message::LibraryNextPage),
                Some("Already on the last page"),
            ),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );

    let page_view: Element<'_, Message> = match selection_bar(state) {
        Some(bar) => stack![
            layout.width(Fill).height(Fill),
            container(bar)
                .width(Fill)
                .height(Fill)
                .align_bottom(Fill)
                .center_x(Fill)
                .padding(iced::Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 52.0,
                    left: 0.0,
                }),
        ]
        .into(),
        None => layout.width(Fill).height(Fill).into(),
    };
    if !state.library_manage_builds_open {
        return page_view;
    }
    crate::chrome::dialog(
        page_view,
        container(scrollable(manage_builds_dialog(state)).height(Length::Shrink))
            .padding(18)
            .width(Length::Fixed(820.0))
            .max_height(670)
            .into(),
        Message::LibraryManageBuildsToggle,
    )
}

/// Manage builds: one card per build with an editable name, a meter for
/// its share of library disk use, a reclaimable badge and hold-to-remove.
fn manage_builds_dialog(state: &State) -> Element<'_, Message> {
    use crate::ui_kit::{BadgeKind, badge};
    let total: u64 = state
        .library_managed_builds
        .iter()
        .map(|build| build.extracted_bytes)
        .sum::<u64>()
        .max(1);
    let mut content = column![
        row![
            design::heading("Manage game builds"),
            space().width(Fill),
            crate::ui_kit::icon_button(
                crate::icons::refresh(),
                "Refresh builds",
                Some(Message::LibraryManageBuildsRefresh),
                None,
            ),
            crate::ui_kit::icon_label_button(crate::icons::download(), "Import or update…")
                .style(design::secondary)
                .on_press_maybe(
                    (!state.busy && !state.library_import_scanning)
                        .then_some(Message::LibraryUpdatePickFolder)
                ),
            design::button("Close")
                .style(design::secondary)
                .on_press(Message::LibraryManageBuildsToggle),
        ]
        .align_y(iced::Center)
        .spacing(8),
        design::caption(
            "Click a build's name to rename it. Removing deletes the imported snapshot history and only extractions no other build uses. Original game installations and mod projects are never deleted."
        ),
    ]
    .spacing(12);
    if state.library_managed_builds.is_empty() {
        content = content.push(design::caption(
            "No managed build snapshots found. Older standalone PAK imports stay available in the Asset Library.",
        ));
    }
    for build in &state.library_managed_builds {
        let editing = state.library_rename_build.as_deref() == Some(build.id.as_str());
        let name = if build.display_name.is_empty() {
            build.source_root.clone()
        } else {
            build.display_name.clone()
        };
        let title: Element<'_, Message> = if editing {
            row![
                design::text_input("Build name…", &state.library_rename_value)
                    .on_input(Message::LibraryRenameBuildValue)
                    .on_submit(Message::LibraryRenameBuildSave(build.id.clone()))
                    .width(Fill),
                design::button("Save")
                    .style(design::primary)
                    .on_press_maybe(
                        (!state.busy).then_some(Message::LibraryRenameBuildSave(build.id.clone()))
                    ),
                design::button("Cancel")
                    .style(design::text_button)
                    .on_press(Message::LibraryRenameBuildCancel),
            ]
            .spacing(6)
            .align_y(iced::Center)
            .into()
        } else {
            design::tooltip(
                design::button(design::heading(name))
                    .padding([2, 4])
                    .style(design::text_button)
                    .on_press_maybe(
                        (!state.busy).then_some(Message::LibraryRenameBuildBegin(build.id.clone())),
                    ),
                "Click to rename",
                None,
                iced::widget::tooltip::Position::Top,
            )
        };
        let mut badges = row![
            badge(BadgeKind::Neutral, format!("{} PAKs", build.pak_count)),
            badge(BadgeKind::Neutral, format!("{} history IDs", build.aliases)),
        ]
        .spacing(6);
        if build.reclaimable_bytes > 0 {
            badges = badges.push(badge(
                BadgeKind::Modified,
                format!(
                    "Reclaimable {}",
                    crate::ui_kit::format_bytes(build.reclaimable_bytes as f64)
                ),
            ));
        }
        let card = column![
            title,
            design::caption(format!("Source: {}", build.source_root)),
            badges,
            crate::chrome::meter(
                "Share of library disk use".into(),
                build.extracted_bytes as f32 / total as f32,
                crate::ui_kit::format_bytes(build.extracted_bytes as f64),
            ),
            crate::ui_kit::hold_button(
                &state.ui,
                &format!("remove-build:{}", build.id),
                "Hold to remove build",
                Message::LibraryDeleteBuildNow(build.id.clone()),
                !state.busy,
                state.preferences.reduce_motion,
            ),
        ]
        .spacing(8);
        content = content.push(panel(container(card).padding(12)));
    }
    content.into()
}

#[cfg(test)]
mod browser_tests {
    use super::*;
    use crate::level_archive::source_label;

    fn archive(id: &str, group: &str, name: &str, source: &str) -> evolve_core::ImportedPak {
        evolve_core::ImportedPak {
            id: id.to_owned(),
            name: name.to_owned(),
            source: source.to_owned(),
            directory: format!("/library/{id}"),
            group: group.to_owned(),
            description: "Authoritative imported record".to_owned(),
            entries: 30,
            extracted_bytes: 300,
            warnings: 0,
            builds: vec!["Stage 2".to_owned()],
        }
    }

    fn fixture() -> State {
        let items = vec![
            archive(
                "a",
                "Animations",
                "AnimationsCAF.pak",
                "C:\\Games\\Stage2\\Game\\AnimationsCAF.pak",
            ),
            archive(
                "b",
                "Animations",
                "AnimationsCAF.pak",
                "C:\\Games\\Stage2\\Game\\AnimationsCAF.pak",
            ),
            archive(
                "c",
                "Animations",
                "AnimationsCAF.pak",
                "C:\\Games\\Legacy\\Game\\AnimationsCAF.pak",
            ),
            archive(
                "d",
                "Levels",
                "Levels_Tutorial.pak",
                "C:\\Games\\Stage2\\Game\\Levels_Tutorial.pak",
            ),
        ];
        State {
            library_catalog: Some(evolve_core::ImportedLibrary {
                root: "/library".to_owned(),
                archives: items,
                builds: vec!["Stage 2".to_owned()],
                total_bytes: 1200,
                ignored: 0,
            }),
            library_matches: vec![0, 1, 2, 3],
            ..State::default()
        }
    }

    #[test]
    fn categories_collapsed_by_default_without_losing_underlying_records() {
        let mut state = fixture();
        let rows = browser_rows(&state);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| matches!(row, BrowserRow::Category { .. }))
        );
        assert!(visible_archive_indices(&state).is_empty());
        state
            .library_expanded_categories
            .insert("animations".to_owned());
        let rows = browser_rows(&state);
        assert!(
            rows.iter()
                .any(|row| matches!(row, BrowserRow::SourceGroup { count: 2, .. }))
        );
        assert_eq!(visible_archive_indices(&state), vec![2]);
        assert_eq!(state.library_catalog.as_ref().unwrap().archives.len(), 4);
    }

    #[test]
    fn duplicate_source_group_is_nondestructive_and_each_record_remains_selectable() {
        let mut state = fixture();
        state
            .library_expanded_categories
            .insert("animations".to_owned());
        let source_key = source_group_key(&state.library_catalog.as_ref().unwrap().archives[0]);
        state.library_expanded_sources.insert(source_key);
        let mut visible = visible_archive_indices(&state);
        visible.sort_unstable();
        assert_eq!(visible, vec![0, 1, 2]);
        assert_ne!(
            state.library_catalog.as_ref().unwrap().archives[0].id,
            state.library_catalog.as_ref().unwrap().archives[1].id
        );
        state.library_flat_list = true;
        assert_eq!(visible_archive_indices(&state), vec![0, 1, 2, 3]);
    }

    #[test]
    fn source_labels_preserve_build_provenance_and_search_opens_matches() {
        assert_eq!(
            source_label("C:\\Games\\ModdedEvolve\\EvolveGame\\Game\\X.pak"),
            "EvolveGame"
        );
        assert_eq!(
            source_label("C:/Games/2012 Evolve/build_3/Game/X.pak"),
            "2012 Evolve / build_3"
        );
        let mut state = fixture();
        state.library_matches = vec![3];
        state.library_search = "tutorial".to_owned();
        assert_eq!(visible_archive_indices(&state), vec![3]);
        assert_eq!(state.library_catalog.as_ref().unwrap().archives.len(), 4);
    }

    /// 2012 maps import as `level.pak` and `terraintexture.pak` siblings. Only
    /// the level PAK is offered to the Level Editor, and it is titled by its
    /// owner folder instead of the generic filename.
    #[test]
    fn map_owned_2012_archives_offer_only_the_level_pak() {
        let items = vec![
            archive(
                "level",
                "Levels",
                "level.pak",
                "C:\\Games\\2012 Evolve\\build_3\\Game\\Levels\\Forest_Rapids\\level.pak",
            ),
            archive(
                "terrain",
                "Levels",
                "terraintexture.pak",
                "C:\\Games\\2012 Evolve\\build_3\\Game\\Levels\\Forest_Rapids\\terraintexture.pak",
            ),
        ];
        let mut state = State {
            library_catalog: Some(evolve_core::ImportedLibrary {
                root: "/library".to_owned(),
                archives: items,
                builds: vec!["Stage 2".to_owned()],
                total_bytes: 600,
                ignored: 0,
            }),
            library_matches: vec![0, 1],
            ..State::default()
        };
        let level_action = |state: &State| {
            menu_commands(state)
                .into_iter()
                .find(|(_, label, ..)| *label == "Open in Level Editor")
                .and_then(|(_, _, action, ..)| action)
        };
        state.library_selected = Some(0);
        assert!(level_action(&state).is_some());
        state.library_selected = Some(1);
        assert!(level_action(&state).is_none());
        assert_eq!(
            archive_title(&state.library_catalog.as_ref().unwrap().archives[0]),
            "Forest_Rapids"
        );
    }
}
