//! Exact, static CDF character-assembly preview.
//! Meshes keep their source-space coordinates; no attachment transforms,
//! bone animation, skin weights or material slots are invented. Never call
//! this a rigged or animated full-character export.
use crate::cdf::{AssetResolution, CdfDefinition, CdfError, read_cdf, resolve_cdf};
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

/// Full attachment index is checked with the same exact resolver as the
/// inspector; not even one unresolved/ambiguous part is silently omitted.
pub fn read_static_cdf_assembly(
    cdf_file: &Path,
    library: &Inventory,
) -> Result<(RenderMesh, AssemblyReport), AssemblyError> {
    let root = fs::canonicalize(Path::new(&library.root))?;
    let cdf_metadata = fs::symlink_metadata(cdf_file)?;
    let cdf_location = fs::canonicalize(cdf_file)?;
    if !cdf_metadata.file_type().is_file() || !cdf_location.starts_with(&root) {
        return Err(AssemblyError::Incomplete(
            "CDF file must be a real indexed file inside the selected asset root".into(),
        ));
    }
    let definition = read_cdf(&cdf_location)?;
    let (skin_set, selected) = select_attachments(&definition)?;
    let dependencies = resolve_cdf(library, &definition);
    if !matches!(dependencies.skeleton, AssetResolution::Found(_)) {
        return Err(AssemblyError::Incomplete(
            "CDF skeleton missing or ambiguous".into(),
        ));
    }
    let mut names_seen = HashSet::new();
    let mut meshes = Vec::new();
    for (attachment, (_, resolution)) in
        definition.attachments.iter().zip(&dependencies.attachments)
    {
        let part = &attachment.name;
        if !selected.contains(&part.as_str()) {
            continue;
        }
        let AssetResolution::Found(relative) = resolution else {
            return Err(AssemblyError::Incomplete(format!(
                "Missing/ambiguous CDF part {part}"
            )));
        };
        if !names_seen.insert(relative.to_ascii_lowercase()) {
            return Err(AssemblyError::Incomplete(format!(
                "Repeated mesh attachment without verified transform: {relative}"
            )));
        }
        let candidate = root.join(relative);
        let meta = fs::symlink_metadata(&candidate)?;
        let resolved = fs::canonicalize(&candidate)?;
        if !meta.file_type().is_file() || !resolved.starts_with(&root) {
            return Err(AssemblyError::Incomplete(format!(
                "Unsafe attachment path: {relative}"
            )));
        }
        meshes.push((part.clone(), read_model(&resolved)?));
    }
    if meshes.len() != selected.len() {
        return Err(AssemblyError::Incomplete(
            "Not all CDF attachments resolved uniquely".into(),
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
