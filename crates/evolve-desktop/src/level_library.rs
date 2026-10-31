//! Verified map discovery for a selected imported PAK.
//!
//! Only manifest-listed files in the selected archive are used. There is no
//! guessed global map root, automatic editing, or modification of game files.
use crate::{
    asset_cache::SharedCache, auto_resources, level_environment, level_scene_renderer,
    modding_projects::ProjectRow,
};
use evolve_core::{
    ImportedPak, Inventory, LevelScene, ModelInfo, RenderMesh, TerrainInfo, imported_archive_files,
    imported_archive_preview_inventory, imported_library, parse_level_scene,
    parse_level_scene_read_only, read_terrain, read_terrain_paint, read_terrain_rgb24_tiles,
    read_terrain_texture_atlas, read_terrain_texture_header, scan_directory,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(super) struct MapCandidate {
    pub name: String,
    pub layers: Vec<PathBuf>,
    pub terrain: Option<PathBuf>,
    pub vegetation: Option<PathBuf>,
    pub terrain_texture_xml: Option<PathBuf>,
    pub terrain_texture_pak: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(super) struct LoadedMap {
    pub archive: ImportedPak,
    pub maps: Vec<MapCandidate>,
    pub index: usize,
    pub scene: Option<LevelScene>,
    pub source: Option<PathBuf>,
    pub terrain: Option<(RenderMesh, TerrainInfo)>,
    pub full_scene: Option<level_scene_renderer::Assembled>,
    pub project: Option<ProjectRow>,
    pub diagnostics: Vec<String>,
}

/// Most imported map archives are Levels_*.pak. Other imports can be opened
/// explicitly using the Asset Library's Open in Level Editor action.
pub(super) fn likely_level_pak(pak: &ImportedPak) -> bool {
    let name = pak.name.to_ascii_lowercase();
    pak.group.eq_ignore_ascii_case("Levels")
        || name.starts_with("map_")
        || name.starts_with("maps_")
        || name.contains("_level")
        || name.contains("_map")
}

fn map_group(relative: &str) -> (String, String) {
    let parts: Vec<&str> = relative.split('/').filter(|p| !p.is_empty()).collect();
    if let Some((index, name)) = parts.iter().enumerate().find_map(|(index, part)| {
        (part.eq_ignore_ascii_case("levels") || part.eq_ignore_ascii_case("maps"))
            .then(|| parts.get(index + 1).map(|name| (index, name)))
            .flatten()
    }) {
        return (
            parts[..=index + 1].join("/").to_ascii_lowercase(),
            (*name).to_string(),
        );
    }
    // A Layer subdirectory is not itself a separate map.
    let folder = parts.len().saturating_sub(1);
    let map_index = if folder > 0 && parts[folder - 1].eq_ignore_ascii_case("layers") {
        folder.saturating_sub(2)
    } else {
        folder.saturating_sub(1)
    };
    let name = parts.get(map_index).copied().unwrap_or("Imported level");
    (
        parts[..=map_index].join("/").to_ascii_lowercase(),
        name.to_string(),
    )
}

/// Map-owned resources may be imported from a different same-build PAK.
/// Derive the *exact* virtual folder from an authored layer/heightmap path;
/// never use a map basename or a global terraintexture.pak search.
fn map_virtual_folder(map: &MapCandidate) -> Option<String> {
    map.terrain
        .iter()
        .chain(map.layers.iter())
        .chain(map.vegetation.iter())
        .map(|path| map_group(&path.to_string_lossy()).0)
        .find(|folder| !folder.is_empty())
}

/// The original exported CryEngine map layout places metadata in
/// LevelData but the baked terrain package at the map root. An older
/// flattened import can contain a root-level XML; only try that legacy
/// exact map-owned path after the primary source path is absent.
fn terrain_virtual_candidates(folder: &str, member: &str) -> Result<Vec<String>, String> {
    match member.to_ascii_lowercase().as_str() {
        "terraintexture.xml" => Ok(vec![
            format!("{folder}/LevelData/TerrainTexture.xml"),
            format!("{folder}/TerrainTexture.xml"),
        ]),
        "terraintexture.pak" => Ok(vec![format!("{folder}/terraintexture.pak")]),
        _ => Err(format!("Unsupported map-owned terrain member: {member}")),
    }
}

/// An archive's *source folder* is part of its CryEngine virtual namespace.
/// For example, Game/levels/Tutorial/level.pak contains LevelData/TerrainTexture.xml,
/// while Game/levels/Tutorial/terraintexture.pak is an imported PAK in its own
/// right. These are not necessarily game-virtual member names in a manifest.
/// Require the exact map directory and the same original Game installation
/// as the selected level PAK: similarly named Legacy imports cannot leak in.
fn source_game_root(pak: &ImportedPak) -> Option<PathBuf> {
    let source = Path::new(&pak.source);
    source
        .ancestors()
        .find(|parent| {
            parent
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Game"))
        })
        .and_then(|game| fs::canonicalize(game).ok())
}

fn source_map_parent(pak: &ImportedPak, game: &Path, map_folder: &str) -> bool {
    let Some(parent) = Path::new(&pak.source).parent() else {
        return false;
    };
    let Ok(canonical) = fs::canonicalize(parent) else {
        return false;
    };
    let Ok(relative) = canonical.strip_prefix(game) else {
        return false;
    };
    relative
        .to_string_lossy()
        .replace('\\', "/")
        .eq_ignore_ascii_case(map_folder)
}

/// Validate the manifest's extracted member bytes and containment before
/// treating them as possible terrain inputs. No original on-disk PAK is
/// modified, scanned as an unrelated ZIP, or interpreted by filename alone.
fn imported_terrain_member(
    inventory: &Inventory,
    relative: &str,
) -> Result<Option<PathBuf>, String> {
    auto_resources::exact(inventory, relative, None, None)
}

#[derive(Clone, Debug)]
enum TerrainResource {
    Cry(PathBuf),
    Rgb24 { paths: Vec<PathBuf>, side: usize },
}

impl TerrainResource {
    #[cfg(test)]
    fn file(&self) -> Option<&Path> {
        match self {
            Self::Cry(path) => Some(path),
            Self::Rgb24 { .. } => None,
        }
    }
}

/// Verify a complete square tile grid within a SINGLE imported PAK.
/// Accepted members have the original "tileX_Y.raw" spelling, optionally
/// beneath the import's "files/" prefix. No unrelated filename or missing
/// coordinate may be substituted. Individual file contents are validated by
/// the bounded RGB24 decoder after these manifest-backed paths resolve.
fn imported_rgb24_tile_grid(
    inventory: &Inventory,
) -> Result<Option<(Vec<PathBuf>, usize)>, String> {
    let mut tiles = BTreeMap::<(usize, usize), String>::new();
    for entry in &inventory.entries {
        let lower = entry.relative_path.replace('\\', "/").to_ascii_lowercase();
        let name = lower.strip_prefix("files/").unwrap_or(&lower);
        if !(name.starts_with("tile") && name.ends_with(".raw")) || name.contains('/') {
            continue;
        }
        let coordinates = &name[4..name.len() - 4];
        let (x, y) = coordinates.split_once('_').ok_or_else(|| {
            format!(
                "Malformed imported RGB24 terrain tile: {}",
                entry.relative_path
            )
        })?;
        if x.is_empty()
            || y.is_empty()
            || !x.bytes().all(|b| b.is_ascii_digit())
            || !y.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!(
                "Malformed imported RGB24 terrain coordinate: {}",
                entry.relative_path
            ));
        }
        let x = x.parse::<usize>().map_err(|e| e.to_string())?;
        let y = y.parse::<usize>().map_err(|e| e.to_string())?;
        if x >= 8 || y >= 8 {
            return Err(
                "Imported RGB24 terrain grid exceeds the bounded eight-tile-per-axis limit".into(),
            );
        }
        if tiles.insert((x, y), entry.relative_path.clone()).is_some() {
            return Err(format!("Ambiguous duplicate terrain RGB24 tile {x}_{y}"));
        }
    }
    if tiles.is_empty() {
        return Ok(None);
    }
    let side = tiles
        .keys()
        .map(|(x, y)| x.max(y))
        .max()
        .copied()
        .unwrap_or(0)
        + 1;
    if side.checked_mul(side) != Some(tiles.len()) {
        return Err(format!(
            "Incomplete imported terrain RGB24 grid: {} members for expected {side}x{side} tiles",
            tiles.len()
        ));
    }
    let mut verified = Vec::with_capacity(tiles.len());
    for y in 0..side {
        for x in 0..side {
            let name = tiles
                .get(&(x, y))
                .ok_or_else(|| format!("Missing imported terrain RGB24 tile{x}_{y}.raw"))?;
            verified
                .push(imported_terrain_member(inventory, name)?.ok_or_else(|| {
                    format!("Unavailable verified imported terrain tile: {name}")
                })?);
        }
    }
    Ok(Some((verified, side)))
}

fn imported_map_terrain(
    selected: &ImportedPak,
    folder: &str,
    member_name: &str,
) -> Result<Option<TerrainResource>, String> {
    let Some(game) = source_game_root(selected) else {
        return Ok(None);
    };
    let Some(library_root) = Path::new(&selected.directory).parent() else {
        return Ok(None);
    };
    let library = imported_library(library_root).map_err(|error| error.to_string())?;
    // Legacy registration labels may overlap the same physical source build.
    // The exact Game directory and map owner below provide stronger source
    // provenance than the number of historical labels attached to a PAK.
    let selected_build = selected.builds.first().map(String::as_str);
    let mut found: Vec<TerrainResource> = Vec::new();
    let mut matching_pak_details = Vec::new();
    let mut matching_pak_members = Vec::new();
    for archive in &library.archives {
        // Unlabeled archives imported from the exact same Game directory
        // are admissible. A *known different* build is never admissible.
        // This handles independently imported Tutorial level/terrain PAKs.
        let same_build = match selected_build {
            Some(label) => {
                archive.builds.is_empty() || archive.builds.iter().any(|build| build == label)
            }
            // An unlabelled selected map may have individually imported
            // same-source siblings with one known label. The original Game
            // root and exact map directory still have to match, and duplicate
            // candidate archives fail closed below.
            None => archive.builds.len() <= 1,
        };
        if archive.id == selected.id
            || !same_build
            || source_game_root(archive).as_deref() != Some(game.as_path())
            || !source_map_parent(archive, &game, folder)
        {
            continue;
        }
        let is_terrain_pak = archive.name.eq_ignore_ascii_case("terraintexture.pak");
        let is_level_pak = archive.name.eq_ignore_ascii_case("level.pak");
        if !is_terrain_pak && !is_level_pak {
            continue;
        }
        let inventory = imported_archive_preview_inventory(archive).map_err(|e| e.to_string())?;
        if member_name.eq_ignore_ascii_case("TerrainTexture.xml") {
            for relative in [
                "LevelData/TerrainTexture.xml",
                "TerrainTexture.xml",
                &format!("{folder}/LevelData/TerrainTexture.xml"),
                &format!("{folder}/TerrainTexture.xml"),
            ] {
                if let Some(path) = imported_terrain_member(&inventory, relative)? {
                    // Multiple accepted virtual spellings may resolve to one
                    // exact manifest file. Only separate files are ambiguous.
                    if !found.iter().any(|existing| {
                        matches!(existing, TerrainResource::Cry(other) if other == &path)
                    }) {
                        found.push(TerrainResource::Cry(path));
                    }
                }
            }
        } else if member_name.eq_ignore_ascii_case("terraintexture.pak") && is_terrain_pak {
            // The imported 'terraintexture.pak' ZIP is a container, not an
            // extracted file named terraintexture.pak. Look inside *that exact
            // source archive* for a member with a verified CRY terrain header.
            // Do not pass the outer ZIP archive to the baked-quadtree decoder.
            matching_pak_details.push(format!(
                "{} ({} extracted members)",
                archive.name,
                inventory.entries.len()
            ));
            for entry in &inventory.entries {
                if matching_pak_members.len() < 24 {
                    matching_pak_members.push(entry.relative_path.clone());
                }
                if entry.byte_size < 24 || entry.byte_size > 2 * 1024 * 1024 * 1024 {
                    continue;
                }
                let Some(path) = imported_terrain_member(&inventory, &entry.relative_path)? else {
                    continue;
                };
                if read_terrain_texture_header(&path).is_ok()
                    && !found.iter().any(|existing| {
                        matches!(existing, TerrainResource::Cry(other) if other == &path)
                    })
                {
                    found.push(TerrainResource::Cry(path));
                }
            }
            // Tile RGB24 layouts vary by map (Tutorial 2x2,
            // Ice_Crashsite 4x4). Detect the complete NxN set from the
            // original manifest, not a fixed number or a map-name whitelist.
            if let Some((paths, side)) = imported_rgb24_tile_grid(&inventory)? {
                found.push(TerrainResource::Rgb24 { paths, side });
            }
        }
    }
    match found.len() {
        0 if !matching_pak_details.is_empty() => Err(format!(
            "Imported {} for {folder}, but none of its extracted members has a valid CRY baked-terrain tile header. The imported ZIP/signed archive's member layout must be decoded explicitly rather than feeding the outer PAK to the raw tile decoder. Imported files: {}",
            matching_pak_details.join(", "),
            matching_pak_members.join(", ")
        )),
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(format!(
            "Multiple same-build imported sources have {member_name} for {folder}; refusing an ambiguous terrain selection"
        )),
    }
}

fn terrain_resource(
    source_root: &Path,
    selected: &ImportedPak,
    inventory: Option<&Inventory>,
    map: &MapCandidate,
    manifest_entry: Option<&PathBuf>,
    member_name: &str,
) -> Result<Option<TerrainResource>, String> {
    if let Some(relative) = manifest_entry {
        return verified_source(source_root, relative, 2 * 1024 * 1024 * 1024)
            .map(|path| Some(TerrainResource::Cry(path)));
    }
    let folder = map_virtual_folder(map)
        .ok_or_else(|| "Cannot determine the original map's virtual folder".to_owned())?;
    let loaded_inventory = if inventory.is_none() {
        Some(imported_archive_files(selected).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let selected_inventory = inventory
        .or(loaded_inventory.as_ref())
        .ok_or_else(|| "No verified imported map inventory".to_owned())?;
    let build = (selected.builds.len() == 1).then(|| selected.builds[0].as_str());
    // Cross-PAK lookup without a unique build label can silently take a
    // terrain package from another game version. Local selected-file hits
    // remain valid even when external dependency lookup is disabled.
    let library_root = build.and_then(|_| Path::new(&selected.directory).parent());
    for requested in terrain_virtual_candidates(&folder, member_name)? {
        // Preserve the original exact virtual map owner and selected build.
        // A conflicting or unsafe match is an error, not permission to try
        // a similarly named resource from a different level.
        if let Some(path) =
            auto_resources::exact(selected_inventory, &requested, library_root, build)?
        {
            return Ok(Some(TerrainResource::Cry(path)));
        }
    }
    // The original source archive's folder may define the virtual prefix.
    // In the Tutorial import, a separate "level.pak" can contain the map's
    // LevelData XML, and "terraintexture.pak" is itself a ZIP/imported archive.
    imported_map_terrain(selected, &folder, member_name)
}

fn verified_source(root: &Path, relative: &Path, max_bytes: u64) -> Result<PathBuf, String> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(format!(
            "Level source unavailable or too large: {}",
            path.display()
        ));
    }
    let exact = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if !exact.starts_with(root) {
        return Err("Map source escaped the verified imported archive".into());
    }
    Ok(exact)
}

// Exact, authored map owner + selected build only; no filename guessing.
fn authored_time_of_day(
    archive: &ImportedPak,
    map: &MapCandidate,
    inventory: &Inventory,
) -> Result<Option<PathBuf>, String> {
    let folder = map_virtual_folder(map).ok_or("Unknown original map folder")?;
    let build = (archive.builds.len() == 1).then(|| archive.builds[0].as_str());
    let library = build.and_then(|_| Path::new(&archive.directory).parent());
    for name in [
        format!("{folder}/LevelData/TimeOfDay.xml"),
        format!("{folder}/TimeOfDay.xml"),
    ] {
        if let Some(path) = auto_resources::exact(inventory, &name, library, build)? {
            return Ok(Some(path));
        }
    }
    // A single map's original level.pak may contain map-relative LevelData/.
    if source_game_root(archive).is_some_and(|game| source_map_parent(archive, &game, &folder)) {
        for name in ["LevelData/TimeOfDay.xml", "TimeOfDay.xml"] {
            if let Some(path) = auto_resources::exact(inventory, name, None, None)? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

// Resolve Environment.xml using the same exact selected-map owner and
// unique-build provenance rules as TimeOfDay.xml; no global name search.
fn authored_environment(
    archive: &ImportedPak,
    map: &MapCandidate,
    inventory: &Inventory,
) -> Result<Option<PathBuf>, String> {
    let folder = map_virtual_folder(map).ok_or("Unknown original map folder")?;
    let build = (archive.builds.len() == 1).then(|| archive.builds[0].as_str());
    let library = build.and_then(|_| Path::new(&archive.directory).parent());
    for name in [
        format!("{folder}/LevelData/Environment.xml"),
        format!("{folder}/Environment.xml"),
    ] {
        if let Some(path) = auto_resources::exact(inventory, &name, library, build)? {
            return Ok(Some(path));
        }
    }
    if source_game_root(archive).is_some_and(|game| source_map_parent(archive, &game, &folder)) {
        for name in ["LevelData/Environment.xml", "Environment.xml"] {
            if let Some(path) = auto_resources::exact(inventory, name, None, None)? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn authored_fx_catalog(
    root: &Path,
    inventory: &Inventory,
    scene: Option<&LevelScene>,
) -> level_environment::ResourceSummary {
    let mut data = level_environment::ResourceSummary {
        emitters: scene.map_or(0, |scene| {
            scene
                .objects
                .iter()
                .filter(|obj| obj.kind.to_ascii_lowercase().contains("particle"))
                .count()
        }),
        ..Default::default()
    };
    let mut inspected_fx = 0;
    let mut inspected_mtl = 0;
    for entry in &inventory.entries {
        let lower = entry.relative_path.replace('\\', "/").to_ascii_lowercase();
        let is_fx = (lower.starts_with("libs/particles/") || lower.contains("/libs/particles/"))
            && lower.ends_with(".xml");
        if is_fx {
            data.particle_libraries += 1;
        }
        if lower.ends_with(".mtl") {
            data.material_files += 1;
        }
        if lower.ends_with(".cfx") {
            data.shader_sources += 1;
        }
        let (tag, field) = if is_fx && inspected_fx < 32 {
            inspected_fx += 1;
            (b"Item".as_slice(), b"Name".as_slice())
        } else if lower.ends_with(".mtl") && inspected_mtl < 64 {
            inspected_mtl += 1;
            (b"Material".as_slice(), b"Shader".as_slice())
        } else {
            continue;
        };
        if entry.byte_size > 512 * 1024 {
            continue;
        }
        let Ok(file) = verified_source(root, Path::new(&entry.relative_path), 512 * 1024) else {
            continue;
        };
        let Ok(source) = fs::read(file) else {
            continue;
        };
        let dest = if is_fx {
            &mut data.particle_effects
        } else {
            &mut data.authored_shaders
        };
        for item in level_environment::named_items(&source, tag, field) {
            if dest.len() < 256 && !dest.contains(&item) {
                dest.push(item);
            }
        }
    }
    data
}
fn discover(archive: &ImportedPak) -> Result<(PathBuf, Vec<MapCandidate>), String> {
    // Revalidate the imported manifest and every original extracted file before
    // the editor uses a single byte.
    let inventory = imported_archive_files(archive).map_err(|e| e.to_string())?;
    discover_inventory(&inventory)
}

fn discover_inventory(inventory: &Inventory) -> Result<(PathBuf, Vec<MapCandidate>), String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let mut groups = BTreeMap::<String, MapCandidate>::new();
    for entry in &inventory.entries {
        let relative = entry.relative_path.replace('\\', "/");
        let lower = relative.to_ascii_lowercase();
        let mission = lower.ends_with("/mission.xml") || lower == "mission.xml";
        let layer = lower.ends_with(".lyr");
        let terrain = lower.ends_with("/heightmap.dat") || lower == "heightmap.dat";
        let vegetation = lower.ends_with("/vegetationmap.dat") || lower == "vegetationmap.dat";
        let surface_xml = lower.ends_with("/terraintexture.xml") || lower == "terraintexture.xml";
        let surface_pak = lower.ends_with("/terraintexture.pak") || lower == "terraintexture.pak";
        if !(mission || layer || terrain || vegetation || surface_xml || surface_pak) {
            continue;
        }
        let (key, name) = map_group(&relative);
        let group = groups.entry(key).or_insert_with(|| MapCandidate {
            name,
            layers: Vec::new(),
            terrain: None,
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
        });
        let path = PathBuf::from(&entry.relative_path);
        if surface_xml {
            if group.terrain_texture_xml.is_none() {
                group.terrain_texture_xml = Some(path);
            }
        } else if surface_pak {
            if group.terrain_texture_pak.is_none() {
                group.terrain_texture_pak = Some(path);
            }
        } else if vegetation {
            if group.vegetation.is_none() {
                group.vegetation = Some(path);
            }
        } else if terrain {
            if group.terrain.is_none() {
                group.terrain = Some(path);
            }
        } else if mission || layer {
            group.layers.push(path);
        }
    }
    let mut maps: Vec<MapCandidate> = groups.into_values().collect();
    for candidate in &mut maps {
        // Prefer original .lyr placements; mission XML is a fallback when no
        // layer files were present, to avoid duplicate object declarations.
        candidate.layers.sort_by_key(|p| {
            let lower = p.to_string_lossy().to_ascii_lowercase();
            (u8::from(!lower.ends_with(".lyr")), lower)
        });
        if candidate
            .layers
            .iter()
            .any(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".lyr"))
        {
            candidate
                .layers
                .retain(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".lyr"));
        }
    }
    maps.retain(|map| !map.layers.is_empty() || map.terrain.is_some() || map.vegetation.is_some());
    maps.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    if maps.is_empty() {
        return Err("This imported PAK contains no verified mission XML, LYR placements, or Heightmap.dat. Select a map archive instead.".into());
    }
    Ok((root, maps))
}

pub(super) fn open(
    archive: ImportedPak,
    index: usize,
    cache: &SharedCache,
) -> Result<LoadedMap, String> {
    let (root, maps) = discover(&archive)?;
    open_selected(archive, root, maps, index, cache, None, None)
}

/// Open a verified copy owned by the Modding project. The caller supplies the
/// manager-indexed ProjectRow; this function never treats an arbitrary folder
/// as an editable project or creates new files.
pub(super) fn open_project(
    project: ProjectRow,
    index: usize,
    cache: &SharedCache,
    library_root: &Path,
) -> Result<LoadedMap, String> {
    let inventory = scan_directory(&project.workspace.join("files")).map_err(|e| e.to_string())?;
    let (root, maps) = discover_inventory(&inventory)?;
    let build = if library_root.is_dir() && !project.source_build.trim().is_empty() {
        vec![project.source_build.clone()]
    } else {
        Vec::new()
    };
    let archive = ImportedPak {
        id: "editable-level-project".into(),
        name: project.target.clone(),
        source: project.workspace.display().to_string(),
        directory: library_root
            .join("editable-level-project")
            .display()
            .to_string(),
        group: "Levels".into(),
        description: "Editable modding project".into(),
        entries: inventory.entries.len(),
        extracted_bytes: inventory.total_bytes,
        warnings: 0,
        builds: build,
    };
    open_selected(
        archive,
        root,
        maps,
        index,
        cache,
        Some(inventory),
        Some(project),
    )
}

fn open_selected(
    archive: ImportedPak,
    root: PathBuf,
    maps: Vec<MapCandidate>,
    index: usize,
    cache: &SharedCache,
    project_inventory: Option<Inventory>,
    project: Option<ProjectRow>,
) -> Result<LoadedMap, String> {
    let index = index.min(maps.len().saturating_sub(1));
    let chosen = &maps[index];
    let mut diagnostics = Vec::new();
    let inventory = project_inventory
        .clone()
        .map(Ok)
        .unwrap_or_else(|| imported_archive_files(&archive).map_err(|e| e.to_string()))?;
    let original_time = match authored_time_of_day(&archive, chosen, &inventory) {
        Ok(Some(file)) => match fs::read(file) {
            Ok(bytes) => match level_environment::TimeOfDay::parse(&bytes) {
                Ok(profile) => {
                    diagnostics.push(format!(
                        "Imported original TimeOfDay.xml: {} variables, {} keyframed curves; skipped {} out-of-day editor spline keys; {}.",
                        profile.count(),
                        profile.keyed_variables,
                        profile.ignored_out_of_range_keys,
                        profile.playback_window_description()
                    ));
                    Some(profile)
                }
                Err(reason) => {
                    diagnostics.push(format!(
                        "Original TimeOfDay.xml unsupported: {reason}; fallback preview."
                    ));
                    None
                }
            },
            Err(reason) => {
                diagnostics.push(format!("Could not read original TimeOfDay.xml: {reason}"));
                None
            }
        },
        Ok(None) => None,
        Err(reason) => {
            diagnostics.push(format!(
                "Could not resolve original TimeOfDay.xml: {reason}"
            ));
            None
        }
    };
    let original_fog = match authored_environment(&archive, chosen, &inventory) {
        Ok(Some(file)) => {
            match fs::symlink_metadata(&file) {
                Ok(meta) if meta.file_type().is_file() && meta.len() <= 4 * 1024 * 1024 => {
                    match fs::read(&file)
                        .map_err(|reason| reason.to_string())
                        .and_then(|bytes| level_environment::FogProfile::parse(&bytes))
                    {
                        Ok(fog) => {
                            diagnostics.push(format!(
                            "Original Environment.xml: Fog ViewDistance={:.1}, LDRGlobalDensMult={:.2}. Distance-only fog preview, NOT CryEngine-native.",
                            fog.view_distance, fog.ldr_density_multiplier
                        ));
                            Some(fog)
                        }
                        Err(reason) => {
                            diagnostics.push(format!("Original Environment.xml Fog unsupported: {reason}; fog preview disabled."));
                            None
                        }
                    }
                }
                _ => {
                    diagnostics.push("Original Environment.xml missing, linked, or oversized; fog preview disabled.".into());
                    None
                }
            }
        }
        Ok(None) => {
            diagnostics.push(
                "No exact original Environment.xml for the selected map; fog preview disabled."
                    .into(),
            );
            None
        }
        Err(reason) => {
            diagnostics.push(format!(
                "Could not resolve exact Environment.xml: {reason}; fog preview disabled."
            ));
            None
        }
    };
    let mut sources = Vec::<(String, Vec<u8>)>::new();
    // Keep both memory and parser work bounded. A map with more than this
    // requires an incremental scene loader rather than silent unbounded reads.
    for path in chosen.layers.iter().take(128) {
        let verified = verified_source(&root, path, 48 * 1024 * 1024)?;
        let bytes = fs::read(&verified).map_err(|e| e.to_string())?;
        sources.push((path.to_string_lossy().into_owned(), bytes));
    }
    if chosen.layers.len() > 128 {
        diagnostics.push(format!(
            "Only 128 of {} layer sources were read.",
            chosen.layers.len()
        ));
    }
    let mut source = chosen.layers.first().map(|p| root.join(p));
    let mut scene = None;
    if !sources.is_empty() {
        let slices = sources
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        match parse_level_scene(&slices) {
            Ok(parsed) => scene = Some(parsed),
            Err(error) => {
                diagnostics.push(format!(
                    "Combined layers could not be assembled: {error}. Trying individual sources."
                ));
                // An identical duplicate declaration in separate source layers
                // should not force every layer to resolve its parents in
                // isolation. Recover ALL unique placements into one read-only
                // scene and resolve unambiguous cross-layer parent transforms.
                // Conflicting IDs remain explicitly unresolved; editing still
                // uses the strict parser and never silently rewrites GUIDs.
                if project.is_none() {
                    match parse_level_scene_read_only(&slices) {
                        Ok(recovery) => {
                            diagnostics.push(format!(
                                "Read-only combined scene recovery: {} unique placements, {} repeated GUID declarations omitted ({} conflicting IDs); unambiguous cross-layer parent transforms resolved.",
                                recovery.scene.objects.len(),
                                recovery.duplicate_declarations,
                                recovery.conflicting_guids,
                            ));
                            diagnostics.extend(recovery.scene.diagnostics.iter().take(4).cloned());
                            scene = Some(recovery.scene);
                        }
                        Err(recovery_error) => {
                            diagnostics.push(format!(
                                "Read-only combined recovery could not validate all layers: {recovery_error}; falling back to individually validated layers."
                            ));
                            let mut recovered = LevelScene {
                                schema: 1,
                                objects: Vec::new(),
                                diagnostics: Vec::new(),
                            };
                            let mut seen_ids = std::collections::HashSet::new();
                            for (name, bytes) in &sources {
                                match parse_level_scene(&[(name.as_str(), bytes)]) {
                                    Ok(layer) => {
                                        for object in layer.objects {
                                            if seen_ids.insert(object.id.clone()) {
                                                recovered.objects.push(object);
                                            }
                                        }
                                    }
                                    Err(error) => diagnostics.push(format!(
                                        "Layer {name} could not be recovered: {error}"
                                    )),
                                }
                            }
                            if !recovered.objects.is_empty() {
                                diagnostics.push(
                                    "Per-layer fallback has incomplete cross-layer parent transforms."
                                        .into(),
                                );
                                scene = Some(recovered);
                            }
                        }
                    }
                }
                if scene.is_none() {
                    for (name, bytes) in &sources {
                        if let Ok(parsed) = parse_level_scene(&[(name.as_str(), bytes)]) {
                            source = Some(root.join(name));
                            scene = Some(parsed);
                            diagnostics.push(
                                "Showing a single readable layer; some placements may be absent."
                                    .into(),
                            );
                            break;
                        }
                    }
                }
            }
        }
    }
    if let Some(relative) = &chosen.vegetation {
        let path = verified_source(&root, relative, 192 * 1024 * 1024)?;
        let data = scene.get_or_insert_with(|| LevelScene {
            schema: 1,
            objects: Vec::new(),
            diagnostics: Vec::new(),
        });
        match crate::vegetation_map::inspect_file(&path, data) {
            Ok(veg) => {
                diagnostics.push(format!(
                    "Decoded {} vegetation instances from {} source prototypes (format v{}).",
                    veg.instances, veg.prototypes, veg.version
                ));
                diagnostics.extend(veg.warnings);
            }
            Err(reason) => diagnostics.push(format!(
                "VegetationMap.dat exists but could not be decoded: {reason}"
            )),
        }
    }
    let terrain_xml = if chosen.terrain.is_some() {
        terrain_resource(
            &root,
            &archive,
            project_inventory.as_ref(),
            chosen,
            chosen.terrain_texture_xml.as_ref(),
            "TerrainTexture.xml",
        )
    } else {
        Ok(None)
    };
    let terrain_pak = if chosen.terrain.is_some() {
        terrain_resource(
            &root,
            &archive,
            project_inventory.as_ref(),
            chosen,
            chosen.terrain_texture_pak.as_ref(),
            "terraintexture.pak",
        )
    } else {
        Ok(None)
    };
    for (name, result) in [
        ("TerrainTexture.xml", &terrain_xml),
        ("terraintexture.pak", &terrain_pak),
    ] {
        if let Err(error) = result {
            diagnostics.push(format!("Could not resolve original {name}: {error}"));
        }
    }
    let terrain_xml = terrain_xml.ok().flatten();
    let terrain_pak = terrain_pak.ok().flatten();

    let terrain = if let Some(relative) = &chosen.terrain {
        let exact = verified_source(&root, relative, 512 * 1024 * 1024)?;
        match read_terrain_paint(&exact) {
            Ok(Some(paint)) => {
                let mut counts = std::collections::BTreeMap::<u8, usize>::new();
                for id in paint.ids {
                    *counts.entry(id).or_default() += 1;
                }
                diagnostics.push(format!(
                    "Original terrain layer bitmap decoded: {} used layer IDs; {} authored names. Original baked terrain tile textures are separate.",
                    counts.len(), paint.layer_names.len()
                ));
            }
            Ok(None) => diagnostics.push(
                "This original heightmap contains no readable terrain layer-ID section.".into(),
            ),
            Err(error) => diagnostics.push(format!("Terrain paint data unsupported: {error}")),
        }
        match &terrain_pak {
            Some(TerrainResource::Cry(pak)) => match read_terrain_texture_header(pak) {
                Ok(header) => diagnostics.push(format!(
                    "Original map-owned baked CRY terrain v{} found ({} layers, {} sectors).",
                    header.version,
                    header.layers.len(),
                    header.layers.iter().map(|layer| format!(
                        "{}×{} / format {} / {} bytes",
                        layer.sector_pixels, layer.sector_pixels,
                        layer.texture_format, layer.sector_bytes,
                    )).collect::<Vec<_>>().join("; ")
                )),
                Err(error) => diagnostics.push(format!(
                    "Original baked terrain package {} could not be decoded: {error}",
                    pak.display()
                )),
            },
            Some(TerrainResource::Rgb24 { paths, side }) => diagnostics.push(format!(
                "Original map-owned baked terrain RGB24 tile grid: {side}×{side} ({} verified imported raw members).",
                paths.len()
            )),
            None => diagnostics.push(
                "Original baked terrain color unavailable: no verified map-owned CRY tile stream or complete RGB24 tile grid in the selected build.".into()
            ),
        }
        if terrain_xml.is_none() {
            diagnostics.push(
                "Original map TerrainTexture.xml is not available; the baked ground color can still be decoded from a complete RGB24 tile grid.".into()
            );
        }
        match read_terrain(&exact, 129) {
            Ok(data) => {
                let info = data.info.clone();
                Some((
                    RenderMesh {
                        info: ModelInfo {
                            format: "CryEngine Heightmap.dat".into(),
                            version: 1,
                            chunk_count: 0,
                            vertex_count: data.vertices.len(),
                            triangle_count: data.indices.len() / 3,
                            original_uv0: true,
                            original_normals: true,
                        },
                        positions: data.vertices,
                        indices: data.indices,
                        uv0: Some(data.uv),
                        normals: Some(data.normals),
                    },
                    info,
                ))
            }
            Err(error) => {
                diagnostics.push(format!("Terrain decode unavailable: {error}"));
                None
            }
        }
    } else {
        diagnostics.push("Heightmap.dat was not in this PAK; terrain cannot be shown yet.".into());
        None
    };
    if scene.is_none() && terrain.is_none() {
        return Err(diagnostics.join(" "));
    }
    let mut full_scene = level_scene_renderer::assemble(
        &archive,
        scene.as_ref(),
        terrain.as_ref().map(|(mesh, _)| mesh),
        cache,
        project_inventory.as_ref(),
    )?;
    if terrain.is_some()
        && let Some(package) = &terrain_pak
    {
        let decoded = match package {
            TerrainResource::Cry(path) => read_terrain_texture_atlas(path),
            TerrainResource::Rgb24 { paths, side } => read_terrain_rgb24_tiles(paths, *side),
        };
        match decoded.map_err(|err| err.to_string()) {
            Ok(atlas) => {
                let dimensions = atlas.image.dimensions();
                diagnostics.push(terrain_pixel_report(&atlas.image));
                let mut applied = false;
                if let (Some(assembled), Some((terrain_mesh, _))) =
                    (full_scene.as_mut(), terrain.as_ref())
                    && let Some(surface) = assembled.surfaces.first_mut()
                    && surface.first_index == 0
                    && surface.index_count == terrain_mesh.indices.len() as u32
                {
                    let diffuse = std::sync::Arc::new(atlas.image);
                    surface.diffuse = Some(diffuse.clone());
                    // The level's authored terrain now renders as a world
                    // mesh via the GPU instancing pipeline, not CPU NDC
                    // triangles. Bind the decoded original atlas there too.
                    let mut world = (*assembled.world).clone();
                    if let Some(ground) = world.groups.first_mut()
                        && ground
                            .instances
                            .first()
                            .is_some_and(|entry| entry.object_index == usize::MAX)
                        && let Some(material) = ground.surfaces.first_mut()
                    {
                        material.diffuse = Some(diffuse);
                        applied = true;
                    }
                    assembled.world = std::sync::Arc::new(world);
                }
                diagnostics.push(format!(
                    "Original baked terrain: decoded {} tiles ({}) into {}×{} ground-color atlas. {}",
                    atlas.decoded_tiles,
                    if atlas.color_format == 3 { "RGB24".to_owned() }
                    else { format!("CRY format {}", atlas.color_format) },
                    dimensions.0, dimensions.1,
                    if applied { "Applied to terrain GPU material." }
                    else { "No verified terrain material span to receive the color atlas." }
                ));
            }
            Err(error) => diagnostics.push(format!(
                "Original terrain color unavailable: {error}; keeping neutral ground material."
            )),
        }
    }
    let resources = authored_fx_catalog(&root, &inventory, scene.as_ref());
    diagnostics.push(resources.description());
    if let Some(assembled) = full_scene.as_mut() {
        let mut world = (*assembled.world).clone();
        world.time_of_day = original_time.map(std::sync::Arc::new);
        world.fog_profile = original_fog;
        world.source_resources = resources;
        // Map assembly already runs in an operation worker. Hash source
        // image identities here so the first viewport frame never spends
        // time hashing full-resolution decoded DDS and terrain atlases.
        world.precompute_gpu_source_hashes();
        assembled.world = std::sync::Arc::new(world);
        diagnostics.push(assembled.report.status());
    }
    Ok(LoadedMap {
        archive,
        maps,
        index,
        scene,
        source,
        terrain,
        full_scene,
        project,
        diagnostics,
    })
}

/// Diagnostic of original decoded atlas bytes, before shader lighting or
/// the GPU sRGB conversion. Never recolor, gamma-adjust or rewrite source.
fn terrain_pixel_report(img: &image::RgbaImage) -> String {
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return "baked RGB source empty".into();
    }
    let mut sums = [0u64; 3];
    let mut count = 0u64;
    let step_x = (width / 64).max(1);
    let step_y = (height / 64).max(1);
    for y in (0..height).step_by(step_y as usize) {
        for x in (0..width).step_by(step_x as usize) {
            let pixel = img.get_pixel(x, y);
            for component in 0..3 {
                sums[component] += u64::from(pixel[component]);
            }
            count += 1;
        }
    }
    let avg = sums.map(|sum| sum / count);
    let first = img.get_pixel(0, 0);
    let center = img.get_pixel(width / 2, height / 2);
    // Record the baked 2x2 tile orientation without guessing whether game UV
    // axes are flipped. Use authored source pixels before any light/fog/shader.
    let sample = |x: u32, y: u32| -> String {
        let p = img.get_pixel(x.min(width - 1), y.min(height - 1));
        format!("[{},{},{}]", p[0], p[1], p[2])
    };
    let upper_left = sample(width / 4, height / 4);
    let upper_right = sample(width.saturating_mul(3) / 4, height / 4);
    let lower_left = sample(width / 4, height.saturating_mul(3) / 4);
    let lower_right = sample(width.saturating_mul(3) / 4, height.saturating_mul(3) / 4);
    format!(
        "Original decoded terrain RGB8 (pre-lighting/sRGB): sampled average [{},{},{}], corner [{},{},{}], center [{},{},{}]; atlas quadrants TL={} TR={} BL={} BR={}; no correction applied.",
        avg[0],
        avg[1],
        avg[2],
        first[0],
        first[1],
        first[2],
        center[0],
        center[1],
        center[2],
        upper_left,
        upper_right,
        lower_left,
        lower_right,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_color_diagnostic_reports_decoded_pixels_without_adjustment() {
        let image = image::RgbaImage::from_fn(2, 2, |x, y| {
            if x == 0 && y == 0 {
                image::Rgba([20, 40, 60, 255])
            } else {
                image::Rgba([80, 100, 120, 255])
            }
        });
        let text = terrain_pixel_report(&image);
        assert!(text.contains("sampled average [65,85,105]"));
        assert!(text.contains("corner [20,40,60]"));
        assert!(text.contains("no correction applied"));
        assert!(text.contains("atlas quadrants TL=[20,40,60]"));
    }

    #[test]
    fn baked_terrain_resources_use_exact_map_owner_not_a_global_filename() {
        let map = MapCandidate {
            name: "Desert_Canyon".into(),
            layers: vec![PathBuf::from(
                "Levels/Desert_Canyon/Layers/add_new_assets_here.lyr",
            )],
            terrain: Some(PathBuf::from("Levels/Desert_Canyon/Heightmap.dat")),
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
        };
        assert_eq!(
            map_virtual_folder(&map).as_deref(),
            Some("levels/desert_canyon")
        );
    }

    #[test]
    fn terrain_resources_follow_original_leveldata_export_layout() {
        assert_eq!(
            terrain_virtual_candidates("levels/tutorial", "TerrainTexture.xml").unwrap(),
            vec![
                "levels/tutorial/LevelData/TerrainTexture.xml",
                "levels/tutorial/TerrainTexture.xml",
            ]
        );
        assert_eq!(
            terrain_virtual_candidates("levels/tutorial", "terraintexture.pak").unwrap(),
            vec!["levels/tutorial/terraintexture.pak"]
        );
        assert!(terrain_virtual_candidates("levels/tutorial", "../terraintexture.pak").is_err());
        assert!(
            !terrain_virtual_candidates("levels/tutorial", "TerrainTexture.xml")
                .unwrap()
                .iter()
                .any(|candidate| candidate.contains("levels/desert_canyon"))
        );
    }

    #[test]
    fn map_owned_baked_terrain_resolves_across_selected_build_archives() {
        let root = tempfile::tempdir().unwrap();
        let map_id = "11111111111111111111";
        let tex_id = "22222222222222222222";
        let map_folder = root.path().join(format!("map.pak__{map_id}"));
        let tex_folder = root.path().join(format!("terrain.pak__{tex_id}"));
        for folder in [&map_folder, &tex_folder] {
            std::fs::create_dir_all(folder.join("files")).unwrap();
        }
        let map_path = "Levels/Tutorial/LevelData/Heightmap.dat";
        let xml_path = "Levels/Tutorial/LevelData/TerrainTexture.xml";
        let pak_path = "Levels/Tutorial/terraintexture.pak";
        for (folder, virtual_name, bytes) in [
            (&map_folder, map_path, b"MAP!".as_slice()),
            (&tex_folder, xml_path, b"XML!".as_slice()),
            (&tex_folder, pak_path, b"PAK!".as_slice()),
        ] {
            let target = folder.join("files").join(virtual_name);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        for (folder, name, entries) in [
            (&map_folder, "map.pak", vec![map_path]),
            (&tex_folder, "terrain.pak", vec![xml_path, pak_path]),
        ] {
            std::fs::write(
                folder.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version": 1,
                    "name": name,
                    "source": "/verified",
                    "entries": entries.iter().map(|path|
                        serde_json::json!({"path": path, "bytes": 4})
                    ).collect::<Vec<_>>()
                }))
                .unwrap(),
            )
            .unwrap();
        }
        std::fs::write(
            root.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "builds": {
                    "stage-two": {"label": "Stage 2",
                        "members": {"11111111111111111111": {}, "22222222222222222222": {}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let library = evolve_core::imported_library(root.path()).unwrap();
        let selected = library
            .archives
            .iter()
            .find(|archive| archive.name == "map.pak")
            .unwrap();
        let inventory = evolve_core::imported_archive_preview_inventory(selected).unwrap();
        let map = MapCandidate {
            name: "Tutorial".into(),
            layers: Vec::new(),
            terrain: Some(PathBuf::from(map_path)),
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
        };
        let source_root = Path::new(&inventory.root);
        let xml = terrain_resource(
            source_root,
            selected,
            Some(&inventory),
            &map,
            None,
            "TerrainTexture.xml",
        )
        .unwrap()
        .unwrap();
        let pak = terrain_resource(
            source_root,
            selected,
            Some(&inventory),
            &map,
            None,
            "terraintexture.pak",
        )
        .unwrap()
        .unwrap();
        assert_eq!(std::fs::read(xml.file().unwrap()).unwrap(), b"XML!");
        assert_eq!(std::fs::read(pak.file().unwrap()).unwrap(), b"PAK!");
        assert!(
            terrain_resource(
                source_root,
                selected,
                Some(&inventory),
                &map,
                None,
                "other.pak"
            )
            .is_err()
        );
    }

    #[test]
    fn imported_grid_autodetection_supports_tutorial_and_ice_crashsite() {
        for side in [2usize, 4usize] {
            let dir = tempfile::tempdir().unwrap();
            let files = dir.path().join("files");
            std::fs::create_dir_all(&files).unwrap();
            for y in 0..side {
                for x in 0..side {
                    let filename = files.join(format!("tile{x}_{y}.raw"));
                    let mut bytes = Vec::new();
                    bytes.extend_from_slice(&4u32.to_le_bytes());
                    bytes.extend_from_slice(&4u32.to_le_bytes());
                    for _ in 0..16 {
                        bytes.extend_from_slice(&[x as u8 * 50, y as u8 * 50, 128]);
                    }
                    std::fs::write(&filename, bytes).unwrap();
                }
            }
            let inventory = scan_directory(dir.path()).unwrap();
            let (paths, resolved_side) = imported_rgb24_tile_grid(&inventory).unwrap().unwrap();
            assert_eq!(resolved_side, side);
            assert_eq!(paths.len(), side * side);
            let atlas = read_terrain_rgb24_tiles(&paths, resolved_side).unwrap();
            assert_eq!(
                atlas.image.dimensions(),
                ((4 * side) as u32, (4 * side) as u32)
            );
            assert_eq!(
                atlas
                    .image
                    .get_pixel((4 * side - 1) as u32, (4 * side - 1) as u32)
                    .0,
                [((side - 1) * 50) as u8, ((side - 1) * 50) as u8, 128, 255]
            );
            std::fs::remove_file(files.join(format!("tile{}_{}.raw", side - 1, side - 1))).unwrap();
            let missing = scan_directory(dir.path()).unwrap();
            assert!(imported_rgb24_tile_grid(&missing).is_err());
        }
    }

    #[test]
    fn map_groups_resolve_parent_map_not_layers_subfolder() {
        assert_eq!(map_group("Levels/Distillery/Layers/A.lyr").1, "Distillery");
        assert_eq!(map_group("Levels/Distillery/Heightmap.dat").1, "Distillery");
        assert_eq!(map_group("MyMap/Layers/A.lyr").1, "MyMap");
    }

    #[test]
    fn imported_tutorial_archive_containers_supply_map_relative_terrain() {
        let dir = tempfile::tempdir().unwrap();
        let library_root = dir.path().join("AssetLibrary");
        let game = dir.path().join("Stage2/Game");
        let map_dir = game.join("levels/Tutorial");
        let legacy = dir.path().join("Legacy/Game/levels/Tutorial");
        std::fs::create_dir_all(&library_root).unwrap();
        std::fs::create_dir_all(&map_dir).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        let mut cry_tiles = Vec::new();
        cry_tiles.extend_from_slice(b"CRY\0");
        cry_tiles.extend_from_slice(&[100, 0]);
        cry_tiles.extend_from_slice(&9u16.to_le_bytes());
        cry_tiles.extend_from_slice(&1u16.to_le_bytes());
        cry_tiles.extend_from_slice(&0u16.to_le_bytes());
        cry_tiles.extend_from_slice(&1.0f32.to_le_bytes());
        cry_tiles.extend_from_slice(&4u16.to_le_bytes());
        cry_tiles.extend_from_slice(&0u16.to_le_bytes());
        cry_tiles.extend_from_slice(&2u32.to_le_bytes());
        cry_tiles.extend_from_slice(&64u32.to_le_bytes());
        cry_tiles.extend_from_slice(&5u16.to_le_bytes());
        for index in [0i16, -1, -1, -1, -1] {
            cry_tiles.extend_from_slice(&index.to_le_bytes());
        }
        for _ in 0..16 {
            cry_tiles.extend_from_slice(&[48, 96, 144, 255]);
        }

        let install = |id: &str, name: &str, source: &Path, members: Vec<(&str, Vec<u8>)>| {
            let directory = library_root.join(format!("{name}__{id}"));
            let root = directory.join("files");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(source, b"original archive untouched").unwrap();
            let mut entries = Vec::new();
            for (relative, data) in members {
                let target = root.join(relative);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(&target, &data).unwrap();
                entries.push(serde_json::json!({"path":relative,"bytes":data.len()}));
            }
            std::fs::write(
                directory.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version": 1, "name": name,
                    "source": source.to_string_lossy(), "entries": entries
                }))
                .unwrap(),
            )
            .unwrap();
        };
        install(
            "11111111111111111111",
            "levels_tutorial.pak",
            &game.join("levels_tutorial.pak"),
            vec![("Levels/Tutorial/Heightmap.dat", b"MAP!".to_vec())],
        );
        install(
            "22222222222222222222",
            "level.pak",
            &map_dir.join("level.pak"),
            vec![(
                "LevelData/TerrainTexture.xml",
                b"<TerrainTexture/>".to_vec(),
            )],
        );
        install(
            "33333333333333333333",
            "terraintexture.pak",
            &map_dir.join("terraintexture.pak"),
            vec![
                ("terrain/sector.dat", cry_tiles.clone()),
                ("other/0.bin", vec![1]),
                ("other/1.bin", vec![2]),
                ("other/2.bin", vec![3]),
            ],
        );
        install(
            "44444444444444444444",
            "terraintexture.pak",
            &legacy.join("terraintexture.pak"),
            vec![("terrain/sector.dat", cry_tiles)],
        );
        std::fs::write(
            library_root.join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "builds": {
                    "stage-two": {"label":"Stage 2", "members":{
                        "11111111111111111111":{},
                        "33333333333333333333":{},
                        "44444444444444444444":{}
                    }}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let catalog = imported_library(&library_root).unwrap();
        let selected = catalog
            .archives
            .iter()
            .find(|pak| pak.name == "levels_tutorial.pak")
            .unwrap();
        let inventory = imported_archive_files(selected).unwrap();
        let map = MapCandidate {
            name: "Tutorial".into(),
            layers: Vec::new(),
            terrain: Some(PathBuf::from("Levels/Tutorial/Heightmap.dat")),
            vegetation: None,
            terrain_texture_xml: None,
            terrain_texture_pak: None,
        };
        let root = Path::new(&inventory.root);
        let xml = terrain_resource(
            root,
            selected,
            Some(&inventory),
            &map,
            None,
            "TerrainTexture.xml",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            std::fs::read(xml.file().unwrap()).unwrap(),
            b"<TerrainTexture/>"
        );
        let pak = terrain_resource(
            root,
            selected,
            Some(&inventory),
            &map,
            None,
            "terraintexture.pak",
        )
        .unwrap()
        .unwrap();
        let path = pak.file().unwrap();
        // Windows canonical paths carry a \\?\ device prefix; compare
        // normalized absolute paths, not a raw tempdir path.
        assert!(path.starts_with(std::fs::canonicalize(&library_root).unwrap()));
        assert_eq!(read_terrain_texture_header(path).unwrap().version, 9);
        assert_eq!(read_terrain_texture_atlas(path).unwrap().decoded_tiles, 1);
    }

    #[test]
    fn only_level_archives_auto_open() {
        let mut pak = ImportedPak {
            id: "a".repeat(20),
            name: "Textures.pak".into(),
            source: String::new(),
            directory: String::new(),
            group: "Textures".into(),
            description: String::new(),
            entries: 0,
            extracted_bytes: 0,
            warnings: 0,
            builds: vec![],
        };
        assert!(!likely_level_pak(&pak));
        pak.name = "Levels_Island.pak".into();
        pak.group = "Levels".into();
        assert!(likely_level_pak(&pak));
    }
}
