//! Private-manager-style imported PAK browser. Original archives and
//! extracted Asset Library files remain read-only; mod editing lives elsewhere.
use super::{Message, Page, State};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Length};
use std::path::Path;

const PANEL: Color = Color::from_rgb8(31, 30, 36);
const HEADER: Color = Color::from_rgb8(35, 34, 41);
const EDGE: Color = Color::from_rgb8(48, 45, 53);
const MUTED: Color = Color::from_rgb8(181, 178, 188);
const PAGE_SIZE: usize = 110;

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content).width(Fill).style(|_| container::Style {
        background: Some(Background::Color(PANEL)),
        border: Border {
            color: EDGE,
            width: 1.0,
            radius: 5.0.into(),
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
    let item = button(content)
        .padding([7, 8])
        .width(Fill)
        .style(button::text)
        .on_press(Message::LibrarySelect(index));
    container(item)
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(if selected {
                Color::from_rgb8(65, 38, 48)
            } else if index.is_multiple_of(2) {
                PANEL
            } else {
                Color::from_rgb8(34, 33, 40)
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
        button("File")
            .on_press(Message::LibraryMenu("File"))
            .style(button::text),
        button("View")
            .on_press(Message::LibraryMenu("View"))
            .style(button::text),
        button("Tools")
            .on_press(Message::LibraryMenu("Tools"))
            .style(button::text),
    ]
    .spacing(8);
    let menu_details: Element<'_, Message> = match state.library_menu {
        Some("File") => row![
            button("Refresh catalog").on_press(Message::LibraryRefresh),
            button("Import signed PAK…").on_press(Message::LibraryToggleConfiguration),
            button("Storage settings").on_press(Message::Navigate(Page::Settings)),
        ]
        .spacing(8)
        .into(),
        Some("View") => row![
            button("All Assets").on_press(Message::LibrarySetByBuild(false)),
            button("By Build").on_press(Message::LibrarySetByBuild(true)),
        ]
        .spacing(8)
        .into(),
        Some("Tools") => row![
            button("Preview selected PAK").on_press(Message::LibraryPreview),
            button("Create editable project").on_press(Message::LibraryMakeModProject),
            button("Open Modding").on_press(Message::Navigate(Page::Files)),
        ]
        .spacing(8)
        .into(),
        _ => text("").into(),
    };

    let mut layout = column![
        text("Asset Library").size(24),
        text("Preview and export PAKs you have already extracted. The Asset Library is read-only; editing, building and signing are performed in Modding.")
            .size(12).color(MUTED),
        container(menu).padding([4, 8]).width(Fill).style(|_| container::Style {
            background: Some(Background::Color(HEADER)),
            ..container::Style::default()
        }),
        menu_details,
        row![
            button("Import PAKs").style(button::primary)
                .on_press(Message::LibraryToggleConfiguration),
            button("Refresh").on_press(Message::LibraryRefresh),
            space().width(Fill),
            text("◈").size(18).color(MUTED),
        ].spacing(9),
    ].spacing(10);

    if state.library_show_configuration {
        layout = layout.push(panel(container(column![
            row![
                text("IMPORT SIGNED STAGE 2 PAK").size(14),
                space().width(Fill),
                button("Close").on_press(Message::LibraryToggleConfiguration).style(button::text),
            ].spacing(8),
            text("The Asset Library location is detected automatically. Imports use the configured setup's local verification key; original PAK files are not modified.")
                .size(12).color(MUTED),
            row![
                text_input("Full path to a local Stage 2 .pak...", &state.library_import_source)
                    .on_input(Message::LibraryImportSourceChanged).width(Fill),
                button("Import signed PAK").style(button::primary)
                    .on_press(Message::LibraryImportStart),
            ].spacing(8),
            row![
                button("Manage storage location in Settings").on_press(Message::Navigate(Page::Settings)).style(button::text),
                button("Advanced archive tools in Modding").on_press(Message::Navigate(Page::Files)).style(button::text),
            ].spacing(8),
        ].spacing(8)).padding(12)));
    }

    let by_build = state.library_by_build;
    let build = library.and_then(|c| c.builds.get(state.library_build_index));
    let tabs = row![
        button("All Assets")
            .style(if !by_build {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::LibrarySetByBuild(false)),
        button("By Build")
            .style(if by_build {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::LibrarySetByBuild(true)),
        space().width(Fill),
        if by_build {
            button(text(build.map_or("No build snapshots".to_owned(), |s| {
                format!("Build: {s}  ▾")
            })))
            .on_press(Message::LibraryNextBuild)
        } else {
            button("All imported PAKs").on_press(Message::LibrarySetByBuild(false))
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
        .color(MUTED),
    );
    layout = layout.push(
        row![
            text_input(
                "Search imported PAKs, names, folders...",
                &state.library_search
            )
            .on_input(Message::LibrarySearchChanged)
            .width(Fill),
            button("Refresh").on_press(Message::LibraryRefresh),
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
                ));
                shown += 1;
            }
        }
    }
    if matching == 0 {
        rows = rows.push(
            container(
                text(if library.is_none() {
                    "No imported PAKs at the detected manager Asset Library location. Use Import PAKs to add an archive."
                } else if by_build && build.is_none() {
                    "No build snapshot is available. Use All Assets or import snapshot metadata."
                } else {
                    "No imported PAKs match this search."
                })
                .size(13)
                .color(MUTED),
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
    .style(|_| container::Style {
        background: Some(Background::Color(HEADER)),
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
        )).size(12).color(MUTED).width(Fill),
        button("‹").on_press_maybe((state.library_page > 0).then_some(Message::LibraryPreviousPage)),
        text(format!("{} / {}  •  {} shown", state.library_page + 1, pages, shown)).size(12),
        button("›").on_press_maybe((state.library_page + 1 < pages).then_some(Message::LibraryNextPage)),
    ].spacing(10).align_y(iced::Center));
    layout = layout.push(row![
        text(format!("{} selected", state.library_selection.indices.len())).size(12).color(MUTED),
        button("Preview Files").on_press_maybe(selected.map(|_| Message::LibraryPreview)),
        button("Edit / Add to Mod Project")
            .on_press_maybe(selected.map(|_| Message::LibraryMakeModProject)),
        text(if selected.is_some() {
            "Shift: range (including across pages) • Ctrl: toggle. Editing requires an exact staged PAK; the Library remains read-only."
        } else {
            "Select an imported archive to preview its extracted files."
        }).size(12).color(MUTED),
    ].spacing(9).align_y(iced::Center));
    layout.width(Fill).height(Fill).into()
}
