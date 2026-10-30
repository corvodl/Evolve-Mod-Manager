//! Safe, read-only skinned preview of a converter-owned GLB.
//! This does not assert that the converted rig matches a separately selected
//! CryEngine CDF. It animates only the mesh/skin stored together in this GLB.
use crate::animation::PoseSample;
use crate::model::{ModelInfo, RenderMesh};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

type Mat4 = [[f32; 4]; 4];
const MAX_VERTICES: usize = 250_000;
const MAX_INDICES: usize = 1_500_000;
const MAX_JOINTS: usize = 1_024;

#[derive(Debug, Clone)]
struct Node {
    rest_translation: [f32; 3],
    rest_rotation: [f32; 4],
    rest_scale: [f32; 3],
    matrix: Option<Mat4>,
}

#[derive(Debug, Clone)]
pub(crate) struct RiggedGlbPreview {
    nodes: Vec<Node>,
    parents: Vec<Option<usize>>,
    mesh_node: usize,
    joints: Vec<usize>,
    inverse_bind: Vec<Mat4>,
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    uv0: Vec<[f32; 2]>,
    joint_ids: Vec<[usize; 8]>,
    weights: Vec<[f32; 8]>,
}

fn n(value: &Value, key: &str) -> Result<usize, String> {
    value[key]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| format!("Missing {key} index"))
}
fn array<'a>(obj: &'a Value, name: &str) -> Result<&'a [Value], String> {
    obj[name]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("Missing {name} array"))
}
fn vec_values<const N: usize>(
    value: Option<&Value>,
    fallback: [f32; N],
) -> Result<[f32; N], String> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    let items = value
        .as_array()
        .filter(|v| v.len() == N)
        .ok_or("Malformed glTF transform component")?;
    let mut out = [0.; N];
    for (index, item) in items.iter().enumerate() {
        let value = item.as_f64().ok_or("Non-numeric glTF component")? as f32;
        if !value.is_finite() || value.abs() > 1e7 {
            return Err("Non-finite/unbounded glTF component".into());
        }
        out[index] = value;
    }
    Ok(out)
}

fn read_access(
    doc: &Value,
    binary: &[u8],
    index: usize,
    width: usize,
) -> Result<(Vec<Vec<f32>>, u64), String> {
    let access = array(doc, "accessors")?
        .get(index)
        .ok_or("Invalid accessor index")?;
    if !access["sparse"].is_null() {
        return Err("Sparse glTF accessors unsupported".into());
    }
    let count = n(access, "count")?;
    if count == 0 || count > MAX_INDICES {
        return Err("GLB accessor exceeds element budget".into());
    }
    let ty = match width {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        16 => "MAT4",
        _ => return Err("Unsupported accessor width".into()),
    };
    if access["type"] != ty {
        return Err("Unexpected GLB accessor type".into());
    }
    let kind = access["componentType"]
        .as_u64()
        .ok_or("Missing accessor component type")?;
    let byte_width = match kind {
        5121 => 1,
        5123 => 2,
        5125 | 5126 => 4,
        _ => return Err("Unsupported GLB component type".into()),
    };
    let view = array(doc, "bufferViews")?
        .get(n(access, "bufferView")?)
        .ok_or("Invalid buffer view")?;
    if view["buffer"].as_u64().unwrap_or(0) != 0 {
        return Err("External buffers are unsupported".into());
    }
    let view_start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let view_length = n(view, "byteLength")?;
    let view_end = view_start
        .checked_add(view_length)
        .ok_or("Buffer overflow")?;
    if view_end > binary.len() {
        return Err("Buffer view outside GLB".into());
    }
    let length = width * byte_width;
    let stride = view["byteStride"].as_u64().unwrap_or(length as u64) as usize;
    if stride < length || stride > 256 {
        return Err("Unsupported GLB buffer stride".into());
    }
    let first = view_start
        .checked_add(access["byteOffset"].as_u64().unwrap_or(0) as usize)
        .ok_or("Accessor offset overflow")?;
    let end = first
        .checked_add((count - 1).checked_mul(stride).ok_or("Stride overflow")?)
        .and_then(|v| v.checked_add(length))
        .ok_or("Accessor span overflow")?;
    if first < view_start || end > view_end {
        return Err("Accessor exceeds its buffer view".into());
    }
    let normalized = access["normalized"].as_bool().unwrap_or(false);
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let at = first + index * stride;
        let mut values = Vec::with_capacity(width);
        for component in 0..width {
            let offset = at + component * byte_width;
            let value = match kind {
                5121 => f32::from(binary[offset]),
                5123 => f32::from(u16::from_le_bytes(
                    binary[offset..offset + 2].try_into().map_err(|_| "u16")?,
                )),
                5125 => {
                    u32::from_le_bytes(binary[offset..offset + 4].try_into().map_err(|_| "u32")?)
                        as f32
                }
                5126 => {
                    f32::from_le_bytes(binary[offset..offset + 4].try_into().map_err(|_| "f32")?)
                }
                _ => unreachable!(),
            };
            let value = if normalized {
                match kind {
                    5121 => value / 255.0,
                    5123 => value / 65535.0,
                    5126 => value,
                    _ => return Err("Unsupported normalized component".into()),
                }
            } else {
                value
            };
            if !value.is_finite() {
                return Err("Non-finite GLB accessor".into());
            }
            values.push(value);
        }
        result.push(values);
    }
    Ok((result, kind))
}
fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}
fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| (0..3).map(|c| m[r][c] * p[c]).sum::<f32>() + m[r][3])
}
fn inverse(matrix: &Mat4) -> Result<Mat4, String> {
    let mut rows = [[0f32; 8]; 4];
    for r in 0..4 {
        for c in 0..4 {
            rows[r][c] = matrix[r][c];
            rows[r][c + 4] = if r == c { 1. } else { 0. };
        }
    }
    for axis in 0..4 {
        let pivot = (axis..4)
            .max_by(|a, b| rows[*a][axis].abs().total_cmp(&rows[*b][axis].abs()))
            .ok_or("Invalid matrix")?;
        rows.swap(axis, pivot);
        let value = rows[axis][axis];
        if !value.is_finite() || value.abs() < 1e-7 {
            return Err("Singular GLB skin matrix".into());
        }
        for component in &mut rows[axis] {
            *component /= value;
        }
        let pivot_row = rows[axis];
        for (index, row) in rows.iter_mut().enumerate() {
            if index == axis {
                continue;
            }
            let multiplier = row[axis];
            for (component, pivot_value) in row.iter_mut().zip(pivot_row) {
                *component -= pivot_value * multiplier;
            }
        }
    }
    Ok(std::array::from_fn(|r| {
        std::array::from_fn(|c| rows[r][c + 4])
    }))
}
fn matrix(column_major: &[f32]) -> Result<Mat4, String> {
    if column_major.len() != 16 || column_major.iter().any(|v| !v.is_finite()) {
        return Err("Invalid glTF matrix".into());
    }
    Ok(std::array::from_fn(|r| {
        std::array::from_fn(|c| column_major[c * 4 + r])
    }))
}
fn local(n: &Node, overrides: Option<&HashMap<&str, Vec<f32>>>) -> Result<Mat4, String> {
    if let Some(matrix) = n.matrix {
        if overrides.is_some_and(|values| !values.is_empty()) {
            return Err("Cannot animate a matrix-only joint".into());
        }
        return Ok(matrix);
    }
    let get = |key: &str, default: &[f32]| -> Vec<f32> {
        overrides
            .and_then(|values| values.get(key).cloned())
            .unwrap_or_else(|| default.to_vec())
    };
    let t = get("translation", &n.rest_translation);
    let q = get("rotation", &n.rest_rotation);
    let s = get("scale", &n.rest_scale);
    if t.len() != 3 || q.len() != 4 || s.len() != 3 {
        return Err("Invalid animated TRS component".into());
    }
    let norm = q.iter().map(|v| v * v).sum::<f32>();
    if !norm.is_finite() || norm < 1e-8 {
        return Err("Invalid joint quaternion".into());
    }
    let factor = (2.0 / norm).sqrt();
    let (x, y, z, w) = (q[0] * factor, q[1] * factor, q[2] * factor, q[3] * factor);
    let mut m = [
        [1. - y * y - z * z, x * y - z * w, x * z + y * w, t[0]],
        [x * y + z * w, 1. - x * x - z * z, y * z - x * w, t[1]],
        [x * z - y * w, y * z + x * w, 1. - x * x - y * y, t[2]],
        [0., 0., 0., 1.],
    ];
    for row in m.iter_mut().take(3) {
        for (component, value) in row.iter_mut().take(3).zip(&s) {
            *component *= *value;
        }
    }
    if m.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Non-finite sampled transform".into());
    }
    Ok(m)
}
impl RiggedGlbPreview {
    pub(crate) fn from_doc(doc: &Value, binary: &[u8]) -> Result<Self, String> {
        let nodes_raw = array(doc, "nodes")?;
        if nodes_raw.is_empty() || nodes_raw.len() > 4096 {
            return Err("Invalid GLB node count".into());
        }
        let nodes = nodes_raw
            .iter()
            .map(|node| {
                let matrix = node["matrix"]
                    .as_array()
                    .map(|values| {
                        let numbers = values
                            .iter()
                            .map(|v| v.as_f64().map(|v| v as f32).ok_or("Invalid matrix"))
                            .collect::<Result<Vec<_>, _>>()?;
                        matrix(&numbers)
                    })
                    .transpose()?;
                Ok(Node {
                    rest_translation: vec_values(node.get("translation"), [0., 0., 0.])?,
                    rest_rotation: vec_values(node.get("rotation"), [0., 0., 0., 1.])?,
                    rest_scale: vec_values(node.get("scale"), [1., 1., 1.])?,
                    matrix,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut parents = vec![None; nodes.len()];
        for (i, node) in nodes_raw.iter().enumerate() {
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    let index = child
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|v| *v < parents.len())
                        .ok_or("Invalid child node")?;
                    if parents[index].replace(i).is_some() {
                        return Err("Ambiguous GLB node parent".into());
                    }
                }
            }
        }
        let owners = nodes_raw
            .iter()
            .enumerate()
            .filter(|(_, node)| node["mesh"].is_number() && node["skin"].is_number())
            .collect::<Vec<_>>();
        if owners.len() != 1 {
            return Err("Converted GLB must have exactly one skinned mesh".into());
        }
        let (mesh_node, node) = owners[0];
        let skin = array(doc, "skins")?
            .get(n(node, "skin")?)
            .ok_or("Skin index missing")?;
        let joints_raw = array(skin, "joints")?;
        if joints_raw.is_empty() || joints_raw.len() > MAX_JOINTS {
            return Err("Invalid rig joint count".into());
        }
        let mut unique = HashSet::new();
        let joints = joints_raw
            .iter()
            .map(|index| {
                let id = index
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .filter(|v| *v < nodes.len())
                    .ok_or("Skin joint outside node list")?;
                if !unique.insert(id) {
                    return Err("Duplicate GLB joint".into());
                }
                Ok(id)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (bindings, kind) = read_access(doc, binary, n(skin, "inverseBindMatrices")?, 16)?;
        if kind != 5126 || bindings.len() != joints.len() {
            return Err("Invalid inverse-bind matrix count/type".into());
        }
        let inverse_bind = bindings
            .iter()
            .map(|v| matrix(v))
            .collect::<Result<Vec<_>, _>>()?;
        let mesh = array(doc, "meshes")?
            .get(n(node, "mesh")?)
            .ok_or("Mesh index missing")?;
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut uv0 = Vec::new();
        let mut joint_ids = Vec::new();
        let mut weights = Vec::new();
        let mut reused = HashMap::<
            (
                usize,
                usize,
                usize,
                Option<usize>,
                Option<usize>,
                Option<usize>,
            ),
            (usize, usize),
        >::new();
        for primitive in array(mesh, "primitives")? {
            if primitive["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let attrs = &primitive["attributes"];
            let signature = (
                n(attrs, "POSITION")?,
                n(attrs, "JOINTS_0")?,
                n(attrs, "WEIGHTS_0")?,
                attrs["TEXCOORD_0"].as_u64().map(|v| v as usize),
                attrs["JOINTS_1"].as_u64().map(|v| v as usize),
                attrs["WEIGHTS_1"].as_u64().map(|v| v as usize),
            );
            if signature.4.is_some() != signature.5.is_some() {
                return Err("Incomplete secondary skin influences".into());
            }
            let (first, count) = if let Some(&(first, count)) = reused.get(&signature) {
                (first, count)
            } else {
                let (pos, pos_type) = read_access(doc, binary, signature.0, 3)?;
                let (ids, id_type) = read_access(doc, binary, signature.1, 4)?;
                let (amounts, weight_type) = read_access(doc, binary, signature.2, 4)?;
                if pos_type != 5126
                    || !matches!(id_type, 5121 | 5123)
                    || !matches!(weight_type, 5121 | 5123 | 5126)
                    || pos.len() != ids.len()
                    || pos.len() != amounts.len()
                {
                    return Err("Invalid skinned vertex channels".into());
                }
                let extra = signature
                    .4
                    .zip(signature.5)
                    .map(|(a, b)| {
                        let (ids, kind_a) = read_access(doc, binary, a, 4)?;
                        let (weights, kind_b) = read_access(doc, binary, b, 4)?;
                        if !matches!(kind_a, 5121 | 5123) || !matches!(kind_b, 5121 | 5123 | 5126) {
                            return Err("Invalid secondary skin channel type".into());
                        }
                        Ok::<_, String>((ids, weights))
                    })
                    .transpose()?;
                let tex = signature
                    .3
                    .map(|accessor| read_access(doc, binary, accessor, 2))
                    .transpose()?
                    .map(|(values, _)| values);
                let first = positions.len();
                if first + pos.len() > MAX_VERTICES {
                    return Err("Animated mesh exceeds vertex budget".into());
                }
                for i in 0..pos.len() {
                    let mut id = [0usize; 8];
                    let mut weight = [0f32; 8];
                    for lane in 0..4 {
                        id[lane] = ids[i][lane] as usize;
                        weight[lane] = amounts[i][lane];
                        if let Some((extra_ids, extra_weights)) = &extra {
                            id[lane + 4] = extra_ids[i][lane] as usize;
                            weight[lane + 4] = extra_weights[i][lane];
                        }
                    }
                    if id
                        .iter()
                        .zip(weight)
                        .any(|(&joint, w)| w > 0. && joint >= joints.len())
                    {
                        return Err("Joint index exceeds skin".into());
                    }
                    let total: f32 = weight.iter().sum();
                    if !(0.95..=1.05).contains(&total) {
                        return Err("Unnormalized skin weights".into());
                    }
                    for lane in &mut weight {
                        *lane /= total;
                    }
                    positions.push([pos[i][0], pos[i][1], pos[i][2]]);
                    joint_ids.push(id);
                    weights.push(weight);
                    uv0.push(
                        tex.as_ref()
                            .and_then(|v| v.get(i))
                            .map(|v| [v[0], v[1]])
                            .unwrap_or([0., 0.]),
                    );
                }
                reused.insert(signature, (first, pos.len()));
                (first, pos.len())
            };
            let (primitive_indices, kind) = read_access(doc, binary, n(primitive, "indices")?, 1)?;
            if !matches!(kind, 5121 | 5123 | 5125)
                || primitive_indices.is_empty()
                || primitive_indices.len() % 3 != 0
                || indices.len() + primitive_indices.len() > MAX_INDICES
            {
                return Err("Unsupported triangle index buffer".into());
            }
            for index in primitive_indices {
                let local = index[0] as usize;
                if local >= count {
                    return Err("Triangle index outside primitive".into());
                }
                indices.push(u32::try_from(first + local).map_err(|_| "Vertex index overflow")?);
            }
        }
        if positions.is_empty() || indices.is_empty() {
            return Err("No supported skinned triangle mesh".into());
        }
        let preview = Self {
            nodes,
            parents,
            mesh_node,
            joints,
            inverse_bind,
            positions,
            indices,
            uv0,
            joint_ids,
            weights,
        };
        // A converted mesh must be internally coherent at bind pose before any
        // animation is considered. Invalid converter skinning fails closed.
        preview.mesh_at(&[])?;
        Ok(preview)
    }

    fn world(
        &self,
        changes: &HashMap<usize, HashMap<&str, Vec<f32>>>,
    ) -> Result<Vec<Mat4>, String> {
        fn visit(
            id: usize,
            nodes: &[Node],
            parents: &[Option<usize>],
            overrides: &HashMap<usize, HashMap<&str, Vec<f32>>>,
            cache: &mut [Option<Mat4>],
            active: &mut HashSet<usize>,
        ) -> Result<Mat4, String> {
            if let Some(matrix) = cache[id] {
                return Ok(matrix);
            }
            if !active.insert(id) {
                return Err("Cycle in GLB hierarchy".into());
            }
            let local = local(&nodes[id], overrides.get(&id))?;
            let world = if let Some(parent) = parents[id] {
                multiply(
                    &visit(parent, nodes, parents, overrides, cache, active)?,
                    &local,
                )
            } else {
                local
            };
            active.remove(&id);
            cache[id] = Some(world);
            Ok(world)
        }
        let mut cache = vec![None; self.nodes.len()];
        let mut active = HashSet::new();
        (0..self.nodes.len())
            .map(|i| {
                visit(
                    i,
                    &self.nodes,
                    &self.parents,
                    changes,
                    &mut cache,
                    &mut active,
                )
            })
            .collect()
    }

    /// Default authored TRS for a converted joint that is not keyed by the
    /// base clip. A matrix-only joint cannot be blended without a verified
    /// decomposition, so layered playback refuses it.
    pub(crate) fn rest_channel(&self, node: usize, path: &str) -> Result<Vec<f32>, String> {
        let joint = self
            .nodes
            .get(node)
            .ok_or("Layer references missing GLB joint")?;
        if joint.matrix.is_some() {
            return Err("Matrix-only GLB joints cannot be layered".into());
        }
        match path {
            "translation" => Ok(joint.rest_translation.to_vec()),
            "rotation" => Ok(joint.rest_rotation.to_vec()),
            "scale" => Ok(joint.rest_scale.to_vec()),
            _ => Err("Unsupported layered GLB channel".into()),
        }
    }

    pub(crate) fn mesh_at(&self, samples: &[PoseSample]) -> Result<RenderMesh, String> {
        let mut changes: HashMap<usize, HashMap<&str, Vec<f32>>> = HashMap::new();
        for sample in samples {
            if sample.node >= self.nodes.len()
                || !matches!(sample.path.as_str(), "rotation" | "translation" | "scale")
            {
                return Err("Invalid converter animation target".into());
            }
            let node = changes.entry(sample.node).or_default();
            if node
                .insert(sample.path.as_str(), sample.value.clone())
                .is_some()
            {
                return Err("Duplicate animated node channel".into());
            }
        }
        let world = self.world(&changes)?;
        let inverse_mesh = inverse(&world[self.mesh_node])?;
        let palette = self
            .joints
            .iter()
            .zip(&self.inverse_bind)
            .map(|(&node, bind)| multiply(&multiply(&inverse_mesh, &world[node]), bind))
            .collect::<Vec<_>>();
        let mut out = Vec::with_capacity(self.positions.len());
        for (i, pos) in self.positions.iter().enumerate() {
            let mut p = [0f32; 3];
            for lane in 0..8 {
                let w = self.weights[i][lane];
                if w <= 0. {
                    continue;
                }
                let shifted = transform(&palette[self.joint_ids[i][lane]], *pos);
                for axis in 0..3 {
                    p[axis] += shifted[axis] * w;
                }
            }
            if p.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
                return Err("Non-finite deformed GLB vertex".into());
            }
            // Match the private viewer's glTF Y-up -> CryEngine Z-up basis.
            out.push([p[0], -p[2], p[1]]);
        }
        Ok(RenderMesh {
            info: ModelInfo {
                format: "Converted GLB own-rig animation preview".into(),
                version: 2,
                chunk_count: 0,
                vertex_count: out.len(),
                triangle_count: self.indices.len() / 3,
                original_uv0: true,
                original_normals: false,
            },
            positions: out,
            indices: self.indices.clone(),
            uv0: Some(self.uv0.clone()),
            normals: None,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn preview_doc() -> (Value, Vec<u8>) {
        let mut bytes = Vec::new();
        let mut views = Vec::new();
        let mut add = |data: Vec<u8>| {
            let offset = bytes.len();
            bytes.extend_from_slice(&data);
            let index = views.len();
            views.push(json!({"buffer":0, "byteOffset":offset, "byteLength":data.len()}));
            index
        };
        let packed_f32 = |values: &[f32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        };
        let packed_u16 = |values: &[u16]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        };
        let pos = add(packed_f32(&[0., 0., 0., 1., 0., 0., 0., 1., 0.]));
        let ids = add(packed_u16(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        let weights = add(packed_f32(&[
            1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
        ]));
        let uv = add(packed_f32(&[0., 0., 1., 0., 0., 1.]));
        let indices = add(packed_u16(&[0, 1, 2]));
        let inverse_bind = add(packed_f32(&[
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]));
        let doc = json!({
            "nodes":[
                {"name":"converted_mesh","mesh":0,"skin":0},
                {"name":"rig_root"}
            ],
            "skins":[{"joints":[1], "inverseBindMatrices":5}],
            "meshes":[{"primitives":[{"mode":4,
                "attributes":{
                    "POSITION":0, "JOINTS_0":1, "WEIGHTS_0":2, "TEXCOORD_0":3
                }, "indices":4
            }]}],
            "accessors":[
                {"bufferView":pos,"componentType":5126,"count":3,"type":"VEC3"},
                {"bufferView":ids,"componentType":5123,"count":3,"type":"VEC4"},
                {"bufferView":weights,"componentType":5126,"count":3,"type":"VEC4"},
                {"bufferView":uv,"componentType":5126,"count":3,"type":"VEC2"},
                {"bufferView":indices,"componentType":5123,"count":3,"type":"SCALAR"},
                {"bufferView":inverse_bind,"componentType":5126,"count":1,"type":"MAT4"}
            ],
            "bufferViews":views
        });
        (doc, bytes)
    }

    #[test]
    fn sampled_converter_joint_deforms_actual_vertices_but_preserves_mesh_topology_and_uv() {
        let (doc, binary) = preview_doc();
        let rig = RiggedGlbPreview::from_doc(&doc, &binary).unwrap();
        let rest = rig.mesh_at(&[]).unwrap();
        let moved = rig
            .mesh_at(&[PoseSample {
                node: 1,
                path: "translation".into(),
                value: vec![2., 0., 0.],
            }])
            .unwrap();
        assert_eq!(rest.positions.len(), 3);
        assert_eq!(moved.positions[0], [2., 0., 0.]);
        assert_eq!(moved.positions[1], [3., 0., 0.]);
        assert_eq!(moved.positions[2], [2., 0., 1.]);
        assert_eq!(rest.indices, moved.indices);
        assert_eq!(rest.uv0, moved.uv0);
    }

    #[test]
    fn rejects_malformed_weights_hierarchy_and_non_matching_joint_data() {
        let (mut doc, binary) = preview_doc();
        doc["skins"][0]["joints"] = json!([2]);
        assert!(RiggedGlbPreview::from_doc(&doc, &binary).is_err());
        let (mut doc, binary) = preview_doc();
        doc["nodes"][0]["children"] = json!([1]);
        doc["nodes"][1]["children"] = json!([0]);
        assert!(RiggedGlbPreview::from_doc(&doc, &binary).is_err());
        let (doc, binary) = preview_doc();
        let rig = RiggedGlbPreview::from_doc(&doc, &binary).unwrap();
        assert!(
            rig.mesh_at(&[PoseSample {
                node: 9000,
                path: "rotation".into(),
                value: vec![0., 0., 0., 1.],
            }])
            .is_err()
        );
    }
}
