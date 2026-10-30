//! Verified map discovery for a selected imported PAK.
//!
//! Only manifest-listed files in the selected archive are used. There is no
//! guessed global map root, automatic editing, or modification of game files.
use crate::{
    asset_cache::SharedCache, auto_resources, level_scene_renderer, modding_projects::ProjectRow,
};
use evolve_core::{
    ImportedPak, Inventory, LevelScene, ModelInfo, RenderMesh, TerrainInfo, imported_archive_files,
    imported_archive_preview_inventory, imported_library,
    parse_level_scene, read_terrain, read_terrain_paint, read_terrain_texture_atlas,
    read_terrain_texture_header, scan_directory,
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
    source.ancestors().find(|parent| {
        parent.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Game"))
    }).and_then(|game| fs::canonicalize(game).ok())
}

fn source_map_parent(pak: &ImportedPak, game: &Path, map_folder: &str) -> bool {
    let Some(parent) = Path::new(&pak.source).parent() else { return false };
    let Ok(canonical) = fs::canonicalize(parent) else { return false };
    let Ok(relative) = canonical.strip_prefix(game) else { return false };
    relative.to_string_lossy().replace('\\', "/").eq_ignore_ascii_case(map_folder)
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

fn imported_map_terrain(
    selected: &ImportedPak,
    folder: &str,
    member_name: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(game) = source_game_root(selected) else { return Ok(None) };
    let Some(library_root) = Path::new(&selected.directory).parent() else { return Ok(None) };
    let library = imported_library(library_root).map_err(|error| error.to_string())?;
    let selected_build = (selected.builds.len() == 1).then(|| selected.builds[0].as_str());
    let mut found = Vec::new();
    let mut matching_pak_details = Vec::new();
    for archive in &library.archives {
        if archive.id == selected.id
            || (selected_build.is_none()
                || !archive.builds.iter().any(|build| Some(build.as_str()) == selected_build))
            || source_game_root(archive).as_deref() != Some(game.as_path())
            || !source_map_parent(archive, &game, folder)
        {
            continue;
        }
        let is_terrain_pak = archive.name.eq_ignore_ascii_case("terraintexture.pak");
        let is_level_pak = archive.name.eq_ignore_ascii_case("level.pak");
        if !is_terrain_pak && !is_level_pak { continue }
        let inventory = imported_archive_preview_inventory(archive).map_err(|e| e.to_string())?;
        if member_name.eq_ignore_ascii_case("TerrainTexture.xml") {
            for relative in [
                "LevelData/TerrainTexture.xml",
                "TerrainTexture.xml",
                &format!("{folder}/LevelData/TerrainTexture.xml"),
                &format!("{folder}/TerrainTexture.xml"),
            ] {
                if let Some(path) = imported_terrain_member(&inventory, relative)? {
                    found.push(path);
                }
            }
        } else if member_name.eq_ignore_ascii_case("terraintexture.pak") && is_terrain_pak {
            // The imported 'terraintexture.pak' ZIP is a container, not an
            // extracted file named terraintexture.pak. Look inside *that exact
            // source archive* for a member with a verified CRY terrain header.
            // Do not pass the outer ZIP archive to the baked-quadtree decoder.
            matching_pak_details.push(format!(
                "{} ({} extracted members)",
                archive.name, inventory.entries.len()
            ));
            for entry in &inventory.entries {
                if entry.byte_size < 24 || entry.byte_size > 2 * 1024 * 1024 * 1024 {
                    continue;
                }
                let Some(path) = imported_terrain_member(&inventory, &entry.relative_path)? else {
                    continue;
                };
                if read_terrain_texture_header(&path).is_ok() {
                    found.push(path);
                }
            }
        }
    }
    found.sort();
    found.dedup();
    match found.len() {
        0 if !matching_pak_details.is_empty() => Err(format!(
            "Imported {} for {folder}, but none of its extracted members has a valid CRY baked-terrain tile header. It is indexed as a ZIP/signed archive rather than a raw baked-terrain stream; inspect its original member paths before implementing that variant.",
            matching_pak_details.join(", ")
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
) -> Result<Option<PathBuf>, String> {
    if let Some(relative) = manifest_entry {
        return verified_source(source_root, relative, 2 * 1024 * 1024 * 1024).map(Some);
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
        if let Some(path) = auto_resources::exact(
            selected_inventory,
            &requested,
            library_root,
            build,
        )? {
            return Ok(Some(path));
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
                // A cross-layer duplicate ID must not make the imported map
                // lose every other valid layer. For a READ-ONLY preview, parse
                // independently and union verified placements without changing
                // source GUIDs or authoring files. Cross-layer parenting cannot
                // be reconstructed reliably on this recovery path.
                if project.is_none() {
                    let mut recovered = LevelScene {
                        schema: 1,
                        objects: Vec::new(),
                        diagnostics: Vec::new(),
                    };
                    let mut valid_layers = 0usize;
                    let mut duplicate_ids = 0usize;
                    let mut seen_ids = std::collections::HashSet::new();
                    for (name, bytes) in &sources {
                        match parse_level_scene(&[(name.as_str(), bytes)]) {
                            Ok(parsed) => {
                                valid_layers += 1;
                                for object in parsed.objects {
                                    if seen_ids.insert(object.id.clone()) {
                                        recovered.objects.push(object);
                                    } else {
                                        duplicate_ids += 1;
                                    }
                                }
                            }
                            Err(layer_error) => diagnostics.push(format!(
                                "Layer {name} could not be recovered: {layer_error}"
                            )),
                        }
                    }
                    if !recovered.objects.is_empty() {
                        diagnostics.push(format!(
                            "Read-only recovery assembled {valid_layers} individually valid layers ({} unique placements; {duplicate_ids} repeated IDs omitted). Cross-layer parent transforms may be incomplete.",
                            recovered.objects.len()
                        ));
                        scene = Some(recovered);
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
        match (&terrain_xml, &terrain_pak) {
            (Some(xml), Some(pak)) => match read_terrain_texture_header(pak) {
                Ok(header) => {
                    diagnostics.push(format!(
                        "Original map-owned baked terrain v{} found in the imported build ({} texture layers; formats/sizes: {}). TerrainTexture.xml: {}",
                        header.version, header.layers.len(),
                        header.layers.iter().map(|layer| format!(
                            "{}×{} / format {} / {} bytes",
                            layer.sector_pixels, layer.sector_pixels,
                            layer.texture_format, layer.sector_bytes
                        )).collect::<Vec<_>>().join("; "),
                        xml.display()
                    ));
                }
                Err(error) => diagnostics.push(format!(
                    "Original baked terrain package {} could not be decoded: {error}",
                    pak.display()
                )),
            },
            (Some(_), None) => diagnostics.push(
                "Original baked ground color unavailable: map-owned terraintexture.pak missing from the selected imported build. Import the matching map terrain PAK; another map's atlas must not be substituted.".into()
            ),
            (None, Some(_)) => diagnostics.push(
                "Original baked ground color unavailable: map-owned TerrainTexture.xml missing from the selected imported build. Import the matching map terrain metadata.".into()
            ),
            (None, None) => diagnostics.push(
                "Original baked ground color unavailable: both map-owned TerrainTexture.xml and terraintexture.pak are missing from the selected imported build. The terrain layer-ID bitmap does not include baked ground color.".into()
            ),
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
        && terrain_xml.is_some()
        && let Some(package) = &terrain_pak
    {
        match read_terrain_texture_atlas(package).map_err(|err| err.to_string()) {
            Ok(atlas) => {
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
                    "Original baked terrain: decoded {} indexed surface tiles (BC/RGBA format {}) into 2048×2048 ground-color atlas. {}",
                    atlas.decoded_tiles, atlas.color_format,
                    if applied { "Applied to terrain GPU material." }
                    else { "No verified terrain material span to receive the color atlas." }
                ));
            }
            Err(error) => diagnostics.push(format!(
                "Original terrain color unavailable: {error}; keeping neutral ground material."
            )),
        }
    }
    if let Some(assembled) = &full_scene {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!terrain_virtual_candidates("levels/tutorial", "TerrainTexture.xml")
            .unwrap()
            .iter()
            .any(|candidate| candidate.contains("levels/desert_canyon")));
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
                })).unwrap(),
            ).unwrap();
        }
        std::fs::write(
            root.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "builds": {
                    "stage-two": {"label": "Stage 2",
                        "members": {"11111111111111111111": {}, "22222222222222222222": {}}}
                }
            })).unwrap(),
        ).unwrap();
        let library = evolve_core::imported_library(root.path()).unwrap();
        let selected = library.archives.iter()
            .find(|archive| archive.name == "map.pak").unwrap();
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
            source_root, selected, Some(&inventory), &map, None, "TerrainTexture.xml"
        ).unwrap().unwrap();
        let pak = terrain_resource(
            source_root, selected, Some(&inventory), &map, None, "terraintexture.pak"
        ).unwrap().unwrap();
        assert_eq!(std::fs::read(xml).unwrap(), b"XML!");
        assert_eq!(std::fs::read(pak).unwrap(), b"PAK!");
        assert!(terrain_resource(
            source_root, selected, Some(&inventory), &map, None, "other.pak"
        ).is_err());
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
                })).unwrap(),
            ).unwrap();
        };
        install(
            "11111111111111111111", "levels_tutorial.pak",
            &game.join("levels_tutorial.pak"),
            vec![("Levels/Tutorial/Heightmap.dat", b"MAP!".to_vec())],
        );
        install(
            "22222222222222222222", "level.pak",
            &map_dir.join("level.pak"),
            vec![("LevelData/TerrainTexture.xml", b"<TerrainTexture/>".to_vec())],
        );
        install(
            "33333333333333333333", "terraintexture.pak",
            &map_dir.join("terraintexture.pak"),
            vec![
                ("terrain/sector.dat", cry_tiles.clone()),
                ("other/0.bin", vec![1]),
                ("other/1.bin", vec![2]),
                ("other/2.bin", vec![3]),
            ],
        );
        install(
            "44444444444444444444", "terraintexture.pak",
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
                        "22222222222222222222":{},
                        "33333333333333333333":{},
                        "44444444444444444444":{}
                    }}
                }
            })).unwrap(),
        ).unwrap();
        let catalog = imported_library(&library_root).unwrap();
        let selected = catalog.archives.iter()
            .find(|pak| pak.name == "levels_tutorial.pak").unwrap();
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
            root, selected, Some(&inventory), &map, None, "TerrainTexture.xml"
        ).unwrap().unwrap();
        assert_eq!(std::fs::read(xml).unwrap(), b"<TerrainTexture/>");
        let pak = terrain_resource(
            root, selected, Some(&inventory), &map, None, "terraintexture.pak"
        ).unwrap().unwrap();
        assert!(pak.starts_with(&library_root));
        assert_eq!(read_terrain_texture_header(&pak).unwrap().version, 9);
        assert_eq!(read_terrain_texture_atlas(&pak).unwrap().decoded_tiles, 1);
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
