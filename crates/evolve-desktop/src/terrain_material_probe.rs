//! Read-only, build-scoped audit of CryEngine terrain material dependencies.
//! This deliberately does not invent a terrain blending equation. Original
//! Terrain.Layer parameters and split DDS sources remain game-authored.
use evolve_core::{
    ImportedPak, TerrainPaintMap, imported_archive_preview_inventory, imported_library,
    read_material, scoped_texture_candidates, inspect_dds,
};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_ARCHIVES: usize = 1024;
const MAX_TERRAIN_ASSETS: usize = 200_000;
const MAX_REFERENCES: usize = 256;

#[derive(Clone, Debug)]
struct Entry {
    archive: String,
    root: PathBuf,
    relative: String,
    size: u64,
}

fn safe_virtual(source: &str) -> Option<String> {
    let normalized = source.trim().replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    let pieces: Vec<_> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if pieces.is_empty() || pieces.iter().any(|p| *p == "." || *p == "..") {
        return None;
    }
    Some(pieces.join("/").to_ascii_lowercase())
}

fn game_dir(pak: &ImportedPak) -> Option<PathBuf> {
    Path::new(&pak.source).ancestors()
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("game")))
        .and_then(|p| fs::canonicalize(p).ok())
}

fn verified(entry: &Entry) -> Result<PathBuf, String> {
    let root = fs::canonicalize(&entry.root).map_err(|e| e.to_string())?;
    let joined = entry.root.join(&entry.relative);
    let md = fs::symlink_metadata(&joined).map_err(|e| e.to_string())?;
    if !md.file_type().is_file() || md.len() != entry.size {
        return Err(format!("{}: extracted member changed or unsafe", entry.archive));
    }
    let path = fs::canonicalize(joined).map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err(format!("{}: member outside verified extraction", entry.archive));
    }
    Ok(path)
}

fn find<'a>(catalog: &'a HashMap<String, Vec<Entry>>, path: &str)
    -> Result<Option<&'a Entry>, String>
{
    let key = safe_virtual(path).ok_or_else(|| format!("Unsafe authored path {path}"))?;
    let Some(hits) = catalog.get(&key) else { return Ok(None); };
    if hits.len() != 1 {
        return Err(format!("{key}: {} matching imported PAKs; refusing arbitrary precedence", hits.len()));
    }
    Ok(hits.first())
}

fn is_terrain_dependency(path: &str) -> bool {
    let is_cooked = path.ends_with(".mtl") || path.ends_with(".dds")
        || (path.contains(".dds.") && path.rsplit_once(".dds.")
            .is_some_and(|(_, suffix)| suffix.parse::<u8>().is_ok_and(|n| n <= 19)));
    is_cooked && (
        path.starts_with("materials/terrain/") ||
        path.starts_with("textures/terrain/") ||
        (path.starts_with("levels/") && path.contains("/textures/terrain/"))
    )
}

fn material_path(path: &str) -> Option<String> {
    let key = safe_virtual(path)?;
    if key.is_empty() { return None; }
    if key.ends_with(".mtl") { Some(key) } else { Some(format!("{key}.mtl")) }
}

/// Reviews original materials from imported PAKs in the *same installed Game
/// directory* as the map. Missing/ambiguous materials never fall back to a
/// different build, similar basename, or a supplied ZIP copied into Git.
pub(super) fn audit(selected: &ImportedPak, paint: &TerrainPaintMap) -> Vec<String> {
    let mut messages = Vec::new();
    let mut paths = BTreeSet::new();
    for surface in &paint.surface_types {
        if let Some(name) = material_path(&surface.detail_material) {
            paths.insert(name);
        }
    }
    for layer in &paint.layers {
        if let Some(name) = material_path(&layer.surface_type) {
            paths.insert(name);
        }
    }
    if paths.is_empty() { return messages; }
    let Some(game) = game_dir(selected) else {
        return vec!["Terrain material dependency audit unavailable: selected map's original installed Game directory cannot be verified.".into()];
    };
    let Some(library_root) = Path::new(&selected.directory).parent() else {
        return vec!["Terrain material dependency audit unavailable: no imported Asset Library.".into()];
    };
    let library = match imported_library(library_root) {
        Ok(library) => library,
        Err(error) => return vec![format!("Cannot inspect same-build terrain material PAKs: {error}")],
    };
    let mut archives = vec![selected.clone()];
    let selected_build = selected.builds.first().map(String::as_str);
    for pak in library.archives {
        if pak.id == selected.id || game_dir(&pak).as_deref() != Some(game.as_path()) {
            continue;
        }
        if let Some(build) = selected_build {
            if !pak.builds.is_empty() && !pak.builds.iter().any(|name| name == build) {
                continue;
            }
        } else if pak.builds.len() > 1 {
            continue;
        }
        archives.push(pak);
        if archives.len() >= MAX_ARCHIVES { break; }
    }
    let mut catalog = HashMap::<String, Vec<Entry>>::new();
    let mut indexed = 0usize;
    for pak in &archives {
        let Ok(inventory) = imported_archive_preview_inventory(pak) else { continue; };
        for asset in inventory.entries {
            let Some(original) = safe_virtual(&asset.relative_path) else { continue; };
            // The importer's optional "files/" wrapper is not a game path.
            let key = original.strip_prefix("files/").unwrap_or(&original);
            if !is_terrain_dependency(key) { continue; }
            indexed += 1;
            if indexed > MAX_TERRAIN_ASSETS {
                return vec!["Terrain material lookup exceeded bounded source inventory; no arbitrary asset was selected.".into()];
            }
            catalog.entry(key.to_string()).or_default().push(Entry {
                archive: pak.name.clone(),
                root: PathBuf::from(&inventory.root),
                relative: asset.relative_path,
                size: asset.byte_size,
            });
        }
    }
    let mut loaded = 0usize;
    let mut absent = 0usize;
    let mut ambiguous = 0usize;
    let mut decoded = 0usize;
    let mut missing_textures = BTreeSet::new();
    let mut broken_textures = BTreeSet::new();
    let mut mismatched_shaders = BTreeSet::new();
    let mut details = Vec::new();
    for material in paths.iter().take(MAX_REFERENCES) {
        let entry = match find(&catalog, material) {
            Ok(Some(entry)) => entry,
            Ok(None) => { absent += 1; details.push(format!("{material}: missing .mtl")); continue; },
            Err(error) => { ambiguous += 1; details.push(error); continue; }
        };
        let source = match verified(entry).and_then(|path| read_material(&path).map_err(|e| e.to_string())) {
            Ok(doc) => doc,
            Err(error) => { details.push(format!("{material}: {error}")); continue; }
        };
        loaded += 1;
        for slot in &source.slots {
            if !slot.shader.eq_ignore_ascii_case("Terrain.Layer") {
                mismatched_shaders.insert(format!("{material}: {}", slot.shader));
            }
            for map in &slot.maps {
                // Authored .tif references resolve to the cooked .dds.0
                // in materials_basic.pak / other exact same-build sources.
                // The existing DDS inspector verifies every required split
                // mip and rejects an incomplete or mismatched chain.
                let candidates = match scoped_texture_candidates(&map.path, material) {
                    Ok(candidates) => candidates,
                    Err(error) => { broken_textures.insert(format!("{material}: {error}")); continue; }
                };
                let mut found = false;
                for candidate in candidates {
                    match find(&catalog, &candidate) {
                        Ok(Some(entry)) => {
                            found = true;
                            match verified(entry).and_then(|path| inspect_dds(&path).map_err(|e| e.to_string())) {
                                Ok(_) => decoded += 1,
                                Err(error) => {
                                    broken_textures.insert(format!("{material} {}: {error}", map.role));
                                }
                            }
                            break;
                        }
                        Ok(None) => {},
                        Err(error) => {
                            broken_textures.insert(error);
                            found = true;
                            break;
                        }
                    }
                }
                if !found { missing_textures.insert(format!("{material} {} -> {}", map.role, map.path)); }
            }
        }
    }
    messages.push(format!(
        "Same-install terrain material audit: {loaded}/{} declared .mtl definitions loaded; {decoded} cooked DDS references have complete verified mip chains; {absent} missing definitions, {ambiguous} ambiguous definitions, {} missing and {} invalid/ambiguous texture references. Sources remain original imported PAK files; shaders are NOT yet blended into the ground.",
        paths.len(), missing_textures.len(), broken_textures.len(),
    ));
    if !mismatched_shaders.is_empty() {
        messages.push(format!("Non-Terrain.Layer source shaders (up to 5): {}", mismatched_shaders.iter().take(5).cloned().collect::<Vec<_>>().join("; ")));
    }
    for (name, samples) in [
        ("Unresolved terrain materials", &details),
        ("Missing terrain DDS", &missing_textures.iter().cloned().collect::<Vec<_>>()),
        ("Invalid terrain DDS", &broken_textures.iter().cloned().collect::<Vec<_>>()),
    ] {
        if !samples.is_empty() {
            messages.push(format!("{name} (first 6): {}", samples.iter().take(6).cloned().collect::<Vec<_>>().join("; ")));
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_cooked_terrain_names_are_indexed() {
        assert!(is_terrain_dependency("materials/terrain/acid_dirt_a_terr_highpass.dds.0"));
        assert!(is_terrain_dependency("materials/terrain/acid_dirt_a_terr_highpass.dds.6"));
        assert!(is_terrain_dependency("materials/terrain/acid_dirt_a_highpass.mtl"));
        assert!(is_terrain_dependency("levels/forest_slagtown/textures/terrain/ash.dds.0"));
        assert!(!is_terrain_dependency("other_actor/textures/acid_dirt_a.dds.0"));
        assert!(!is_terrain_dependency("materials/terrain/evil.dds.999"));
        assert_eq!(material_path("Materials\\Terrain\\Acid_Dirt_A_Highpass"),
            Some("materials/terrain/acid_dirt_a_highpass.mtl".into()));
        assert!(safe_virtual("../materials/terrain/acid_dirt_a.mtl").is_none());
    }

    #[test]
    fn never_choose_a_duplicate_source_by_import_order() {
        let mut catalog = HashMap::new();
        let duplicated = Entry {
            archive: "a.pak".into(), root: PathBuf::new(),
            relative: "materials/terrain/dirt.mtl".into(), size: 10,
        };
        catalog.insert("materials/terrain/dirt.mtl".into(), vec![duplicated.clone(), duplicated]);
        assert!(find(&catalog, "materials/terrain/dirt.mtl").is_err());
        assert!(find(&catalog, "materials/terrain/missing.mtl").unwrap().is_none());
    }
}
