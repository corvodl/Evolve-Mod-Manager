//! Read-only, build-scoped audit of CryEngine terrain material dependencies.
//! This deliberately does not invent a terrain blending equation. Original
//! Terrain.Layer parameters and split DDS sources remain game-authored.
use evolve_core::{
    ImportedPak, TerrainPaintMap, imported_archive_preview_inventory, imported_library,
    inspect_dds, read_material, scoped_texture_candidates,
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
    Path::new(&pak.source)
        .ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("game"))
        })
        .and_then(|p| fs::canonicalize(p).ok())
}

fn verified(entry: &Entry) -> Result<PathBuf, String> {
    let root = fs::canonicalize(&entry.root).map_err(|e| e.to_string())?;
    let joined = entry.root.join(&entry.relative);
    let md = fs::symlink_metadata(&joined).map_err(|e| e.to_string())?;
    if !md.file_type().is_file() || md.len() != entry.size {
        return Err(format!(
            "{}: extracted member changed or unsafe",
            entry.archive
        ));
    }
    let path = fs::canonicalize(joined).map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err(format!(
            "{}: member outside verified extraction",
            entry.archive
        ));
    }
    Ok(path)
}

fn find<'a>(
    catalog: &'a HashMap<String, Vec<Entry>>,
    path: &str,
) -> Result<Option<&'a Entry>, String> {
    let key = safe_virtual(path).ok_or_else(|| format!("Unsafe authored path {path}"))?;
    let Some(hits) = catalog.get(&key) else {
        return Ok(None);
    };
    if hits.len() != 1 {
        return Err(format!(
            "{key}: {} matching imported PAKs; refusing arbitrary precedence",
            hits.len()
        ));
    }
    Ok(hits.first())
}

fn is_terrain_dependency(path: &str) -> bool {
    let is_cooked = path.ends_with(".mtl")
        || path.ends_with(".dds")
        || (path.contains(".dds.")
            && path
                .rsplit_once(".dds.")
                .is_some_and(|(_, suffix)| suffix.parse::<u8>().is_ok_and(|n| n <= 19)));
    is_cooked
        && (path.starts_with("materials/terrain/")
            || (path.ends_with(".mtl") && path.starts_with("materials/universal/default/"))
            || path.starts_with("textures/terrain/")
            || (path.starts_with("levels/") && path.contains("/textures/terrain/")))
}

fn material_path(path: &str) -> Option<String> {
    let key = safe_virtual(path)?;
    if key.is_empty() {
        return None;
    }
    if key.ends_with(".mtl") {
        Some(key)
    } else {
        Some(format!("{key}.mtl"))
    }
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
    if paths.is_empty() {
        return messages;
    }
    let Some(game) = game_dir(selected) else {
        return vec!["Terrain material dependency audit unavailable: selected map's original installed Game directory cannot be verified.".into()];
    };
    let Some(library_root) = Path::new(&selected.directory).parent() else {
        return vec![
            "Terrain material dependency audit unavailable: no imported Asset Library.".into(),
        ];
    };
    let library = match imported_library(library_root) {
        Ok(library) => library,
        Err(error) => {
            return vec![format!(
                "Cannot inspect same-build terrain material PAKs: {error}"
            )];
        }
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
        if archives.len() >= MAX_ARCHIVES {
            break;
        }
    }
    let mut catalog = HashMap::<String, Vec<Entry>>::new();
    let mut indexed = 0usize;
    for pak in &archives {
        let Ok(inventory) = imported_archive_preview_inventory(pak) else {
            continue;
        };
        for asset in inventory.entries {
            let Some(original) = safe_virtual(&asset.relative_path) else {
                continue;
            };
            // The importer's optional "files/" wrapper is not a game path.
            let key = original.strip_prefix("files/").unwrap_or(&original);
            if !is_terrain_dependency(key) {
                continue;
            }
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
            Ok(None) => {
                absent += 1;
                details.push(format!("{material}: missing .mtl"));
                continue;
            }
            Err(error) => {
                ambiguous += 1;
                details.push(error);
                continue;
            }
        };
        let source = match verified(entry)
            .and_then(|path| read_material(&path).map_err(|e| e.to_string()))
        {
            Ok(doc) => doc,
            Err(error) => {
                details.push(format!("{material}: {error}"));
                continue;
            }
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
                    Err(error) => {
                        broken_textures.insert(format!("{material}: {error}"));
                        continue;
                    }
                };
                let mut found = false;
                for candidate in candidates {
                    match find(&catalog, &candidate) {
                        Ok(Some(entry)) => {
                            found = true;
                            match verified(entry)
                                .and_then(|path| inspect_dds(&path).map_err(|e| e.to_string()))
                            {
                                Ok(_) => decoded += 1,
                                Err(error) => {
                                    broken_textures
                                        .insert(format!("{material} {}: {error}", map.role));
                                }
                            }
                            break;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            broken_textures.insert(error);
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    missing_textures.insert(format!("{material} {} -> {}", map.role, map.path));
                }
            }
        }
    }
    messages.push(format!(
        "Same-install terrain material audit: {loaded}/{} declared .mtl definitions loaded; {decoded} cooked DDS heads pass header inspection (full mip decoding is checked separately); {absent} missing definitions, {ambiguous} ambiguous definitions, {} missing and {} invalid/ambiguous texture references. Sources remain original imported PAK files; shaders are NOT yet blended into the ground.",
        paths.len(), missing_textures.len(), broken_textures.len(),
    ));
    if !mismatched_shaders.is_empty() {
        messages.push(format!(
            "Non-Terrain.Layer source shaders (up to 5): {}",
            mismatched_shaders
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    for (name, samples) in [
        ("Unresolved terrain materials", &details),
        (
            "Missing terrain DDS",
            &missing_textures.iter().cloned().collect::<Vec<_>>(),
        ),
        (
            "Invalid terrain DDS",
            &broken_textures.iter().cloned().collect::<Vec<_>>(),
        ),
    ] {
        if !samples.is_empty() {
            messages.push(format!(
                "{name} (first 6): {}",
                samples
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_cooked_terrain_names_are_indexed() {
        assert!(is_terrain_dependency(
            "materials/terrain/acid_dirt_a_terr_highpass.dds.0"
        ));
        assert!(is_terrain_dependency(
            "materials/terrain/acid_dirt_a_terr_highpass.dds.6"
        ));
        assert!(is_terrain_dependency(
            "materials/terrain/acid_dirt_a_highpass.mtl"
        ));
        assert!(is_terrain_dependency(
            "levels/forest_slagtown/textures/terrain/ash.dds.0"
        ));
        assert!(!is_terrain_dependency(
            "other_actor/textures/acid_dirt_a.dds.0"
        ));
        assert!(!is_terrain_dependency("materials/terrain/evil.dds.999"));
        assert_eq!(
            material_path("Materials\\Terrain\\Acid_Dirt_A_Highpass"),
            Some("materials/terrain/acid_dirt_a_highpass.mtl".into())
        );
        assert!(safe_virtual("../materials/terrain/acid_dirt_a.mtl").is_none());
    }

    #[test]
    fn never_choose_a_duplicate_source_by_import_order() {
        let mut catalog = HashMap::new();
        let duplicated = Entry {
            archive: "a.pak".into(),
            root: PathBuf::new(),
            relative: "materials/terrain/dirt.mtl".into(),
            size: 10,
        };
        catalog.insert(
            "materials/terrain/dirt.mtl".into(),
            vec![duplicated.clone(), duplicated],
        );
        assert!(find(&catalog, "materials/terrain/dirt.mtl").is_err());
        assert!(
            find(&catalog, "materials/terrain/missing.mtl")
                .unwrap()
                .is_none()
        );
    }
}

// ---- Terrain.Layer high-pass detail compositing (preview) ----
//
// The 3.5.x SDK documents ADD-SIGNED high-pass composition with the baked
// color atlas, white material tint, and per-layer detail strength. We retain
// original source colors and never use unsupported filenames as a fallback.
// The layer-ID bitmap is categorical, NOT an alpha/height blending mask;
// interpolating neighbors is an explicit editor preview approximation.
#[derive(Clone)]
struct DetailLayer {
    image: image::RgbaImage,
    normal: Option<image::RgbaImage>,
    height: Option<image::RgbaImage>,
    tile: [f32; 2],
    strength: f32,
    blend_factor: f32,
    blend_falloff: f32,
    pom_displacement: f32,
}
/// Decoded original diffuse maps stay at their own source resolution until
/// the GPU texture-array upload. No source material is sampled into the
/// low-resolution baked atlas when this verified path is supported.
#[derive(Clone)]
pub(super) struct GpuTerrainLayer {
    pub id: u8,
    pub image: image::RgbaImage,
    pub normal: Option<image::RgbaImage>,
    pub height: Option<image::RgbaImage>,
    pub blend_factor: f32,
    pub blend_falloff: f32,
    pub pom_displacement: f32,
    pub tile: [f32; 2],
    pub strength: f32,
}

#[derive(Clone)]
pub(super) struct GpuTerrainSource {
    /// Categorical LayerId map; RGBA8 red channel is the exact ID byte.
    pub paint: image::RgbaImage,
    pub unit: f32,
    pub layers: Vec<GpuTerrainLayer>,
}

pub(super) struct Preview {
    pub fallback: image::RgbaImage,
    pub gpu: Option<GpuTerrainSource>,
    pub report: String,
}

/// An array texture requires equal source dimensions and at most 16 layers.
fn gpu_source(
    paint: &TerrainPaintMap,
    unit: f32,
    layers: &HashMap<u8, DetailLayer>,
) -> Option<GpuTerrainSource> {
    if layers.is_empty() || layers.len() > 16 {
        return None;
    }
    let dimensions = layers.values().next()?.image.dimensions();
    if dimensions.0 > 2048
        || dimensions.1 > 2048
        || dimensions.0 == 0
        || dimensions.1 == 0
        || layers
            .values()
            .any(|layer| layer.image.dimensions() != dimensions)
    {
        return None;
    }
    let bytes = (dimensions.0 as u64) * (dimensions.1 as u64) * 4 * layers.len() as u64;
    if bytes > 256 * 1024 * 1024 {
        return None;
    }
    let paint_image = image::RgbaImage::from_fn(paint.width, paint.height, |x, y| {
        image::Rgba([paint.id_at(x, y).unwrap_or(255), 0, 0, 255])
    });
    let mut order = layers.iter().collect::<Vec<_>>();
    order.sort_by_key(|item| *item.0);
    Some(GpuTerrainSource {
        paint: paint_image,
        unit,
        layers: order
            .into_iter()
            .map(|(&id, layer)| GpuTerrainLayer {
                id,
                image: layer.image.clone(),
                // Array layers share one size, so a differing map is resampled.
                normal: layer.normal.as_ref().map(|normal| {
                    if normal.dimensions() == dimensions {
                        normal.clone()
                    } else {
                        image::imageops::resize(
                            normal,
                            dimensions.0,
                            dimensions.1,
                            image::imageops::FilterType::Triangle,
                        )
                    }
                }),
                height: layer.height.as_ref().map(|height| {
                    if height.dimensions() == dimensions {
                        height.clone()
                    } else {
                        image::imageops::resize(
                            height,
                            dimensions.0,
                            dimensions.1,
                            image::imageops::FilterType::Triangle,
                        )
                    }
                }),
                blend_factor: layer.blend_factor,
                blend_falloff: layer.blend_falloff,
                pom_displacement: layer.pom_displacement,
                tile: layer.tile,
                strength: layer.strength,
            })
            .collect(),
    })
}

#[derive(Default)]
struct AuthoredDetail {
    height: Option<String>,
    normal: Option<String>,
    tile: [f32; 2],
    strength: f32,
    blend_factor: f32,
    blend_falloff: f32,
    pom_displacement: f32,
}

fn attr(element: &quick_xml::events::BytesStart<'_>, wanted: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == wanted)
        .and_then(|a| a.unescape_value().ok().map(|s| s.to_string()))
}
fn finite_attr(element: &quick_xml::events::BytesStart<'_>, wanted: &[u8]) -> Option<f32> {
    attr(element, wanted)?
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}
fn authored_detail(bytes: &[u8]) -> Result<AuthoredDetail, String> {
    let mut parsed = AuthoredDetail {
        tile: [1.0, 1.0],
        strength: 1.0,
        blend_factor: 16.0,
        blend_falloff: 2.0,
        pom_displacement: 0.0,
        ..Default::default()
    };
    // CryEngine material XML may be stored as CryXmlB in imported PAKs.
    // The normal MTL resolver already decodes it; use the same bounded
    // decoder for optional TexMod/PublicParams rather than silently
    // treating a compiled material as an empty XML document.
    let xml = evolve_core::cryxml::xml_bytes(bytes, 4 * 1024 * 1024).map_err(|e| e.to_string())?;
    let mut reader = quick_xml::Reader::from_reader(xml.as_ref());
    let mut diffuse_open = false;
    loop {
        use quick_xml::events::Event;
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) if e.name().as_ref() == b"Texture" => {
                let role = attr(&e, b"Map").unwrap_or_default();
                diffuse_open = role.eq_ignore_ascii_case("Diffuse");
                if role.eq_ignore_ascii_case("Heightmap") {
                    parsed.height = attr(&e, b"File");
                }
                if role.eq_ignore_ascii_case("Bumpmap") {
                    parsed.normal = attr(&e, b"File");
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"Texture" => {
                let role = attr(&e, b"Map").unwrap_or_default();
                if role.eq_ignore_ascii_case("Bumpmap") {
                    parsed.normal = attr(&e, b"File");
                }
                if role.eq_ignore_ascii_case("Heightmap") {
                    parsed.height = attr(&e, b"File");
                }
            }
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"TexMod" && diffuse_open => {
                if let Some(u) = finite_attr(&e, b"TileU").filter(|n| *n > 0.0 && *n <= 1024.0) {
                    parsed.tile[0] = u;
                }
                if let Some(v) = finite_attr(&e, b"TileV").filter(|n| *n > 0.0 && *n <= 1024.0) {
                    parsed.tile[1] = v;
                }
            }
            Event::End(e) if e.name().as_ref() == b"Texture" => diffuse_open = false,
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"PublicParams" => {
                if let Some(v) = finite_attr(&e, b"DetailTextureStrength") {
                    parsed.strength = v.clamp(0.0, 4.0);
                }
                if let Some(v) = finite_attr(&e, b"BlendFactor") {
                    parsed.blend_factor = v.clamp(0.0, 128.0);
                }
                if let Some(v) = finite_attr(&e, b"PomDisplacement") {
                    parsed.pom_displacement = v.clamp(0.0, 1.0);
                }
                if let Some(v) = finite_attr(&e, b"BlendFalloff") {
                    parsed.blend_falloff = v.clamp(0.0, 128.0);
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => return Err("Unsafe MTL XML".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parsed)
}

fn same_install_catalog(selected: &ImportedPak) -> Result<HashMap<String, Vec<Entry>>, String> {
    let game = game_dir(selected).ok_or("Original Game installation not verifiable")?;
    let root = Path::new(&selected.directory)
        .parent()
        .ok_or("Imported Asset Library root unavailable")?;
    let library = imported_library(root).map_err(|e| e.to_string())?;
    let selected_build = (selected.builds.len() == 1).then(|| selected.builds[0].as_str());
    let mut archives = vec![selected.clone()];
    for pak in library.archives {
        if pak.id == selected.id || game_dir(&pak).as_deref() != Some(game.as_path()) {
            continue;
        }
        if selected_build
            .is_some_and(|label| !pak.builds.is_empty() && !pak.builds.iter().any(|s| s == label))
        {
            continue;
        }
        if selected_build.is_none() && pak.builds.len() > 1 {
            continue;
        }
        archives.push(pak);
        if archives.len() >= MAX_ARCHIVES {
            break;
        }
    }
    let mut catalog = HashMap::<String, Vec<Entry>>::new();
    let mut entries = 0usize;
    for archive in archives {
        let Ok(inventory) = imported_archive_preview_inventory(&archive) else {
            continue;
        };
        for asset in inventory.entries {
            let Some(raw) = safe_virtual(&asset.relative_path) else {
                continue;
            };
            let key = raw.strip_prefix("files/").unwrap_or(&raw);
            if !is_terrain_dependency(key) {
                continue;
            }
            entries += 1;
            if entries > MAX_TERRAIN_ASSETS {
                return Err("Too many imported terrain material dependencies".into());
            }
            catalog.entry(key.to_string()).or_default().push(Entry {
                archive: archive.name.clone(),
                root: PathBuf::from(&inventory.root),
                relative: asset.relative_path,
                size: asset.byte_size,
            });
        }
    }
    Ok(catalog)
}

/// Decode the *complete* original .dds.0 + numbered mip stream, rejecting
/// partial streams and cross-installation ambiguity. Material paths are
/// authoritative: never choose a similarly named texture elsewhere.
fn needs_external_fragment(header: &Path, fragment: &Path) -> bool {
    // Both are canonicalized by verified() before this call.
    header.parent() != fragment.parent()
}

fn load_detail_image(
    catalog: &HashMap<String, Vec<Entry>>,
    authored: &str,
    material: &str,
) -> Result<image::RgbaImage, String> {
    let candidates = scoped_texture_candidates(authored, material)?;
    for candidate in candidates {
        let Some(found) = find(catalog, &candidate)? else {
            continue;
        };
        let header = verified(found)?;
        if candidate.ends_with(".dds") {
            return evolve_core::decode_dds_rgba(&header)
                .map(|(_, image)| image)
                .map_err(|e| e.to_string());
        }
        let Some((prefix, suffix)) = candidate.rsplit_once(".dds.") else {
            continue;
        };
        if suffix != "0" {
            continue;
        }
        // decode_dds_rgba_with_parts already enumerates numbered fragments
        // in the .dds.0 file's own directory. Re-supplying those paths as
        // external fragments makes it reject a complete original PAK texture
        // as "duplicate numbered DDS fragment". Only cross-archive parts
        // belong in the additional slice. textures.pak has many such complete
        // same-directory sets (terrain and particle). Keep manifest checks.
        let mut more = Vec::new();
        for i in 1..20 {
            let part = format!("{prefix}.dds.{i}");
            if let Some(fragment) = find(catalog, &part)? {
                let source = verified(fragment)?;
                if needs_external_fragment(&header, &source) {
                    more.push(source);
                }
            }
        }
        return evolve_core::decode_dds_rgba_with_parts(&header, &more)
            .map(|(_, image)| image)
            .map_err(|e| e.to_string());
    }
    Err(format!(
        "{authored} (authored by {material}) has no exact cooked DDS"
    ))
}

fn bilinear_wrap(image: &image::RgbaImage, uv: [f32; 2]) -> [f32; 3] {
    let (w, h) = image.dimensions();
    let x = uv[0].rem_euclid(1.0) * w as f32 - 0.5;
    let y = uv[1].rem_euclid(1.0) * h as f32 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let a = x - x0;
    let b = y - y0;
    let xi = (x0 as i64).rem_euclid(w as i64) as u32;
    let yi = (y0 as i64).rem_euclid(h as i64) as u32;
    let x1 = (xi + 1) % w;
    let y1 = (yi + 1) % h;
    let mut result = [0.0; 3];
    for (px, py, weight) in [
        (xi, yi, (1.0 - a) * (1.0 - b)),
        (x1, yi, a * (1.0 - b)),
        (xi, y1, (1.0 - a) * b),
        (x1, y1, a * b),
    ] {
        let pixel = image.get_pixel(px, py);
        for c in 0..3 {
            result[c] += pixel[c] as f32 * weight / 255.0;
        }
    }
    result
}

// CPU-only terrain overlays cover an entire map at the baked atlas resolution,
// but the original Terrain.Layer DDS may repeat dozens of texels per atlas
// pixel. Sampling level zero once aliases those texels into colored speckles.
// Prefilter by the exact atlas-to-source footprint; this fixes minification,
// not the missing native per-fragment high-resolution terrain sampling.
fn detail_mips(source: &image::RgbaImage) -> Vec<image::RgbaImage> {
    let mut chain = vec![source.clone()];
    while let Some(last) = chain.last() {
        if last.width() == 1 && last.height() == 1 {
            break;
        }
        chain.push(image::imageops::resize(
            last,
            (last.width() / 2).max(1),
            (last.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        ));
    }
    chain
}

fn detail_sample(mips: &[image::RgbaImage], uv: [f32; 2], footprint_texels: f32) -> [f32; 3] {
    let maximum = (mips.len() - 1) as f32;
    let lod = footprint_texels.max(1.0).log2().clamp(0.0, maximum);
    let low = lod.floor() as usize;
    let high = (low + 1).min(mips.len() - 1);
    let amount = lod - low as f32;
    let a = bilinear_wrap(&mips[low], uv);
    let b = bilinear_wrap(&mips[high], uv);
    std::array::from_fn(|channel| a[channel] * (1.0 - amount) + b[channel] * amount)
}

fn detail_footprint(
    image: &image::RgbaImage,
    tile: [f32; 2],
    paint: &TerrainPaintMap,
    atlas_size: (u32, u32),
    terrain_unit: f32,
) -> f32 {
    let x = (paint.width - 1) as f32 * terrain_unit * tile[0].abs() * image.width() as f32
        / atlas_size.0 as f32;
    let y = (paint.height - 1) as f32 * terrain_unit * tile[1].abs() * image.height() as f32
        / atlas_size.1 as f32;
    x.max(y).max(1.0)
}

fn painted_neighbors(paint: &TerrainPaintMap, uv: [f32; 2]) -> [Option<(u8, f32)>; 4] {
    let x = uv[0].clamp(0.0, 1.0) * (paint.width - 1) as f32;
    let y = uv[1].clamp(0.0, 1.0) * (paint.height - 1) as f32;
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(paint.width - 1);
    let y1 = (y0 + 1).min(paint.height - 1);
    let u = x - x0 as f32;
    let v = y - y0 as f32;
    let mut neighbors = [None; 4];
    for (ix, iy, weight) in [
        (x0, y0, (1.0 - u) * (1.0 - v)),
        (x1, y0, u * (1.0 - v)),
        (x0, y1, (1.0 - u) * v),
        (x1, y1, u * v),
    ] {
        let Some(id) = paint.id_at(ix, iy) else {
            continue;
        };
        if let Some((_, accumulated)) = neighbors
            .iter_mut()
            .flatten()
            .find(|(other, _)| *other == id)
        {
            *accumulated += weight;
        } else if let Some(slot) = neighbors.iter_mut().find(|item| item.is_none()) {
            *slot = Some((id, weight));
        }
    }
    neighbors
}

type TerrainPixelSample<'a> = (&'a DetailLayer, [f32; 3], Option<f32>, f32);

fn compose_detail(
    paint: &TerrainPaintMap,
    atlas_size: (u32, u32),
    terrain_unit: f32,
    layers: &HashMap<u8, DetailLayer>,
) -> image::RgbaImage {
    let (width, height) = atlas_size;
    let neutral = 128.0 / 255.0;
    // Prepare each complete authored image's mip chain once, not per pixel.
    // This keeps the existing format-verified source DDS unchanged.
    let filtered: HashMap<_, _> = layers
        .iter()
        .map(|(&id, layer)| {
            (
                id,
                (
                    detail_mips(&layer.image),
                    layer.height.as_ref().map(detail_mips),
                    detail_footprint(&layer.image, layer.tile, paint, atlas_size, terrain_unit),
                    layer.height.as_ref().map(|img| {
                        detail_footprint(img, layer.tile, paint, atlas_size, terrain_unit)
                    }),
                ),
            )
        })
        .collect();
    let mut bytes = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            let mut mixed = [0f32; 3];
            let mut coverage = 0.0f32;
            let neighbors = painted_neighbors(paint, uv);
            // Fixed-size stack storage avoids millions of per-pixel Vec
            // allocations when processing full-resolution terrain tiles.
            let mut samples: [Option<TerrainPixelSample<'_>>; 4] = [None; 4];
            let mut sample_count = 0usize;
            for (id, original_weight) in neighbors.into_iter().flatten() {
                let Some(layer) = layers.get(&id) else {
                    continue;
                };
                let world = [
                    uv[0] * (paint.width - 1) as f32 * terrain_unit,
                    uv[1] * (paint.height - 1) as f32 * terrain_unit,
                ];
                let tiled = [world[0] * layer.tile[0], world[1] * layer.tile[1]];
                let (color_mips, height_mips, color_footprint, height_footprint) = &filtered[&id];
                let detail = detail_sample(color_mips, tiled, *color_footprint);
                let height = height_mips
                    .as_ref()
                    .zip(height_footprint.as_ref())
                    .map(|(mips, footprint)| detail_sample(mips, tiled, *footprint)[0]);
                // Original layer IDs are categorical. Preserve the authored
                // fractional coverage of resolved neighbors at boundaries.
                coverage += original_weight;
                samples[sample_count] = Some((layer, detail, height, original_weight));
                sample_count += 1;
            }
            if sample_count > 0 {
                let transition = sample_count > 1;
                let mut total = 0.0;
                for (layer, _, height, weight) in samples.iter_mut().flatten() {
                    if transition && let Some(height) = height {
                        // FreeSDK documents height-assisted layer
                        // transitions but does not expose Evolve's
                        // exact material-mask equation. This bounded
                        // proxy uses the authored factor and falloff.
                        let displacement = (*height - 0.5)
                            * (layer.blend_factor / 16.0)
                            * (layer.blend_falloff / 2.0).clamp(0.0, 4.0);
                        *weight = (*weight + displacement * *weight * (1.0 - *weight)).max(0.0);
                    }
                    total += *weight;
                }
                if total > 0.0 {
                    for (layer, color, _, weight) in samples.into_iter().flatten() {
                        let w = weight / total;
                        for c in 0..3 {
                            mixed[c] += (color[c] - neutral) * layer.strength * w;
                        }
                    }
                }
            }
            let pos = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                bytes[pos + c] = ((neutral + mixed[c]).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            bytes[pos + 3] = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    image::RgbaImage::from_raw(width, height, bytes).expect("bounded RGBA terrain atlas")
}

/// Select only explicitly authored Terrain.Layer diffuse maps from the
/// verified material decoder. The ad-hoc source parameter scan is never
/// authoritative for DDS selection: real materials can use nested leaf
/// slots, mixed-case attributes or compiled CryXmlB.
fn terrain_layer_diffuse(
    document: &evolve_core::MaterialDocument,
) -> Result<Option<String>, String> {
    let mut paths = BTreeSet::new();
    for slot in document
        .slots
        .iter()
        .filter(|s| s.shader.eq_ignore_ascii_case("Terrain.Layer"))
    {
        for map in slot
            .maps
            .iter()
            .filter(|map| map.role.eq_ignore_ascii_case("diffuse"))
        {
            let key = safe_virtual(&map.path)
                .ok_or_else(|| format!("Unsafe Terrain.Layer diffuse path: {}", map.path))?;
            paths.insert(key);
        }
    }
    match paths.len() {
        0 => Ok(None),
        1 => Ok(paths.into_iter().next()),
        count => Err(format!(
            "{count} different authored Terrain.Layer diffuse paths; refusing arbitrary choice"
        )),
    }
}

/// A high-pass overlay in *neutral signed color*, not a replacement for the
/// map-owned RGB atlas. The fragment shader composes signed detail and then
/// applies ordinary preview lighting/fog. A missing layer is transparent.
pub(super) fn compose(
    selected: &ImportedPak,
    paint: &TerrainPaintMap,
    atlas_size: (u32, u32),
    terrain_unit: f32,
) -> Result<Option<Preview>, String> {
    if paint.width < 2
        || paint.height < 2
        || atlas_size.0 == 0
        || atlas_size.1 == 0
        || !terrain_unit.is_finite()
        || terrain_unit <= 0.0
        || (atlas_size.0 as u64) * (atlas_size.1 as u64) > 4096 * 4096
    {
        return Err("Invalid dimensions for terrain high-pass overlay".into());
    }
    let catalog = same_install_catalog(selected)?;
    let active: BTreeSet<u8> = paint.ids.iter().copied().collect();
    let mut matches = HashMap::<u8, BTreeSet<String>>::new();
    for layer in &paint.layers {
        if active.contains(&layer.id)
            && let Some(path) = material_path(&layer.surface_type)
        {
            matches.entry(layer.id).or_default().insert(path);
        }
    }
    let mut layers = HashMap::new();
    let mut failures = Vec::new();
    let mut verified_resolutions = Vec::new();
    for (&id, names) in &matches {
        if names.len() != 1 {
            failures.push(format!(
                "paint ID {id}: conflicting original material bindings"
            ));
            continue;
        }
        let Some(material) = names.iter().next() else {
            continue;
        };
        let Some(entry) = find(&catalog, material)? else {
            failures.push(format!("{material}: MTL absent"));
            continue;
        };
        let path = verified(entry)?;
        let document = read_material(&path).map_err(|e| e.to_string())?;
        if !document
            .slots
            .iter()
            .any(|s| s.shader.eq_ignore_ascii_case("Terrain.Layer"))
        {
            failures.push(format!("{material}: not Terrain.Layer"));
            continue;
        }
        let diffuse = match terrain_layer_diffuse(&document) {
            Ok(Some(source)) => source,
            Ok(None) => {
                failures.push(format!(
                    "{material}: Terrain.Layer has no authored diffuse map"
                ));
                continue;
            }
            Err(error) => {
                failures.push(format!("{material}: {error}"));
                continue;
            }
        };
        let authored = match authored_detail(&fs::read(&path).map_err(|e| e.to_string())?) {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!(
                    "{material}: material parameters unavailable ({error})"
                ));
                AuthoredDetail {
                    tile: [1.0, 1.0],
                    strength: 1.0,
                    blend_factor: 16.0,
                    blend_falloff: 2.0,
                    pom_displacement: 0.0,
                    ..Default::default()
                }
            }
        };
        // The Terrain.Layer shader contract is the authority. Many original
        // high-pass diffuse resources lack a "_highpass" filename suffix;
        // do not silently omit legitimate Evolve terrain materials by name.
        match load_detail_image(&catalog, &diffuse, material) {
            Ok(image) => {
                verified_resolutions.push(format!(
                    "paint ID {id} -> {}x{} ({}; {})",
                    image.width(),
                    image.height(),
                    material,
                    diffuse
                ));
                let height = authored
                    .height
                    .as_deref()
                    .and_then(|source| load_detail_image(&catalog, source, material).ok());
                let normal = authored
                    .normal
                    .as_deref()
                    .and_then(|source| load_detail_image(&catalog, source, material).ok());
                layers.insert(
                    id,
                    DetailLayer {
                        image,
                        normal,
                        height,
                        tile: authored.tile,
                        strength: authored.strength,
                        blend_factor: authored.blend_factor,
                        blend_falloff: authored.blend_falloff,
                        pom_displacement: authored.pom_displacement,
                    },
                );
            }
            Err(error) => failures.push(error),
        }
    }
    if layers.is_empty() {
        return Err(format!(
            "No usable Terrain.Layer diffuse DDS for {} active paint IDs; {} IDs have authored bindings. Examples: {}",
            active.len(),
            matches.len(),
            if failures.is_empty() {
                "no authored diffuse references were resolved".into()
            } else {
                failures
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        ));
    }
    let report = format!(
        "Terrain.Layer high-pass preview: {}/{} authored painted material IDs reconstructed from complete same-install DDS streams; {} unresolved/unsupported. Original RGB ground atlas preserved. CPU-baked detail is now mip-prefiltered for atlas minification (avoids colored aliasing; does NOT retain native close-up resolution). Add-signed shader enabled; categorical layer borders use interpolated coverage (not original engine blend masks). Source resolutions: {}. Unresolved examples: {}",
        layers.len(),
        active.len(),
        failures.len(),
        verified_resolutions
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
            .join("; "),
        failures.into_iter().take(5).collect::<Vec<_>>().join("; ")
    );
    let gpu = gpu_source(paint, terrain_unit, &layers);
    let report = format!(
        "{report} GPU source-material detail: {}.",
        if gpu.is_some() {
            "original full-resolution DDS array with categorical paint ID sampling"
        } else {
            "unsupported layer dimensions/count; mip-filtered CPU atlas fallback"
        }
    );
    // Do not spend minutes rasterizing millions of CPU atlas pixels if
    // full-resolution authored DDS layers are already headed to the GPU.
    // The neutral fallback still binds safely for all other meshes.
    let fallback = if gpu.is_some() {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 128, 128, 0]))
    } else {
        compose_detail(paint, atlas_size, terrain_unit, &layers)
    };
    Ok(Some(Preview {
        fallback,
        gpu,
        report,
    }))
}

#[cfg(test)]
mod composition_tests {
    use super::*;

    fn fixture() -> TerrainPaintMap {
        TerrainPaintMap {
            width: 2,
            height: 2,
            layer_names: Default::default(),
            layers: vec![],
            surface_types: vec![],
            ids: vec![1, 2, 1, 2],
        }
    }
    #[test]
    fn source_gpu_layers_preserve_original_pixels_and_exact_layer_ids() {
        let paint = fixture();
        let mut layers = HashMap::new();
        let a = image::RgbaImage::from_fn(4, 4, |x, y| {
            image::Rgba([(x * 13 + y * 17) as u8, 60, 100, 255])
        });
        let b = image::RgbaImage::from_pixel(4, 4, image::Rgba([12, 180, 250, 255]));
        layers.insert(
            2,
            DetailLayer {
                image: b.clone(),
                normal: None,
                height: None,
                tile: [0.15, 0.5],
                strength: 0.8,
                blend_factor: 10.0,
                blend_falloff: 4.0,
                pom_displacement: 0.0,
            },
        );
        layers.insert(
            1,
            DetailLayer {
                image: a.clone(),
                normal: None,
                height: None,
                tile: [0.25, 0.25],
                strength: 1.0,
                blend_factor: 16.0,
                blend_falloff: 2.0,
                pom_displacement: 0.0,
            },
        );
        let gpu = gpu_source(&paint, 2.0, &layers).unwrap();
        assert_eq!(
            gpu.layers.iter().map(|layer| layer.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(gpu.layers[0].image.as_raw(), a.as_raw());
        assert_eq!(gpu.layers[1].image.as_raw(), b.as_raw());
        assert_eq!(gpu.layers[1].tile, [0.15, 0.5]);
        // Heightmap LayerId bitmap ordering is source-format-specific; test
        // byte-for-byte preservation rather than assuming row-major IDs.
        assert_eq!(gpu.paint.get_pixel(0, 0)[0], paint.id_at(0, 0).unwrap());
        assert_eq!(gpu.paint.get_pixel(1, 0)[0], paint.id_at(1, 0).unwrap());
        assert_eq!(gpu.unit, 2.0);
    }

    #[test]
    fn incompatible_terrain_array_falls_back_without_dropping_source_detail() {
        let paint = fixture();
        let mut layers = HashMap::new();
        for (id, size) in [(1, 4), (2, 8)] {
            layers.insert(
                id,
                DetailLayer {
                    normal: None,
                    image: image::RgbaImage::from_pixel(
                        size,
                        size,
                        image::Rgba([127, 128, 129, 255]),
                    ),
                    height: None,
                    tile: [1.0, 1.0],
                    strength: 1.0,
                    blend_factor: 16.0,
                    blend_falloff: 2.0,
                    pom_displacement: 0.0,
                },
            );
        }
        assert!(gpu_source(&paint, 1.0, &layers).is_none());
        assert_eq!(
            compose_detail(&paint, (8, 8), 1.0, &layers).dimensions(),
            (8, 8)
        );
    }

    #[test]
    fn terrain_diffuse_comes_from_verified_nested_mtl_leaf_not_optional_xml_scan() {
        let doc = evolve_core::parse_material(
            br#"<Material Name="Outer"><SubMaterials>
                <Material Name="Dirt" Shader="Terrain.Layer"><Textures>
                  <Texture Map="Diffuse" File="Materials/Terrain/Clay_Dirt_B_Highpass.tif"/>
                  <Texture Map="Bumpmap" File="Materials/Terrain/clay_ddn.tif"/>
                </Textures></Material>
            </SubMaterials></Material>"#,
        )
        .unwrap();
        assert_eq!(
            terrain_layer_diffuse(&doc).unwrap().as_deref(),
            Some("materials/terrain/clay_dirt_b_highpass.tif"),
        );
    }

    #[test]
    fn missing_and_ambiguous_diffuse_remain_explicit_not_silent() {
        let no_diffuse = evolve_core::parse_material(
            br#"<Material Shader="Terrain.Layer"><Textures>
            <Texture Map="Bumpmap" File="materials/terrain/dirt_ddn.tif"/>
            </Textures></Material>"#,
        )
        .unwrap();
        assert!(terrain_layer_diffuse(&no_diffuse).unwrap().is_none());
        let competing = evolve_core::parse_material(
            br#"<Material><SubMaterials>
              <Material Shader="Terrain.Layer"><Textures><Texture Map="Diffuse" File="Materials/Terrain/A.tif"/></Textures></Material>
              <Material Shader="Terrain.Layer"><Textures><Texture Map="Diffuse" File="Materials/Terrain/B.tif"/></Textures></Material>
            </SubMaterials></Material>"#,
        ).unwrap();
        assert!(terrain_layer_diffuse(&competing).is_err());
    }

    #[test]
    fn default_terrain_material_is_included_without_indexing_unrelated_assets() {
        assert!(is_terrain_dependency(
            "materials/universal/default/material_terrain_default.mtl"
        ));
        assert!(!is_terrain_dependency(
            "materials/universal/default/unrelated.dds.0"
        ));
        assert!(!is_terrain_dependency(
            "characters/hunters/markov/materials/body.mtl"
        ));
    }

    #[test]
    fn local_split_dds_fragments_are_not_passed_twice_to_decoder() {
        let root = Path::new("/import/game/textures/terrain/arctic");
        let header = root.join("snow_dirt_a_terr.dds.0");
        let local = root.join("snow_dirt_a_terr.dds.1");
        let external = Path::new("/other_pak/files/textures/terrain/arctic/snow_dirt_a_terr.dds.2");
        assert!(!needs_external_fragment(&header, &local));
        assert!(needs_external_fragment(&header, external));
    }

    #[test]
    fn authored_texmod_strength_and_height_params_are_retained() {
        let xml = br#"<Material Shader="Terrain.Layer"><Textures>
          <Texture Map="Diffuse" File="Materials/terrain/highpass.tif"><TexMod TileU="0.25" TileV="0.5"/></Texture>
          <Texture Map="Heightmap" File="Textures/terrain/h_displ.tif"/>
          </Textures><PublicParams DetailTextureStrength="0.85" BlendFactor="16" BlendFalloff="2"/></Material>"#;
        let params = authored_detail(xml).unwrap();
        assert_eq!(params.tile, [0.25, 0.5]);
        assert_eq!(params.strength, 0.85);
        assert_eq!(
            params.height.as_deref(),
            Some("Textures/terrain/h_displ.tif")
        );
    }
    #[test]
    fn layer_transitions_preserve_proportions_and_original_xy_layout() {
        let paint = fixture();
        assert_eq!(paint.id_at(0, 0), Some(1));
        assert_eq!(paint.id_at(0, 1), Some(2));
        let adjacent: Vec<_> = painted_neighbors(&paint, [0.5, 0.5])
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(adjacent.len(), 2);
        assert!(adjacent.iter().all(|(_, w)| (*w - 0.5).abs() < 0.0001));
    }
    #[test]
    fn minified_highpass_checkerboard_does_not_alias_into_false_colors() {
        let mut image = image::RgbaImage::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let value = if (x + y) % 2 == 0 { 0 } else { 255 };
                image.put_pixel(x, y, image::Rgba([value, value, value, 255]));
            }
        }
        let mipmaps = detail_mips(&image);
        assert_eq!(mipmaps.last().unwrap().dimensions(), (1, 1));
        // One source texel per pixel sees original detail, but a 16-texel
        // footprint should converge to the neutral source average.
        let filtered = detail_sample(&mipmaps, [0.123, 0.738], 16.0);
        assert!(filtered.iter().all(|v| (*v - 0.5).abs() < 0.03));
        let sample = detail_sample(&mipmaps, [0.0, 0.0], 1.0);
        assert!(sample.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn neutral_highpass_composition_does_not_recolor_macro_source() {
        let paint = fixture();
        let mut layers = HashMap::new();
        layers.insert(
            1,
            DetailLayer {
                image: image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 128, 255])),
                normal: None,
                height: None,
                tile: [0.25, 0.5],
                strength: 1.0,
                blend_factor: 16.0,
                blend_falloff: 2.0,
                pom_displacement: 0.0,
            },
        );
        let overlay = compose_detail(&paint, (8, 8), 1.0, &layers);
        assert!(
            overlay
                .pixels()
                .all(|p| p[0] == 128 && p[1] == 128 && p[2] == 128)
        );
        assert!(overlay.pixels().any(|p| p[3] != 255));
    }
}
