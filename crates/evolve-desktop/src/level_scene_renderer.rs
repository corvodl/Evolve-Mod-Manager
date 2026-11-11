//! One source-authored mesh in the Level Editor viewport.
//!
//! Reuse the verified imported inventory and original material loader, rather
//! than drawing a guessed map mesh or borrowing same-named files from another
//! game build. All loads run in the existing blocking Operation worker.
use crate::{
    assembly_materials,
    asset_cache::{PreviewCache, SharedCache},
    textured::Surface,
    world_instances,
};
use evolve_core::read_static_cdf_assembly_with_imported;
use evolve_core::zookeeper::{PopulationKind, ZooKeeper};
use evolve_core::{
    ImportedPak, Inventory, LevelScene, ModelInfo, PlacedObject, RenderMesh, SharedImportedLookup,
    Transform, begin_imported_lookup_session, begin_imported_lookup_session_for_source,
    imported_archive_files, imported_archive_preview_inventory, imported_library,
    read_world_model_parts_with_subsets,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_ARCHIVES: usize = 1_024;
// Unique model/material GPU groups. Every pair is already decoded before this
// applies and texture memory has its own residency budget, so this only bounds
// draw calls. Legacy-ported maps add their terrain.dat brushes on top of the
// mission's objects and exceeded the previous 1,024.
const MAX_MODELS: usize = 8_192;
// A 2048-sample heightmap alone is about 4.2M vertices and 25M indices.
const MAX_VERTICES: usize = 4_500_000;
const MAX_INDICES: usize = 26_000_000;
const MAX_SURFACES: usize = 2_048;

#[derive(Clone, Debug, Default)]
pub(super) struct Report {
    pub spawn_distances: Option<evolve_core::zookeeper::SpawnDistances>,
    pub unresolved_populations: Vec<String>,
    pub resolved_models: Vec<(String, String)>,
    pub placed: usize,
    pub geometries: usize,
    pub textured: usize,
    /// Distinct authored model/material pairs without a verified diffuse map.
    /// This does not imply the diffuse exists in the imported build.
    pub without_diffuse: usize,
    pub unresolved: usize,
    pub recovered_companions: usize,
    pub notes: Vec<String>,
}

impl Report {
    pub fn status(&self) -> String {
        format!(
            "{} model instances · {} unique meshes · {} textured material sets · {} model/material pairs without verified diffuse · {} missing/unsupported geometry dependencies · {} exact compiled companions recovered. {}",
            self.placed,
            self.geometries,
            self.textured,
            self.without_diffuse,
            self.unresolved,
            self.recovered_companions,
            self.notes
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
    fn note(&mut self, message: String) {
        if self.notes.len() < 16 {
            self.notes.push(message);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Assembled {
    pub mesh: RenderMesh,
    pub surfaces: Vec<Surface>,
    pub picks: Vec<PickRange>,
    pub report: Report,
    pub world: std::sync::Arc<world_instances::World>,
}

#[derive(Clone, Debug)]
pub(super) struct PickRange {
    pub object_index: usize,
    pub first_index: usize,
    pub index_count: usize,
}

#[derive(Clone)]
struct AssetLocation {
    archive: String,
    path: PathBuf,
}

struct Index {
    files: HashMap<String, Vec<AssetLocation>>,
    inventories: HashMap<String, Inventory>,
    build: Option<String>,
    library_root: Option<PathBuf>,
    selected_id: String,
    matching_build: HashSet<String>,
}

fn safe_virtual(raw: &str) -> Option<String> {
    let path = raw.trim().replace('\\', "/");
    if path.is_empty() || path.starts_with('/') || path.contains(':') {
        return None;
    }
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| matches!(*part, "." | "..")) {
        return None;
    }
    Some(parts.join("/").to_ascii_lowercase())
}

/// Compiled render companions use the same authored virtual owner and do
/// not authorize unrelated basename matches.
fn compiled_render_path(model: &str) -> String {
    let source = Path::new(model);
    let ext = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("skin") {
        source.with_extension("skinm").to_string_lossy().to_string()
    } else if ext.eq_ignore_ascii_case("chr") {
        source.with_extension("chrm").to_string_lossy().to_string()
    } else {
        model.to_string()
    }
}

/// Prefer the exact source path; compiled geometry companions are only
/// accepted at the exact same virtual owner/stem in the same imported build.
fn render_candidates(raw: &str) -> Vec<String> {
    let exact = compiled_render_path(raw);
    let path = Path::new(&exact);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let mut candidates = vec![exact.clone()];
    let compiled = match ext.to_ascii_lowercase().as_str() {
        "cgf" => Some("cgfm"),
        "cga" => Some("cgam"),
        _ => None,
    };
    if let Some(ext) = compiled {
        candidates.push(path.with_extension(ext).to_string_lossy().into_owned());
    }
    candidates
}

fn relevant_asset(path: &str) -> bool {
    [
        ".cgf", ".cga", ".cgfm", ".cgam", ".skinm", ".chrm", ".mtl", ".cdf", ".xml",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
}

/// Load an authored model from an exact virtual path. Many Evolve `.cgf`
/// members contain metadata, physics nodes, or mesh headers without complete
/// render streams; their *same-stem* `.cgfm` render companion can be valid.
/// A present but undecodable source must not block that verified companion.
///
/// Never search by basename, choose an ambiguous archive arbitrarily, or
/// treat missing geometry as an invented proxy mesh.
fn decodable_render_source<L, D, T>(
    model: &str,
    mut lookup: L,
    mut decode: D,
) -> Result<(AssetLocation, Vec<T>, Option<String>), Vec<String>>
where
    L: FnMut(&str) -> Result<Option<AssetLocation>, String>,
    D: FnMut(&Path) -> Result<Vec<T>, String>,
{
    let mut reasons = Vec::new();
    let candidates = render_candidates(model);
    for (index, candidate) in candidates.iter().enumerate() {
        match lookup(candidate) {
            Ok(Some(source)) => match decode(&source.path) {
                Ok(parts) if !parts.is_empty() => {
                    let note = (index > 0).then(|| {
                        format!(
                            "Using exact compiled render companion {candidate} after original source was unavailable or could not decode"
                        )
                    });
                    return Ok((source, parts, note));
                }
                Ok(_) => reasons.push(format!("{candidate}: no renderable geometry")),
                Err(error) => reasons.push(format!("{candidate}: {error}")),
            },
            Ok(None) => reasons.push(format!(
                "{candidate}: not present in matching imported PAKs"
            )),
            Err(error) => reasons.push(format!("{candidate}: {error}")),
        }
    }
    Err(reasons)
}

/// Source installation identity rather than mutable label/version spelling.
/// Never includes arbitrary assets from a different installed Game directory.
fn source_game_directory(pak: &ImportedPak) -> Option<PathBuf> {
    Path::new(&pak.source)
        .ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Game"))
        })
        .and_then(|path| fs::canonicalize(path).ok())
}

fn same_verified_build(
    selected: &ImportedPak,
    pak: &ImportedPak,
    selected_game: Option<&PathBuf>,
) -> bool {
    selected_game.is_some_and(|game| source_game_directory(pak).as_ref() == Some(game))
        || pak
            .builds
            .iter()
            .any(|label| selected.builds.contains(label))
}

// An exact virtual asset in several PAKs is only unambiguous when the
// decoded *source files* themselves have identical bytes. Avoid selecting the
// first matching basename or assuming Stage 2 and Legacy PAKs interoperate.
fn identical_verified_sources(assets: &[AssetLocation]) -> Result<bool, String> {
    if assets.len() <= 1 {
        return Ok(true);
    }
    let mut first: Option<(u64, [u8; 32])> = None;
    for asset in assets {
        let metadata = fs::symlink_metadata(&asset.path)
            .map_err(|error| format!("Cannot inspect duplicate source: {error}"))?;
        if !metadata.file_type().is_file() || metadata.len() > 128 * 1024 * 1024 {
            return Ok(false);
        }
        let mut file = fs::File::open(&asset.path)
            .map_err(|error| format!("Cannot read duplicate source: {error}"))?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let current = (metadata.len(), digest.finalize().into());
        if let Some(previous) = first {
            if previous != current {
                return Ok(false);
            }
        } else {
            first = Some(current);
        }
    }
    Ok(true)
}

impl Index {
    fn new(
        selected: &ImportedPak,
        wanted: &HashSet<String>,
        report: &mut Report,
        project_inventory: Option<&Inventory>,
    ) -> Result<Self, String> {
        let build = (selected.builds.len() == 1).then(|| selected.builds[0].clone());
        let library_root = Path::new(&selected.directory)
            .parent()
            .map(Path::to_path_buf);
        let mut index = Self {
            files: HashMap::new(),
            inventories: HashMap::new(),
            build,
            library_root,
            selected_id: selected.id.clone(),
            matching_build: HashSet::new(),
        };
        let mut archives = vec![selected.clone()];
        let selected_game = source_game_directory(selected);
        if let Some(root) = &index.library_root {
            if let Some(source_game) = &selected_game {
                let library = imported_library(root).map_err(|e| e.to_string())?;
                for pak in library
                    .archives
                    .into_iter()
                    .filter(|pak| {
                        pak.id != selected.id
                            && source_game_directory(pak).as_ref() == Some(source_game)
                    })
                    .take(MAX_ARCHIVES - 1)
                {
                    archives.push(pak);
                }
                report.note(
                    "Resolving exact asset paths within the selected installed Game directory."
                        .into(),
                );
            } else {
                report.note(
                    "Installed Game directory unverified; cross-PAK dependencies disabled.".into(),
                );
            }
        }
        if archives.len() == MAX_ARCHIVES {
            report.note(
                "Cross-PAK index capped at 1024 archives; some assets may be unavailable.".into(),
            );
        }
        // The selected archive is fully validated. Other archives supply
        // read-only manifest names; only exact scene dependencies are stat'ed
        // and checked for original byte length, canonical containment and
        // symlinks. Validating *all* extracted files across 700+ unrelated
        // archives made level opening filesystem-bound for many minutes.
        let started = Instant::now();
        let mut checked = 0usize;
        let mut accepted = 0usize;
        for pak in archives {
            // The exact installed Game directory is stronger provenance than
            // an old, mutable build label. This is also the identity used by
            // the scoped material/CDF dependency lookup below. In particular,
            // an unlabeled import must not become unavailable just because
            // that lookup installs a source-game token in `sources.build`.
            if same_verified_build(selected, &pak, selected_game.as_ref()) {
                index.matching_build.insert(pak.id.clone());
            }
            let inventory = if pak.id == selected.id {
                if let Some(project) = project_inventory {
                    project.clone()
                } else {
                    imported_archive_files(&pak).map_err(|e| e.to_string())?
                }
            } else {
                match imported_archive_preview_inventory(&pak) {
                    Ok(found) => found,
                    Err(error) => {
                        report.note(format!("Skipped unreadable {}: {error}", pak.name));
                        continue;
                    }
                }
            };
            let root = PathBuf::from(&inventory.root);
            // Canonicalize both sides for containment checks. Windows may
            // introduce a \\?\ transport prefix only on the canonical side;
            // comparing that against a persisted C:\ path rejects valid
            // imports even though both paths identify the same file.
            let Ok(canonical_root) = fs::canonicalize(&root) else {
                report.note(format!(
                    "Imported extraction folder not accessible: {}",
                    pak.name
                ));
                continue;
            };
            let mut matched = false;
            for asset in &inventory.entries {
                let Some(key) = safe_virtual(&asset.relative_path) else {
                    continue;
                };
                if wanted.contains(&key) && relevant_asset(&key) {
                    checked += 1;
                    let path = root.join(&asset.relative_path);
                    // Only requested assets are authorized to be read. Never
                    // trust unchecked manifest paths as safe extracted files.
                    let Ok(meta) = fs::symlink_metadata(&path) else {
                        continue;
                    };
                    if !meta.file_type().is_file() || meta.len() != asset.byte_size {
                        continue;
                    }
                    let Ok(canonical) = fs::canonicalize(&path) else {
                        continue;
                    };
                    if !canonical.starts_with(&canonical_root) {
                        continue;
                    }
                    accepted += 1;
                    matched = true;
                    index.files.entry(key).or_default().push(AssetLocation {
                        archive: pak.id.clone(),
                        // Retain an ordinary Windows path for downstream
                        // decoders. The resolved canonical path above is
                        // used only to verify containment, never persisted.
                        path,
                    });
                }
            }
            // Retain only inventories that contain exact, safe dependencies.
            // Material lookup validates the actual file again before reading.
            if matched || pak.id == selected.id {
                index.inventories.insert(pak.id.clone(), inventory);
            }
        }
        report.note(format!(
            "Exact-build scene lookup: {accepted}/{checked} requested file references verified in {} ms.",
            started.elapsed().as_millis()
        ));
        if !index.inventories.contains_key(&selected.id) {
            return Err("Selected imported map archive could not be verified".into());
        }
        Ok(index)
    }

    fn lookup(&self, raw: &str) -> Result<Option<&AssetLocation>, String> {
        let Some(key) = safe_virtual(raw) else {
            return Err(format!("Unsafe source asset path: {raw}"));
        };
        let Some(found) = self.files.get(&key) else {
            return Ok(None);
        };
        // Prefer the selected map PAK's exact member. Across other PAKs,
        // duplicates cannot safely be selected by filename or import order.
        if let Some(selected) = found.iter().find(|a| a.archive == self.selected_id) {
            return Ok(Some(selected));
        }
        let same_build = found
            .iter()
            .filter(|source| self.matching_build.contains(&source.archive))
            .cloned()
            .collect::<Vec<_>>();
        if !same_build.is_empty() {
            if same_build.len() == 1 || identical_verified_sources(&same_build)? {
                return Ok(found
                    .iter()
                    .find(|source| source.archive == same_build[0].archive));
            }
            return Err(format!("Conflicting {raw} in the selected build"));
        }
        if self.build.is_some() {
            return Err(format!("No verified selected-build source for {raw}"));
        }
        if found.len() == 1 || identical_verified_sources(found)? {
            // Multiple identical verified files are equivalent. The selected
            // archive still takes precedence. Never collapse changed bytes.
            Ok(found.first())
        } else {
            Err(format!(
                "Ambiguous imported asset {raw} differs across {} PAKs",
                found.len()
            ))
        }
    }
}

// A model can have one good diffuse and several missing authored material
// sections. The existing per-pair "without_diffuse" counter only catches
// fully untextured models, so log mixed-quality materials separately.
fn missing_diffuse_sections(parts: &[Surface]) -> usize {
    parts
        .iter()
        .filter(|part| part.visible && part.index_count > 0 && part.diffuse.is_none())
        .count()
}

fn neutral(first_index: u32, index_count: u32) -> Surface {
    Surface {
        first_index,
        index_count,
        visible: true,
        transparent: false,
        foliage_cutout: false,
        distance_clouds: None,
        uv_transform: None,
        material_tint: None,
        material_specular: None,
        material_emissive: None,
        emission_on_diffuse: false,
        emission_alpha_mask: false,
        material_shininess: None,
        waterfall: None,
        detail_map: None,
        blend_layer: None,
        blend_mask: None,
        vertex_colors: false,
        blend_normal: None,
        detail_tile: None,
        layer_params: None,
        foliage_transmit: None,
        layer_specular: None,
        alpha_cutoff: 0.5,
        diffuse: None,
        normal: None,
        emissive: None,
        armor_emissive: None,
        emission_strength: 1.0,
        emissive_uv: None,
        specular: None,
        gloss: None,
        opacity: None,
    }
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let u = [x, y, z];
    let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let factor = w * w - x * x - y * y - z * z;
    [
        factor * v[0] + 2.0 * dot * u[0] + 2.0 * w * cross[0],
        factor * v[1] + 2.0 * dot * u[1] + 2.0 * w * cross[1],
        factor * v[2] + 2.0 * dot * u[2] + 2.0 * w * cross[2],
    ]
}

fn transformed(tf: Transform, v: [f32; 3], normal: bool) -> [f32; 3] {
    let scaled = if normal {
        [
            v[0] / if tf.scale[0].abs() < 0.000001 {
                1.0
            } else {
                tf.scale[0]
            },
            v[1] / if tf.scale[1].abs() < 0.000001 {
                1.0
            } else {
                tf.scale[1]
            },
            v[2] / if tf.scale[2].abs() < 0.000001 {
                1.0
            } else {
                tf.scale[2]
            },
        ]
    } else {
        [v[0] * tf.scale[0], v[1] * tf.scale[1], v[2] * tf.scale[2]]
    };
    let result = rotate(tf.quaternion, scaled);
    if normal {
        let length = result.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-8);
        [result[0] / length, result[1] / length, result[2] / length]
    } else {
        [
            result[0] + tf.position[0],
            result[1] + tf.position[1],
            result[2] + tf.position[2],
        ]
    }
}

fn append(
    target: &mut RenderMesh,
    ranges: &mut Vec<Surface>,
    part: &RenderMesh,
    materials: &[Surface],
    transform: Transform,
) -> bool {
    let nverts = target.positions.len();
    let nindices = target.indices.len();
    if nverts + part.positions.len() > MAX_VERTICES
        || nindices + part.indices.len() > MAX_INDICES
        || ranges.len() + materials.len().max(1) > MAX_SURFACES
        || part
            .indices
            .iter()
            .any(|index| *index as usize >= part.positions.len())
    {
        return false;
    }
    let Ok(base) = u32::try_from(nverts) else {
        return false;
    };
    let Ok(start) = u32::try_from(nindices) else {
        return false;
    };
    for (i, pos) in part.positions.iter().copied().enumerate() {
        target.positions.push(transformed(transform, pos, false));
        target.uv0.as_mut().unwrap().push(
            part.uv0
                .as_ref()
                .and_then(|uv| uv.get(i))
                .copied()
                .unwrap_or([0.0, 0.0]),
        );
        if let Some(paint) = target.vertex_alpha.as_mut() {
            paint.push(
                part.vertex_alpha
                    .as_ref()
                    .and_then(|alpha| alpha.get(i))
                    .copied()
                    .unwrap_or(255),
            );
        }
        target.normals.as_mut().unwrap().push(transformed(
            transform,
            part.normals
                .as_ref()
                .and_then(|normals| normals.get(i))
                .copied()
                .unwrap_or([0.0, 0.0, 1.0]),
            true,
        ));
    }
    target
        .indices
        .extend(part.indices.iter().map(|index| base + *index));
    if materials.is_empty() {
        ranges.push(neutral(start, part.indices.len() as u32));
    } else {
        for surface in materials {
            let mut copied = surface.clone();
            copied.first_index = start + surface.first_index;
            ranges.push(copied);
        }
    }
    true
}

// One GPU group can have dozens of independently textured material spans.
// Split oversized *multi-material* groups before GPU upload; each sub-group
// shares the exact source CPU mesh and authored instances but retains only its
// own material spans. A single larger-than-target source still requires sparse
// GPU texture residency and remains explicitly reported as deferred.
const WORLD_RESIDENCY_TARGET: u64 = 512 * 1024 * 1024;
fn rgba_mip_footprint(width: u32, height: u32) -> u64 {
    let mut total = 0u64;
    let (mut w, mut h) = (width.max(1), height.max(1));
    loop {
        total = total.saturating_add(u64::from(w) * u64::from(h) * 4);
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

fn surface_gpu_estimate(surface: &Surface) -> u64 {
    [
        surface.diffuse.as_ref(),
        surface.opacity.as_ref(),
        surface.normal.as_ref(),
        surface.specular.as_ref(),
        surface.gloss.as_ref(),
        surface.emissive.as_ref(),
    ]
    .into_iter()
    .map(|source| source.map_or(4, |image| rgba_mip_footprint(image.width(), image.height())))
    .sum()
}

fn surface_batch_ranges(costs: &[u64], geometry: u64) -> Vec<std::ops::Range<usize>> {
    if costs.is_empty() {
        return std::iter::once(0..0).collect();
    }
    if geometry.saturating_add(costs.iter().copied().sum::<u64>()) <= WORLD_RESIDENCY_TARGET {
        return std::iter::once(0..costs.len()).collect();
    }
    let mut result = Vec::new();
    let mut from = 0;
    let mut used = geometry;
    for (index, cost) in costs.iter().copied().enumerate() {
        if index > from && used.saturating_add(cost) > WORLD_RESIDENCY_TARGET {
            result.push(from..index);
            from = index;
            used = geometry;
        }
        used = used.saturating_add(cost);
    }
    result.push(from..costs.len());
    result
}

type WorldSurfaces = (Vec<Vec<Surface>>, Vec<String>);

#[derive(Clone)]
struct DecodedMaterial {
    virtual_source: String,
    path: Result<Option<PathBuf>, String>,
    surfaces: Option<Result<WorldSurfaces, String>>,
}

/// One model/material pair, resolved once on a worker thread and consumed once
/// by the ordered assembly. Repeating this work serially dominated map loads.
/// Meshes and textures are shared: a reload of the same map reuses them, so
/// its GPU groups can be kept (see [`begin_decode_reuse`]).
/// A model's source, its meshes and a note when a compiled companion was used.
type DecodedRender = (AssetLocation, Vec<Arc<RenderMesh>>, Option<String>);

#[derive(Clone)]
struct DecodedPair {
    render: Result<DecodedRender, Vec<String>>,
    material: Option<DecodedMaterial>,
}

type DecodedPairs = HashMap<(String, String), DecodedPair>;

/// Pairs decoded by the last load, kept for reloading the same map after an
/// edit. Only level sources change in such an edit, never a model, material
/// or texture, so an unchanged model/material pair decodes to the same result.
static DECODE_REUSE: Mutex<Option<(String, DecodedPairs)>> = Mutex::new(None);

/// Start a map load. `Some(scope)` names the project and map: a load for the
/// same scope reuses the pairs the last one decoded. Any other load, a
/// different scope or `None`, drops them first, so a fresh open never sees
/// stale assets and never holds two maps' assets at once.
pub(super) fn begin_decode_reuse(scope: Option<String>, reuse: bool) {
    let Ok(mut cache) = DECODE_REUSE.lock() else {
        return;
    };
    match (scope, cache.as_mut()) {
        (Some(scope), Some((current, _))) if reuse && *current == scope => {}
        (Some(scope), _) => *cache = Some((scope, DecodedPairs::new())),
        (None, _) => *cache = None,
    }
}

/// Decode every unique scene model and its textures on worker threads. Results
/// are applied in authored order below, so diagnostics, budgets and group
/// order stay exactly as they were when this ran serially.
fn decode_scene_sources(
    scene: &LevelScene,
    sources: &Index,
    library_root: Option<&Path>,
    build: Option<&str>,
) -> DecodedPairs {
    let cache = PreviewCache::for_scene();
    let reusable = DECODE_REUSE
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().map(|(_, pairs)| pairs.clone()))
        .unwrap_or_default();
    let mut unique = HashSet::new();
    let mut reused = DecodedPairs::new();
    let pairs = scene
        .objects
        .iter()
        .filter(|object| !object.hidden && !object.model.is_empty())
        .filter(|object| unique.insert((object.model.as_str(), object.material.as_str())))
        .filter(|object| {
            let key = (object.model.clone(), object.material.clone());
            match reusable.get(&key) {
                Some(pair) => {
                    reused.insert(key, pair.clone());
                    false
                }
                None => true,
            }
        })
        .collect::<Vec<_>>();
    let decoded = Mutex::new(reused);
    let next = AtomicUsize::new(0);
    let lookup = SharedImportedLookup::current();
    // Leave the interface and GPU upload threads room to stay responsive.
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get().saturating_sub(2))
        .clamp(1, 16);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let _lookup = lookup.as_ref().map(SharedImportedLookup::enter);
                while let Some(object) = pairs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let pair = decode_scene_pair(object, sources, library_root, build, &cache);
                    if let Ok(mut table) = decoded.lock() {
                        table.insert((object.model.clone(), object.material.clone()), pair);
                    }
                }
            });
        }
    });
    let decoded = decoded.into_inner().unwrap_or_default();
    if let Ok(mut cache) = DECODE_REUSE.lock()
        && let Some((_, kept)) = cache.as_mut()
    {
        kept.clone_from(&decoded);
    }
    decoded
}

fn decode_scene_pair(
    object: &PlacedObject,
    sources: &Index,
    library_root: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
) -> DecodedPair {
    if object.model.to_ascii_lowercase().ends_with(".cdf") {
        let result = (|| -> Result<DecodedPair, String> {
            let source = sources
                .lookup(&object.model)?
                .ok_or_else(|| format!("Missing exact CDF {}", object.model))?
                .clone();
            let inventory = sources
                .inventories
                .get(&source.archive)
                .ok_or("CDF PAK inventory missing")?;
            let (mesh, assembly) = read_static_cdf_assembly_with_imported(
                &source.path,
                inventory,
                library_root,
                build,
            )
            .map_err(|error| error.to_string())?;
            let (surfaces, warnings) = assembly_materials::load_with_progress(
                &assembly,
                inventory,
                library_root,
                build,
                cache,
                None,
                evolve_core::KalaTubingState::default(),
            );
            Ok(DecodedPair {
                render: Ok((source, vec![Arc::new(mesh)], None)),
                material: Some(DecodedMaterial {
                    virtual_source: object.model.clone(),
                    path: Ok(None),
                    surfaces: Some(Ok((vec![surfaces], warnings))),
                }),
            })
        })();
        return result.unwrap_or_else(|error| DecodedPair {
            render: Err(vec![error]),
            material: None,
        });
    }
    // Source metadata/physics chunks may not contain render streams even though
    // a verified same-stem compiled companion does. Decode candidates in order,
    // retaining exact paths and the selected build rather than guessing objects.
    let render = decodable_render_source(
        &object.model,
        |candidate| sources.lookup(candidate).map(|found| found.cloned()),
        |path| read_world_model_parts_with_subsets(path).map_err(|error| error.to_string()),
    );
    let material = render.as_ref().ok().and_then(|(source, meshes, _)| {
        let inventory = sources.inventories.get(&source.archive)?;
        // A world model may declare MtlName in its compiled mesh, even when
        // there is NO same-stem .mtl. Reuse the Model Viewer's exact original
        // material resolver rather than assuming the geometry filename is the
        // material name. For a compiled fallback, inspect that verified virtual
        // owner so its own embedded material hint can be used.
        let virtual_source = render_candidates(&object.model)
            .into_iter()
            .find(|candidate| {
                let candidate_extension = Path::new(candidate).extension().and_then(|e| e.to_str());
                candidate_extension.is_some_and(|extension| {
                    source
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(extension))
                })
            })
            .unwrap_or_else(|| object.model.clone());
        let explicit = (!object.material.trim().is_empty()).then_some(object.material.as_str());
        let path = assembly_materials::material_for_part(
            &virtual_source,
            explicit,
            inventory,
            library_root,
            build,
        );
        // Each MeshChunk's referenced SUBSETS table is preserved by the world
        // decoder. No material leaf may be guessed from mesh order or the number
        // of extracted mesh nodes.
        let surfaces = match &path {
            Ok(Some(material)) => Some(assembly_materials::load_world_model_surfaces(
                &source.path,
                material,
                meshes,
                inventory,
                library_root,
                build,
                cache,
            )),
            _ => None,
        };
        Some(DecodedMaterial {
            virtual_source,
            path,
            surfaces,
        })
    });
    DecodedPair {
        render: render.map(|(source, meshes, note)| {
            (
                source,
                meshes.into_iter().map(|part| Arc::new(part.mesh)).collect(),
                note,
            )
        }),
        material,
    }
}

/// A source-space indicator, not a reconstructed animal. CreatureSpawn
/// populations only name game archetypes; the level files contain no mesh
/// path. Keep their true locations visible without pretending to resolve one.
fn wildlife_marker_mesh() -> RenderMesh {
    let positions = vec![
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 3.0],
        [0.0, 0.0, -0.25],
    ];
    let indices = vec![
        0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, 4, 2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, 5,
    ];
    RenderMesh {
        info: ModelInfo {
            format: "Wildlife spawn location marker (editor only)".into(),
            version: 1,
            chunk_count: 0,
            vertex_count: positions.len(),
            triangle_count: indices.len() / 3,
            original_uv0: false,
            original_normals: false,
        },
        positions,
        indices,
        uv0: None,
        normals: None,
        vertex_alpha: None,
        vertex_color: None,
    }
}

/// Many placements of one model that are drawn but never edited or picked,
/// such as a Legacy-ported map's merged grass and small rocks.
#[derive(Clone, Debug)]
pub(super) struct BulkInstances {
    pub model: String,
    pub transforms: Vec<Transform>,
}

pub(super) fn assemble(
    archive: &ImportedPak,
    scene: Option<&LevelScene>,
    terrain: Option<&RenderMesh>,
    project_inventory: Option<&Inventory>,
    resolve_wildlife: bool,
    bulk: &[BulkInstances],
) -> Result<Option<Assembled>, String> {
    let mut report = Report::default();
    // Each bulk model resolves through one stand-in object appended after the
    // real ones; its placements then carry no selectable object index.
    let real_objects = scene.map_or(0, |scene| scene.objects.len());
    let with_bulk;
    let scene = match scene {
        Some(scene) if !bulk.is_empty() => {
            let mut extended = scene.clone();
            extended
                .objects
                .extend(bulk.iter().enumerate().map(|(index, group)| PlacedObject {
                    id: format!("__bulk_{index}"),
                    parent: String::new(),
                    layer: String::new(),
                    name: group.model.clone(),
                    kind: "BulkInstances".into(),
                    model: group.model.clone(),
                    material: String::new(),
                    hidden: false,
                    local: Transform::default(),
                    world: Transform::default(),
                    status: String::new(),
                    populations: Vec::new(),
                    home_area: String::new(),
                    shape_points: Vec::new(),
                    wildlife: Default::default(),
                }));
            with_bulk = extended;
            Some(&with_bulk)
        }
        other => other,
    };
    let mut combined = RenderMesh {
        info: ModelInfo {
            format: "Original Evolve level geometry preview".into(),
            version: 1,
            chunk_count: 0,
            vertex_count: 0,
            triangle_count: 0,
            original_uv0: true,
            original_normals: true,
        },
        positions: Vec::new(),
        indices: Vec::new(),
        uv0: Some(Vec::new()),
        normals: Some(Vec::new()),
        vertex_alpha: Some(Vec::new()),
        vertex_color: None,
    };
    let mut surfaces = Vec::new();
    let picks = Vec::new();
    let mut world = world_instances::World::default();
    if let Some(terrain) = terrain
        && !append(
            &mut combined,
            &mut surfaces,
            terrain,
            &[],
            Transform::default(),
        )
    {
        report.note("Terrain exceeded the current viewport geometry budget.".into());
    }
    if let Some(scene) = scene {
        let mut wanted = HashSet::new();
        for object in scene
            .objects
            .iter()
            .filter(|o| !o.hidden && !o.model.is_empty())
        {
            if let Some(model) = safe_virtual(&compiled_render_path(&object.model)) {
                for candidate in render_candidates(&object.model) {
                    if let Some(key) = safe_virtual(&candidate) {
                        wanted.insert(key);
                    }
                }
                if object.material.is_empty() {
                    wanted.insert(
                        Path::new(&model)
                            .with_extension("mtl")
                            .to_string_lossy()
                            .to_string(),
                    );
                }
            }
            if !object.material.is_empty() {
                let material = if object.material.to_ascii_lowercase().ends_with(".mtl") {
                    object.material.clone()
                } else {
                    format!("{}.mtl", object.material)
                };
                if let Some(path) = safe_virtual(&material) {
                    wanted.insert(path);
                }
            }
        }
        let xml_paths = [
            "libs/zookeeper/zookeeper.xml",
            "libs/entityarchetypes/ai.xml",
            "libs/zookeeper/wildlifespawning.xml",
        ];
        if resolve_wildlife {
            wanted.extend(xml_paths.map(str::to_owned));
        }
        let mut sources = Index::new(archive, &wanted, &mut report, project_inventory)?;
        let mut preview_scene = scene.clone();
        let mut wildlife_models = 0usize;
        let zookeeper = resolve_wildlife.then(|| {
            (|| {
                let files = xml_paths.map(|name| -> Result<Vec<u8>, String> {
                    let source = sources
                        .lookup(name)?
                        .ok_or_else(|| format!("Missing same-installation {name}"))?;
                    fs::read(&source.path).map_err(|error| error.to_string())
                });
                let [populations, archetypes, spawning] = files;
                ZooKeeper::from_xml(&populations?, &archetypes?, &spawning?)
            })()
        });
        if let Some(Err(error)) = &zookeeper {
            report.note(format!(
                "ZooKeeper source unavailable: {error}; spawns remain markers."
            ));
        }
        report.spawn_distances = zookeeper
            .as_ref()
            .and_then(|value| value.as_ref().ok())
            .and_then(|data| data.distances);
        let mut missing = HashSet::new();
        let mut non_creatures = 0;
        for object in &mut preview_scene.objects {
            if !object.kind.eq_ignore_ascii_case("CreatureSpawn") || !object.model.is_empty() {
                continue;
            }
            let populations = object
                .populations
                .iter()
                .map(String::as_str)
                .filter(|p| !p.trim().is_empty())
                .collect::<Vec<_>>();
            // Multiple populations are runtime-weighted candidates: never
            // silently present the first as the creature that will spawn.
            if populations.len() != 1 {
                if populations.len() > 1 {
                    missing.insert(format!(
                        "{}: multiple population candidates ({})",
                        object.name,
                        populations.join(", ")
                    ));
                }
                continue;
            }
            if let Some(Ok(data)) = &zookeeper {
                match data.resolve(populations[0]) {
                    Ok(resolved) if resolved.kind == PopulationKind::Creature => {
                        if let Some(model) = resolved.model.and_then(|model| safe_virtual(&model)) {
                            wanted.insert(model.clone());
                            report
                                .resolved_models
                                .push((populations[0].to_owned(), model.clone()));
                            object.model = model;
                            wildlife_models += 1;
                        } else {
                            missing
                                .insert(format!("{}: no authored CDF archetype", populations[0]));
                        }
                    }
                    Ok(resolved) if resolved.kind == PopulationKind::Unknown => {
                        missing.insert(format!(
                            "{}: class-only population ({}) without an authored CDF",
                            populations[0],
                            resolved.class.as_deref().unwrap_or("no class")
                        ));
                    }
                    Ok(resolved) => {
                        non_creatures += 1;
                        missing.insert(format!(
                            "{}: non-creature population ({:?})",
                            populations[0], resolved.kind
                        ));
                    }
                    Err(error) => {
                        missing.insert(error);
                    }
                }
            }
        }
        if wildlife_models > 0 {
            sources = Index::new(archive, &wanted, &mut report, project_inventory)?;
        }
        report.note(format!(
            "{wildlife_models} creature placements linked to authored CDF paths; {non_creatures} non-creature spawns retained as markers."
        ));
        report.unresolved_populations = missing.into_iter().collect();
        report.unresolved_populations.sort();
        // Scope material/DDS resolution to the same real installed Game
        // directory as geometry. Historical labels can be duplicated, so a
        // single old build label is not strong enough provenance by itself.
        let lookup_started = Instant::now();
        let _scoped_lookup = if let Some(root) = sources.library_root.as_deref() {
            if source_game_directory(archive).is_some() {
                let (label, guard) =
                    begin_imported_lookup_session_for_source(root, &archive.source)
                        .map_err(|error| error.to_string())?;
                sources.build = Some(label);
                Some(guard)
            } else {
                sources
                    .build
                    .as_deref()
                    .map(|build| begin_imported_lookup_session(root, build))
                    .transpose()
                    .map_err(|error| error.to_string())?
            }
        } else {
            None
        };
        report.note(format!(
            "Exact-build material lookup index initialized in {} ms.",
            lookup_started.elapsed().as_millis()
        ));
        let decode_started = Instant::now();
        let mut decoded_pairs = decode_scene_sources(
            &preview_scene,
            &sources,
            sources.library_root.as_deref(),
            sources.build.as_deref(),
        );
        report.note(format!(
            "Decoded {} scene model/material pairs and their textures in {} ms.",
            decoded_pairs.len(),
            decode_started.elapsed().as_millis()
        ));
        // Geometry is decoded once per exact model/material pair; each authored
        // actor transform creates a distinct visible scene instance.
        let mut resolved = HashMap::<(String, String), Vec<usize>>::new();
        // A compiled mesh is decoded once, not copied into each object's
        // vertex/index arrays. GPU draw calls use instanced transforms.
        let mut models = Vec::<world_instances::ModelGroup>::new();
        let mut failures = HashSet::<(String, String)>::new();
        let mut resolved_spawns = HashSet::new();
        let mut budget_hit = false;
        for (object_index, object) in preview_scene
            .objects
            .iter()
            .enumerate()
            .filter(|(_, obj)| !obj.hidden && !obj.model.is_empty())
        {
            let key = (object.model.clone(), object.material.clone());
            if failures.contains(&key) {
                continue;
            }
            let model_indices = if let Some(indices) = resolved.get(&key) {
                indices.clone()
            } else {
                // At the cap, skip only *new* unique models. Further placements
                // of models already resolved must still be drawn.
                if models.len() >= MAX_MODELS {
                    budget_hit = true;
                    failures.insert(key);
                    continue;
                }
                let Some(decoded) = decoded_pairs.remove(&key) else {
                    failures.insert(key);
                    continue;
                };
                let (source, meshes, fallback_note) = match decoded.render {
                    Ok(loaded) => loaded,
                    Err(reasons) => {
                        report.unresolved += 1;
                        crate::crash_diagnostics::record(
                            "level_unresolved_geometry",
                            &format!(
                                "model={} material={} reasons={}",
                                object.model,
                                object.material,
                                reasons.join("; ")
                            ),
                        );
                        report.note(format!(
                            "Cannot resolve/decode {}: {}",
                            object.model,
                            reasons.join("; ")
                        ));
                        failures.insert(key);
                        continue;
                    }
                };
                if let Some(note) = fallback_note {
                    report.recovered_companions += 1;
                    report.note(note);
                }
                if models.len().saturating_add(meshes.len()) > MAX_MODELS {
                    budget_hit = true;
                    failures.insert(key);
                    continue;
                }
                let mut per_mesh_materials: Vec<Vec<Surface>> = Vec::new();
                // Preserve the actual resolver/decoder reason for every
                // material pair lacking diffuse pixels. The summary counter
                // cannot distinguish missing MTL, missing DDS, unsupported
                // format or an ambiguous high-mip fragment.
                let mut diffuse_reasons = Vec::<String>::new();
                if let Some(outcome) = decoded.material {
                    let source_virtual = outcome.virtual_source;
                    match outcome.path {
                        Ok(Some(material)) => {
                            let resolved = outcome.surfaces.unwrap_or_else(|| {
                                Err("Material surfaces were not decoded".into())
                            });
                            match resolved {
                                Ok((parts, warnings)) => {
                                    let has_diffuse =
                                        parts.iter().flatten().any(|part| part.diffuse.is_some());
                                    report.textured += usize::from(has_diffuse);
                                    let missing_sections = parts
                                        .iter()
                                        .map(|mesh_parts| missing_diffuse_sections(mesh_parts))
                                        .sum::<usize>();
                                    if has_diffuse && missing_sections > 0 {
                                        crate::crash_diagnostics::record(
                                            "level_partially_missing_diffuse",
                                            &format!(
                                                "model={} material={} source={} missing_sections={} total_sections={} warnings={}",
                                                object.model,
                                                object.material,
                                                source.archive,
                                                missing_sections,
                                                parts.iter().map(Vec::len).sum::<usize>(),
                                                warnings
                                                    .iter()
                                                    .take(3)
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                                    .join(" | "),
                                            ),
                                        );
                                    }
                                    if !has_diffuse {
                                        diffuse_reasons.extend(
                                            warnings
                                                .iter()
                                                .filter(|warning| {
                                                    let lower = warning.to_ascii_lowercase();
                                                    lower.contains("diffuse")
                                                        || lower.contains("dds")
                                                        || lower.contains("texture")
                                                })
                                                .take(3)
                                                .cloned(),
                                        );
                                    }
                                    per_mesh_materials = parts;
                                    for note in warnings.into_iter().take(2) {
                                        report.note(format!("{}: {note}", object.name));
                                    }
                                }
                                Err(error) => {
                                    diffuse_reasons.push(error.to_string());
                                    report.note(format!(
                                        "{}: source material {} cannot be assigned to {} mesh sections: {error}",
                                        object.name,
                                        material.display(),
                                        meshes.len()
                                    ));
                                }
                            }
                        }
                        Ok(None) if source_virtual.to_ascii_lowercase().ends_with(".cdf") => {
                            if let Some(Ok((parts, warnings))) = outcome.surfaces {
                                report.textured += usize::from(
                                    parts
                                        .iter()
                                        .flatten()
                                        .any(|surface| surface.diffuse.is_some()),
                                );
                                per_mesh_materials = parts;
                                for warning in warnings.into_iter().take(2) {
                                    report.note(format!("{}: {warning}", object.name));
                                }
                            }
                        }
                        Ok(None) => {
                            diffuse_reasons
                                .push(format!("No verified original MTL for {source_virtual}"));
                            report.note(format!(
                                "No verified original MTL reference for {} ({source_virtual}; {} mesh sections)",
                                object.name,
                                meshes.len()
                            ));
                        }
                        Err(error) => {
                            diffuse_reasons.push(format!("MTL lookup: {error}"));
                            report.note(format!(
                                "{}: exact source material resolution failed: {error}",
                                object.name
                            ));
                        }
                    }
                } else {
                    diffuse_reasons.push("Source PAK inventory unavailable".into());
                    report.note(format!(
                        "{}: source PAK inventory unavailable for texture resolution",
                        object.name
                    ));
                }
                // The normal geometry-dependency counter does not capture
                // successfully decoded meshes whose materials were missing,
                // ambiguous, unsupported or lacked an authored diffuse.
                // Count each resource pair once, not its thousands of instances.
                if !per_mesh_materials
                    .iter()
                    .flatten()
                    .any(|part| part.diffuse.is_some())
                {
                    report.without_diffuse += 1;
                    if diffuse_reasons.is_empty() {
                        diffuse_reasons
                            .push("No verified diffuse assigned by original material slots".into());
                    }
                    crate::crash_diagnostics::record(
                        "level_missing_verified_diffuse",
                        &format!(
                            "model={} material={} source={} sections={} reason={}",
                            object.model,
                            object.material,
                            source.archive,
                            meshes.len(),
                            diffuse_reasons.join(" | "),
                        ),
                    );
                }
                let mut indices = Vec::with_capacity(meshes.len());
                let mut partial = false;
                for (part_index, mesh) in meshes.into_iter().enumerate() {
                    let radius = world_instances::radius(&mesh);
                    let parts = per_mesh_materials
                        .get(part_index)
                        .cloned()
                        .unwrap_or_else(|| vec![neutral(0, mesh.indices.len() as u32)]);
                    let geometry = (mesh.positions.len() * 32
                        + mesh.indices.len() * 4
                        + object.model.len()) as u64;
                    let costs = parts.iter().map(surface_gpu_estimate).collect::<Vec<_>>();
                    let batches = surface_batch_ranges(&costs, geometry);
                    if models.len().saturating_add(batches.len()) > MAX_MODELS {
                        budget_hit = true;
                        partial = true;
                        break;
                    }
                    let shared = mesh;
                    if batches.len() > 1 {
                        report.note(format!(
                            "Split {} into {} independent GPU material-residency groups",
                            object.model,
                            batches.len(),
                        ));
                    }
                    for batch in batches {
                        indices.push(models.len());
                        models.push(world_instances::ModelGroup {
                            mesh: shared.clone(),
                            surfaces: parts[batch].to_vec(),
                            instances: Vec::new(),
                            radius,
                        });
                    }
                    report.geometries += 1;
                }
                if partial {
                    // Groups already pushed for this model stay empty (no
                    // instances), so a partially resident model is never drawn.
                    failures.insert(key);
                    continue;
                }
                resolved.insert(key, indices.clone());
                indices
            };
            if let Some(group) = object_index
                .checked_sub(real_objects)
                .and_then(|index| bulk.get(index))
            {
                for model in model_indices {
                    models[model]
                        .instances
                        .extend(group.transforms.iter().map(|transform| {
                            world_instances::Instance {
                                object_index: world_instances::NOT_SELECTABLE,
                                transform: *transform,
                            }
                        }));
                }
                report.placed += group.transforms.len();
                continue;
            }
            for group in model_indices {
                models[group].instances.push(world_instances::Instance {
                    object_index,
                    transform: object.world,
                });
            }
            if object.is_wildlife_spawn() {
                resolved_spawns.insert(object_index);
            }
            report.placed += 1;
        }
        if budget_hit {
            report.note("Original unique model resource cap reached; some geometry resources could not be resolved.".into());
        }
        // Expose actual CreatureSpawn / ZooKeeperShape positions as bounded
        // read-only indicator geometry. No unrelated wildlife model is guessed.
        let markers = scene
            .objects
            .iter()
            .enumerate()
            .filter(|(index, o)| {
                !o.hidden && o.is_wildlife_spawn() && !resolved_spawns.contains(index)
            })
            .take(20000)
            .map(|(object_index, object)| world_instances::Instance {
                object_index,
                transform: Transform {
                    position: object.world.position,
                    // Editor markers stay upright regardless of author yaw.
                    quaternion: [1.0, 0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            })
            .collect::<Vec<_>>();
        if !markers.is_empty() {
            // One shared mesh, so a reload keeps its GPU group.
            static MARKER: std::sync::LazyLock<Arc<RenderMesh>> =
                std::sync::LazyLock::new(|| Arc::new(wildlife_marker_mesh()));
            let mesh = MARKER.clone();
            let count = markers.len();
            models.push(world_instances::ModelGroup {
                radius: world_instances::radius(&mesh),
                surfaces: vec![neutral(0, mesh.indices.len() as u32)],
                mesh,
                instances: markers,
            });
            report.note(format!(
                "{count} unresolved wildlife spawn/area markers available in the visibility category."
            ));
            report.placed += count;
        }
        world.groups = models;
        world.instance_count = report.placed;
    }
    // Terrain is now a genuine world-space GPU mesh. The GPU's homogeneous
    // near-plane clipping avoids CPU projections that explode when the
    // editor approaches the ground; do not bake the terrain twice.
    if let Some(terrain) = terrain {
        world.groups.insert(
            0,
            world_instances::ModelGroup {
                mesh: std::sync::Arc::new(terrain.clone()),
                surfaces: vec![neutral(0, terrain.indices.len() as u32)],
                instances: vec![world_instances::Instance {
                    object_index: usize::MAX, // Ground is not a selectable placed object.
                    transform: Transform::default(),
                }],
                radius: world_instances::radius(terrain),
            },
        );
        world.instance_count += 1;
    }
    if combined.indices.is_empty() && world.instance_count == 0 {
        return Ok(None);
    }
    combined.info.vertex_count = combined.positions.len();
    combined.info.triangle_count = combined.indices.len() / 3;
    Ok(Some(Assembled {
        mesh: combined,
        surfaces,
        picks,
        report,
        world: std::sync::Arc::new(world),
    }))
}

/// Resolve only the wildlife category on first use. This deliberately does
/// not re-decode the map, terrain, or ordinary world models.
pub(super) fn assemble_wildlife(
    archive: &ImportedPak,
    scene: &LevelScene,
) -> Result<(std::sync::Arc<world_instances::World>, Report), String> {
    let mut wildlife = scene.clone();
    for object in &mut wildlife.objects {
        if !object.is_wildlife_spawn() {
            object.hidden = true;
        }
    }
    let assembled = assemble(archive, Some(&wildlife), None, None, true, &[])?
        .ok_or("No wildlife placements could be assembled")?;
    let mut world = (*assembled.world).clone();
    world.wildlife_distances = assembled.report.spawn_distances;
    world.wildlife_shapes = std::sync::Arc::new(
        scene
            .objects
            .iter()
            .filter(|object| {
                !object.hidden
                    && object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                    && object.shape_points.len() >= 2
            })
            .take(1024)
            .map(|object| {
                (
                    object.wildlife.use_terrain_height.unwrap_or(false),
                    object
                        .shape_points
                        .iter()
                        .map(|point| transformed(object.world, *point, false))
                        .collect(),
                )
            })
            .collect(),
    );
    let areas = scene
        .objects
        .iter()
        .filter(|object| !object.hidden && object.kind.eq_ignore_ascii_case("ZooKeeperShape"))
        .map(|object| (object.name.to_ascii_lowercase(), object.world.position))
        .collect::<HashMap<_, _>>();
    world.wildlife_links = std::sync::Arc::new(
        scene
            .objects
            .iter()
            .filter(|object| {
                !object.hidden
                    && object.kind.eq_ignore_ascii_case("CreatureSpawn")
                    && !object.home_area.is_empty()
            })
            .filter_map(|spawn| {
                areas
                    .get(&spawn.home_area.to_ascii_lowercase())
                    .map(|home| (spawn.world.position, *home))
            })
            .take(20000)
            .collect(),
    );
    Ok((std::sync::Arc::new(world), assembled.report))
}

/// Update baked mesh vertices for already-loaded source instances without
/// recompiling textures or re-indexing the entire imported map. The source
/// object triangle ranges never change and are owned by the scene copy.
// Retained for uncommitted axis-drag previews; project edits now save through
// the verified source-member transaction and reload the full scene.
#[allow(dead_code)]
pub(super) fn reproject_after_edit(
    original: &RenderMesh,
    ranges: &[PickRange],
    previous: &LevelScene,
    updated: &LevelScene,
) -> Option<RenderMesh> {
    let mut mesh = original.clone();
    for range in ranges {
        let old = previous.objects.get(range.object_index)?.world;
        let new = updated.objects.get(range.object_index)?.world;
        if old == new {
            continue;
        }
        if old.scale.iter().any(|scale| scale.abs() < 1e-6) {
            return None;
        }
        let inv = [
            old.quaternion[0],
            -old.quaternion[1],
            -old.quaternion[2],
            -old.quaternion[3],
        ];
        let start = range.first_index;
        let end = start.checked_add(range.index_count)?;
        let indices = mesh.indices.get(start..end)?;
        let unique = indices.iter().map(|v| *v as usize).collect::<HashSet<_>>();
        for i in unique {
            let point = mesh.positions.get_mut(i)?;
            let relative = [
                point[0] - old.position[0],
                point[1] - old.position[1],
                point[2] - old.position[2],
            ];
            let rotated = rotate(inv, relative);
            let local = [
                rotated[0] / old.scale[0],
                rotated[1] / old.scale[1],
                rotated[2] / old.scale[2],
            ];
            *point = transformed(new, local, false);
            if let Some(normal) = mesh.normals.as_mut().and_then(|normals| normals.get_mut(i)) {
                let old_normal = rotate(inv, *normal);
                let source_normal = [
                    old_normal[0] * old.scale[0],
                    old_normal[1] * old.scale[1],
                    old_normal[2] * old.scale[2],
                ];
                *normal = transformed(new, source_normal, true);
            }
        }
    }
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlabeled_imports_from_the_same_installation_match_without_crossing_games() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("first/Game");
        let other = root.path().join("second/Game");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&other).unwrap();
        let archive = |id: &str, source: &Path| ImportedPak {
            id: id.into(),
            name: "data.pak".into(),
            source: source.display().to_string(),
            directory: String::new(),
            group: String::new(),
            description: String::new(),
            entries: 0,
            extracted_bytes: 0,
            warnings: 0,
            builds: vec![],
        };
        let selected = archive("map", &game.join("levels_tutorial.pak"));
        let companion = archive("creatures", &game.join("characters_creatures.pak"));
        let unrelated = archive("other", &other.join("characters_creatures.pak"));
        let verified_game = source_game_directory(&selected).unwrap();
        assert!(same_verified_build(
            &selected,
            &companion,
            Some(&verified_game)
        ));
        assert!(!same_verified_build(
            &selected,
            &unrelated,
            Some(&verified_game)
        ));
        assert!(!same_verified_build(&selected, &companion, None));
    }

    #[test]
    #[ignore = "requires a local Stage 2 Asset Library (EVOLVE_BENCH_LIBRARY)"]
    fn imported_stage2_tutorial_resolves_real_creature_models() {
        let root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let library = imported_library(Path::new(&root)).unwrap();
        let map_name = std::env::var("EVOLVE_BENCH_MAP").unwrap_or_else(|_| "tutorial".into());
        let archive = library
            .archives
            .iter()
            .find(|archive| {
                archive
                    .name
                    .eq_ignore_ascii_case(&format!("levels_{map_name}.pak"))
                    && archive.source.to_ascii_lowercase().contains("moddedevolve")
            })
            .unwrap();
        println!("MAP {} builds={:?}", archive.id, archive.builds);
        for pak in library
            .archives
            .iter()
            .filter(|pak| pak.name.eq_ignore_ascii_case("libs.pak"))
        {
            println!("LIBS {} builds={:?}", pak.id, pak.builds);
        }
        let map = crate::level_library::open(archive.clone(), 0).unwrap();
        let scene = map.scene.as_ref().unwrap();
        let (world, report) = assemble_wildlife(archive, scene).unwrap();
        for note in &report.unresolved_populations {
            println!("UNRESOLVED {note}");
        }
        for note in &report.notes {
            println!("WILDLIFE {note}");
        }
        for (population, model) in &report.resolved_models {
            println!("RESOLVED {population} {model}");
        }
        for object in &scene.objects {
            if object.populations.iter().any(|name| {
                report
                    .resolved_models
                    .iter()
                    .any(|(resolved, _)| resolved == name)
            }) {
                println!(
                    "SPAWN {} {:?} {:?}",
                    object.name, object.world.position, object.populations
                );
            }
        }
        assert_eq!(report.spawn_distances.unwrap().minimum, 30.0);
        assert_eq!(report.spawn_distances.unwrap().maximum, 110.0);
        assert!(!world.groups.is_empty());
        if map_name.eq_ignore_ascii_case("tutorial") {
            assert!(
                world.groups.iter().any(|group| group.mesh.info.format
                    != "Wildlife spawn location marker (editor only)")
            );
            assert!(
                report
                    .resolved_models
                    .iter()
                    .any(|(name, model)| name == "TyrantElite_Tutorial"
                        && model == "characters/creatures/tyrant/tyrant_elite.cdf")
            );
        }
    }
    #[test]
    fn multi_material_group_splits_before_exceeding_residency_target() {
        let mb = 1024 * 1024u64;
        assert_eq!(
            surface_batch_ranges(&[230 * mb, 230 * mb, 230 * mb], 4 * mb),
            vec![0..2, 2..3],
        );
        assert_eq!(
            surface_batch_ranges(&[100 * mb, 100 * mb, 100 * mb], 4 * mb),
            vec![0..3],
        );
        // A single oversized source remains unaltered for later sparse-GPU
        // work rather than being silently downscaled or omitted.
        assert_eq!(surface_batch_ranges(&[800 * mb], 4 * mb), vec![0..1]);
    }

    #[test]
    fn matching_byte_identical_cgf_assets_are_not_falsely_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.cgf");
        let b = dir.path().join("b.cgf");
        std::fs::write(&a, b"exact-verified-geometry").unwrap();
        std::fs::write(&b, b"exact-verified-geometry").unwrap();
        let original = AssetLocation {
            archive: "A".into(),
            path: a,
        };
        let candidate = AssetLocation {
            archive: "B".into(),
            path: b.clone(),
        };
        assert!(identical_verified_sources(&[original.clone(), candidate.clone()]).unwrap());
        std::fs::write(&b, b"conflicting-geometry").unwrap();
        assert!(!identical_verified_sources(&[original, candidate]).unwrap());
    }

    #[test]
    fn exact_static_mesh_companions_do_not_search_other_folders() {
        assert_eq!(
            render_candidates("objects/natural/rock.cgf"),
            vec!["objects/natural/rock.cgf", "objects/natural/rock.cgfm"]
        );
        assert_eq!(
            render_candidates("objects/props/door.cga"),
            vec!["objects/props/door.cga", "objects/props/door.cgam"]
        );
    }

    #[test]
    fn compiled_character_render_companion_stays_in_authored_folder() {
        assert_eq!(
            compiled_render_path("objects/characters/hunter.skin"),
            "objects/characters/hunter.skinm",
        );
        assert_eq!(
            compiled_render_path("objects/characters/hunter.chr"),
            "objects/characters/hunter.chrm",
        );
        assert_eq!(
            compiled_render_path("objects/world/rock.cgf"),
            "objects/world/rock.cgf",
        );
    }

    #[test]
    fn source_without_streams_falls_back_to_exact_compiled_render_companion() {
        let mut decoded = Vec::new();
        let (source, parts, note) = decodable_render_source(
            "Objects/natural/rocks/riverbank02_a.cgf",
            |candidate| {
                Ok(Some(AssetLocation {
                    archive: "same-imported-build".into(),
                    path: PathBuf::from(candidate),
                }))
            },
            |path| {
                let virtual_name = path.to_string_lossy().to_string();
                decoded.push(virtual_name.clone());
                if virtual_name.ends_with(".cgf") {
                    return Err("mesh lacks referenced vertex/index streams".into());
                }
                Ok(vec![RenderMesh {
                    vertex_alpha: None,
                    vertex_color: None,
                    info: ModelInfo {
                        format: "test".into(),
                        version: 7,
                        chunk_count: 1,
                        vertex_count: 3,
                        triangle_count: 1,
                        original_uv0: false,
                        original_normals: false,
                    },
                    positions: vec![[0.0, 0.0, 0.0]; 3],
                    indices: vec![0, 1, 2],
                    uv0: None,
                    normals: None,
                }])
            },
        )
        .unwrap();
        assert_eq!(
            decoded,
            vec![
                "Objects/natural/rocks/riverbank02_a.cgf",
                "Objects/natural/rocks/riverbank02_a.cgfm",
            ]
        );
        assert_eq!(source.archive, "same-imported-build");
        assert_eq!(
            source.path,
            PathBuf::from("Objects/natural/rocks/riverbank02_a.cgfm")
        );
        assert_eq!(parts.len(), 1);
        assert!(note.unwrap().contains("compiled render companion"));
    }

    #[test]
    fn missing_or_ambiguous_model_companions_fail_closed_without_proxy_meshes() {
        let result = decodable_render_source(
            "Objects/world/rock.cgf",
            |candidate| {
                if candidate.ends_with(".cgfm") {
                    Err("Ambiguous imported asset in 2 PAKs".into())
                } else {
                    Ok(None)
                }
            },
            |_path| -> Result<Vec<RenderMesh>, String> {
                panic!("An unavailable or ambiguous source must never be decoded")
            },
        );
        let reasons = result.err().expect("no safe source");
        assert_eq!(reasons.len(), 2);
        assert!(reasons[0].contains("not present"));
        assert!(reasons[1].contains("Ambiguous"));
    }

    #[test]
    fn an_exact_render_source_still_has_priority_over_its_companion() {
        let (source, _, note) = decodable_render_source(
            "Objects/world/rock.cgf",
            |candidate| {
                Ok(Some(AssetLocation {
                    archive: "verified".into(),
                    path: PathBuf::from(candidate),
                }))
            },
            |_path| {
                Ok(vec![RenderMesh {
                    vertex_alpha: None,
                    vertex_color: None,
                    info: ModelInfo {
                        format: "test".into(),
                        version: 7,
                        chunk_count: 1,
                        vertex_count: 3,
                        triangle_count: 1,
                        original_uv0: false,
                        original_normals: false,
                    },
                    positions: vec![[0.0, 0.0, 0.0]; 3],
                    indices: vec![0, 1, 2],
                    uv0: None,
                    normals: None,
                }])
            },
        )
        .unwrap();
        assert_eq!(
            source.path.extension().and_then(|e| e.to_str()),
            Some("cgf")
        );
        assert!(note.is_none());
    }

    #[test]
    fn safe_lookup_rejects_paths_outside_game_virtual_root() {
        assert_eq!(
            safe_virtual("Objects\\World\\Tree.cgf").as_deref(),
            Some("objects/world/tree.cgf")
        );
        for path in [
            "../tree.cgf",
            "C:/Windows/x.cgf",
            "/etc/passwd",
            "objects/./tree.cgf",
        ] {
            assert!(safe_virtual(path).is_none());
        }
    }
    #[test]
    fn partial_material_failures_count_untextured_visible_sections() {
        let mut solid = neutral(0, 3);
        let mut untextured = neutral(3, 6);
        assert_eq!(
            missing_diffuse_sections(&[solid.clone(), untextured.clone()]),
            2
        );
        solid.diffuse = Some(std::sync::Arc::new(image::RgbaImage::new(2, 2)));
        assert_eq!(missing_diffuse_sections(&[solid, untextured.clone()]), 1);
        untextured.visible = false;
        assert_eq!(missing_diffuse_sections(&[untextured]), 0);
        assert_eq!(missing_diffuse_sections(&[neutral(0, 0)]), 0);
    }

    #[test]
    fn material_diagnostics_do_not_conflate_untextured_with_missing_mesh() {
        let report = Report {
            placed: 200,
            geometries: 8,
            textured: 3,
            without_diffuse: 5,
            unresolved: 0,
            ..Report::default()
        };
        assert!(
            report
                .status()
                .contains("5 model/material pairs without verified diffuse")
        );
        assert!(
            report
                .status()
                .contains("0 missing/unsupported geometry dependencies")
        );
    }

    #[test]
    fn combined_scene_offsets_indices_and_material_ranges_per_instance() {
        let source = RenderMesh {
            vertex_alpha: None,
            vertex_color: None,
            info: ModelInfo {
                format: "Fixture".into(),
                version: 1,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: true,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: Some(vec![[0., 0., 1.]; 3]),
        };
        let mut combined = RenderMesh {
            info: source.info.clone(),
            positions: Vec::new(),
            indices: Vec::new(),
            uv0: Some(Vec::new()),
            normals: Some(Vec::new()),
            vertex_alpha: Some(Vec::new()),
            vertex_color: None,
        };
        let mut ranges = Vec::new();
        assert!(append(
            &mut combined,
            &mut ranges,
            &source,
            &[neutral(0, 3)],
            Transform::default()
        ));
        let translated = Transform {
            position: [20.0, 0.0, 5.0],
            ..Transform::default()
        };
        assert!(append(
            &mut combined,
            &mut ranges,
            &source,
            &[neutral(0, 3)],
            translated
        ));
        assert_eq!(combined.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].first_index, 0);
        assert_eq!(ranges[1].first_index, 3);
        assert_eq!(combined.positions[3], [20.0, 0.0, 5.0]);
        assert_eq!(combined.positions[5], [20.0, 1.0, 5.0]);
    }

    #[test]
    fn duplicate_cross_archive_paths_are_not_silently_selected() {
        let key = "objects/tree.cgf".to_string();
        let mut index = Index {
            files: HashMap::new(),
            inventories: HashMap::new(),
            build: None,
            library_root: None,
            selected_id: "selected".into(),
            matching_build: HashSet::new(),
        };
        index.files.insert(
            key.clone(),
            vec![
                AssetLocation {
                    archive: "other-a".into(),
                    path: PathBuf::from("a"),
                },
                AssetLocation {
                    archive: "other-b".into(),
                    path: PathBuf::from("b"),
                },
            ],
        );
        assert!(index.lookup(&key).is_err());
        index.files.get_mut(&key).unwrap().push(AssetLocation {
            archive: "selected".into(),
            path: PathBuf::from("selected"),
        });
        assert_eq!(
            index.lookup(&key).unwrap().unwrap().path,
            PathBuf::from("selected")
        );
    }

    #[test]
    fn authored_world_transform_rotates_vertices_and_normals() {
        let tf = Transform {
            position: [10.0, 0.0, 2.0],
            quaternion: [0.70710677, 0.0, 0.0, 0.70710677],
            scale: [2.0, 2.0, 2.0],
        };
        let result = transformed(tf, [1.0, 0.0, 0.0], false);
        assert!((result[0] - 10.0).abs() < 0.001);
        assert!((result[1] - 2.0).abs() < 0.001);
        assert!((result[2] - 2.0).abs() < 0.001);
        let normal = transformed(tf, [1.0, 0.0, 0.0], true);
        assert!(normal[0].abs() < 0.001);
        assert!((normal[1] - 1.0).abs() < 0.001);
    }
}
