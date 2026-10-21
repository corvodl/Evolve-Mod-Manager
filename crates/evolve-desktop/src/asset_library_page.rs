//! Private-manager-style imported PAK browser. Original archives and
//! extracted Asset Library files remain read-only; mod editing lives elsewhere.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{column, container, row, scrollable, space, text};
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
        text("Preview and export PAKs you have already extracted. The Asset Library is read-only; editing, building and signing are performed in Modding.")
            .size(12).style(design::muted),
        container(menu).padding([4, 8]).width(Fill).style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            ..container::Style::default()
        }),
        row![
            design::button("Import PAKs").style(design::primary)
                .on_press(Message::LibraryToggleConfiguration),
            design::button("Library Settings").style(design::secondary)
                .on_press(Message::OpenLibrarySettings),
            design::button("Refresh").on_press(Message::LibraryRefresh),
            space().width(Fill),
            text("◈").size(18).style(design::muted),
        ].spacing(9),
    ].spacing(10);

    if state.library_show_configuration {
        layout = layout.push(panel(container(column![
            row![
                text("IMPORT SIGNED STAGE 2 PAKS").size(14),
                space().width(Fill),
                design::button("Close").on_press(Message::LibraryToggleConfiguration).style(design::text_button),
            ].spacing(8),
            text("The Asset Library location is detected automatically. Select one signed .pak or a directory containing up to 64 .pak files. Imports authenticate each PAK using the bundled original public RSA key and any configured original/custom public keys. Original archives are not modified.")
                .size(12).style(design::muted),
            row![
                design::text_input("Stage 2 .pak file or folder path...", &state.library_import_source)
                    .on_input(Message::LibraryImportSourceChanged).width(Fill),
                design::button("Import selected PAK(s)").style(design::primary)
                    .on_press(Message::LibraryImportStart),
            ].spacing(8),
            row![
                design::button("Manage storage location in Settings").on_press(Message::OpenLibrarySettings).style(design::text_button),
                design::button("Advanced archive tools in Modding").on_press(Message::Navigate(Page::Files)).style(design::text_button),
            ].spacing(8),
        ].spacing(8)).padding(12)));
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
        text(if by_build {
            "Imported archives associated with the selected build snapshot"
        } else {
            "All imported archives across snapshots"
        })
        .size(12)
        .style(design::muted),
    );
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
        design::button("Preview Files").on_press_maybe(selected.map(|_| Message::LibraryPreview)),
        design::button("Edit / Add to Mod Project")
            .on_press_maybe(selected.map(|_| Message::LibraryMakeModProject)),
        text(if selected.is_some() {
            "Shift: range (including across pages) • Ctrl: toggle. Editing requires an exact staged PAK; the Library remains read-only."
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
    crate::menu_popup::overlay(
        layout.width(Fill).height(Fill),
        menu_details,
        x,
        127.0,
        280.0,
    )
}
