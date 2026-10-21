//! Exact, static CDF character-assembly preview.
//! Original CDF skin meshes remain in source skeleton space. CA_BONE rigid
//! props can be positioned only using verified authored sockets and bind pose.
//! Playback and export still require separately validated skinning.
use crate::cdf::{
    AssetResolution, AttachmentKind, CdfDefinition, CdfError, read_cdf, resolve_virtual,
};
use crate::character_equipment::{EquipmentOverrides, validated_equipment_overrides};
use crate::inventory::Inventory;
use crate::legacy_rig::{parse_legacy_skeleton, read_legacy_skin};
use crate::library_catalog::{LibraryReferenceStatus, resolve_imported_reference};
use crate::model::{ModelError, ModelInfo, RenderMesh, read_model};
use crate::rig::{CompiledSkeleton, parse_compiled_skeleton, remap_verified_weights};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const MAX_COMBINED_VERTICES: usize = 1_000_000;
const MAX_COMBINED_INDICES: usize = 6_000_000;

#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    #[error("CDF parse failed: {0}")]
    Cdf(#[from] CdfError),
    #[error("Source geometry failed to decode: {0}")]
    Model(#[from] ModelError),
    #[error("Unsafe or incomplete assembled character: {0}")]
    Incomplete(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyReport {
    pub cdf_skeleton: String,
    pub selected_skin_set: Option<String>,
    pub original_parts: Vec<String>,
    pub vertices: usize,
    pub triangles: usize,
    pub original_uv0_complete: bool,
    pub original_normals_complete: bool,
    pub skeletal_animation_enabled: bool,
    #[serde(default)]
    pub source_parts: Vec<AssemblyPart>,
    /// One entry per original render mesh, independently of submaterial count.
    #[serde(default)]
    pub geometry_parts: Vec<AssemblyGeometry>,
    #[serde(default)]
    pub verified_bones: Option<usize>,
    /// Canonical, verified compiled skeleton location (not a guess).
    #[serde(default)]
    pub skeleton_source: Option<std::path::PathBuf>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Read-only source identity and position spans for validated motion binding.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyGeometry {
    pub name: String,
    pub source: std::path::PathBuf,
    pub vertex_first: usize,
    pub vertex_count: usize,
    pub bone_socket: Option<String>,
    pub verified_skin_weights: bool,
}

/// Original index ranges allow separate GPU material bindings without
/// painting the entire character with its first diffuse texture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyPart {
    pub name: String,
    pub binding: String,
    pub first_index: u32,
    pub index_count: u32,
    /// Original CryEngine subset material index; 255 means no authored leaf.
    #[serde(default)]
    pub material_slot: u32,
    pub material_hint: Option<String>,
    pub verified_skin_weights: bool,
}

/// Kala's original 3P CDF supplies mutually exclusive weapon visibility groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KalaTubingState {
    #[default]
    Banshee,
    ArmorReducer,
}

/// Original CDF verification shared by attachment selection and desktop options.
pub fn is_original_kala_3p(cdf: &CdfDefinition) -> bool {
    cdf.skeleton
        .eq_ignore_ascii_case("characters/hunters/merc_tech/tech_3p_skeleton.chr")
        && cdf.attachments.iter().any(|part| {
            part.name.eq_ignore_ascii_case("bansheegun")
                && part
                    .binding
                    .eq_ignore_ascii_case("characters/hunters/merc_tech/techbansheegun_3p.skin")
        })
        && cdf.attachments.iter().any(|part| {
            part.name.eq_ignore_ascii_case("armorreducer")
                && part
                    .binding
                    .eq_ignore_ascii_case("characters/hunters/merc_tech/techarmorreducer_3p.skin")
        })
        && [
            "hiddenwhenbansheegunselected",
            "hiddenwhenarmorreducerselected",
        ]
        .iter()
        .all(|name| cdf.skin_sets.contains_key(*name))
}

pub fn select_attachments(
    cdf: &CdfDefinition,
) -> Result<(Option<String>, Vec<&str>), AssemblyError> {
    select_attachments_with_kala_state(cdf, KalaTubingState::Banshee)
}

pub fn select_attachments_with_kala_state(
    cdf: &CdfDefinition,
    kala_state: KalaTubingState,
) -> Result<(Option<String>, Vec<&str>), AssemblyError> {
    let chosen = if cdf.skin_sets.is_empty() {
        None
    } else if cdf.skin_sets.contains_key("default") {
        Some("default".to_owned())
    } else if is_original_kala_3p(cdf) {
        Some(
            match kala_state {
                KalaTubingState::Banshee => "hiddenwhenbansheegunselected",
                KalaTubingState::ArmorReducer => "hiddenwhenarmorreducerselected",
            }
            .to_owned(),
        )
    } else {
        // Stage 2 CDFs often use SkinSets named HiddenWhen*Selected as
        // equipment exclusion metadata, not actual renderable variants.
        // Those sets must not make the entire character unviewable.
        let actual_variants = cdf
            .skin_sets
            .keys()
            .filter(|name| !name.starts_with("hiddenwhen"))
            .collect::<Vec<_>>();
        if actual_variants.len() == 1 {
            Some((*actual_variants[0]).clone())
        } else {
            None
        }
    };
    let keys = chosen.as_deref().and_then(|name| cdf.skin_sets.get(name));
    let selected = cdf
        .attachments
        .iter()
        .filter(|part| {
            keys.is_none_or(|keys| {
                let normalized: String = part
                    .name
                    .to_ascii_lowercase()
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect();
                if is_original_kala_3p(cdf)
                    && matches!(
                        chosen.as_deref(),
                        Some("hiddenwhenbansheegunselected" | "hiddenwhenarmorreducerselected")
                    )
                {
                    // This Kala SkinSet is an exclusion list, not a list
                    // of meshes to render. Keep the body and active gun.
                    !keys.contains(&normalized)
                } else {
                    keys.contains(&normalized)
                        || !cdf.skin_sets.values().any(|set| set.contains(&normalized))
                }
            })
        })
        .map(|part| part.name.as_str())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(AssemblyError::Incomplete(
            "Selected CDF has no loadable mesh attachments".into(),
        ));
    }
    Ok((chosen, selected))
}

/// Merge individual verified meshes, preserving all source vertex attributes
/// and offsetting triangle indices. Loss of UV0/normals is explicit: mixed
/// incomplete attributes yield no fabricated combined attribute stream.
pub fn combine_source_meshes(
    parts: Vec<(String, RenderMesh)>,
) -> Result<(RenderMesh, Vec<String>), AssemblyError> {
    if parts.is_empty() || parts.len() > 512 {
        return Err(AssemblyError::Incomplete(
            "No source parts, or too many assembly parts".into(),
        ));
    }
    let uv_complete = parts.iter().all(|(_, mesh)| {
        mesh.uv0
            .as_ref()
            .is_some_and(|uv| uv.len() == mesh.positions.len())
    });
    let normals_complete = parts.iter().all(|(_, mesh)| {
        mesh.normals
            .as_ref()
            .is_some_and(|n| n.len() == mesh.positions.len())
    });
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut uv0 = uv_complete.then(Vec::new);
    let mut normals = normals_complete.then(Vec::new);
    let mut names = Vec::with_capacity(parts.len());
    for (name, mesh) in parts {
        if mesh.positions.is_empty()
            || mesh.indices.is_empty()
            || !mesh.indices.len().is_multiple_of(3)
            || mesh
                .indices
                .iter()
                .any(|&index| index as usize >= mesh.positions.len())
        {
            return Err(AssemblyError::Incomplete(format!(
                "Invalid or empty source part: {name}"
            )));
        }
        let offset = u32::try_from(positions.len())
            .map_err(|_| AssemblyError::Incomplete("Assembly too large".into()))?;
        let vertices = positions
            .len()
            .checked_add(mesh.positions.len())
            .ok_or_else(|| AssemblyError::Incomplete("Vertex arithmetic overflow".into()))?;
        let faces = indices
            .len()
            .checked_add(mesh.indices.len())
            .ok_or_else(|| AssemblyError::Incomplete("Index arithmetic overflow".into()))?;
        if vertices > MAX_COMBINED_VERTICES || faces > MAX_COMBINED_INDICES {
            return Err(AssemblyError::Incomplete(
                "Combined character exceeds safe geometry limits".into(),
            ));
        }
        for index in mesh.indices {
            indices.push(
                index
                    .checked_add(offset)
                    .ok_or_else(|| AssemblyError::Incomplete("Index overflow".into()))?,
            );
        }
        positions.extend(mesh.positions);
        if let (Some(out), Some(source)) = (&mut uv0, mesh.uv0) {
            out.extend(source);
        }
        if let (Some(out), Some(source)) = (&mut normals, mesh.normals) {
            out.extend(source);
        }
        names.push(name);
    }
    let info = ModelInfo {
        format: "Verified static CDF assembly (NOT skinned)".into(),
        version: 0,
        chunk_count: names.len(),
        vertex_count: positions.len(),
        triangle_count: indices.len() / 3,
        original_uv0: uv0.is_some(),
        original_normals: normals.is_some(),
    };
    Ok((
        RenderMesh {
            info,
            positions,
            indices,
            uv0,
            normals,
        },
        names,
    ))
}

/// An exact virtual path is resolved within the indexed extraction first,
/// then through validated imported PAK manifests. Ambiguous assets fail
/// instead of mixing builds or matching another character by basename.
fn preview_source(
    inventory: &Inventory,
    authored: &str,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<std::path::PathBuf>, AssemblyError> {
    let root = fs::canonicalize(&inventory.root)?;
    let (location, allowed_root) = match resolve_virtual(inventory, authored) {
        AssetResolution::Found(relative) => (root.join(relative), root),
        AssetResolution::Ambiguous(_) => {
            return Err(AssemblyError::Incomplete(format!(
                "Ambiguous local asset: {authored}"
            )));
        }
        AssetResolution::Missing => {
            let Some(imported) = imported else {
                return Ok(None);
            };
            let imported_root = fs::canonicalize(imported)?;
            match resolve_imported_reference(imported, authored, build)
                .map_err(|e| AssemblyError::Incomplete(e.to_string()))?
            {
                LibraryReferenceStatus::Found(hit) => {
                    (std::path::PathBuf::from(hit.absolute_path), imported_root)
                }
                LibraryReferenceStatus::Missing => return Ok(None),
                LibraryReferenceStatus::Ambiguous(_) => {
                    return Err(AssemblyError::Incomplete(format!(
                        "Ambiguous imported asset: {authored}"
                    )));
                }
            }
        }
    };
    let metadata = fs::symlink_metadata(&location)?;
    let canonical = fs::canonicalize(&location)?;
    if !metadata.file_type().is_file() || !canonical.starts_with(allowed_root) {
        return Err(AssemblyError::Incomplete(format!(
            "Source is no longer a safe regular asset: {authored}"
        )));
    }
    Ok(Some(canonical))
}

fn compiled_companion(binding: &str) -> Option<String> {
    let (stem, extension) = binding.rsplit_once('.')?;
    match extension.to_ascii_lowercase().as_str() {
        "skin" => Some(format!("{stem}.skinm")),
        "chr" => Some(format!("{stem}.chrm")),
        "cgf" => Some(format!("{stem}.cgfm")),
        "cga" => Some(format!("{stem}.cgam")),
        _ => None,
    }
}

fn transform_equipment(
    mesh: &mut RenderMesh,
    attachment: &crate::cdf::CdfAttachment,
    skeleton: &CompiledSkeleton,
) -> Result<(), AssemblyError> {
    let bone = attachment.bone_name.as_deref().ok_or_else(|| {
        AssemblyError::Incomplete(format!(
            "Equipment {} has no authored BoneName",
            attachment.name
        ))
    })?;
    let matches = skeleton
        .names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.eq_ignore_ascii_case(bone))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(AssemblyError::Incomplete(format!(
            "Equipment {} references absent/ambiguous compiled bone: {bone}",
            attachment.name
        )));
    }
    let matrix = &skeleton.world[matches[0]];
    let [w, x, y, z] = attachment.rotation.unwrap_or([1., 0., 0., 0.]);
    let norm = w * w + x * x + y * y + z * z;
    if !norm.is_finite() || (norm - 1.).abs() > 0.1 {
        return Err(AssemblyError::Incomplete(format!(
            "Equipment {} has invalid authored quaternion",
            attachment.name
        )));
    }
    let f = 1. / norm;
    let (w, x, y, z) = (w * f.sqrt(), x * f.sqrt(), y * f.sqrt(), z * f.sqrt());
    let rotation = [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ],
    ];
    let offset = attachment.position.unwrap_or([0., 0., 0.]);
    let rotate = |p: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|i| (0..3).map(|j| rotation[i][j] * p[j]).sum::<f32>())
    };
    for point in &mut mesh.positions {
        let local = rotate(*point);
        let transformed: [f32; 3] = std::array::from_fn(|i| {
            (0..3)
                .map(|j| matrix[i][j] * (local[j] + offset[j]))
                .sum::<f32>()
                + matrix[i][3]
        });
        if transformed.iter().any(|c| !c.is_finite() || c.abs() > 1e7) {
            return Err(AssemblyError::Incomplete(
                "Equipment transform out of range".into(),
            ));
        }
        *point = transformed;
    }
    if let Some(normals) = &mut mesh.normals {
        for normal in normals {
            let rotated = rotate(*normal);
            let transformed: [f32; 3] =
                std::array::from_fn(|i| (0..3).map(|j| matrix[i][j] * rotated[j]).sum::<f32>());
            let len = transformed.iter().map(|c| c * c).sum::<f32>().sqrt();
            if !len.is_finite() || len < 1e-6 {
                return Err(AssemblyError::Incomplete(
                    "Invalid equipment normal matrix".into(),
                ));
            }
            *normal = transformed.map(|c| c / len);
        }
    }
    Ok(())
}

/// Compatibility API for indexed local extractions; richer previews may
/// resolve a character's author-declared geometry from shared imported PAKs.
pub fn read_static_cdf_assembly(
    cdf_file: &Path,
    library: &Inventory,
) -> Result<(RenderMesh, AssemblyReport), AssemblyError> {
    read_static_cdf_assembly_with_imported(cdf_file, library, None, None)
}

pub fn read_static_cdf_assembly_with_imported(
    cdf_file: &Path,
    library: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<(RenderMesh, AssemblyReport), AssemblyError> {
    read_static_cdf_assembly_with_equipment(
        cdf_file,
        library,
        imported,
        build,
        &EquipmentOverrides::new(),
    )
}

/// Alternate bindings must be verified source variants. Never patch originals.
pub fn read_static_cdf_assembly_with_equipment(
    cdf_file: &Path,
    library: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    requested: &EquipmentOverrides,
) -> Result<(RenderMesh, AssemblyReport), AssemblyError> {
    read_static_cdf_assembly_with_kala_state(
        cdf_file,
        library,
        imported,
        build,
        requested,
        KalaTubingState::Banshee,
    )
}

/// Preserve all source geometry and select only the original authored
/// active-weapon exclusions for the requested Kala preview state.
pub fn read_static_cdf_assembly_with_kala_state(
    cdf_file: &Path,
    library: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    requested: &EquipmentOverrides,
    kala_state: KalaTubingState,
) -> Result<(RenderMesh, AssemblyReport), AssemblyError> {
    let root = fs::canonicalize(&library.root)?;
    let cdf_metadata = fs::symlink_metadata(cdf_file)?;
    let cdf_location = fs::canonicalize(cdf_file)?;
    if !cdf_metadata.file_type().is_file() || !cdf_location.starts_with(&root) {
        return Err(AssemblyError::Incomplete(
            "CDF file must be a real indexed file inside the selected asset root".into(),
        ));
    }
    let definition = read_cdf(&cdf_location)?;
    let overrides = validated_equipment_overrides(&definition, library, requested)
        .map_err(AssemblyError::Incomplete)?;
    let (skin_set, selected) = select_attachments_with_kala_state(&definition, kala_state)?;
    let mut notes = Vec::new();
    // Static skinned mesh geometry can be rendered without a .chr skeleton.
    // Lack of a verified bind pose must only disable rigid bone sockets and
    // animation, not make a character's valid authored .skinm invisible.
    let skeleton_path = match preview_source(library, &definition.skeleton, imported, build) {
        Ok(path) => path,
        Err(error) => {
            notes.push(format!("CDF skeleton: {error}"));
            None
        }
    };
    let skeleton_bytes = skeleton_path.as_ref().and_then(|path| fs::read(path).ok());
    let skeleton = skeleton_bytes.as_ref().and_then(|bytes| {
        parse_compiled_skeleton(bytes)
            .or_else(|_| parse_legacy_skeleton(bytes))
            .ok()
    });
    if skeleton.is_none() {
        notes.push(format!(
            "Verified bind-pose skeleton unavailable ({}); static skin geometry remains viewable, but rigid sockets and animation are disabled",
            definition.skeleton
        ));
    }
    let mut meshes = Vec::new();
    let mut source_parts = Vec::new();
    let mut geometry_parts = Vec::new();
    let mut vertex_cursor: usize = 0;
    let mut index_cursor: u32 = 0;
    for attachment in &definition.attachments {
        if !selected.contains(&attachment.name.as_str()) {
            continue;
        }
        let binding = overrides
            .get(&attachment.name)
            .map(String::as_str)
            .unwrap_or(&attachment.binding);
        // The .skin is the authored skeletal binding, not necessarily a
        // renderable mesh, and may live in a different source PAK. A precise
        // virtual-path .skinm companion is sufficient for STATIC preview;
        // it does not authorize animation without original .skin weights.
        let bound = match preview_source(library, binding, imported, build) {
            Ok(path) => path,
            Err(error) => {
                notes.push(format!("{}: authored binding: {error}", attachment.name));
                None
            }
        };
        let companion = compiled_companion(binding);
        let compiled = match companion.as_deref() {
            Some(path) => match preview_source(library, path, imported, build) {
                Ok(found) => found,
                Err(error) => {
                    notes.push(format!("{}: render companion: {error}", attachment.name));
                    None
                }
            },
            None => None,
        };
        let rendered = if let Some(rendered) = compiled {
            if bound.is_none() {
                notes.push(format!(
                    "{}: original binding {} absent; rendering exact compiled companion only (skin weights unverified)",
                    attachment.name, binding
                ));
            }
            rendered
        } else if let Some(bound) = bound {
            bound
        } else {
            notes.push(format!(
                "{}: missing original binding and exact render companion",
                attachment.name
            ));
            continue;
        };
        let mut mesh = match read_model(&rendered) {
            Ok(mesh) => mesh,
            Err(error) => {
                notes.push(format!(
                    "{}: unsupported geometry ({error})",
                    attachment.name
                ));
                continue;
            }
        };
        if matches!(attachment.kind, AttachmentKind::Bone) {
            let Some(verified) = skeleton.as_ref() else {
                notes.push(format!(
                    "{}: no verified skeleton for bone socket",
                    attachment.name
                ));
                continue;
            };
            if let Err(error) = transform_equipment(&mut mesh, attachment, verified) {
                notes.push(format!("{}: {error}", attachment.name));
                continue;
            }
        }
        let verified_skin_weights = if matches!(
            attachment.kind,
            AttachmentKind::Skin | AttachmentKind::UniqueSkin
        ) {
            if let (Some(skeleton), Some(master)) = (&skeleton, &skeleton_bytes) {
                fs::read(&rendered).ok().is_some_and(|raw| {
                    if raw.starts_with(b"CryTek\0\0") {
                        read_legacy_skin(&raw, master, mesh.positions.len()).is_ok()
                    } else {
                        remap_verified_weights(&raw, mesh.positions.len(), skeleton.names.len())
                            .is_ok()
                    }
                })
            } else {
                false
            }
        } else {
            false
        };
        if !verified_skin_weights && !matches!(attachment.kind, AttachmentKind::Bone) {
            notes.push(format!(
                "{}: original weights not yet verified against selected skeleton",
                attachment.name
            ));
        }
        geometry_parts.push(AssemblyGeometry {
            name: attachment.name.clone(),
            source: rendered.clone(),
            vertex_first: vertex_cursor,
            vertex_count: mesh.positions.len(),
            bone_socket: (attachment.kind == AttachmentKind::Bone)
                .then(|| attachment.bone_name.clone())
                .flatten(),
            verified_skin_weights,
        });
        vertex_cursor = vertex_cursor
            .checked_add(mesh.positions.len())
            .ok_or_else(|| {
                AssemblyError::Incomplete("CDF assembly vertex offset overflow".into())
            })?;
        let count = u32::try_from(mesh.indices.len())
            .map_err(|_| AssemblyError::Incomplete("CDF part exceeds index limits".into()))?;
        let ranges =
            crate::model::original_mesh_subsets(&fs::read(&rendered)?, mesh.indices.len())?;
        for (first, indices, material_slot) in ranges {
            source_parts.push(AssemblyPart {
                name: attachment.name.clone(),
                binding: binding.to_owned(),
                first_index: index_cursor.checked_add(first).ok_or_else(|| {
                    AssemblyError::Incomplete("Material index offset overflow".into())
                })?,
                index_count: indices,
                material_slot,
                material_hint: attachment.material.clone(),
                verified_skin_weights,
            });
        }
        index_cursor = index_cursor
            .checked_add(count)
            .ok_or_else(|| AssemblyError::Incomplete("Assembly index ranges overflow".into()))?;
        meshes.push((attachment.name.clone(), mesh));
    }
    if meshes.is_empty() {
        return Err(AssemblyError::Incomplete(
            "No authored CDF attachments could be rendered; inspect the part diagnostics".into(),
        ));
    }
    let (mesh, original_parts) = combine_source_meshes(meshes)?;
    let report = AssemblyReport {
        cdf_skeleton: definition.skeleton,
        selected_skin_set: skin_set,
        original_parts,
        vertices: mesh.info.vertex_count,
        triangles: mesh.info.triangle_count,
        original_uv0_complete: mesh.uv0.is_some(),
        original_normals_complete: mesh.normals.is_some(),
        skeletal_animation_enabled: false,
        source_parts,
        geometry_parts,
        verified_bones: skeleton.as_ref().map(|s| s.names.len()),
        skeleton_source: skeleton.as_ref().and(skeleton_path),
        warnings: notes,
    };
    Ok((mesh, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_scorpid_cdf_renders_five_authored_chr_parts_with_unbound_sockets() {
        let temp = tempfile::tempdir().unwrap();
        // Test the common case where the user selects just this character
        // directory, rather than its grandparent containing "characters".
        let folder = temp.path().join("characters/monsters/Scorpid_2");
        fs::create_dir_all(&folder).unwrap();
        let parts = [
            "Scorpid1_Rig.chr",
            "Scorpid1_Default_Head.chr",
            "Scorpid1_Default_Petapalps.chr",
            "Scorpid1_L_Basher.chr",
            "Scorpid1_R_Basher.chr",
        ];
        for part in parts {
            fs::write(folder.join(part), crate::model::tests::fixture()).unwrap();
        }
        let cdf = folder.join("scorpid1.cdf");
        fs::write(
            &cdf,
            br#"<CharacterDefinition>
            <Model File="objects/characters/monsters/scorpid_2/scorpid1_skeleton.chr"/>
            <AttachmentList>
                <Attachment AName="torso" Type="CA_SKIN"
                    Binding="objects/characters/monsters/scorpid_2/scorpid1_rig.chr"/>
                <Attachment AName="head" Type="CA_SKIN"
                    Binding="objects/characters/monsters/scorpid_2/scorpid1_default_head.chr"/>
                <Attachment AName="palps" Type="CA_SKIN"
                    Binding="objects/characters/monsters/scorpid_2/scorpid1_default_petapalps.chr"/>
                <Attachment AName="basher_l" Type="CA_SKIN"
                    Binding="objects/characters/monsters/scorpid_2/scorpid1_l_basher.chr"/>
                <Attachment AName="basher_r" Type="CA_SKIN"
                    Binding="objects/characters/monsters/scorpid_2/scorpid1_r_basher.chr"/>
                <Attachment AName="lefthandpalp" Type="CA_BONE" BoneName="Bip01 L Handpalp"
                    Binding=""/>
                <Attachment AName="righthandpalp" Type="CA_BONE" BoneName="Bip01 R Handpalp"
                    Binding=""/>
            </AttachmentList>
            </CharacterDefinition>"#,
        )
        .unwrap();
        let inventory = crate::scan_directory(&folder).unwrap();
        let (mesh, report) = read_static_cdf_assembly(&cdf, &inventory).unwrap();
        assert_eq!(
            report.original_parts,
            ["torso", "head", "palps", "basher_l", "basher_r"]
        );
        assert_eq!(report.source_parts.len(), 5);
        assert_eq!(report.geometry_parts.len(), 5);
        assert_eq!(mesh.indices.len(), 15);
        assert!(mesh.uv0.is_some());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("Verified bind-pose"))
        );
    }

    #[test]
    fn historical_parasitehost_cdf_loads_render_chr_from_characters_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("characters");
        let folder = root.join("monsters/ParasiteHost");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("ParasiteHost3.chr"),
            crate::model::tests::fixture(),
        )
        .unwrap();
        let cdf = folder.join("ParasiteHost3.cdf");
        fs::write(
            &cdf,
            br#"<CharacterDefinition>
            <Model File="objects/characters/monsters/parasitehost/parasitehost3_skeleton.chr"/>
            <AttachmentList>
                <Attachment AName="Mesh" Type="CA_SKIN"
                    Binding="objects/characters/monsters/parasitehost/parasitehost3.chr"/>
            </AttachmentList>
            </CharacterDefinition>"#,
        )
        .unwrap();
        let inventory = crate::scan_directory(&root).unwrap();
        let (mesh, report) = read_static_cdf_assembly(&cdf, &inventory).unwrap();
        assert_eq!(report.original_parts, ["Mesh"]);
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(report.geometry_parts.len(), 1);
        assert!(
            report.geometry_parts[0]
                .source
                .ends_with("ParasiteHost3.chr")
        );
    }

    #[test]
    fn original_kala_banshee_selection_hides_red_armor_reducer_and_other_weapons() {
        // Synthetic fixture with the names and exclusive SkinSets from
        // merc_tech.7z/tech_3p.cdf; no game asset bytes are committed.
        let source = br#"<CharacterDefinition>
            <Model File="characters/hunters/merc_tech/tech_3p_skeleton.chr"/>
            <AttachmentList>
                <Attachment Type="CA_SKIN" AName="body"
                    Binding="characters/hunters/merc_tech/tech_3p.skin"/>
                <Attachment Type="CA_SKIN" AName="armorreducer"
                    Binding="characters/hunters/merc_tech/techarmorreducer_3p.skin"/>
                <Attachment Type="CA_SKIN" AName="bansheegun"
                    Binding="characters/hunters/merc_tech/techbansheegun_3p.skin"/>
                <Attachment Type="CA_SKIN" AName="pistoltech"
                    Binding="characters/hunters/merc_tech/techpistol_3p.skin"/>
                <Attachment Type="CA_SKIN" AName="riftdevice"
                    Binding="characters/hunters/merc_tech/techriftdevice_3p.skin"/>
                <Attachment Type="CA_SKIN" AName="riftdevicearm"
                    Binding="characters/hunters/merc_tech/techriftdevice_3p_arm.skin"/>
            </AttachmentList>
            <SkinSets>
                <SkinSet name="HiddenWhenBansheeGunSelected">
                    <skin name="armorreducer"/><skin name="pistoltech"/>
                    <skin name="riftdevice"/><skin name="riftdevicearm"/>
                </SkinSet>
                <SkinSet name="HiddenWhenRiftDeviceSelected">
                    <skin name="armorreducer"/><skin name="bansheegun"/>
                    <skin name="pistoltech"/>
                </SkinSet>
                <SkinSet name="HiddenWhenArmorReducerSelected">
                    <skin name="bansheegun"/><skin name="pistoltech"/>
                    <skin name="riftdevice"/><skin name="riftdevicearm"/>
                </SkinSet>
            </SkinSets>
        </CharacterDefinition>"#;
        let cdf = crate::cdf::parse_cdf(source).unwrap();
        let (selected_set, rendered) = select_attachments(&cdf).unwrap();
        assert_eq!(
            selected_set.as_deref(),
            Some("hiddenwhenbansheegunselected")
        );
        assert_eq!(rendered, ["body", "bansheegun"]);
        assert!(is_original_kala_3p(&cdf));
        let (red_set, red_rendered) =
            select_attachments_with_kala_state(&cdf, KalaTubingState::ArmorReducer).unwrap();
        assert_eq!(red_set.as_deref(), Some("hiddenwhenarmorreducerselected"));
        assert_eq!(red_rendered, ["body", "armorreducer"]);

        // Don't impose Kala's exclusive-arm defaults on unrelated CDFs.
        let mut unrelated = cdf.clone();
        unrelated.skeleton = "objects/characters/monsters/scorpid_2/scorpid1.chr".into();
        let (set, rendered) = select_attachments(&unrelated).unwrap();
        assert!(set.is_none());
        assert!(rendered.contains(&"armorreducer"));
        assert!(rendered.contains(&"bansheegun"));
    }

    #[test]
    fn assembly_matches_skin_set_names_to_attachment_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("characters")).unwrap();
        fs::write(root.join("characters/rig.chr"), b"indexed skeleton").unwrap();
        fs::write(
            root.join("characters/body.skin"),
            crate::model::tests::fixture(),
        )
        .unwrap();
        let cdf = root.join("character.cdf");
        fs::write(&cdf, br#"<CharacterDefinition><Model File="characters/rig.chr"/>
            <AttachmentList><Attachment Type="CA_SKIN" AName="body" Binding="characters/body.skin"/>
            <Attachment Type="CA_SKIN" AName="alternate" Binding="characters/missing.skin"/></AttachmentList>
            <SkinSets><SkinSet name="default"><skin name="body"/></SkinSet>
            <SkinSet name="alternate"><skin name="alternate"/></SkinSet></SkinSets></CharacterDefinition>"#).unwrap();
        let inventory = crate::scan_directory(root).unwrap();
        let (mesh, report) = read_static_cdf_assembly(&cdf, &inventory).unwrap();
        assert_eq!(report.original_parts, ["body"]);
        assert_eq!(report.selected_skin_set.as_deref(), Some("default"));
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert_eq!(mesh.uv0.unwrap()[2], [0., 1.]);
        assert!(!report.skeletal_animation_enabled);
    }

    #[test]
    fn static_cdf_loads_exact_skinm_without_separate_skin_or_skeleton() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("characters")).unwrap();
        fs::write(
            root.join("characters/armor.skinm"),
            crate::model::tests::fixture(),
        )
        .unwrap();
        let cdf = root.join("armor.cdf");
        fs::write(
            &cdf,
            br#"<CharacterDefinition>
            <Model File="characters/missing_skeleton.chr"/>
            <AttachmentList><Attachment Type="CA_SKIN" AName="armor"
                Binding="characters/armor.skin"/></AttachmentList>
            </CharacterDefinition>"#,
        )
        .unwrap();
        let inv = crate::scan_directory(root).unwrap();
        let (mesh, report) = read_static_cdf_assembly(&cdf, &inv).unwrap();
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(report.original_parts, ["armor"]);
        assert_eq!(report.source_parts[0].material_slot, 0);
        assert_eq!(report.verified_bones, None);
        assert!(report.skeleton_source.is_none());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("static skin geometry"))
        );
        assert!(report.warnings.iter().any(|w| w.contains("binding")));
    }

    #[test]
    fn repeated_authored_bindings_remain_separate_assembly_instances() {
        // An authored CDF may reuse an exact compiled mesh in two differently
        // named attachment slots. Deduplicating by Binding silently removed
        // the second instance and its original material/geometry spans.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("characters")).unwrap();
        fs::write(
            root.join("characters/armor.skinm"),
            crate::model::tests::fixture(),
        )
        .unwrap();
        let cdf = root.join("both.cdf");
        fs::write(
            &cdf,
            br#"<CharacterDefinition><Model File="characters/rig.chr"/>
            <AttachmentList>
                <Attachment Type="CA_SKIN" AName="left_armor" Binding="characters/armor.skin"/>
                <Attachment Type="CA_SKIN" AName="right_armor" Binding="characters/armor.skin"/>
            </AttachmentList></CharacterDefinition>"#,
        )
        .unwrap();
        let inventory = crate::scan_directory(root).unwrap();
        let (mesh, report) = read_static_cdf_assembly(&cdf, &inventory).unwrap();
        assert_eq!(report.original_parts, ["left_armor", "right_armor"]);
        assert_eq!(mesh.indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(report.geometry_parts.len(), 2);
        assert_eq!(report.source_parts.len(), 2);
        assert_eq!(report.source_parts[0].first_index, 0);
        assert_eq!(report.source_parts[1].first_index, 3);
        assert_eq!(
            report.source_parts[0].binding,
            report.source_parts[1].binding
        );
    }

    #[test]
    fn rigid_equipment_uses_authored_socket_transform_not_world_origin() {
        let identity = [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ];
        let mut socket = identity;
        socket[0][3] = 10.;
        socket[1][3] = 20.;
        socket[2][3] = 30.;
        let skeleton = CompiledSkeleton {
            controller_ids: vec![17],
            names: vec!["RightHand".into()],
            parents: vec![None],
            world: vec![socket],
            inverse: vec![identity],
        };
        let attachment = crate::cdf::CdfAttachment {
            name: "rifle".into(),
            binding: "characters/rifle.cgf".into(),
            kind: AttachmentKind::Bone,
            bone_name: Some("RightHand".into()),
            material: None,
            position: Some([1., 2., 3.]),
            rotation: Some([1., 0., 0., 0.]),
        };
        let mut rifle = part(3, true);
        rifle.positions[0] = [1., 0., 0.];
        transform_equipment(&mut rifle, &attachment, &skeleton).unwrap();
        assert_eq!(rifle.positions[0], [12., 22., 33.]);
        assert_eq!(rifle.normals.as_ref().unwrap()[0], [0., 0., 1.]);
        let mut wrong = attachment;
        wrong.bone_name = Some("OtherWeaponSocket".into());
        assert!(transform_equipment(&mut rifle, &wrong, &skeleton).is_err());
    }

    fn part(count: usize, uv: bool) -> RenderMesh {
        RenderMesh {
            info: ModelInfo {
                format: "test".into(),
                version: 7,
                chunk_count: 1,
                vertex_count: count,
                triangle_count: 1,
                original_uv0: uv,
                original_normals: true,
            },
            positions: vec![[0.0, 0.0, 0.0]; count],
            indices: vec![0, 1, 2],
            uv0: uv.then(|| vec![[0.5, 0.5]; count]),
            normals: Some(vec![[0.0, 0.0, 1.0]; count]),
        }
    }

    #[test]
    fn preserves_triangle_offsets_and_unmodified_uv0() {
        let (mesh, names) = combine_source_meshes(vec![
            ("body".into(), part(3, true)),
            ("equipment".into(), part(3, true)),
        ])
        .unwrap();
        assert_eq!(mesh.indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(mesh.uv0.as_ref().unwrap().len(), 6);
        assert_eq!(mesh.info.triangle_count, 2);
        assert_eq!(names, ["body", "equipment"]);
    }

    #[test]
    fn missing_uv_cannot_be_reconstructed_from_other_parts() {
        let (mesh, _) = combine_source_meshes(vec![
            ("body".into(), part(3, true)),
            ("equipment".into(), part(3, false)),
        ])
        .unwrap();
        assert!(mesh.uv0.is_none());
        assert!(mesh.normals.is_some());
    }
}
