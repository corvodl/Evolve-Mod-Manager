//! Verified map discovery for a selected imported PAK.
//!
//! Only manifest-listed files in the selected archive are used. There is no
//! guessed global map root, automatic editing, or modification of game files.
use crate::{asset_cache::SharedCache, level_scene_renderer};
use evolve_core::{
    ImportedPak, LevelScene, ModelInfo, RenderMesh, TerrainInfo, imported_archive_files,
    parse_level_scene, read_terrain,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(super) struct MapCandidate {
    pub name: String,
    pub layers: Vec<PathBuf>,
    pub terrain: Option<PathBuf>,
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
    if let Some(index) = parts.iter().position(|p| {
        p.eq_ignore_ascii_case("levels") || p.eq_ignore_ascii_case("maps")
    }) {
        if let Some(name) = parts.get(index + 1) {
            return (
                parts[..=index + 1].join("/").to_ascii_lowercase(),
                (*name).to_string(),
            );
        }
    }
    // A Layer subdirectory is not itself a separate map.
    let folder = parts.len().saturating_sub(1);
    let map_index = if folder > 0 && parts[folder - 1].eq_ignore_ascii_case("layers") {
        folder.saturating_sub(2)
    } else {
        folder.saturating_sub(1)
    };
    let name = parts.get(map_index).copied().unwrap_or("Imported level");
    (parts[..=map_index].join("/").to_ascii_lowercase(), name.to_string())
}

fn verified_source(root: &Path, relative: &Path, max_bytes: u64) -> Result<PathBuf, String> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(format!("Level source unavailable or too large: {}", path.display()));
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
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let mut groups = BTreeMap::<String, MapCandidate>::new();
    for entry in &inventory.entries {
        let relative = entry.relative_path.replace('\\', "/");
        let lower = relative.to_ascii_lowercase();
        let mission = lower.ends_with("/mission.xml") || lower == "mission.xml";
        let layer = lower.ends_with(".lyr");
        let terrain = lower.ends_with("/heightmap.dat") || lower == "heightmap.dat";
        if !(mission || layer || terrain) {
            continue;
        }
        let (key, name) = map_group(&relative);
        let group = groups.entry(key).or_insert_with(|| MapCandidate {
            name,
            layers: Vec::new(),
            terrain: None,
        });
        let path = PathBuf::from(&entry.relative_path);
        if terrain {
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
        if candidate.layers.iter().any(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".lyr")) {
            candidate.layers.retain(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".lyr"));
        }
    }
    maps.retain(|map| !map.layers.is_empty() || map.terrain.is_some());
    maps.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    if maps.is_empty() {
        return Err("This imported PAK contains no verified mission XML, LYR placements, or Heightmap.dat. Select a map archive instead.".into());
    }
    Ok((root, maps))
}

pub(super) fn open(archive: ImportedPak, index: usize, cache: &SharedCache) -> Result<LoadedMap, String> {
    let (root, maps) = discover(&archive)?;
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
        diagnostics.push(format!("Only 128 of {} layer sources were read.", chosen.layers.len()));
    }
    let mut source = chosen.layers.first().map(|p| root.join(p));
    let mut scene = None;
    if !sources.is_empty() {
        let slices = sources.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect::<Vec<_>>();
        match parse_level_scene(&slices) {
            Ok(parsed) => scene = Some(parsed),
            Err(error) => {
                diagnostics.push(format!("Combined layers could not be assembled: {error}. Trying individual sources."));
                for (name, bytes) in &sources {
                    if let Ok(parsed) = parse_level_scene(&[(name.as_str(), bytes)]) {
                        source = Some(root.join(name));
                        scene = Some(parsed);
                        diagnostics.push("Showing a single readable layer; some placements may be absent.".into());
                        break;
                    }
                }
            }
        }
    }
    let terrain = if let Some(relative) = &chosen.terrain {
        let exact = verified_source(&root, relative, 512 * 1024 * 1024)?;
        match read_terrain(&exact, 129) {
            Ok(data) => {
                let info = data.info.clone();
                Some((RenderMesh {
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
                }, info))
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
    let full_scene = level_scene_renderer::assemble(
        &archive, scene.as_ref(), terrain.as_ref().map(|(mesh, _)| mesh), cache,
    )?;
    if let Some(assembled) = &full_scene {
        diagnostics.push(assembled.report.status());
    }
    Ok(LoadedMap { archive, maps, index, scene, source, terrain, full_scene, diagnostics })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_groups_resolve_parent_map_not_layers_subfolder() {
        assert_eq!(map_group("Levels/Distillery/Layers/A.lyr").1, "Distillery");
        assert_eq!(map_group("Levels/Distillery/Heightmap.dat").1, "Distillery");
        assert_eq!(map_group("MyMap/Layers/A.lyr").1, "MyMap");
    }

    #[test]
    fn only_level_archives_auto_open() {
        let mut pak = ImportedPak {
            id: "a".repeat(20), name: "Textures.pak".into(), source: String::new(),
            directory: String::new(), group: "Textures".into(), description: String::new(),
            entries: 0, extracted_bytes: 0, warnings: 0, builds: vec![],
        };
        assert!(!likely_level_pak(&pak));
        pak.name = "Levels_Island.pak".into();
        pak.group = "Levels".into();
        assert!(likely_level_pak(&pak));
    }
}
