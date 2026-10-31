//! Private-manager-style imported PAK browser. Original archives and
//! extracted Asset Library files remain read-only; mod editing lives elsewhere.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{column, container, mouse_area, row, scrollable, space, stack, text};
use iced::{Background, Border, Element, Fill, Length};
use std::path::Path;

const PAGE_SIZE: usize = 110;

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
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

fn header_cell(label: &'static str, width: u16) -> Element<'static, Message> {
    text(label)
        .size(13)
        .width(Length::FillPortion(width))
        .into()
}

fn archive_row<'a>(
    index: usize,
    archive: &'a evolve_core::ImportedPak,
    selected: bool,
    compact: bool,
) -> Element<'a, Message> {
    let folder = Path::new(&archive.source)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let content = row![
        text(archive.group.as_str())
            .size(12)
            .width(Length::FillPortion(3)),
        text(folder).size(12).width(Length::FillPortion(4)),
        text(archive.description.as_str())
            .size(12)
            .width(Length::FillPortion(5)),
        text(archive.name.as_str())
            .size(12)
            .width(Length::FillPortion(4)),
        text(format!("{}", archive.entries))
            .size(12)
            .width(Length::FillPortion(1)),
    ]
    .spacing(9)
    .align_y(iced::Center);
    let item = design::button(content)
        .padding([if compact { 2 } else { 5 }, 8])
        .width(Fill)
        .style(design::text_button)
        .on_press(Message::LibrarySelect(index));
    container(item)
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

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let library = state.library_catalog.as_ref();
    let selected = state
        .library_selected
        .and_then(|i| library.and_then(|c| c.archives.get(i)));

    let menu = row![
        design::button("File")
            .on_press(Message::LibraryMenu("File"))
            .style(design::text_button),
        design::button("View")
            .on_press(Message::LibraryMenu("View"))
            .style(design::text_button),
        design::button("Tools")
            .on_press(Message::LibraryMenu("Tools"))
            .style(design::text_button),
    ]
    .spacing(8);
    let menu_details: Option<Element<'_, Message>> = match state.library_menu {
        Some("File") => Some(
            column![
                design::button("Refresh catalog")
                    .on_press(Message::MenuCommand(Box::new(Message::LibraryRefresh))),
                design::button("Open as Modding Project").on_press(Message::MenuCommand(Box::new(
                    Message::LibraryMakeModProject
                ))),
                design::button("Export files and assemblies…")
                    .on_press(Message::MenuCommand(Box::new(Message::LibraryExport))),
                design::button("Open Asset Library Folder in Explorer")
                    .on_press(Message::MenuCommand(Box::new(Message::LibraryOpenFolder)),),
                design::button("Import signed PAK…").on_press(Message::MenuCommand(Box::new(
                    Message::LibraryToggleConfiguration
                ))),
                design::button("Storage settings").on_press(Message::MenuCommand(Box::new(
                    Message::Navigate(Page::Settings)
                ))),
            ]
            .spacing(3)
            .into(),
        ),
        Some("View") => Some(
            column![
                design::button("All Assets").on_press(Message::MenuCommand(Box::new(
                    Message::LibrarySetByBuild(false)
                ))),
                design::button("By Build").on_press(Message::MenuCommand(Box::new(
                    Message::LibrarySetByBuild(true)
                ))),
            ]
            .spacing(3)
            .into(),
        ),
        Some("Tools") => Some(
            column![
                design::button("Manage Game Builds").on_press(Message::MenuCommand(Box::new(
                    Message::LibraryManageBuildsToggle
                ))),
                design::button("Open in Level Editor")
                    .on_press_maybe(state.library_selected.map(Message::LibraryOpenMap)),
                design::button("Preview selected PAK")
                    .on_press(Message::MenuCommand(Box::new(Message::LibraryPreview))),
                design::button("Create editable project").on_press(Message::MenuCommand(Box::new(
                    Message::LibraryMakeModProject
                ))),
                design::button("Open Modding").on_press(Message::MenuCommand(Box::new(
                    Message::Navigate(Page::Files)
                ))),
            ]
            .spacing(3)
            .into(),
        ),
        _ => None,
    };

    let mut layout = column![
        text("Asset Library").size(24),
        container(menu)
            .padding([4, 8])
            .width(Fill)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::field(theme))),
                ..container::Style::default()
            }),
        row![
            design::button("Import PAKs")
                .style(design::secondary)
                .on_press(Message::LibraryToggleConfiguration),
            design::button("Update Game Build…")
                .style(design::primary)
                .on_press_maybe(
                    (!state.busy && !state.library_import_scanning)
                        .then_some(Message::LibraryUpdatePickFolder)
                ),
            design::button(if state.library_manage_builds_open {
                "Close Build Manager"
            } else {
                "Manage Game Builds"
            })
            .style(design::secondary)
            .on_press_maybe(
                (!state.busy && !state.library_import_scanning)
                    .then_some(Message::LibraryManageBuildsToggle)
            ),
            design::button("Storage Settings")
                .style(design::secondary)
                .on_press(Message::OpenLibrarySettings),
        ]
        .spacing(9)
        .align_y(iced::Center),
    ]
    .spacing(14);

    if let Some(plan) = &state.library_update_plan {
        let importing = state.library_import_cancel.is_some() && state.busy;
        let mut changed_paths = column![].spacing(2);
        for change in plan.changed.iter().take(8) {
            changed_paths = changed_paths.push(
                text(format!(
                    "{} · {}{}",
                    if change.was_present { "Updated" } else { "New" },
                    change.relative,
                    if change.reuse_archive_id.is_some() {
                        " (already imported — reuse)"
                    } else {
                        ""
                    }
                ))
                .size(11)
                .style(design::muted),
            );
        }
        if plan.changed.len() > 8 {
            changed_paths = changed_paths.push(
                text(format!(
                    "…and {} other changed/new PAKs",
                    plan.changed.len() - 8
                ))
                .size(11)
                .style(design::muted),
            );
        }
        let progress = state.library_import_progress.as_ref().map_or(0, |counter| {
            counter.load(std::sync::atomic::Ordering::Relaxed)
        });
        let summary = column![
            row![
                text("UPDATE IMPORTED BUILD").size(15),
                space().width(Fill),
                design::button("Close").style(design::text_button)
                    .on_press_maybe((!state.busy).then_some(Message::LibraryUpdateScanned(
                        Err("Update preview dismissed".into())
                    ))),
            ].spacing(8),
            text(format!("Source installation: {}", evolve_core::library_paths::ordinary_path(&plan.source_root.to_string_lossy()))).size(12),
            text(format!(
                "{} new  •  {} changed  •  {} unchanged  •  {} removed  •  {} reused  •  {:.2} GiB to import",
                plan.new_count, plan.changed_count, plan.unchanged, plan.removed.len(),
                plan.reused_count, plan.import_bytes as f64 / 1_073_741_824.0
            )).size(13),
            text("SHA-256 scan only: matching archives across other installations reuse the same imported extraction, so only genuinely new/changed PAK bytes are copied. A changed encrypted PAK still needs verification and re-extraction. Installed games and mod projects stay untouched.")
                .size(11).style(design::muted),
            changed_paths,
            row![
                design::button(text(if plan.changed.is_empty() && plan.removed.is_empty() {
                    "Register Current Build Snapshot".to_owned()
                } else {
                    format!(
                        "Apply build update · {} new/changed, {} removed",
                        plan.changed.len(), plan.removed.len()
                    )
                }))
                    .style(design::primary)
                    .on_press_maybe(
                        (!state.busy && !state.library_import_scanning)
                            .then_some(Message::LibraryUpdateStart),
                    ),
                text(if importing {
                    format!("Updated import: {progress} / {}", plan.changed.len())
                } else {
                    "Review this build's changes before importing".to_owned()
                }).size(11).style(design::muted),
                design::button("Cancel after current PAK")
                    .style(design::secondary)
                    .on_press_maybe(importing.then_some(Message::LibraryImportCancel)),
            ].spacing(9)
        ].spacing(8);
        layout = layout.push(panel(container(summary).padding(12)));
    } else if state.library_import_scanning {
        layout = layout.push(
            text("Scanning this build and hashing original PAKs. No files are being imported.")
                .size(12)
                .style(design::muted),
        );
    }

    if state.library_show_configuration {
        let picking = state.library_import_scanning;
        let importing = state.library_import_cancel.is_some();
        let disabled = picking || state.busy;
        let mut importer = column![
            row![
                text("IMPORT PAKS INTO ASSET LIBRARY").size(16),
                space().width(Fill),
                design::button("Close").style(design::text_button)
                    .on_press_maybe((!disabled).then_some(Message::LibraryToggleConfiguration)),
            ].spacing(8),
            text("Select individual PAK files here, or use Import or Update Game Build Folder to scan all nested PAKs incrementally. Previously imported copies are reused across game folders. Original PAKs are never modified.")
                .size(12).style(design::muted),
            row![
                design::button("Add PAK files…").style(design::secondary)
                    .on_press_maybe((!disabled).then_some(Message::LibraryPickFiles)),
                design::button("Import or Update Game Build Folder…").style(design::secondary)
                    .on_press_maybe((!disabled).then_some(Message::LibraryUpdatePickFolder)),
            ].spacing(10),
            text("Each PAK is independently verified using the bundled original public key and configured public keys. Existing complete imports are reused. A corrupt or incompatible archive will be reported without discarding successful imports.")
                .size(12).style(design::muted),
        ].spacing(10);
        if picking {
            importer = importer.push(
                text("Scanning selected files and building the import preview…")
                    .size(13)
                    .style(design::muted),
            );
        }
        if let Some(plan) = &state.library_import_plan {
            importer = importer.push(
                text(format!(
                    "{} PAKs found  •  {:.2} GiB packed  •  {} previously imported",
                    plan.files.len(),
                    plan.packed_bytes as f64 / 1_073_741_824.0,
                    plan.already_imported,
                ))
                .size(14),
            );
            let mut filenames = column![];
            for path in plan.files.iter().take(10) {
                filenames = filenames.push(
                    text(evolve_core::library_paths::ordinary_path(
                        &path.to_string_lossy(),
                    ))
                    .size(11)
                    .style(design::muted),
                );
            }
            if plan.files.len() > 10 {
                filenames = filenames.push(
                    text(format!("… plus {} more PAKs", plan.files.len() - 10,))
                        .size(12)
                        .style(design::muted),
                );
            }
            importer = importer.push(
                container(filenames.spacing(3))
                    .padding(8)
                    .width(Fill)
                    .style(|theme| container::Style {
                        background: Some(Background::Color(design::field(theme))),
                        ..container::Style::default()
                    }),
            );
            importer = importer.push(
                row![
                    design::button(text(format!("Confirm import of {} PAKs", plan.files.len())))
                        .style(design::primary)
                        .on_press_maybe((!disabled).then_some(Message::LibraryImportStart)),
                    text("Large full-game imports can use significant disk space.")
                        .size(12)
                        .style(design::muted),
                ]
                .spacing(12),
            );
        }
        if importing {
            let count = state
                .library_import_progress
                .as_ref()
                .map_or(0, |progress| {
                    progress.load(std::sync::atomic::Ordering::Relaxed)
                });
            let total = state
                .library_import_plan
                .as_ref()
                .map_or(0, |plan| plan.files.len());
            importer = importer.push(
                row![
                    text(format!("Importing: {count} / {total} archives completed")).size(13),
                    design::button("Cancel after current PAK")
                        .on_press(Message::LibraryImportCancel),
                ]
                .spacing(12),
            );
        }
        importer = importer.push(
            row![
                design::button("Manage Asset Library storage")
                    .style(design::text_button)
                    .on_press_maybe((!disabled).then_some(Message::OpenLibrarySettings)),
                design::button("Advanced archive tools in Modding")
                    .style(design::text_button)
                    .on_press(Message::Navigate(Page::Files)),
            ]
            .spacing(8),
        );
        layout = layout.push(panel(container(importer.spacing(8)).padding(14)));
    }

    let by_build = state.library_by_build;
    let build = library.and_then(|c| c.builds.get(state.library_build_index));
    let tabs = row![
        design::button("All Assets")
            .style(if !by_build {
                design::secondary
            } else {
                design::text_button
            })
            .on_press(Message::LibrarySetByBuild(false)),
        design::button("By Build")
            .style(if by_build {
                design::secondary
            } else {
                design::text_button
            })
            .on_press(Message::LibrarySetByBuild(true)),
        space().width(Fill),
        if by_build {
            design::button(text(build.map_or("No build snapshots".to_owned(), |s| {
                format!("Build: {s}  ▾")
            })))
            .on_press(Message::LibraryNextBuild)
        } else {
            design::button("All imported PAKs").on_press(Message::LibrarySetByBuild(false))
        },
    ]
    .spacing(12);
    layout = layout.push(tabs);
    layout = layout.push(
        row![
            design::text_input(
                "Search imported PAKs, names, folders...",
                &state.library_search
            )
            .on_input(Message::LibrarySearchChanged)
            .width(Fill),
            design::button("Refresh").on_press(Message::LibraryRefresh),
        ]
        .spacing(8),
    );

    let mut rows = column![].spacing(0);
    // The filter is rebuilt only when inputs change; render one bounded page.
    let matching = state.library_matches.len();
    let mut shown = 0usize;
    if let Some(catalog) = library {
        let begin = state.library_page.saturating_mul(PAGE_SIZE);
        for &index in state.library_matches.iter().skip(begin).take(PAGE_SIZE) {
            if let Some(archive) = catalog.archives.get(index) {
                rows = rows.push(archive_row(
                    index,
                    archive,
                    state.library_selection.indices.contains(&index),
                    state.preferences.ui_density == "Compact",
                ));
                shown += 1;
            }
        }
    }
    if matching == 0 {
        rows = rows.push(
            container(
                text(if state.library_startup_loading {
                    "Indexing imported PAK manifests from the detected manager Asset Library location…"
                } else if library.is_none() {
                    "No imported PAKs at the detected manager Asset Library location. Use Import PAKs to add an archive."
                } else if by_build && build.is_none() {
                    "No build snapshot is available. Use All Assets or import snapshot metadata."
                } else {
                    "No imported PAKs match this search."
                })
                .size(13)
                .style(design::muted),
            )
            .padding(18),
        );
    }
    let table_heading = container(
        row![
            header_cell("Dependency Group   ↟", 3),
            header_cell("Folder", 4),
            header_cell("Description", 5),
            header_cell("File", 4),
            header_cell("Extracted", 1),
        ]
        .spacing(9)
        .padding([10, 8]),
    )
    .style(|theme| container::Style {
        background: Some(Background::Color(design::field(theme))),
        ..container::Style::default()
    })
    .width(Fill);
    let pages = matching.div_ceil(PAGE_SIZE).max(1);
    let table = panel(
        column![table_heading, scrollable(rows).height(Fill),]
            .spacing(0)
            .height(Fill),
    );
    layout = layout.push(table.height(Fill));
    let (count, bytes, ignored) = library
        .map(|c| (c.archives.len(), c.total_bytes, c.ignored))
        .unwrap_or((0, 0, 0));
    layout = layout.push(row![
        text(format!(
            "{count} imported PAKs • {:.1} GiB manifest-listed extraction • read-only previews{}",
            bytes as f64 / 1_073_741_824.0,
            if ignored > 0 { format!(" • {ignored} invalid manifests skipped") } else { String::new() }
        )).size(12).style(design::muted).width(Fill),
        design::button("‹").on_press_maybe((state.library_page > 0).then_some(Message::LibraryPreviousPage)),
        text(format!("{} / {}  •  {} shown", state.library_page + 1, pages, shown)).size(12),
        design::button("›").on_press_maybe((state.library_page + 1 < pages).then_some(Message::LibraryNextPage)),
    ].spacing(10).align_y(iced::Center));
    layout = layout.push(row![
        text(format!("{} selected", state.library_selection.indices.len())).size(12).style(design::muted),
        design::button("Preview Files").style(design::secondary).on_press_maybe(selected.map(|_| Message::LibraryPreview)),
        design::button("Edit / Add to Mod Project")
            .style(design::primary)
            .on_press_maybe(selected.map(|_| Message::LibraryMakeModProject)),
        text(if selected.is_some() {
            "Shift: range (including across pages) • Ctrl: toggle. Editing creates a separate staged copy when required; imported originals stay read-only."
        } else {
            "Select an imported archive to preview its extracted files."
        }).size(12).style(design::muted),
    ].spacing(9).align_y(iced::Center));
    let x = match state.library_menu {
        Some("File") => 16.0,
        Some("View") => 80.0,
        Some("Tools") => 142.0,
        _ => 16.0,
    };
    let page = crate::menu_popup::overlay(
        layout.width(Fill).height(Fill),
        menu_details,
        x,
        127.0,
        280.0,
    );
    if !state.library_manage_builds_open {
        return page;
    }
    // A floating modal never reallocates space in the archive table.
    let backdrop = mouse_area(container(space().width(Fill).height(Fill)))
        .on_press(Message::LibraryManageBuildsToggle);
    let dialog = container(scrollable(manage_builds_dialog(state)).height(Length::Shrink))
        .padding(18)
        .width(Length::Fixed(820.0))
        .max_height(670)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });
    let center = container(dialog)
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill);
    stack![page, backdrop, center].into()
}

fn manage_builds_dialog(state: &State) -> Element<'_, Message> {
    let mut content = column![
        row![
            text("Manage Game Builds").size(21),
            space().width(Fill),
            design::button("Close").style(design::secondary)
                .on_press(Message::LibraryManageBuildsToggle),
        ].align_y(iced::Center).spacing(8),
        text("Rename registered builds, refresh imported PAKs or remove obsolete library snapshots. Original game installations and mod projects are never deleted.")
            .size(12).style(design::muted),
        row![
            text(format!("{} registered build(s)", state.library_managed_builds.len())).size(12),
            space().width(Fill),
            design::button("Import / Update Build…").style(design::secondary)
                .on_press_maybe((!state.busy && !state.library_import_scanning)
                    .then_some(Message::LibraryUpdatePickFolder)),
            design::button("Refresh").style(design::secondary)
                .on_press(Message::LibraryManageBuildsRefresh),
        ].align_y(iced::Center).spacing(8),
    ].spacing(12);
    if state.library_managed_builds.is_empty() {
        content = content.push(
            text("No managed incremental build snapshots found. Older standalone PAK imports remain available in Asset Library.")
                .size(12).style(design::muted),
        );
    }
    for build in &state.library_managed_builds {
        let editing = state.library_rename_build.as_deref() == Some(build.id.as_str());
        let deleting = state.library_delete_build_confirm.as_deref() == Some(build.id.as_str());
        let mut item = column![
            text(if build.display_name.is_empty() {
                build.source_root.clone()
            } else {
                build.display_name.clone()
            }).size(15),
            text(format!("Source: {}", build.source_root)).size(12).style(design::muted),
            text(format!(
                "{} PAKs • {:.2} GiB extracted • {} history ID(s) • {:.2} GiB potentially reclaimable",
                build.pak_count,
                build.extracted_bytes as f64 / 1_073_741_824.0,
                build.aliases,
                build.reclaimable_bytes as f64 / 1_073_741_824.0
            )).size(11).style(design::muted),
        ].spacing(5);
        if editing {
            item = item.push(
                row![
                    design::text_input("Build display name…", &state.library_rename_value)
                        .on_input(Message::LibraryRenameBuildValue)
                        .width(Fill),
                    design::button("Save Name")
                        .style(design::primary)
                        .on_press_maybe(
                            (!state.busy)
                                .then_some(Message::LibraryRenameBuildSave(build.id.clone()))
                        ),
                    design::button("Cancel").on_press(Message::LibraryRenameBuildCancel),
                ]
                .align_y(iced::Center)
                .spacing(7),
            );
        } else if deleting {
            item = item.push(column![
                text("Remove the imported snapshot history and only proven unshared extractions? Original Evolve files will remain untouched.").size(12),
                row![
                    design::button("Confirm Remove").style(design::primary)
                        .on_press_maybe((!state.busy).then_some(Message::LibraryDeleteBuildNow(build.id.clone()))),
                    design::button("Cancel").on_press(Message::LibraryDeleteBuildCancel),
                ].spacing(8),
            ].spacing(7));
        } else {
            item = item.push(
                row![
                    design::button("Rename")
                        .style(design::secondary)
                        .on_press_maybe(
                            (!state.busy)
                                .then_some(Message::LibraryRenameBuildBegin(build.id.clone()))
                        ),
                    design::button("Remove Imported Build")
                        .style(design::secondary)
                        .on_press_maybe(
                            (!state.busy)
                                .then_some(Message::LibraryDeleteBuildConfirm(build.id.clone()))
                        ),
                ]
                .spacing(8),
            );
        }
        content = content.push(panel(container(item).padding(12)));
    }
    content.into()
}
