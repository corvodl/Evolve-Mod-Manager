//! Minimal, bounded glTF 2.0 binary mesh writer for validated CryEngine source
//! triangles. Raw UV0 and normals are passed through unchanged. This geometry
//! bridge intentionally does NOT manufacture skin weights, materials, skeletons
//! or animation Actions.
use crate::model::RenderMesh;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const MAX_EXPORT_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GlbError {
    #[error("Missing or invalid geometry attributes: {0}")]
    Invalid(String),
    #[error("GLB geometry exceeds safe export limit")]
    Limits,
    #[error("Output file exists")]
    OutputExists,
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("glTF JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn align4(data: &mut Vec<u8>) {
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
}

/// Construct complete glTF 2.0 bytes suitable for Blender import.
/// It contains only original source vertices/triangles and exact UV0 when
/// the corresponding parser has independently verified its layout.
pub fn geometry_glb_bytes(mesh: &RenderMesh) -> Result<Vec<u8>, GlbError> {
    let count = mesh.positions.len();
    if !(3..=250000).contains(&count)
        || mesh.indices.len() < 3
        || mesh.indices.len() > 1_500_000
        || !mesh.indices.len().is_multiple_of(3)
        || mesh.indices.iter().any(|idx| *idx as usize >= count)
    {
        return Err(GlbError::Invalid("invalid vertex/index stream".into()));
    }
    if mesh
        .positions
        .iter()
        .any(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e8))
    {
        return Err(GlbError::Invalid("invalid world coordinates".into()));
    }
    if mesh.uv0.as_ref().is_some_and(|data| {
        data.len() != count || data.iter().any(|uv| uv.iter().any(|f| !f.is_finite()))
    }) {
        return Err(GlbError::Invalid("original UV0 mismatch".into()));
    }
    if mesh.normals.as_ref().is_some_and(|data| {
        data.len() != count || data.iter().any(|n| n.iter().any(|f| !f.is_finite()))
    }) {
        return Err(GlbError::Invalid("source normal array mismatch".into()));
    }
    let min = (0..3)
        .map(|i| {
            mesh.positions
                .iter()
                .map(|p| p[i])
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let max = (0..3)
        .map(|i| {
            mesh.positions
                .iter()
                .map(|p| p[i])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect::<Vec<_>>();
    let mut binary = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();

    let start = binary.len();
    for point in &mesh.positions {
        for component in point {
            binary.extend_from_slice(&component.to_le_bytes());
        }
    }
    views.push(
        json!({"buffer":0,"byteOffset":start,"byteLength":binary.len()-start,"target":34962}),
    );
    accessors.push(json!({"bufferView":views.len()-1,"componentType":5126,"count":count,"type":"VEC3","min":min,"max":max}));

    let mut attrs = json!({"POSITION":0});
    if let Some(normals) = &mesh.normals {
        let start = binary.len();
        for norm in normals {
            for component in norm {
                binary.extend_from_slice(&component.to_le_bytes());
            }
        }
        views.push(
            json!({"buffer":0,"byteOffset":start,"byteLength":binary.len()-start,"target":34962}),
        );
        accessors.push(
            json!({"bufferView":views.len()-1,"componentType":5126,"count":count,"type":"VEC3"}),
        );
        attrs["NORMAL"] = json!(accessors.len() - 1);
    }
    if let Some(uv) = &mesh.uv0 {
        let start = binary.len();
        for vec in uv {
            for component in vec {
                binary.extend_from_slice(&component.to_le_bytes());
            }
        }
        views.push(
            json!({"buffer":0,"byteOffset":start,"byteLength":binary.len()-start,"target":34962}),
        );
        accessors.push(
            json!({"bufferView":views.len()-1,"componentType":5126,"count":count,"type":"VEC2"}),
        );
        attrs["TEXCOORD_0"] = json!(accessors.len() - 1);
    }
    let start = binary.len();
    for index in &mesh.indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    views.push(
        json!({"buffer":0,"byteOffset":start,"byteLength":binary.len()-start,"target":34963}),
    );
    accessors.push(json!({"bufferView":views.len()-1,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}));
    align4(&mut binary);
    let mut document = serde_json::to_vec(&json!({
        "asset":{"version":"2.0","generator":"Evolve Mod Manager Rust geometric source export"},
        "scene":0,
        "scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"name":"source_geometry"}],
        "meshes":[{"primitives":[{"attributes":attrs,"indices":accessors.len()-1,"mode":4}]}],
        "buffers":[{"byteLength":binary.len()}],
        "bufferViews":views,
        "accessors":accessors
    }))?;
    while !document.len().is_multiple_of(4) {
        document.push(b' ');
    }
    let total = 12usize
        .checked_add(8)
        .and_then(|v| v.checked_add(document.len()))
        .and_then(|v| v.checked_add(8))
        .and_then(|v| v.checked_add(binary.len()))
        .ok_or(GlbError::Limits)?;
    if total > MAX_EXPORT_BYTES || total > u32::MAX as usize {
        return Err(GlbError::Limits);
    }
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(document.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&document);
    output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&binary);
    Ok(output)
}

/// Retain verified original CryEngine subset IDs on a standalone static model.
/// Unlike a generic geometry GLB, this creates one primitive for each authored
/// triangle range and a material table with holes retained (e.g. source ID 10).
/// Missing/unsupported source tables do not create fabricated nonzero IDs.
pub fn geometry_glb_bytes_with_original_material_ids(
    mesh: &RenderMesh,
    source_bytes: &[u8],
) -> Result<Vec<u8>, GlbError> {
    let ranges = crate::model::original_mesh_subsets(source_bytes, mesh.indices.len())
        .map_err(|e| GlbError::Invalid(format!("Cannot resolve original material IDs: {e}")))?;
    if ranges.is_empty()
        || ranges.iter().any(|&(_, _, id)| id == 255 || id >= 128)
    {
        return Err(GlbError::Invalid(
            "Original source material range is unresolved".into(),
        ));
    }
    let data = geometry_glb_bytes(mesh)?;
    let json_len = u32::from_le_bytes(
        data[12..16].try_into().map_err(|_| GlbError::Limits)?,
    ) as usize;
    let json_end = 20usize.checked_add(json_len).ok_or(GlbError::Limits)?;
    let mut document: serde_json::Value =
        serde_json::from_slice(data.get(20..json_end).ok_or(GlbError::Limits)?)?;
    let bin = data.get(json_end + 8..).ok_or(GlbError::Limits)?;
    let original = document["meshes"][0]["primitives"][0].clone();
    let accessor = original["indices"]
        .as_u64()
        .ok_or_else(|| GlbError::Invalid("Standalone model has no index buffer".into()))?
        as usize;
    let view = document["accessors"][accessor]["bufferView"]
        .as_u64()
        .ok_or(GlbError::Limits)?;
    if document["accessors"][accessor]["componentType"] != 5125 {
        return Err(GlbError::Invalid("Original indices are not uint32".into()));
    }
    let max_id = ranges.iter().map(|&(_, _, id)| id).max().unwrap_or(0) as usize;
    let mut materials = Vec::with_capacity(max_id + 1);
    for id in 0..=max_id {
        materials.push(json!({
            "name":format!("CryEngine slot {id:03} - Original material ID {id}"),
            "doubleSided":true,
            "pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}
        }));
    }
    let mut primitives = Vec::with_capacity(ranges.len());
    let mut cursor = 0u32;
    for (first, count, material_id) in ranges {
        let end = first.checked_add(count).ok_or(GlbError::Limits)?;
        if first != cursor || count == 0 || !count.is_multiple_of(3)
            || end as usize > mesh.indices.len()
        {
            return Err(GlbError::Invalid(
                "Noncontiguous original subset material ranges".into(),
            ));
        }
        let mut primitive = original.clone();
        let accessors = document["accessors"].as_array_mut().ok_or(GlbError::Limits)?;
        let offset = accessors.len();
        accessors.push(json!({
            "bufferView":view, "byteOffset":u64::from(first)*4,
            "componentType":5125, "count":count, "type":"SCALAR"
        }));
        primitive["indices"] = json!(offset);
        primitive["material"] = json!(material_id);
        primitives.push(primitive);
        cursor = end;
    }
    if cursor as usize != mesh.indices.len() {
        return Err(GlbError::Invalid(
            "Source material IDs do not cover the entire mesh".into(),
        ));
    }
    document["meshes"][0]["primitives"] = json!(primitives);
    document["materials"] = json!(materials);
    let mut json_bytes = serde_json::to_vec(&document)?;
    align4(&mut json_bytes);
    // glTF JSON chunks are padded with spaces, not zero bytes.
    for byte in json_bytes.iter_mut().rev() {
        if *byte == 0 {
            *byte = b' ';
        } else {
            break;
        }
    }
    let size = 28usize
        .checked_add(json_bytes.len())
        .and_then(|n| n.checked_add(bin.len()))
        .ok_or(GlbError::Limits)?;
    if size > MAX_EXPORT_BYTES || size > u32::MAX as usize {
        return Err(GlbError::Limits);
    }
    let mut result = Vec::with_capacity(size);
    result.extend_from_slice(b"glTF");
    result.extend_from_slice(&2u32.to_le_bytes());
    result.extend_from_slice(&(size as u32).to_le_bytes());
    result.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    result.extend_from_slice(b"JSON");
    result.extend_from_slice(&json_bytes);
    result.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    result.extend_from_slice(b"BIN\\0");
    result.extend_from_slice(bin);
    Ok(result)
}

pub fn export_model_glb(mesh: &RenderMesh, destination: &Path) -> Result<usize, GlbError> {
    let data = geometry_glb_bytes(mesh)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                GlbError::OutputExists
            } else {
                GlbError::Io(e)
            }
        })?;
    if let Err(e) = file.write_all(&data).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(destination);
        return Err(GlbError::Io(e));
    }
    Ok(data.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelInfo;
    fn mesh() -> RenderMesh {
        RenderMesh {
            info: ModelInfo {
                format: "test".into(),
                version: 7,
                chunk_count: 0,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: false,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: None,
        }
    }
    #[test]
    fn builds_complete_gltf_binary_with_source_uvs() {
        let glb = geometry_glb_bytes(&mesh()).unwrap();
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
        let size = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + size]).unwrap();
        assert_eq!(doc["accessors"][2]["type"], "SCALAR");
        assert!(doc["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"].is_number());
        assert_eq!(&glb[20 + size + 4..20 + size + 8], b"BIN\0");
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("body.glb");
        export_model_glb(&mesh(), &out).unwrap();
        assert!(matches!(
            export_model_glb(&mesh(), &out),
            Err(GlbError::OutputExists)
        ));
    }
    #[test]
    fn rejects_fabricated_index_or_invalid_original_uv() {
        let mut bad = mesh();
        bad.indices[2] = 4;
        assert!(geometry_glb_bytes(&bad).is_err());
        bad.indices[2] = 2;
        bad.uv0.as_mut().unwrap().pop();
        assert!(geometry_glb_bytes(&bad).is_err());
    }
}
