//! Native desktop shell. GPU-powered Iced UI; custom CryEngine scene rendering
//! will be added once format parity and asset safety checks are complete.
use evolve_core::{AssetEntry, Inventory, scan_directory};
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
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
        Self::Home, Self::Files, Self::AssetLibrary,
        Self::Models, Self::Levels, Self::Settings,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Files => "Modding",
            Self::AssetLibrary => "Asset Library",
            Self::Models => "Models & Exports",
            Self::Levels => "Level Editor",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Default)]
struct State {
    input: String,
    search: String,
    page: Option<Page>,
    inventory: Option<Inventory>,
    selected: Option<usize>,
    status: String,
    scanning: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    InputChanged(String),
    SearchChanged(String),
    Scan,
    Scanned(Result<Inventory, String>),
    Select(usize),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => state.page = Some(page),
        Message::InputChanged(input) => state.input = input,
        Message::SearchChanged(query) => state.search = query,
        Message::Select(index) => state.selected = Some(index),
        Message::Scan if !state.scanning => {
            state.scanning = true;
            state.selected = None;
            let root = PathBuf::from(state.input.trim());
            return Task::perform(async move { scan_directory(&root).map_err(|err| err.to_string()) }, Message::Scanned);
        }
        Message::Scan => {}
        Message::Scanned(result) => {
            state.scanning = false;
            match result {
                Ok(inventory) => {
                    state.status = format!("{} local files indexed without modification.", inventory.entries.len());
                    state.inventory = Some(inventory);
                    state.page = Some(Page::Files);
                }
                Err(reason) => state.status = format!("Scan failed: {reason}"),
            }
        }
    }
    Task::none()
}

fn asset_row<'a>(index: usize, entry: &'a AssetEntry) -> Element<'a, Message> {
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
        column![text("EVOLVE").size(26), text("MOD MANAGER").size(14)].spacing(12),
        |content, item| content.push(
            button(text(item.title()))
                .width(Fill)
                .on_press(Message::Navigate(item)),
        ),
    );
    let sidebar = container(navigation.spacing(10).padding(16)).width(205);

    let headline = column![
        text(page.title()).size(26),
        text("Rust + Iced / wgpu foundation | development preview").size(13),
    ]
    .spacing(6);

    let location = row![
        text_input("Enter a local extracted PAK or game asset folder...", &state.input)
            .on_input(Message::InputChanged)
            .width(Fill),
        button("Scan directory").on_press(Message::Scan),
    ]
    .spacing(8);

    let workspace: Element<'_, Message> = match page {
        Page::Home => column![
            text("Local game assets stay on this computer.").size(19),
            text("Start by scanning a directory. This preview inventories files; it does not modify PAKs, install mods, or claim complete export parity."),
            text("Production 1.0 is gated on feature-parity tests and real Windows builds."),
        ].spacing(14).into(),
        Page::Files | Page::AssetLibrary => {
            let mut items = column![].spacing(3);
            if let Some(inventory) = &state.inventory {
                let needle = state.search.to_lowercase();
                for (index, entry) in inventory.entries.iter().enumerate()
                    .filter(|(_, entry)| entry.relative_path.to_lowercase().contains(&needle))
                    .take(350)
                {
                    items = items.push(asset_row(index, entry));
                }
            }
            let details = state.inventory.as_ref()
                .and_then(|i| state.selected.and_then(|n| i.entries.get(n)))
                .map(|e| format!("Selected: {} | {} | {} bytes", e.relative_path, e.kind.display(), e.byte_size))
                .unwrap_or_else(|| "Select an indexed asset to inspect its type and size.".to_owned());
            column![
                text_input("Filter indexed files...", &state.search).on_input(Message::SearchChanged),
                text(details).size(14),
                scrollable(items).height(Fill),
                text("Preview shows up to 350 matching rows; the full inventory remains in memory.").size(12),
            ].spacing(10).into()
        }
        Page::Models => column![
            text("Model export parity").size(19),
            text("Planned: CDF assemblies, rigged FBX, highest-resolution DDS, UV0, 1P/3P equipment and monster stage scenes."),
            text("Verified animation Actions will be optional and disabled by default. Native FBX conversion is NOT available in this preview."),
        ].spacing(14).into(),
        Page::Levels => column![
            text("Level Editor").size(19),
            text("CryEngine map/terrain parsing, dependency resolution, GPU visualization, and Blender scene export are pending."),
        ].spacing(14).into(),
        Page::Settings => column![
            text("Safety policy").size(19),
            text("No writes to installed game files. No network updates or hidden background import. No game keys shipped."),
            text("Renderer: Iced 0.14 with the wgpu 27 backend; the 3D pipeline is not yet implemented."),
        ].spacing(14).into(),
    };

    let body = column![
        headline,
        location,
        text(if state.scanning { "Scanning..." } else { state.status.as_str() }).size(13),
        workspace,
        Space::new().height(0),
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
    // wgpu 27 is intentionally kept aligned with Iced's GPU renderer.
    let _supported_backends = wgpu::Backends::PRIMARY;
    iced::application(State::default, update, view)
        .title("Evolve Mod Manager — Rust Development Preview")
        .theme(|_state: &State| Theme::Dark)
        .run()
}
