//! The Level Editor's map chooser: search, filters and recents above card
//! grids for the current map, editable projects and imported archives.
//! Maps and their placement previews are indexed on bounded background workers.

use crate::level_library::MapCandidate;
use crate::ui_kit::BadgeKind;
use crate::{Message, State, design, icons};
use evolve_core::ImportedPak;
use iced::widget::{column, container, image, mouse_area, row, scrollable, space, stack, text};
use iced::{Background, Border, Element, Fill, Length, Theme};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Filter chips above the cards.
pub const FILTERS: [&str; 3] = ["Projects", "Archives", "Legacy-ported"];
/// Recent maps kept in preferences.
pub const MAX_RECENT: usize = 5;
/// Load stages shown while a map opens, in order.
pub const LOAD_STEPS: [&str; 4] = ["Discover", "Read layers", "Build terrain", "Assemble scene"];
/// The load stage the worker reached (an index into [`LOAD_STEPS`]).
pub static LOAD_STEP: AtomicUsize = AtomicUsize::new(0);
/// Columns in the last drawn grid, so arrow keys move by rows.
static COLUMNS: AtomicUsize = AtomicUsize::new(3);
const THUMB_SIZE: u32 = 160;

#[derive(Debug, Clone)]
pub enum Msg {
    Search(String),
    Filter(&'static str),
    /// Arrow keys: move the focused card by this many places.
    Move(i32),
    /// Up/Down: move by one row.
    MoveRow(i32),
    OpenFocused,
    ThumbsLoaded(Vec<(String, Vec<u8>)>),
    IndexedArchive(
        u64,
        String,
        Result<(ImportedPak, Vec<MapCandidate>), String>,
    ),
    IndexedProject(u64, String, Result<Vec<MapCandidate>, String>),
    PreviewReady(u64, String, Option<Vec<u8>>),
    OpenIndexedArchive(String, String),
    OpenIndexedProject(String, String),
    Lightbox(Option<String>),
    OpenRecent(usize),
}

#[derive(Debug, Default)]
pub struct ChooserState {
    pub search: String,
    pub filters: std::collections::BTreeSet<&'static str>,
    pub focus: Option<usize>,
    pub thumbs: HashMap<String, image::Handle>,
    thumbs_loaded: bool,
    thumbs_loading: bool,
    pub lightbox: Option<String>,
    /// A map to open as soon as its source's map list arrives (recents).
    pub pending_map: Option<String>,
    archives: HashMap<String, Option<(ImportedPak, Vec<MapCandidate>)>>,
    projects: HashMap<String, Option<Vec<MapCandidate>>>,
    indexing: bool,
    previewing: bool,
    preview_done: HashSet<String>,
    source_signature: Vec<String>,
    generation: u64,
}

/// One card in the chooser.
pub struct Card {
    pub section: &'static str,
    pub title: String,
    pub subtitle: String,
    pub badges: Vec<(BadgeKind, String)>,
    pub thumb: Option<String>,
    pub message: Message,
    pub current: bool,
    kind: &'static str,
    legacy: bool,
}

/// File-name-safe key for a map's thumbnail.
pub fn thumb_key(source: &str, map: &str) -> String {
    // Version the cache: old entries contained terrain alone, not placements.
    let raw = format!("scene2_{source}__{map}");
    let prefix: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(108)
        .collect();
    let hash = Sha256::digest(raw.as_bytes());
    let hash: String = hash[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix}_{hash}")
}

pub fn thumbs_dir() -> Option<PathBuf> {
    crate::settings::settings_path()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("MapThumbnails")))
}

/// Reads at most 512 current-version thumbnails (each at most 512 KiB).
pub fn load_thumbs_task() -> iced::Task<Message> {
    iced::Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                let Some(dir) = thumbs_dir() else {
                    return Vec::new();
                };
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return Vec::new();
                };
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        (path.extension()? == "png").then_some(())?;
                        let key = path.file_stem()?.to_string_lossy().into_owned();
                        key.starts_with("scene2_").then_some(())?;
                        let metadata = std::fs::symlink_metadata(&path).ok()?;
                        (metadata.file_type().is_file() && metadata.len() <= 512 * 1024)
                            .then_some(())?;
                        Some((key, std::fs::read(&path).ok()?))
                    })
                    .take(512)
                    .collect()
            })
            .await
            .unwrap_or_default()
        },
        |thumbs| Message::Map(Msg::ThumbsLoaded(thumbs)),
    )
}

/// Saves a thumbnail under each key; failures only lose the preview.
pub fn save_thumb(png: &[u8], keys: &[String]) {
    let Some(dir) = thumbs_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for key in keys {
        let _ = std::fs::write(dir.join(format!("{key}.png")), png);
    }
}

/// A top-down hillshade of the ground mesh: brighter is higher, with light
/// from the north-west. Returns PNG bytes.
pub fn terrain_thumbnail(positions: &[[f32; 3]]) -> Option<Vec<u8>> {
    if positions.len() < 4 {
        return None;
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in positions {
        for axis in 0..3 {
            if point[axis].is_finite() {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
    }
    let span = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    // Terrain is wide and flat: the axis with the smallest span is up.
    let up = (0..3)
        .min_by(|a, b| span[*a].total_cmp(&span[*b]))
        .unwrap_or(2);
    let (u, v) = match up {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    if span[u] <= 0.0 || span[v] <= 0.0 {
        return None;
    }
    let size = THUMB_SIZE as usize;
    let mut height = vec![f32::NAN; size * size];
    for point in positions {
        let x = ((point[u] - min[u]) / span[u] * (size - 1) as f32).round() as usize;
        let y = ((point[v] - min[v]) / span[v] * (size - 1) as f32).round() as usize;
        let cell = &mut height[(size - 1 - y.min(size - 1)) * size + x.min(size - 1)];
        if cell.is_nan() || point[up] > *cell {
            *cell = point[up];
        }
    }
    let low = min[up];
    let range = span[up].max(1e-3);
    let sample = |x: usize, y: usize| height[y * size + x];
    let mut pixels = ::image::RgbaImage::new(THUMB_SIZE, THUMB_SIZE);
    for y in 0..size {
        for x in 0..size {
            let h = sample(x, y);
            if h.is_nan() {
                pixels.put_pixel(x as u32, y as u32, ::image::Rgba([0, 0, 0, 0]));
                continue;
            }
            let neighbour = sample(x.saturating_sub(1), y.saturating_sub(1));
            let slope = if neighbour.is_nan() {
                0.0
            } else {
                (h - neighbour) / range
            };
            let level = (h - low) / range;
            let shade = (0.25 + 0.6 * level + 6.0 * slope).clamp(0.05, 1.0);
            let value = (shade * 255.0) as u8;
            pixels.put_pixel(
                x as u32,
                y as u32,
                ::image::Rgba([
                    value,
                    (value as f32 * 0.96) as u8,
                    (value as f32 * 0.88) as u8,
                    255,
                ]),
            );
        }
    }
    let mut png = Vec::new();
    ::image::DynamicImage::ImageRgba8(pixels)
        .write_to(
            &mut std::io::Cursor::new(&mut png),
            ::image::ImageFormat::Png,
        )
        .ok()?;
    Some(png)
}

fn map_badges(map: &MapCandidate) -> Vec<(BadgeKind, String)> {
    let mut badges = Vec::new();
    if map.legacy_ported {
        badges.push((BadgeKind::Nightly, "Legacy-ported".into()));
    } else if map.compiled_terrain.is_some() && map.terrain.is_none() {
        badges.push((BadgeKind::Neutral, "Compiled source".into()));
    }
    if map.terrain.is_some() || map.compiled_terrain.is_some() {
        badges.push((BadgeKind::Neutral, "Terrain".into()));
    }
    if map.vegetation.is_some() {
        badges.push((BadgeKind::Neutral, "Vegetation".into()));
    }
    if !map.layers.is_empty() {
        badges.push((
            BadgeKind::Neutral,
            format!(
                "{} layer{}",
                map.layers.len(),
                if map.layers.len() == 1 { "" } else { "s" }
            ),
        ));
    }
    badges
}

fn map_card(
    section: &'static str,
    source_key: &str,
    map: &MapCandidate,
    message: Message,
    current: bool,
) -> Card {
    Card {
        section,
        title: map.name.clone(),
        subtitle: String::new(),
        badges: map_badges(map),
        thumb: Some(thumb_key(source_key, &map.name)),
        message,
        current,
        kind: "map",
        legacy: map.legacy_ported,
    }
}

/// Every card, in section order, after search and filters.
pub fn cards(state: &State) -> Vec<Card> {
    let mut cards = Vec::new();
    if let Some((project, maps)) = &state.level_project_choices {
        let source = project.workspace.display().to_string();
        for (index, map) in maps.iter().enumerate() {
            cards.push(map_card(
                "Maps in project",
                &source,
                map,
                Message::LevelChooseProjectMap(index),
                false,
            ));
        }
    } else if let Some((archive, maps)) = &state.level_map_choices {
        for (index, map) in maps.iter().enumerate() {
            cards.push(map_card(
                "Maps in archive",
                &archive.id,
                map,
                Message::LevelChooseMap(index),
                false,
            ));
        }
    } else {
        if let Some((archive, maps, current)) = &state.library_map {
            for (index, map) in maps.iter().enumerate() {
                cards.push(map_card(
                    "Current",
                    &archive.id,
                    map,
                    Message::LevelMapSelect(index),
                    index == *current,
                ));
            }
        }
        if let Some(projects) = &state.modding_projects {
            for (index, project) in projects.iter().enumerate() {
                let source = project.workspace.display().to_string();
                if let Some(Some(maps)) = state.map_chooser.projects.get(&source) {
                    for map in maps {
                        let mut card = map_card(
                            "Editable maps",
                            &source,
                            map,
                            Message::Map(Msg::OpenIndexedProject(source.clone(), map.name.clone())),
                            false,
                        );
                        card.subtitle = project.label.clone();
                        card.kind = "project-map";
                        cards.push(card);
                    }
                    continue;
                }
                cards.push(Card {
                    section: "Editable projects",
                    title: project.label.clone(),
                    subtitle: project.target.clone(),
                    badges: vec![(BadgeKind::Enabled, "Editable".into())],
                    thumb: None,
                    message: Message::LevelChooseProject(index),
                    current: false,
                    kind: "project",
                    legacy: false,
                });
            }
        }
        if let Some(catalog) = &state.library_catalog {
            let mut shown_sources = HashSet::new();
            for (index, archive) in catalog.archives.iter().enumerate() {
                if !crate::level_library::likely_level_pak(archive) {
                    continue;
                }
                if let Some(Some((owner, maps))) = state.map_chooser.archives.get(&archive.id) {
                    if !shown_sources.insert(owner.id.clone()) {
                        continue;
                    }
                    for map in maps {
                        let mut card = map_card(
                            "Imported maps",
                            &owner.id,
                            map,
                            Message::Map(Msg::OpenIndexedArchive(
                                archive.id.clone(),
                                map.name.clone(),
                            )),
                            false,
                        );
                        card.subtitle = format!("{} · {}", archive.name, archive.builds.join(", "));
                        card.kind = "archive-map";
                        cards.push(card);
                    }
                    continue;
                }
                cards.push(Card {
                    section: "Imported archives",
                    title: archive.name.clone(),
                    subtitle: archive.builds.join(", "),
                    badges: vec![(BadgeKind::ReadOnly, "Read-only".into())],
                    thumb: None,
                    message: Message::LevelChooseArchive(index),
                    current: false,
                    kind: "archive",
                    legacy: false,
                });
            }
        }
    }
    let query = state.map_chooser.search.trim().to_ascii_lowercase();
    let filters = &state.map_chooser.filters;
    cards.retain(|card| {
        let text_ok = query.is_empty()
            || card.title.to_ascii_lowercase().contains(&query)
            || card.subtitle.to_ascii_lowercase().contains(&query);
        let filter_ok = filters.is_empty()
            || (filters.contains("Projects") && matches!(card.kind, "project" | "project-map"))
            || (filters.contains("Archives") && matches!(card.kind, "archive" | "archive-map"))
            || (filters.contains("Legacy-ported") && card.legacy);
        text_ok && filter_ok
    });
    cards
}

pub fn update(state: &mut State, message: Msg) -> iced::Task<Message> {
    match message {
        Msg::Search(value) => {
            state.map_chooser.search = value;
            state.map_chooser.focus = None;
        }
        Msg::Filter(filter) => {
            if !state.map_chooser.filters.remove(filter) {
                state.map_chooser.filters.insert(filter);
            }
            state.map_chooser.focus = None;
        }
        Msg::Move(delta) | Msg::MoveRow(delta) => {
            let step = if matches!(message, Msg::MoveRow(_)) {
                delta * COLUMNS.load(Ordering::Relaxed).max(1) as i32
            } else {
                delta
            };
            let count = cards(state).len();
            if count > 0 {
                let next = state
                    .map_chooser
                    .focus
                    .map_or(0, |focus| (focus as i32 + step).clamp(0, count as i32 - 1));
                state.map_chooser.focus = Some(next as usize);
            }
        }
        Msg::OpenFocused => {
            if state.busy {
                return iced::Task::none();
            }
            let focus = state.map_chooser.focus;
            if let Some(card) = focus.and_then(|index| cards(state).into_iter().nth(index)) {
                state.map_chooser.focus = None;
                return crate::update(state, card.message);
            }
        }
        Msg::ThumbsLoaded(thumbs) => {
            state.map_chooser.thumbs_loaded = true;
            state.map_chooser.thumbs_loading = false;
            for (key, png) in thumbs {
                state
                    .map_chooser
                    .thumbs
                    .insert(key, image::Handle::from_bytes(png));
            }
            return ensure_index(state);
        }
        Msg::IndexedArchive(generation, id, result) => {
            if generation != state.map_chooser.generation {
                state.map_chooser.indexing = false;
                return ensure_index(state);
            }
            state.map_chooser.indexing = false;
            state.map_chooser.archives.insert(id, result.ok());
            state.map_chooser.focus = None;
            return ensure_index(state);
        }
        Msg::IndexedProject(generation, path, result) => {
            if generation != state.map_chooser.generation {
                state.map_chooser.indexing = false;
                return ensure_index(state);
            }
            state.map_chooser.indexing = false;
            state.map_chooser.projects.insert(path, result.ok());
            state.map_chooser.focus = None;
            return ensure_index(state);
        }
        Msg::PreviewReady(generation, key, png) => {
            if generation != state.map_chooser.generation {
                state.map_chooser.previewing = false;
                return ensure_index(state);
            }
            state.map_chooser.previewing = false;
            state.map_chooser.preview_done.insert(key.clone());
            if let Some(png) = png.filter(|_| !state.map_chooser.thumbs.contains_key(&key)) {
                save_thumb(&png, std::slice::from_ref(&key));
                state
                    .map_chooser
                    .thumbs
                    .insert(key, image::Handle::from_bytes(png));
            }
            return ensure_index(state);
        }
        Msg::OpenIndexedArchive(id, name) => {
            if state.busy {
                return iced::Task::none();
            }
            if let Some(index) = state
                .library_catalog
                .as_ref()
                .and_then(|catalog| catalog.archives.iter().position(|a| a.id == id))
            {
                state.map_chooser.pending_map = Some(name);
                return crate::update(state, Message::LevelChooseArchive(index));
            }
        }
        Msg::OpenIndexedProject(path, name) => {
            if state.busy {
                return iced::Task::none();
            }
            if let Some(index) = state.modding_projects.as_ref().and_then(|projects| {
                projects
                    .iter()
                    .position(|p| p.workspace.display().to_string() == path)
            }) {
                state.map_chooser.pending_map = Some(name);
                return crate::update(state, Message::LevelChooseProject(index));
            }
        }
        Msg::Lightbox(key) => state.map_chooser.lightbox = key,
        Msg::OpenRecent(index) => {
            let Some(recent) = state.preferences.recent_maps.get(index).cloned() else {
                return iced::Task::none();
            };
            if state.busy {
                return iced::Task::none();
            }
            if !recent.project.is_empty() {
                let project = state.modding_projects.as_ref().and_then(|projects| {
                    projects
                        .iter()
                        .position(|p| p.workspace.display().to_string() == recent.project)
                });
                if let Some(index) = project {
                    state.map_chooser.pending_map = Some(recent.map);
                    return crate::update(state, Message::LevelChooseProject(index));
                }
            }
            let archive = state.library_catalog.as_ref().and_then(|catalog| {
                catalog
                    .archives
                    .iter()
                    .position(|a| a.id == recent.archive_id)
            });
            match archive {
                Some(index) => {
                    state.map_chooser.pending_map = Some(recent.map);
                    return crate::update(state, Message::LevelChooseArchive(index));
                }
                None => {
                    state.status = format!(
                        "{} is no longer available. Import its archive again or reopen the project.",
                        recent.label
                    );
                }
            }
        }
    }
    iced::Task::none()
}

/// Loads saved thumbnails the first time the chooser opens.
pub fn ensure_thumbs(state: &mut State) -> iced::Task<Message> {
    if state.map_chooser.thumbs_loaded || state.map_chooser.thumbs_loading {
        return iced::Task::none();
    }
    state.map_chooser.thumbs_loading = true;
    load_thumbs_task()
}

pub fn refresh_index(state: &mut State) -> iced::Task<Message> {
    // Editable project files can change without the dashboard row changing.
    state.map_chooser.source_signature.clear();
    ensure_index(state)
}

/// Index one source at a time, then render one missing map preview at a time.
/// Neither queue sets the editor's global busy state or opens a map in the UI.
pub fn ensure_index(state: &mut State) -> iced::Task<Message> {
    let archives = state
        .library_catalog
        .as_ref()
        .map(|catalog| {
            catalog
                .archives
                .iter()
                .filter(|a| crate::level_library::likely_level_pak(a))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let projects = state.modding_projects.clone().unwrap_or_default();
    let signature = archives
        .iter()
        .map(|a| format!("{}|{}|{}|{}", a.id, a.directory, a.entries, a.source))
        .chain(projects.iter().map(|p| {
            format!(
                "{}|{:?}|{}",
                p.workspace.display(),
                p.modified,
                p.source_build
            )
        }))
        .collect::<Vec<_>>();
    if state.map_chooser.source_signature != signature {
        state.map_chooser.source_signature = signature;
        state.map_chooser.generation = state.map_chooser.generation.wrapping_add(1);
        state.map_chooser.archives.clear();
        state.map_chooser.projects.clear();
        state.map_chooser.preview_done.clear();
        state.map_chooser.focus = None;
    }
    let generation = state.map_chooser.generation;
    if state.map_chooser.indexing {
        return iced::Task::none();
    }
    if let Some(archive) = archives
        .iter()
        .find(|a| !state.map_chooser.archives.contains_key(&a.id))
        .cloned()
    {
        state.map_chooser.indexing = true;
        return iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let id = archive.id.clone();
                    (id, crate::level_library::index_maps(&archive))
                })
                .await
                .unwrap_or_else(|error| (String::new(), Err(error.to_string())))
            },
            move |(id, result)| Message::Map(Msg::IndexedArchive(generation, id, result)),
        );
    }
    if let Some(project) = projects
        .iter()
        .find(|p| {
            !state
                .map_chooser
                .projects
                .contains_key(&p.workspace.display().to_string())
        })
        .cloned()
    {
        state.map_chooser.indexing = true;
        let library = PathBuf::from(&state.library_root);
        return iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let path = project.workspace.display().to_string();
                    (
                        path,
                        crate::level_library::index_project_maps_with_source(&project, &library),
                    )
                })
                .await
                .unwrap_or_else(|error| (String::new(), Err(error.to_string())))
            },
            move |(path, result)| Message::Map(Msg::IndexedProject(generation, path, result)),
        );
    }
    if !state.map_chooser.thumbs_loaded
        || state.map_chooser.previewing
        || state.busy
        || !state.level_map_prompt
    {
        return iced::Task::none();
    }
    for archive in &archives {
        if let Some(Some((owner, maps))) = state.map_chooser.archives.get(&archive.id) {
            for map in maps {
                let key = thumb_key(&owner.id, &map.name);
                if state.map_chooser.thumbs.contains_key(&key)
                    || state.map_chooser.preview_done.contains(&key)
                {
                    continue;
                }
                let owner = owner.clone();
                let name = map.name.clone();
                state.map_chooser.previewing = true;
                return iced::Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::level_library::preview_archive(owner, &name)
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    move |png| Message::Map(Msg::PreviewReady(generation, key.clone(), png)),
                );
            }
        }
    }
    for project in &projects {
        let source = project.workspace.display().to_string();
        if let Some(Some(maps)) = state.map_chooser.projects.get(&source) {
            for map in maps {
                let key = thumb_key(&source, &map.name);
                if state.map_chooser.thumbs.contains_key(&key)
                    || state.map_chooser.preview_done.contains(&key)
                {
                    continue;
                }
                let project = project.clone();
                let name = map.name.clone();
                let library = PathBuf::from(&state.library_root);
                state.map_chooser.previewing = true;
                return iced::Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::level_library::preview_project(project, &name, &library)
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    move |png| Message::Map(Msg::PreviewReady(generation, key.clone(), png)),
                );
            }
        }
    }
    iced::Task::none()
}

/// When a recent map's source finished listing its maps, open that map.
pub fn take_pending(state: &mut State, maps: &[MapCandidate], project: bool) -> Option<Message> {
    let name = state.map_chooser.pending_map.take()?;
    let Some(index) = maps.iter().position(|map| map.name == name) else {
        state.status =
            format!("{name} is no longer present in this source. Select a current map instead.");
        return None;
    };
    Some(if project {
        Message::LevelChooseProjectMap(index)
    } else {
        Message::LevelChooseMap(index)
    })
}

/// Remembers an opened map at the front of the recents list.
pub fn remember(
    prefs: &mut crate::settings::Preferences,
    archive_id: &str,
    archive_name: &str,
    map: &str,
    project: Option<&Path>,
) -> crate::settings::RecentMap {
    let recent = crate::settings::RecentMap {
        archive_id: archive_id.to_owned(),
        map: map.to_owned(),
        project: project.map(|p| p.display().to_string()).unwrap_or_default(),
        label: format!("{map} · {archive_name}"),
    };
    prefs.recent_maps.retain(|old| {
        !(old.map == recent.map
            && old.archive_id == recent.archive_id
            && old.project == recent.project)
    });
    prefs.recent_maps.insert(0, recent.clone());
    prefs.recent_maps.truncate(MAX_RECENT);
    recent
}

fn card_view<'a>(state: &'a State, card: Card, focused: bool) -> Element<'a, Message> {
    let busy = state.busy;
    let accent = card.current || focused;
    let thumb: Element<'a, Message> = match card.thumb.as_ref().and_then(|key| {
        state
            .map_chooser
            .thumbs
            .get(key)
            .map(|handle| (key, handle))
    }) {
        Some((key, handle)) => mouse_area(
            image(handle.clone())
                .width(Fill)
                .height(Length::Fixed(110.0))
                .content_fit(iced::ContentFit::Cover),
        )
        .on_press(Message::Map(Msg::Lightbox(Some(key.clone()))))
        .interaction(iced::mouse::Interaction::Pointer)
        .into(),
        None => {
            let alpha = crate::ui_kit::pulse(
                state.ui.clock(std::time::Instant::now()),
                state.preferences.reduce_motion,
            );
            stack![
                crate::ui_kit::skeleton(Length::Fill, 110.0, alpha),
                container(icons::glyph(
                    if card.kind == "project" {
                        icons::pencil()
                    } else {
                        icons::map()
                    },
                    28.0
                ))
                .center(Fill)
            ]
            .height(Length::Fixed(110.0))
            .into()
        }
    };
    let mut badges = row![].spacing(4);
    if card.current {
        badges = badges.push(crate::ui_kit::badge(BadgeKind::Enabled, "Open"));
    }
    for (kind, label) in card.badges {
        badges = badges.push(crate::ui_kit::badge(kind, label));
    }
    let mut body = column![text(card.title).size(design::BODY)].spacing(5);
    if !card.subtitle.is_empty() {
        body = body.push(design::caption(card.subtitle));
    }
    body = body.push(badges.wrap());
    let content = column![thumb, container(body).padding([8, 10])].spacing(0);
    let button = iced::widget::button(content)
        .padding(0)
        .width(Fill)
        .style(move |theme: &Theme, status| {
            let mut style = design::secondary(theme, status);
            style.background = Some(Background::Color(design::surface(theme)));
            style.border = Border {
                color: if accent {
                    theme.palette().primary
                } else {
                    design::border(theme)
                },
                width: if accent { 2.0 } else { 1.0 },
                radius: design::radius(theme, design::Radius::Large).into(),
            };
            style
        })
        .on_press_maybe((!busy).then_some(card.message));
    button.into()
}

fn grid<'a>(state: &'a State, cards: Vec<(usize, Card)>, columns: usize) -> Element<'a, Message> {
    let mut grid = column![].spacing(10);
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let mut line = row![].spacing(10);
        for _ in 0..columns {
            match cards.next() {
                Some((index, card)) => {
                    let focused = state.map_chooser.focus == Some(index);
                    line = line.push(container(card_view(state, card, focused)).width(Fill));
                }
                None => line = line.push(space().width(Fill)),
            }
        }
        grid = grid.push(line);
    }
    grid.into()
}

/// Adds one titled section of cards when it has any.
fn flush<'a>(
    state: &'a State,
    sections: iced::widget::Column<'a, Message>,
    title: Option<&'static str>,
    batch: Vec<(usize, Card)>,
    columns: usize,
) -> iced::widget::Column<'a, Message> {
    match title {
        Some(title) if !batch.is_empty() => sections.push(
            column![
                text(title).size(design::HEADING),
                grid(state, batch, columns)
            ]
            .spacing(8),
        ),
        _ => sections,
    }
}

fn breadcrumbs(state: &State) -> Option<Element<'_, Message>> {
    let source = if let Some((project, _)) = &state.level_project_choices {
        project.label.clone()
    } else if let Some((archive, _)) = &state.level_map_choices {
        archive.name.clone()
    } else {
        return None;
    };
    Some(crate::chrome::breadcrumbs(
        vec![
            crate::chrome::Crumb {
                label: "Maps".into(),
                message: Some(Message::LevelShowMapChooser),
            },
            crate::chrome::Crumb {
                label: source,
                message: None,
            },
        ],
        false,
    ))
}

pub fn view(state: &State) -> Element<'_, Message> {
    let mut page = column![
        design::title("Choose a map"),
        design::caption(
            "Pick the open map, an editable project or an imported archive. Original PAKs stay unchanged."
        ),
    ]
    .spacing(12);
    if let Some(crumbs) = breadcrumbs(state) {
        page = page.push(crumbs);
    }
    if state.busy {
        let step = LOAD_STEP.load(Ordering::Relaxed);
        page = page.push(
            container(crate::ui_kit::stepper(
                &crate::ui_kit::Steps::new(&LOAD_STEPS, step),
                state.loading_wheel_frame,
            ))
            .padding(12)
            .style(design::panel),
        );
    }
    let all = cards(state);
    let mut chips = row![].spacing(6).align_y(iced::Alignment::Center);
    for filter in FILTERS {
        let on = state.map_chooser.filters.contains(filter);
        chips = chips.push(
            design::button(text(filter).size(design::LABEL))
                .padding([4, 10])
                .style(move |theme, status| design::rail(theme, status, on))
                .on_press(Message::Map(Msg::Filter(filter))),
        );
    }
    page = page.push(
        row![
            container(crate::forms::search_field(
                "Search maps, projects and archives…",
                &state.map_chooser.search,
                Some(all.len()),
                |value| Message::Map(Msg::Search(value)),
            ))
            .width(Fill),
            chips,
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
    );
    if state.level_map_choices.is_none()
        && state.level_project_choices.is_none()
        && !state.preferences.recent_maps.is_empty()
    {
        let mut recents = row![design::caption("RECENT")]
            .spacing(6)
            .align_y(iced::Alignment::Center);
        for (index, recent) in state.preferences.recent_maps.iter().enumerate() {
            recents = recents.push(
                design::button(text(recent.label.as_str()).size(design::LABEL))
                    .padding([4, 10])
                    .style(design::secondary)
                    .on_press_maybe((!state.busy).then_some(Message::Map(Msg::OpenRecent(index)))),
            );
        }
        page = page.push(recents.wrap());
    }
    let grids = iced::widget::responsive(move |size| {
        let columns = if size.width >= 760.0 { 3 } else { 2 };
        COLUMNS.store(columns, Ordering::Relaxed);
        let mut sections = column![].spacing(18);
        let mut current: Option<&'static str> = None;
        let mut batch: Vec<(usize, Card)> = Vec::new();
        for (index, card) in cards(state).into_iter().enumerate() {
            if current != Some(card.section) {
                sections = flush(
                    state,
                    sections,
                    current,
                    std::mem::take(&mut batch),
                    columns,
                );
                current = Some(card.section);
            }
            batch.push((index, card));
        }
        sections = flush(state, sections, current, batch, columns);
        scrollable(sections.padding([0, 4])).height(Fill).into()
    });
    if all.is_empty() {
        page = page.push(crate::ui_kit::empty_state(
            Some(icons::map()),
            "No maps match",
            "Clear the search or filters, or import a level PAK in Asset Library.",
            None,
        ));
    } else {
        page = page.push(container(grids).height(Fill));
    }
    let mut footer = row![
        design::caption("←/→/↑/↓ move · Enter opens · Esc returns to the open scene").width(Fill)
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    if state.level_scene.is_some() || state.library_map.is_some() {
        footer = footer
            .push(design::button("Return to open scene").on_press(Message::LevelDismissMapChooser));
    }
    page = page
        .push(footer)
        .push(design::caption(state.status.as_str()));
    let base: Element<'_, Message> = container(page.max_width(1180))
        .center_x(Fill)
        .padding(28)
        .width(Fill)
        .height(Fill)
        .into();
    match state
        .map_chooser
        .lightbox
        .as_ref()
        .and_then(|key| state.map_chooser.thumbs.get(key).map(|h| (key, h)))
    {
        Some((key, handle)) => crate::gallery::image_lightbox(
            base,
            key.replace('_', " "),
            handle.clone(),
            Message::Map(Msg::Lightbox(None)),
        ),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_keys_are_file_safe() {
        let key = thumb_key("abc/def", "Ice Alpine");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        assert_ne!(key, thumb_key("abc/def", ""));
        assert_ne!(
            thumb_key(&"a".repeat(180), "One"),
            thumb_key(&"a".repeat(180), "Two")
        );
    }

    #[test]
    fn terrain_thumbnail_renders_a_png() {
        let mut points = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                points.push([x as f32 * 10.0, y as f32 * 10.0, ((x * y) % 7) as f32]);
            }
        }
        let png = terrain_thumbnail(&points).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(terrain_thumbnail(&points[..2]).is_none());
    }

    #[test]
    fn recents_dedupe_and_cap() {
        let mut prefs = crate::settings::Preferences::default();
        for index in 0..7 {
            remember(&mut prefs, "a", "Archive", &format!("Map {index}"), None);
        }
        remember(&mut prefs, "a", "Archive", "Map 3", None);
        assert_eq!(prefs.recent_maps.len(), MAX_RECENT);
        assert_eq!(prefs.recent_maps[0].map, "Map 3");
        assert_eq!(
            prefs
                .recent_maps
                .iter()
                .filter(|r| r.map == "Map 3")
                .count(),
            1
        );
    }

    #[test]
    fn badges_describe_the_map() {
        let map = MapCandidate {
            name: "Tutorial".into(),
            legacy_ported: false,
            layers: vec![PathBuf::from("a.lyr"), PathBuf::from("b.lyr")],
            terrain: Some(PathBuf::from("terrain")),
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
            compiled_terrain: Some(PathBuf::from("terrain.dat")),
        };
        let labels: Vec<_> = map_badges(&map).into_iter().map(|(_, l)| l).collect();
        assert_eq!(labels, ["Terrain", "2 layers"]);
        assert!(
            !map_card(
                "Maps in project",
                "test",
                &map,
                Message::LevelChooseProjectMap(0),
                false
            )
            .legacy
        );
        let mut compiled = map.clone();
        compiled.terrain = None;
        compiled.layers.clear();
        compiled.legacy_ported = true;
        assert!(
            map_badges(&compiled)
                .iter()
                .any(|(_, label)| label == "Legacy-ported")
        );
    }

    #[test]
    fn indexed_projects_show_individual_map_cards_before_opening() {
        let mut state = State::default();
        let project = crate::modding_projects::ProjectRow {
            label: "My project".into(),
            target: "custom.pak".into(),
            source_build: String::new(),
            status: String::new(),
            details: String::new(),
            workspace: PathBuf::from("test-project"),
            modified: Vec::new(),
            changed_known: false,
            enabled: true,
        };
        let map = MapCandidate {
            name: "Map One".into(),
            legacy_ported: true,
            layers: Vec::new(),
            terrain: None,
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
            compiled_terrain: Some(PathBuf::from("terrain/terrain.dat")),
        };
        state
            .map_chooser
            .projects
            .insert(project.workspace.display().to_string(), Some(vec![map]));
        state.modding_projects = Some(vec![project]);
        let shown = cards(&state);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Map One");
        assert_eq!(shown[0].subtitle, "My project");
        assert!(shown[0].thumb.is_some());
        state.map_chooser.filters.insert("Projects");
        assert_eq!(cards(&state).len(), 1);
        state.map_chooser.filters.clear();
        state.map_chooser.filters.insert("Legacy-ported");
        assert_eq!(cards(&state).len(), 1);
    }
}
