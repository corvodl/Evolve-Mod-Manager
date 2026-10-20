//! Exact, static CDF character-assembly preview.
//! Original CDF skin meshes remain in source skeleton space. CA_BONE rigid
//! props can be positioned only using verified authored sockets and bind pose.
//! Playback and export still require separately validated skinning.
use crate::cdf::{AssetResolution, AttachmentKind, CdfDefinition, CdfError, read_cdf, resolve_virtual};
use crate::library_catalog::{LibraryReferenceStatus, resolve_imported_reference};
use crate::rig::{CompiledSkeleton, parse_compiled_skeleton, remap_verified_weights};
use crate::inventory::Inventory;
use crate::model::{ModelError, ModelInfo, RenderMesh, read_model};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    #[serde(default)]
    pub verified_bones: Option<usize>,
    #[serde(default)]
    pub warnings: Vec<String>,
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

pub fn select_attachments(
    cdf: &CdfDefinition,
) -> Result<(Option<String>, Vec<&str>), AssemblyError> {
    let chosen = if cdf.skin_sets.is_empty() {
        None
    } else if cdf.skin_sets.contains_key("default") {
        Some("default".to_owned())
    } else if cdf.skin_sets.len() == 1 {
        cdf.skin_sets.keys().next().cloned()
    } else {
        return Err(AssemblyError::Incomplete(
            "Multiple named skin variants and no default: choose a variant explicitly".into(),
        ));
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
                keys.contains(&normalized)
                    || !cdf.skin_sets.values().any(|set| set.contains(&normalized))
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
        AssetResolution::Ambiguous(_) => return Err(AssemblyError::Incomplete(
            format!("Ambiguous local asset: {authored}"))),
        AssetResolution::Missing => {
            let Some(imported) = imported else { return Ok(None) };
            let imported_root = fs::canonicalize(imported)?;
            match resolve_imported_reference(imported, authored, build)
                .map_err(|e| AssemblyError::Incomplete(e.to_string()))? {
                LibraryReferenceStatus::Found(hit) => {
                    (std::path::PathBuf::from(hit.absolute_path), imported_root)
                }
                LibraryReferenceStatus::Missing => return Ok(None),
                LibraryReferenceStatus::Ambiguous(_) => return Err(AssemblyError::Incomplete(
                    format!("Ambiguous imported asset: {authored}"))),
            }
        }
    };
    let metadata = fs::symlink_metadata(&location)?;
    let canonical = fs::canonicalize(&location)?;
    if !metadata.file_type().is_file() || !canonical.starts_with(allowed_root) {
        return Err(AssemblyError::Incomplete(
            format!("Source is no longer a safe regular asset: {authored}")));
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
        AssemblyError::Incomplete(format!("Equipment {} has no authored BoneName", attachment.name))
    })?;
    let matches = skeleton.names.iter().enumerate()
        .filter(|(_, name)| name.eq_ignore_ascii_case(bone))
        .map(|(i, _)| i).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(AssemblyError::Incomplete(format!(
            "Equipment {} references absent/ambiguous compiled bone: {bone}", attachment.name)));
    }
    let matrix = &skeleton.world[matches[0]];
    let [w,x,y,z] = attachment.rotation.unwrap_or([1.,0.,0.,0.]);
    let norm = w*w + x*x + y*y + z*z;
    if !norm.is_finite() || (norm - 1.).abs() > 0.1 {
        return Err(AssemblyError::Incomplete(format!(
            "Equipment {} has invalid authored quaternion", attachment.name)));
    }
    let f = 1. / norm;
    let (w,x,y,z) = (w*f.sqrt(), x*f.sqrt(), y*f.sqrt(), z*f.sqrt());
    let rotation = [
        [1.-2.*(y*y+z*z), 2.*(x*y-z*w), 2.*(x*z+y*w)],
        [2.*(x*y+z*w), 1.-2.*(x*x+z*z), 2.*(y*z-x*w)],
        [2.*(x*z-y*w), 2.*(y*z+x*w), 1.-2.*(x*x+y*y)],
    ];
    let offset = attachment.position.unwrap_or([0.,0.,0.]);
    let rotate = |p: [f32;3]| -> [f32;3] {
        std::array::from_fn(|i| (0..3).map(|j| rotation[i][j]*p[j]).sum::<f32>())
    };
    for point in &mut mesh.positions {
        let local = rotate(*point);
        let transformed: [f32;3] = std::array::from_fn(|i|
            (0..3).map(|j| matrix[i][j]*(local[j]+offset[j])).sum::<f32>() + matrix[i][3]);
        if transformed.iter().any(|c| !c.is_finite() || c.abs() > 1e7) {
            return Err(AssemblyError::Incomplete("Equipment transform out of range".into()));
        }
        *point = transformed;
    }
    if let Some(normals) = &mut mesh.normals {
        for normal in normals {
            let rotated = rotate(*normal);
            let transformed: [f32;3] = std::array::from_fn(|i|
                (0..3).map(|j| matrix[i][j]*rotated[j]).sum::<f32>());
            let len = transformed.iter().map(|c| c*c).sum::<f32>().sqrt();
            if !len.is_finite() || len < 1e-6 {
                return Err(AssemblyError::Incomplete("Invalid equipment normal matrix".into()));
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
    let root = fs::canonicalize(&library.root)?;
    let cdf_metadata = fs::symlink_metadata(cdf_file)?;
    let cdf_location = fs::canonicalize(cdf_file)?;
    if !cdf_metadata.file_type().is_file() || !cdf_location.starts_with(&root) {
        return Err(AssemblyError::Incomplete(
            "CDF file must be a real indexed file inside the selected asset root".into()));
    }
    let definition = read_cdf(&cdf_location)?;
    let (skin_set, selected) = select_attachments(&definition)?;
    let skeleton_path = preview_source(library, &definition.skeleton, imported, build)?
        .ok_or_else(|| AssemblyError::Incomplete(
            format!("CDF skeleton absent from imported sources: {}", definition.skeleton)))?;
    let skeleton = fs::read(&skeleton_path).ok()
        .and_then(|bytes| parse_compiled_skeleton(&bytes).ok());
    let mut notes = Vec::new();
    if skeleton.is_none() {
        notes.push("Compiled bind-pose skeleton cannot be verified; rigid sockets and animation disabled".into());
    }
    let mut names_seen = HashSet::new();
    let mut meshes = Vec::new();
    let mut source_parts = Vec::new();
    let mut index_cursor: u32 = 0;
    for attachment in &definition.attachments {
        if !selected.contains(&attachment.name.as_str()) {
            continue;
        }
        let bound = preview_source(library, &attachment.binding, imported, build)?
            .ok_or_else(|| AssemblyError::Incomplete(
                format!("Missing authored CDF binding {}: {}",attachment.name,attachment.binding)))?;
        // A compiled render companion is preferred, but the author-declared
        // original may itself contain geometry (notably older .chr meshes).
        let rendered = match compiled_companion(&attachment.binding) {
            Some(companion) => preview_source(library, &companion, imported, build)?.unwrap_or(bound),
            None => bound,
        };
        if !names_seen.insert(attachment.binding.to_ascii_lowercase()) {
            return Err(AssemblyError::Incomplete(format!(
                "Repeated CDF binding without a distinct authored transform: {}",attachment.binding)));
        }
        let mut mesh = read_model(&rendered)?;
        if matches!(attachment.kind, AttachmentKind::Bone) {
            let verified = skeleton.as_ref().ok_or_else(|| AssemblyError::Incomplete(
                format!("Cannot place rigid prop {} without a verified bind skeleton",attachment.name)))?;
            transform_equipment(&mut mesh, attachment, verified)?;
        }
        let verified_skin_weights = if matches!(attachment.kind, AttachmentKind::Skin | AttachmentKind::UniqueSkin) {
            if let Some(skeleton) = &skeleton {
                fs::read(&rendered).ok()
                    .and_then(|raw| remap_verified_weights(&raw, mesh.positions.len(), skeleton.names.len()).ok())
                    .is_some()
            } else { false }
        } else { false };
        if !verified_skin_weights && !matches!(attachment.kind, AttachmentKind::Bone) {
            notes.push(format!("{}: original weights not yet verified against selected skeleton",attachment.name));
        }
        let count = u32::try_from(mesh.indices.len())
            .map_err(|_| AssemblyError::Incomplete("CDF part exceeds index limits".into()))?;
        let ranges = crate::model::original_mesh_subsets(
            &fs::read(&rendered)?, mesh.indices.len())?;
        for (first, indices, material_slot) in ranges {
            source_parts.push(AssemblyPart {
                name: attachment.name.clone(),
                binding: attachment.binding.clone(),
                first_index: index_cursor.checked_add(first)
                    .ok_or_else(|| AssemblyError::Incomplete("Material index offset overflow".into()))?,
                index_count: indices,
                material_slot,
                material_hint: attachment.material.clone(),
                verified_skin_weights,
            });
        }
        index_cursor = index_cursor.checked_add(count)
            .ok_or_else(|| AssemblyError::Incomplete("Assembly index ranges overflow".into()))?;
        meshes.push((attachment.name.clone(), mesh));
    }
    if meshes.len() != selected.len() {
        return Err(AssemblyError::Incomplete("CDF assembly selection was incomplete".into()));
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
        verified_bones: skeleton.map(|s| s.names.len()),
        warnings: notes,
    };
    Ok((mesh, report))
}

#[cfg(test)]
mod tests {
    use super::*;

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
