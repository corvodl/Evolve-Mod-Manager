//! Native wgpu-backed desktop shell. Asset operations use blocking workers, so
//! file I/O/decompression never runs in the interactive Iced view/update path.
use evolve_core::{
    AssetEntry, CdfDefinition, DdsInfo, Inventory, PakInventory, export_dds_png,
    extract_legacy_pak, inspect_dds, inspect_legacy_pak, read_cdf, rebuild_legacy_pak, scan_directory,
};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Task, Theme};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Home,
    Files,
    AssetLibrary,
    Models,
    Levels,
    Settings,
}

impl Page {
    const ALL: [Self; 6] = [
        Self::Home,
        Self::Files,
        Self::AssetLibrary,
        Self::Models,
        Self::Levels,
        Self::Settings,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Files => "Modding",
            Self::AssetLibrary => "Asset Library",
            Self::Models => "Textures & Models",
            Self::Levels => "Level Editor",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Default)]
struct State {
    input: String,
    output: String,
    replacements: String,
    search: String,
    page: Option<Page>,
    inventory: Option<Inventory>,
    pak: Option<PakInventory>,
    texture: Option<DdsInfo>,
    character: Option<CdfDefinition>,
    selected: Option<usize>,
    status: String,
    busy: bool,
}

#[derive(Debug, Clone)]
enum Operation {
    Scan(PathBuf),
    InspectPak(PathBuf),
    InspectDds(PathBuf),
    InspectCdf(PathBuf),
    ExtractPak(PathBuf, PathBuf),
    RebuildPak(PathBuf, PathBuf, PathBuf),
    ExportPng(PathBuf, PathBuf),
}

#[derive(Debug, Clone)]
enum Completed {
    Inventory(Inventory),
    Pak(PakInventory),
    Texture(DdsInfo),
    Character(CdfDefinition),
    Extracted(usize, PathBuf),
    Rebuilt(usize, PathBuf),
    ExportedPng(DdsInfo, PathBuf),
}

impl Operation {
    fn execute(self) -> Result<Completed, String> {
        match self {
            Self::Scan(source) => scan_directory(&source)
                .map(Completed::Inventory)
                .map_err(|e| e.to_string()),
            Self::InspectPak(source) => inspect_legacy_pak(&source)
                .map(Completed::Pak)
                .map_err(|e| e.to_string()),
            Self::InspectDds(source) => inspect_dds(&source)
                .map(Completed::Texture)
                .map_err(|e| e.to_string()),
            Self::InspectCdf(source) => read_cdf(&source)
                .map(Completed::Character)
                .map_err(|e| e.to_string()),
            Self::ExtractPak(source, dest) => extract_legacy_pak(&source, &dest)
                .map(|pak| Completed::Extracted(pak.entries.len(), dest))
                .map_err(|e| e.to_string()),
            Self::RebuildPak(source, overrides, dest) => rebuild_legacy_pak(&source, &overrides, &dest)
                .map(|report| Completed::Rebuilt(report.changed_entries, dest))
                .map_err(|e| e.to_string()),
            Self::ExportPng(source, dest) => export_dds_png(&source, &dest)
                .map(|info| Completed::ExportedPng(info, dest))
                .map_err(|e| e.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    InputChanged(String),
    OutputChanged(String),
    ReplacementsChanged(String),
    SearchChanged(String),
    StartScan,
    StartPakInspection,
    StartDdsInspection,
    StartCdfInspection,
    StartExtract,
    StartRebuild,
    StartPngExport,
    Finished(Result<Completed, String>),
    Select(usize),
}

fn launch(state: &mut State, operation: Operation) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
    state.busy = true;
    state.status = "Working on explicitly selected local files...".into();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || operation.execute())
                .await
                .map_err(|e| format!("Worker failed: {e}"))?
        },
        Message::Finished,
    )
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => state.page = Some(page),
        Message::InputChanged(input) => state.input = input,
        Message::OutputChanged(output) => state.output = output,
        Message::ReplacementsChanged(replacements) => state.replacements = replacements,
        Message::SearchChanged(query) => state.search = query,
        Message::Select(index) => state.selected = Some(index),
        Message::StartScan => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::Scan(source));
        }
        Message::StartPakInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectPak(source));
        }
        Message::StartDdsInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectDds(source));
        }
        Message::StartCdfInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectCdf(source));
        }
        Message::StartExtract => {
            if state.output.trim().is_empty() {
                state.status = "Choose a NEW extraction directory first.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::ExtractPak(source, output));
        }
        Message::StartRebuild => {
            if state.output.trim().is_empty() || state.replacements.trim().is_empty() {
                state.status = "Choose the overrides folder and a NEW output PAK path.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let overrides = PathBuf::from(&state.replacements);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::RebuildPak(source, overrides, output));
        }
        Message::StartPngExport => {
            if state.output.trim().is_empty() {
                state.status = "Choose a NEW output PNG file first.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::ExportPng(source, output));
        }
        Message::Finished(result) => {
            state.busy = false;
            match result {
                Ok(Completed::Inventory(inventory)) => {
                    state.status = format!(
                        "{} files indexed; no originals modified.",
                        inventory.entries.len()
                    );
                    state.inventory = Some(inventory);
                    state.pak = None;
                    state.texture = None;
                    state.selected = None;
                    state.page = Some(Page::Files);
                }
                Ok(Completed::Pak(pak)) => {
                    state.status = format!(
                        "Validated ordinary ZIP PAK: {} members ({} bytes expanded).",
                        pak.entries.len(),
                        pak.total_bytes
                    );
                    state.pak = Some(pak);
                    state.selected = None;
                    state.page = Some(Page::Files);
                }
                Ok(Completed::Texture(texture)) => {
                    state.status = format!(
                        "DDS: {} × {} • {:?} • {} mipmaps • {} fragment(s)",
                        texture.width,
                        texture.height,
                        texture.format,
                        texture.mipmaps,
                        texture.streaming_parts
                    );
                    state.texture = Some(texture);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Character(character)) => {
                    state.status = format!(
                        "CDF: {} | {} validated skin/CHR attachment(s)",
                        character.skeleton,
                        character.attachments.len()
                    );
                    state.character = Some(character);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Extracted(count, folder)) => {
                    state.status = format!(
                        "Extracted {count} validated ZIP-compatible PAK entries to {}",
                        folder.display()
                    );
                }
                Ok(Completed::Rebuilt(count, target)) => {
                    state.status = format!(
                        "Rebuilt unsigned ZIP-compatible PAK from {count} replacement(s): {}",
                        target.display()
                    );
                }
                Ok(Completed::ExportedPng(info, target)) => {
                    state.status = format!(
                        "Exported {} × {} highest-resolution DDS mip to {}",
                        info.width,
                        info.height,
                        target.display()
                    );
                }
                Err(e) => state.status = format!("Operation refused: {e}"),
            }
        }
    }
    Task::none()
}

fn file_row<'a>(index: usize, entry: &'a AssetEntry) -> Element<'a, Message> {
    button(
        row![
            text(entry.kind.display()).width(130),
            text(entry.relative_path.as_str()).width(Fill),
            text(format!("{} B", entry.byte_size)),
        ]
        .spacing(12),
    )
    .width(Fill)
    .on_press(Message::Select(index))
    .into()
}

fn view(state: &State) -> Element<'_, Message> {
    let page = state.page.unwrap_or(Page::Home);
    let navigation = Page::ALL.into_iter().fold(
        column![text("EVOLVE").size(26), text("MOD MANAGER").size(14)].spacing(10),
        |content, item| {
            content.push(
                button(text(item.title()))
                    .width(Fill)
                    .on_press(Message::Navigate(item)),
            )
        },
    );
    let sidebar = container(navigation.padding(16).spacing(10)).width(205);
    let headline = column![
        text(page.title()).size(26),
        text("Rust + Iced / wgpu | development preview 0.1").size(13),
    ]
    .spacing(6);

    let location = column![
        row![
            text_input("Local asset folder, .pak or .dds path...", &state.input)
                .on_input(Message::InputChanged)
                .width(Fill),
            button("Scan folder").on_press(Message::StartScan),
        ]
        .spacing(8),
        row![
            button("Inspect ZIP PAK").on_press(Message::StartPakInspection),
            button("Inspect DDS").on_press(Message::StartDdsInspection),
            button("Inspect CDF").on_press(Message::StartCdfInspection),
        ]
        .spacing(8),
        row![
            text_input("New output directory or .png filename...", &state.output)
                .on_input(Message::OutputChanged)
                .width(Fill),
            button("Extract PAK").on_press(Message::StartExtract),
            button("Export DDS → PNG").on_press(Message::StartPngExport),
        ].spacing(8),
        row![
            text_input("Folder containing exact original PAK member replacements...", &state.replacements)
                .on_input(Message::ReplacementsChanged).width(Fill),
            button("Rebuild unsigned PAK").on_press(Message::StartRebuild),
        ]
        .spacing(8),
    ]
    .spacing(9);

    let workspace: Element<'_, Message> = match page {
        Page::Home => column![
            text("Inspect local CryEngine assets safely.").size(19),
            text("ZIP-compatible 2012 PAKs can be inspected, extracted and rebuilt from exact replacements; signed Stage 2 CryPAKs are NOT supported yet."),
            text("DDS top-mipmap PNG export supports BC1/2/3/4/5 and 32-bit RGB(A); complete split-stream sets are reconstructed first."),
            text("Mod installation, 3D scenes and Blender FBX export still need porting."),
        ].spacing(14).into(),
        Page::Files | Page::AssetLibrary => {
            let mut rows = column![].spacing(3);
            let needle = state.search.to_lowercase();
            let selected_name;
            if let Some(pak) = &state.pak {
                for (index, entry) in pak.entries.iter().enumerate()
                    .filter(|(_, e)| e.path.to_lowercase().contains(&needle))
                    .take(350)
                {
                    rows = rows.push(button(row![
                        text(entry.path.as_str()).width(Fill),
                        text(format!("{} B", entry.size)),
                    ].spacing(8)).width(Fill).on_press(Message::Select(index)));
                }
                selected_name = state.selected
                    .and_then(|n| pak.entries.get(n))
                    .map(|e| format!("Member: {} / {} bytes / ZIP method {}", e.path, e.size, e.method))
                    .unwrap_or_else(|| format!("Archive: {}", pak.source_file));
            } else if let Some(inventory) = &state.inventory {
                for (index, entry) in inventory.entries.iter().enumerate()
                    .filter(|(_, e)| e.relative_path.to_lowercase().contains(&needle))
                    .take(350)
                {
                    rows = rows.push(file_row(index, entry));
                }
                selected_name = state.selected
                    .and_then(|n| inventory.entries.get(n))
                    .map(|e| format!("Asset: {} / {} bytes", e.relative_path, e.byte_size))
                    .unwrap_or_else(|| format!("Local directory: {}", inventory.root));
            } else {
                selected_name = "Scan a folder or inspect a ZIP-compatible PAK to see files.".to_owned();
            }
            column![
                text_input("Search indexed filenames...", &state.search)
                    .on_input(Message::SearchChanged),
                text(selected_name).size(14),
                scrollable(rows).height(Fill),
                text("Showing up to 350 matching rows; data remains available for filtering.").size(12),
            ].spacing(10).into()
        }
        Page::Models => {
            let description = if let Some(texture) = &state.texture {
                format!(
                    "DDS: {} × {} | {:?} | {} mip(s) | {} part(s) | top mip {} bytes",
                    texture.width, texture.height, texture.format,
                    texture.mipmaps, texture.streaming_parts, texture.top_mip_bytes
                )
            } else {
                "Select a local DDS and choose Inspect DDS.".to_owned()
            };
            column![
                text("Texture inspection and original top-mip export").size(19),
                text(description),
                text("PNG export preserves BC5 channels without pretending to reconstruct normal-map shading."),
                text(if let Some(character) = &state.character {
                    format!("CDF skeleton: {} | {} real skin/CHR attachment(s).",
                        character.skeleton, character.attachments.len())
                } else {
                    "CDF: select a local character definition and choose Inspect CDF.".to_owned()
                }),
                text("GPU character viewer, skeletal animation preview and complete FBX conversion remain unported."),
            ].spacing(14).into()
        }
        Page::Levels => column![
            text("Level Editor").size(19),
            text("CryEngine terrain, placement and material dependency parsing, interactive wgpu scenes, and Blender scene export are pending."),
        ].spacing(14).into(),
        Page::Settings => column![
            text("Workspace safety").size(19),
            text("The ZIP-compatible PAK extractor and rebuilder never overwrite existing destinations and stage outputs before publishing them."),
            text("Only a directory you explicitly enter can receive extraction files. Stage 2 signed PAK import/mod injection are disabled."),
            text("All expensive file operations run outside the UI thread."),
        ].spacing(14).into(),
    };

    let body = column![
        headline,
        location,
        text(if state.busy {
            "Working..."
        } else {
            state.status.as_str()
        })
        .size(13),
        workspace,
    ]
    .spacing(16)
    .padding(20)
    .width(Fill)
    .height(Fill);
    container(row![sidebar, body].height(Fill))
        .width(Fill)
        .height(Fill)
        .into()
}

fn main() -> iced::Result {
    let _supported_backends = wgpu::Backends::PRIMARY;
    iced::application(State::default, update, view)
        .title("Evolve Mod Manager — Rust Development Preview")
        .theme(|_state: &State| Theme::Dark)
        .run()
}
