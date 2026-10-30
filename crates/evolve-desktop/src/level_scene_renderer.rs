//! One source-authored mesh in the Level Editor viewport.
//!
//! Reuse the verified imported inventory and original material loader, rather
//! than drawing a guessed map mesh or borrowing same-named files from another
//! game build. All loads run in the existing blocking Operation worker.
use crate::{assembly_materials, asset_cache::SharedCache, textured::Surface, world_instances};
use evolve_core::{
    ImportedPak, Inventory, LevelScene, ModelInfo, RenderMesh, Transform,
    begin_imported_lookup_session, imported_archive_files, imported_archive_preview_inventory,
    imported_library, read_world_model_parts,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_ARCHIVES: usize = 1_024;
const MAX_MODELS: usize = 1_024;
const MAX_VERTICES: usize = 1_500_000;
const MAX_INDICES: usize = 4_500_000;
const MAX_SURFACES: usize = 2_048;

#[derive(Clone, Debug, Default)]
pub(super) struct Report {
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
    [".cgf", ".cga", ".cgfm", ".cgam", ".skinm", ".chrm", ".mtl"]
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
fn decodable_render_source<L, D>(
    model: &str,
    mut lookup: L,
    mut decode: D,
) -> Result<(AssetLocation, Vec<RenderMesh>, Option<String>), Vec<String>>
where
    L: FnMut(&str) -> Result<Option<AssetLocation>, String>,
    D: FnMut(&Path) -> Result<Vec<RenderMesh>, String>,
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
        };
        let mut archives = vec![selected.clone()];
        if let (Some(root), Some(label)) = (&index.library_root, &index.build) {
            let library = imported_library(root).map_err(|e| e.to_string())?;
            let others = library
                .archives
                .into_iter()
                .filter(|pak| pak.id != selected.id && pak.builds.iter().any(|b| b == label));
            for pak in others.take(MAX_ARCHIVES - 1) {
                archives.push(pak);
            }
            report.note("Using exact virtual asset paths from the selected imported build.".into());
        } else {
            report.note("Build provenance is ambiguous; cross-PAK resources are disabled.".into());
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
                    if !canonical.starts_with(&root) {
                        continue;
                    }
                    accepted += 1;
                    matched = true;
                    index.files.entry(key).or_default().push(AssetLocation {
                        archive: pak.id.clone(),
                        path: canonical,
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
        if found.len() == 1 {
            Ok(found.first())
        } else {
            Err(format!(
                "Ambiguous imported asset {raw} appears in {} PAKs",
                found.len()
            ))
        }
    }
}

fn neutral(first_index: u32, index_count: u32) -> Surface {
    Surface {
        first_index,
        index_count,
        visible: true,
        transparent: false,
        diffuse: None,
        normal: None,
        emissive: None,
        armor_emissive: None,
        emission_strength: 1.0,
        kala_emission_state: None,
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

pub(super) fn assemble(
    archive: &ImportedPak,
    scene: Option<&LevelScene>,
    terrain: Option<&RenderMesh>,
    cache: &SharedCache,
    project_inventory: Option<&Inventory>,
) -> Result<Option<Assembled>, String> {
    let mut report = Report::default();
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
        let sources = Index::new(archive, &wanted, &mut report, project_inventory)?;
        // This scope caches all matching-build manifest paths for the duration
        // of assembly. Every material/texture hit still rechecks its actual
        // file size and canonical containment, but no dependency triggers a
        // fresh enumeration of the entire Asset Library.
        let lookup_started = Instant::now();
        let _scoped_lookup = sources
            .library_root
            .as_deref()
            .zip(sources.build.as_deref())
            .map(|(root, build)| begin_imported_lookup_session(root, build))
            .transpose()
            .map_err(|error| error.to_string())?;
        report.note(format!(
            "Exact-build material lookup index initialized in {} ms.",
            lookup_started.elapsed().as_millis()
        ));
        // Geometry is decoded once per exact model/material pair; each authored
        // actor transform creates a distinct visible scene instance.
        let mut resolved = HashMap::<(String, String), Vec<usize>>::new();
        // A compiled mesh is decoded once, not copied into each object's
        // vertex/index arrays. GPU draw calls use instanced transforms.
        let mut models = Vec::<world_instances::ModelGroup>::new();
        let mut failures = HashSet::<(String, String)>::new();
        let mut budget_hit = false;
        for (object_index, object) in scene
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
                if models.len() >= MAX_MODELS {
                    budget_hit = true;
                    break;
                }
                // Source metadata/physics chunks may not contain render
                // streams even though a verified same-stem compiled companion
                // does. Decode candidates in order, retaining exact paths and
                // the selected build rather than guessing alternate objects.
                let (source, meshes, fallback_note) = match decodable_render_source(
                    &object.model,
                    |candidate| sources.lookup(candidate).map(|found| found.cloned()),
                    |path| read_world_model_parts(path).map_err(|error| error.to_string()),
                ) {
                    Ok(loaded) => loaded,
                    Err(reasons) => {
                        report.unresolved += 1;
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
                    break;
                }
                let original_inventory = sources.inventories.get(&source.archive);
                let mut material_ranges = Vec::new();
                if let Some(inventory) = original_inventory {
                    // A world model may declare MtlName in its compiled mesh,
                    // even when there is NO same-stem .mtl. Reuse the Model
                    // Viewer's exact original-material resolver rather than
                    // assuming the geometry filename is the material name.
                    // For a compiled fallback, inspect that verified virtual
                    // owner so its own embedded material hint can be used.
                    let source_virtual = render_candidates(&object.model)
                        .into_iter()
                        .find(|candidate| {
                            let candidate_extension =
                                Path::new(candidate).extension().and_then(|e| e.to_str());
                            candidate_extension.is_some_and(|extension| {
                                source
                                    .path
                                    .extension()
                                    .and_then(|e| e.to_str())
                                    .is_some_and(|actual| actual.eq_ignore_ascii_case(extension))
                            })
                        })
                        .unwrap_or_else(|| object.model.clone());
                    let explicit =
                        (!object.material.trim().is_empty()).then_some(object.material.as_str());
                    match assembly_materials::material_for_part(
                        &source_virtual,
                        explicit,
                        inventory,
                        sources.library_root.as_deref(),
                        sources.build.as_deref(),
                    ) {
                        Ok(Some(material)) => {
                            let full = (meshes.len() == 1).then(|| {
                                assembly_materials::load_model(
                                    &source.path,
                                    &material,
                                    inventory,
                                    sources.library_root.as_deref(),
                                    sources.build.as_deref(),
                                    cache,
                                )
                            });
                            let resolved = match full {
                                Some(Ok(data)) => Ok(data),
                                _ => assembly_materials::original_single_leaf_surface(
                                    &material,
                                    inventory,
                                    sources.library_root.as_deref(),
                                    sources.build.as_deref(),
                                    cache,
                                )
                                .map(|(surface, warnings)| (vec![surface], warnings)),
                            };
                            match resolved {
                                Ok((parts, warnings)) => {
                                    report.textured += usize::from(
                                        parts.iter().any(|part| part.diffuse.is_some()),
                                    );
                                    material_ranges = parts;
                                    for note in warnings.into_iter().take(2) {
                                        report.note(format!("{}: {note}", object.name));
                                    }
                                }
                                Err(error) => report.note(format!(
                                    "{}: source material {} cannot be assigned to {} mesh sections: {error}",
                                    object.name,
                                    material.display(),
                                    meshes.len()
                                )),
                            }
                        }
                        Ok(None) => report.note(format!(
                            "No verified original MTL reference for {} ({source_virtual}; {} mesh sections)",
                            object.name,
                            meshes.len()
                        )),
                        Err(error) => report.note(format!(
                            "{}: exact source material resolution failed: {error}",
                            object.name
                        )),
                    }
                } else {
                    report.note(format!(
                        "{}: source PAK inventory unavailable for texture resolution",
                        object.name
                    ));
                }
                // The normal geometry-dependency counter does not capture
                // successfully decoded meshes whose materials were missing,
                // ambiguous, unsupported or lacked an authored diffuse.
                // Count each resource pair once, not its thousands of instances.
                if !material_ranges.iter().any(|part| part.diffuse.is_some()) {
                    report.without_diffuse += 1;
                }
                let mut indices = Vec::with_capacity(meshes.len());
                for mesh in meshes {
                    let radius = world_instances::radius(&mesh);
                    let parts = if material_ranges.is_empty() {
                        vec![neutral(0, mesh.indices.len() as u32)]
                    } else if material_ranges.len() == 1 && material_ranges[0].index_count == 0 {
                        let mut original = material_ranges[0].clone();
                        original.index_count = mesh.indices.len() as u32;
                        vec![original]
                    } else {
                        material_ranges.clone()
                    };
                    indices.push(models.len());
                    models.push(world_instances::ModelGroup {
                        mesh: std::sync::Arc::new(mesh),
                        surfaces: parts,
                        instances: Vec::new(),
                        radius,
                    });
                    report.geometries += 1;
                }
                resolved.insert(key, indices.clone());
                indices
            };
            for group in model_indices {
                models[group].instances.push(world_instances::Instance {
                    object_index,
                    transform: object.world,
                });
            }
            report.placed += 1;
        }
        if budget_hit {
            report.note("Original unique model resource cap reached; some geometry resources could not be resolved.".into());
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

/// Update baked mesh vertices for already-loaded source instances without
/// recompiling textures or re-indexing the entire imported map. The source
/// object triangle ranges never change and are owned by the scene copy.
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
