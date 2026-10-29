//! One source-authored mesh in the Level Editor viewport.
//!
//! Reuse the verified imported inventory and original material loader, rather
//! than drawing a guessed map mesh or borrowing same-named files from another
//! game build. All loads run in the existing blocking Operation worker.
use crate::{assembly_materials, asset_cache::SharedCache, textured::Surface};
use evolve_core::{
    ImportedPak, Inventory, LevelScene, ModelInfo, RenderMesh, Transform, imported_archive_files,
    imported_library, read_model,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_ARCHIVES: usize = 256;
const MAX_MODELS: usize = 256;
const MAX_PLACEMENTS: usize = 8_000;
const MAX_VERTICES: usize = 1_500_000;
const MAX_INDICES: usize = 4_500_000;
const MAX_SURFACES: usize = 2_048;

#[derive(Clone, Debug, Default)]
pub(super) struct Report {
    pub placed: usize,
    pub geometries: usize,
    pub textured: usize,
    pub unresolved: usize,
    pub notes: Vec<String>,
}

impl Report {
    pub fn status(&self) -> String {
        format!(
            "{} model instances · {} unique meshes · {} textured material sets · {} missing/unsupported dependencies. {}",
            self.placed,
            self.geometries,
            self.textured,
            self.unresolved,
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

fn relevant_asset(path: &str) -> bool {
    [".cgf", ".cga", ".cgfm", ".cgam", ".skinm", ".chrm", ".mtl"]
        .iter()
        .any(|ext| path.ends_with(ext))
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
                "Cross-PAK index capped at 256 archives; some assets may be unavailable.".into(),
            );
        }
        // Every manifest-listed asset is checked by the existing read-only
        // inventory validator before it can be indexed.
        for pak in archives {
            let inventory = if pak.id == selected.id {
                if let Some(project) = project_inventory {
                    project.clone()
                } else {
                    imported_archive_files(&pak).map_err(|e| e.to_string())?
                }
            } else {
                match imported_archive_files(&pak) {
                    Ok(found) => found,
                    Err(error) => {
                        report.note(format!("Skipped unavailable {}: {error}", pak.name));
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
                    matched = true;
                    index.files.entry(key).or_default().push(AssetLocation {
                        archive: pak.id.clone(),
                        path: root.join(&asset.relative_path),
                    });
                }
            }
            // Retain inventories only for PAKs that actually contain a
            // requested source. Keeping every imported archive inventory
            // alive would exhaust memory on large Stage 2 installations.
            if matched || pak.id == selected.id {
                index.inventories.insert(pak.id.clone(), inventory);
            }
        }
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
    let mut picks = Vec::new();
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
                wanted.insert(model.clone());
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
        // Geometry is decoded once per exact model/material pair; each authored
        // actor transform creates a distinct visible scene instance.
        let mut resolved = HashMap::<(String, String), usize>::new();
        let mut models = Vec::<(RenderMesh, Vec<Surface>)>::new();
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
            let model_index = if let Some(index) = resolved.get(&key) {
                *index
            } else {
                if models.len() >= MAX_MODELS {
                    budget_hit = true;
                    break;
                }
                let source = match sources.lookup(&compiled_render_path(&object.model)) {
                    Ok(Some(source)) => source,
                    Ok(None) => {
                        report.unresolved += 1;
                        report.note(format!("Missing model {}", object.model));
                        failures.insert(key);
                        continue;
                    }
                    Err(error) => {
                        report.unresolved += 1;
                        report.note(error);
                        failures.insert(key);
                        continue;
                    }
                };
                let mesh = match read_model(&source.path) {
                    Ok(mesh) => mesh,
                    Err(error) => {
                        report.unresolved += 1;
                        report.note(format!("Cannot decode {}: {error}", object.model));
                        failures.insert(key);
                        continue;
                    }
                };
                let original_inventory = sources.inventories.get(&source.archive);
                let authored_material = if object.material.is_empty() {
                    // This is an exact sibling candidate, not a global filename
                    // search. A missing authored material stays untextured.
                    Path::new(&object.model)
                        .with_extension("mtl")
                        .to_string_lossy()
                        .to_string()
                } else if object.material.to_ascii_lowercase().ends_with(".mtl") {
                    object.material.clone()
                } else {
                    format!("{}.mtl", object.material)
                };
                let mut material_ranges = Vec::new();
                if let Ok(Some(material)) = sources.lookup(&authored_material) {
                    if let Some(inventory) = original_inventory {
                        match assembly_materials::load_model(
                            &source.path,
                            &material.path,
                            inventory,
                            sources.library_root.as_deref(),
                            sources.build.as_deref(),
                            cache,
                        ) {
                            Ok((parts, warnings)) => {
                                report.textured +=
                                    usize::from(parts.iter().any(|part| part.diffuse.is_some()));
                                material_ranges = parts;
                                for note in warnings.into_iter().take(2) {
                                    report.note(format!("{}: {note}", object.name));
                                }
                            }
                            Err(error) => report.note(format!("{} material: {error}", object.name)),
                        }
                    }
                } else {
                    report.note(format!("No verified material for {}", object.name));
                }
                let index = models.len();
                models.push((mesh, material_ranges));
                resolved.insert(key, index);
                report.geometries += 1;
                index
            };
            let (mesh, parts) = &models[model_index];
            let before = combined.indices.len();
            if !append(&mut combined, &mut surfaces, mesh, parts, object.world) {
                budget_hit = true;
                break;
            }
            picks.push(PickRange {
                object_index,
                first_index: before,
                index_count: combined.indices.len() - before,
            });
            report.placed += 1;
            if report.placed >= MAX_PLACEMENTS {
                budget_hit = true;
                break;
            }
        }
        if budget_hit {
            report.note("Viewport reached a geometry, placement or GPU-surface budget; remaining objects are not rendered.".into());
        }
    }
    if combined.indices.is_empty() {
        return Ok(None);
    }
    combined.info.vertex_count = combined.positions.len();
    combined.info.triangle_count = combined.indices.len() / 3;
    Ok(Some(Assembled {
        mesh: combined,
        surfaces,
        picks,
        report,
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
