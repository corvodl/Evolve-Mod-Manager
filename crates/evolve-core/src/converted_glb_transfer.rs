//! Transfer converter-owned CAF/DBA clip tracks only onto an independently
//! verified source-rigged glTF model. Mirrors the private manager's strict
//! bone-name, parent and rest-transform checks; never retarget by filename.
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const MAX_GLB: usize = 192 * 1024 * 1024;
const MAX_JOINTS: usize = 1024;
const MAX_ANIMATIONS: usize = 512;

#[derive(Clone, Debug)]
pub struct AttachedConvertedMotion {
    pub glb: Vec<u8>,
    pub matching_joints: usize,
    pub clip_names: Vec<String>,
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let value = bytes.get(offset..offset + 4).ok_or("Truncated glTF length")?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| "Invalid glTF length")?) as usize)
}
fn unpack(bytes: &[u8]) -> Result<(Value, Vec<u8>), String> {
    if bytes.len() < 28
        || bytes.len() > MAX_GLB
        || bytes.get(..4) != Some(b"glTF")
        || u32_at(bytes, 4)? != 2
        || u32_at(bytes, 8)? != bytes.len()
        || bytes.get(16..20) != Some(b"JSON")
    {
        return Err("Invalid or oversized source GLB".into());
    }
    let json_size = u32_at(bytes, 12)?;
    let json_end = 20usize.checked_add(json_size).ok_or("glTF length overflow")?;
    if json_size > 12 * 1024 * 1024
        || json_end.checked_add(8).is_none_or(|end| end > bytes.len())
    {
        return Err("Malformed glTF JSON chunk".into());
    }
    let doc: Value = serde_json::from_slice(&bytes[20..json_end])
        .map_err(|error| error.to_string())?;
    let bin_size = u32_at(bytes, json_end)?;
    if bytes.get(json_end + 4..json_end + 8) != Some(b"BIN\0")
        || json_end.checked_add(8).and_then(|n| n.checked_add(bin_size)) != Some(bytes.len())
    {
        return Err("Malformed glTF binary chunk".into());
    }
    Ok((doc, bytes[json_end + 8..].to_vec()))
}
fn pack(mut doc: Value, mut binary: Vec<u8>) -> Result<Vec<u8>, String> {
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    if !doc["buffers"].is_array() || doc["buffers"].as_array().is_none_or(|v| v.len() != 1) {
        return Err("Source GLB must use one embedded buffer".into());
    }
    doc["buffers"][0]["byteLength"] = json!(binary.len());
    let mut json_bytes = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let total = json_bytes
        .len()
        .checked_add(binary.len())
        .and_then(|length| length.checked_add(28))
        .ok_or("GLB size overflow")?;
    if total > MAX_GLB {
        return Err("Transferred glTF exceeds supported size".into());
    }
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json_bytes);
    output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&binary);
    Ok(output)
}

fn array<'a>(doc: &'a Value, name: &str) -> Result<&'a [Value], String> {
    doc[name]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("Missing {name} data"))
}
fn node_index(value: &Value) -> Result<usize, String> {
    value.as_u64().and_then(|u| usize::try_from(u).ok()).ok_or("Invalid glTF node index".into())
}
fn joints(doc: &Value) -> Result<HashMap<String, usize>, String> {
    let skins = array(doc, "skins")?;
    if skins.len() != 1 {
        return Err("Transfer requires exactly one source skin".into());
    }
    let nodes = array(doc, "nodes")?;
    if nodes.len() > 4096 {
        return Err("Too many glTF source nodes".into());
    }
    let bone_ids = skins[0]["joints"].as_array().ok_or("Missing skin joints")?;
    if bone_ids.is_empty() || bone_ids.len() > MAX_JOINTS {
        return Err("Unsupported glTF joint count".into());
    }
    let mut by_name = HashMap::new();
    for id in bone_ids {
        let index = node_index(id)?;
        let node = nodes.get(index).ok_or("Bone node outside source graph")?;
        let name = node["name"].as_str().unwrap_or("").trim().to_lowercase();
        if name.is_empty() || by_name.insert(name, index).is_some() {
            return Err("Missing or duplicate original bone name".into());
        }
    }
    Ok(by_name)
}
fn parent_table(doc: &Value) -> Result<HashMap<usize, usize>, String> {
    let nodes = array(doc, "nodes")?;
    let mut parents = HashMap::new();
    for (parent, node) in nodes.iter().enumerate() {
        if let Some(children) = node["children"].as_array() {
            for child in children {
                let index = node_index(child)?;
                if index >= nodes.len() || parents.insert(index, parent).is_some() {
                    return Err("Ambiguous glTF source hierarchy".into());
                }
            }
        }
    }
    Ok(parents)
}
type Mat4 = [[f32; 4]; 4];
fn local(node: &Value) -> Result<Mat4, String> {
    if let Some(matrix) = node["matrix"].as_array() {
        if matrix.len() != 16 {
            return Err("Malformed glTF bone matrix".into());
        }
        let values = matrix.iter().map(|v| {
            let f = v.as_f64().ok_or("Non-numeric bone matrix")? as f32;
            if !f.is_finite() { return Err("Non-finite bone matrix".into()); }
            Ok(f)
        }).collect::<Result<Vec<_>, String>>()?;
        return Ok(std::array::from_fn(|r| std::array::from_fn(|c| values[c * 4 + r])));
    }
    fn values<const N: usize>(node: &Value, key: &str, default: [f32; N]) -> Result<[f32; N], String> {
        let Some(array) = node.get(key) else { return Ok(default); };
        let arr = array.as_array().filter(|v| v.len() == N).ok_or("Malformed joint TRS")?;
        let mut out = [0f32; N];
        for (index, value) in arr.iter().enumerate() {
            let number = value.as_f64().ok_or("Non-numeric joint TRS")? as f32;
            if !number.is_finite() { return Err("Non-finite joint TRS".into()); }
            out[index] = number;
        }
        Ok(out)
    }
    let t = values(node, "translation", [0., 0., 0.])?;
    let q = values(node, "rotation", [0., 0., 0., 1.])?;
    let s = values(node, "scale", [1., 1., 1.])?;
    let norm = q.iter().map(|v| v * v).sum::<f32>();
    if !norm.is_finite() || norm < 1e-8 {
        return Err("Invalid joint quaternion".into());
    }
    let factor = (2. / norm).sqrt();
    let (x, y, z, w) = (q[0] * factor, q[1] * factor, q[2] * factor, q[3] * factor);
    let mut matrix = [
        [1. - y * y - z * z, x * y - z * w, x * z + y * w, t[0]],
        [x * y + z * w, 1. - x * x - z * z, y * z - x * w, t[1]],
        [x * z - y * w, y * z + x * w, 1. - x * x - y * y, t[2]],
        [0., 0., 0., 1.],
    ];
    for row in matrix.iter_mut().take(3) {
        for (value, scale) in row.iter_mut().take(3).zip(s) {
            *value *= scale;
        }
    }
    Ok(matrix)
}
fn same_rest(a: &Value, b: &Value) -> Result<bool, String> {
    let a = local(a)?;
    let b = local(b)?;
    Ok(a.iter().flatten().zip(b.iter().flatten())
        .all(|(a, b)| (*a - *b).abs() <= 0.002 + 0.002 * b.abs()))
}
fn clip_key(value: &str) -> String {
    value.replace('\\', "/").trim().to_ascii_lowercase()
        .split('/').next_back().unwrap_or("").trim_end_matches(".caf").to_owned()
}

/// A source mesh and a converter output are not interchangeable. Reuse source
/// model/material/weight arrays and copy only explicitly verified controller
/// samples after strict parent/name/rest checks. No original game file writes.
pub fn attach_converted_motion_to_source_glb(
    source_glb: &[u8],
    converted_glb: &[u8],
    requested_clip: Option<&str>,
) -> Result<AttachedConvertedMotion, String> {
    let (mut original, mut source_bytes) = unpack(source_glb)?;
    let (converted, motion_bytes) = unpack(converted_glb)?;
    if array(&converted, "meshes")?.is_empty() {
        return Err("Converted motion contains no source rig".into());
    }
    let target = joints(&original)?;
    let source = joints(&converted)?;
    let original_nodes = array(&original, "nodes")?;
    let converted_nodes = array(&converted, "nodes")?;
    let original_parents = parent_table(&original)?;
    let converted_parents = parent_table(&converted)?;
    let original_joint_set = target.values().copied().collect::<HashSet<_>>();
    let source_joint_set = source.values().copied().collect::<HashSet<_>>();
    let parent_name = |nodes: &[Value], table: &HashMap<usize, usize>, index: usize, owned: &HashSet<usize>| {
        table.get(&index)
            .copied()
            .filter(|index| owned.contains(index))
            .and_then(|index| nodes[index]["name"].as_str())
            .map(str::to_lowercase)
    };
    let mut remap = HashMap::new();
    for (name, &index) in &source {
        let Some(&destination) = target.get(name) else { continue; };
        if parent_name(original_nodes, &original_parents, destination, &original_joint_set)
            != parent_name(converted_nodes, &converted_parents, index, &source_joint_set)
        {
            return Err(format!("Converted bone parent mismatch: {name}"));
        }
        if !same_rest(&original_nodes[destination], &converted_nodes[index])? {
            return Err(format!("Converted bone bind pose mismatch: {name}"));
        }
        remap.insert(index, destination);
    }
    if remap.is_empty() {
        return Err("Converted GLB has no exactly matched original bone".into());
    }
    let mut animations = array(&converted, "animations")?.to_vec();
    if animations.is_empty() || animations.len() > MAX_ANIMATIONS {
        return Err("No bounded converted animation clips".into());
    }
    if let Some(requested) = requested_clip {
        let goal = clip_key(requested);
        animations.retain(|animation| {
            animation["name"].as_str().is_some_and(|name| clip_key(name) == goal)
        });
        if animations.len() != 1 {
            return Err("Requested DBA clip does not identify one exact animation".into());
        }
    }
    let mut written_names = Vec::new();
    let mut keyed = Vec::<(usize, Value)>::new();
    for animation in &mut animations {
        let channels = animation["channels"].as_array_mut().ok_or("Missing clip channels")?;
        if channels.is_empty() {
            return Err("Converted clip contains no controllers".into());
        }
        for channel in channels {
            let target_node = node_index(&channel["target"]["node"])?;
            let destination = *remap
                .get(&target_node)
                .ok_or("Converted clip targets an unverified source joint")?;
            let path = channel["target"]["path"].as_str().ok_or("Invalid track path")?;
            if !matches!(path, "rotation" | "translation" | "scale")
                || converted_nodes[target_node]["matrix"].is_array()
            {
                return Err("Unsupported converted animation channel".into());
            }
            channel["target"]["node"] = json!(destination);
            keyed.push((destination, converted_nodes[target_node].clone()));
        }
        written_names.push(animation["name"].as_str().unwrap_or("Unnamed clip").to_owned());
    }
    let target_view_start = array(&original, "bufferViews")?.len();
    let target_accessor_start = array(&original, "accessors")?.len();
    while !source_bytes.len().is_multiple_of(4) {
        source_bytes.push(0);
    }
    let byte_start = source_bytes.len();
    source_bytes.extend_from_slice(&motion_bytes);
    let mut additional_views = array(&converted, "bufferViews")?.to_vec();
    for view in &mut additional_views {
        if view["buffer"].as_u64().unwrap_or(0) != 0 {
            return Err("External converted buffers unsupported".into());
        }
        let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        view["byteOffset"] = json!(offset.checked_add(byte_start).ok_or("Buffer offset overflow")?);
    }
    let mut additional_accessors = array(&converted, "accessors")?.to_vec();
    for accessor in &mut additional_accessors {
        if !accessor["sparse"].is_null() {
            return Err("Sparse converted tracks unsupported".into());
        }
        let view = node_index(&accessor["bufferView"])?;
        accessor["bufferView"] = json!(view.checked_add(target_view_start).ok_or("View offset overflow")?);
    }
    for animation in &mut animations {
        let samplers = animation["samplers"].as_array_mut().ok_or("Missing clip samplers")?;
        for sampler in samplers {
            for field in ["input", "output"] {
                let index = node_index(&sampler[field])?;
                sampler[field] = json!(index.checked_add(target_accessor_start).ok_or("Accessor index overflow")?);
            }
        }
    }
    original["bufferViews"].as_array_mut().ok_or("Missing source buffer views")?
        .extend(additional_views);
    original["accessors"].as_array_mut().ok_or("Missing source accessors")?
        .extend(additional_accessors);
    let source_nodes = original["nodes"].as_array_mut().ok_or("Missing source joint nodes")?;
    for (destination, converted_node) in keyed {
        let source_node = source_nodes.get_mut(destination).ok_or("Source joint out of bounds")?;
        source_node.as_object_mut().ok_or("Source node is not an object")?.remove("matrix");
        for (name, default) in [
            ("translation", json!([0., 0., 0.])),
            ("rotation", json!([0., 0., 0., 1.])),
            ("scale", json!([1., 1., 1.])),
        ] {
            source_node[name] = converted_node.get(name).cloned().unwrap_or(default);
        }
    }
    original["animations"] = json!(animations);
    let result = pack(original, source_bytes)?;
    Ok(AttachedConvertedMotion {
        glb: result,
        matching_joints: remap.len(),
        clip_names: written_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animated_glb_preview::tests::preview_doc;
    use crate::animation::AnimatedGlb;

    fn source_and_converted() -> (Vec<u8>, Vec<u8>) {
        let (mut doc, bin) = preview_doc();
        doc["buffers"] = json!([{"byteLength":bin.len()}]);
        let source = pack(doc.clone(), bin.clone()).unwrap();
        let mut motion_bin = bin;
        while !motion_bin.len().is_multiple_of(4) {
            motion_bin.push(0);
        }
        let time_offset = motion_bin.len();
        motion_bin.extend_from_slice(&0f32.to_le_bytes());
        motion_bin.extend_from_slice(&1f32.to_le_bytes());
        let values_offset = motion_bin.len();
        for value in [0f32, 0., 0., 2., 0., 0.] {
            motion_bin.extend_from_slice(&value.to_le_bytes());
        }
        let views = doc["bufferViews"].as_array_mut().unwrap();
        let time_view = views.len();
        views.push(json!({"buffer":0, "byteOffset":time_offset, "byteLength":8}));
        let pose_view = views.len();
        views.push(json!({"buffer":0, "byteOffset":values_offset, "byteLength":24}));
        let accessors = doc["accessors"].as_array_mut().unwrap();
        let time_access = accessors.len();
        accessors.push(json!({"bufferView":time_view, "componentType":5126,
            "type":"SCALAR", "count":2}));
        let pose_access = accessors.len();
        accessors.push(json!({"bufferView":pose_view, "componentType":5126,
            "type":"VEC3", "count":2}));
        doc["animations"] = json!([{
            "name":"Walk",
            "samplers":[{"input":time_access, "output":pose_access, "interpolation":"LINEAR"}],
            "channels":[{"sampler":0, "target":{"node":1, "path":"translation"}}]
        }]);
        let converted = pack(doc, motion_bin).unwrap();
        (source, converted)
    }

    #[test]
    fn verified_rig_receives_converted_clip_and_deforms_its_own_original_vertices() {
        let (source, converted) = source_and_converted();
        let result = attach_converted_motion_to_source_glb(
            &source, &converted, Some("walk.caf")
        ).unwrap();
        assert_eq!(result.matching_joints, 1);
        assert_eq!(result.clip_names, vec!["Walk"]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bound.glb");
        std::fs::write(&path, result.glb).unwrap();
        let animation = AnimatedGlb::read(&path).unwrap();
        assert!(animation.has_skinned_preview());
        let rest = animation.mesh_at(0, 0.).unwrap();
        let moved = animation.mesh_at(0, 0.5).unwrap();
        assert!((moved.positions[0][0] - rest.positions[0][0] - 1.).abs() < 0.001);
        assert_eq!(rest.indices, moved.indices);
        assert_eq!(rest.uv0, moved.uv0);
    }

    #[test]
    fn conversion_refuses_wrong_hierarchy_rest_pose_and_unmatched_clip() {
        let (source, converted) = source_and_converted();
        let (mut wrong, data) = unpack(&converted).unwrap();
        wrong["nodes"][1]["translation"] = json!([3., 0., 0.]);
        let wrong = pack(wrong, data).unwrap();
        assert!(attach_converted_motion_to_source_glb(&source, &wrong, None)
            .unwrap_err().contains("bind pose mismatch"));
        assert!(attach_converted_motion_to_source_glb(
            &source, &converted, Some("another.caf")
        ).is_err());
    }
}
