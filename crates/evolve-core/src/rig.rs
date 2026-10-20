//! Verified CryEngine Stage 2 CrChF compiled bone tables, subset-local
//! bone palettes and Blender-compatible glTF skin export. Never substitute
//! guessed indices/weights for missing native source data.
use crate::fbx::{FbxError, FbxReport, convert_glb_to_fbx};
use crate::glb::{GlbError, geometry_glb_bytes};
use crate::model::{ModelError, RenderMesh, read_model};
use crate::native_caf::NativeCafMotion;
use crate::policy::AnimationPolicy;
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const BONES: u32 = 0x08002000;
const STREAM: u32 = 0x08001016;
const SUBSETS: u32 = 0x08001017;
const RECORD: usize = 584;
const MAX_BONES: usize = 1024;
const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RigError {
    #[error("Missing or invalid original CryEngine rigging: {0}")]
    Invalid(String),
    #[error("Rigging source out of safe range")]
    Limits,
    #[error("Mesh source parser: {0}")]
    Model(#[from] ModelError),
    #[error("glTF exporter: {0}")]
    Glb(#[from] GlbError),
    #[error("Blender FBX bridge: {0}")]
    Fbx(#[from] FbxError),
    #[error("Rigged output already exists")]
    Exists,
    #[error("Filesystem: {0}")]
    Io(#[from] io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RigReport {
    pub bone_count: usize,
    pub weighted_vertices: usize,
    pub active_bones: usize,
    pub subset_count: usize,
    pub influences_per_vertex: usize,
    pub animations_included: bool,
}
#[derive(Clone, Debug)]
pub struct CompiledSkeleton {
    /// Authored controller IDs from the compiled bone records.
    pub controller_ids: Vec<u32>,
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub world: Vec<[[f32; 4]; 4]>,
    pub inverse: Vec<[[f32; 4]; 4]>,
}
#[derive(Clone, Debug)]
pub struct VerifiedWeights {
    pub joint_ids: Vec<[u16; 8]>,
    pub weight_bytes: Vec<[u8; 8]>,
    pub influences: usize,
    pub subsets: usize,
}
fn read_u32(data: &[u8], at: usize) -> Result<u32, RigError> {
    let buf = data.get(at..at + 4).ok_or(RigError::Limits)?;
    Ok(u32::from_le_bytes(
        buf.try_into().map_err(|_| RigError::Limits)?,
    ))
}
fn read_f32(data: &[u8], at: usize) -> Result<f32, RigError> {
    Ok(f32::from_bits(read_u32(data, at)?))
}
#[derive(Clone, Copy)]
struct Chunk<'a> {
    kind: u32,
    data: &'a [u8],
}
fn chunks(data: &[u8]) -> Result<Vec<Chunk<'_>>, RigError> {
    if data.len() < 32 || data.len() > MAX_BYTES || data.get(..8) != Some(b"CrChF\x07\0\0") {
        return Err(RigError::Invalid("expected CrChF v7 original file".into()));
    }
    let count = read_u32(data, 8)? as usize;
    let start = read_u32(data, 12)? as usize;
    let end = start
        .checked_add(count.checked_mul(16).ok_or(RigError::Limits)?)
        .ok_or(RigError::Limits)?;
    if count == 0 || count > 10000 || start < 16 || end > data.len() {
        return Err(RigError::Limits);
    }
    let mut found = HashSet::new();
    let mut ranges = Vec::new();
    let mut out = Vec::new();
    for i in 0..count {
        let at = start + i * 16;
        let kind = read_u32(data, at)?;
        let id = read_u32(data, at + 4)?;
        let len = read_u32(data, at + 8)? as usize;
        let offset = read_u32(data, at + 12)? as usize;
        let upper = offset.checked_add(len).ok_or(RigError::Limits)?;
        if len == 0 || offset < end || upper > data.len() || !found.insert((kind, id)) {
            return Err(RigError::Invalid("untrusted bone/mesh chunk ranges".into()));
        }
        out.push(Chunk {
            kind,
            data: &data[offset..upper],
        });
        ranges.push((offset, upper));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(RigError::Invalid("overlapping skeletal chunks".into()));
    }
    Ok(out)
}
fn matrix(data: &[u8], at: usize) -> Result<[[f32; 4]; 4], RigError> {
    let mut m = [[0f32; 4]; 4];
    for (r, row) in m.iter_mut().enumerate().take(3) {
        for (c, v) in row.iter_mut().enumerate() {
            *v = read_f32(data, at + (r * 4 + c) * 4)?;
            if !v.is_finite() || v.abs() > 1e7 {
                return Err(RigError::Limits);
            }
        }
    }
    m[3] = [0., 0., 0., 1.];
    Ok(m)
}
fn multiply(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}
fn column_major(m: &[[f32; 4]; 4]) -> [f32; 16] {
    std::array::from_fn(|index| m[index % 4][index / 4])
}
pub fn parse_compiled_skeleton(data: &[u8]) -> Result<CompiledSkeleton, RigError> {
    let chunks = chunks(data)?;
    let mut sources = chunks.iter().filter(|c| c.kind == BONES);
    let raw = sources
        .next()
        .ok_or_else(|| RigError::Invalid("CompiledBones chunk missing".into()))?
        .data;
    if sources.next().is_some() || raw.len() < 32 || !(raw.len() - 32).is_multiple_of(RECORD) {
        return Err(RigError::Invalid(
            "expected exactly one 584-byte compiled bone table".into(),
        ));
    }
    let count = (raw.len() - 32) / RECORD;
    if !(1..=MAX_BONES).contains(&count) {
        return Err(RigError::Limits);
    }
    let mut names = Vec::with_capacity(count);
    let mut controller_ids = Vec::with_capacity(count);
    let mut unique_ids = HashSet::new();
    let mut parents = Vec::with_capacity(count);
    let mut world = Vec::with_capacity(count);
    let mut inverse = Vec::with_capacity(count);
    let mut unique = HashSet::new();
    let mut roots = 0usize;
    for i in 0..count {
        let at = 32 + i * RECORD;
        let controller_id = read_u32(raw, at)?;
        if !unique_ids.insert(controller_id) {
            return Err(RigError::Invalid("Duplicate compiled controller ID".into()));
        }
        controller_ids.push(controller_id);
        let raw_name = raw.get(at + 312..at + 568).ok_or(RigError::Limits)?;
        let end = raw_name
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| RigError::Invalid("unterminated bone name".into()))?;
        let name = std::str::from_utf8(&raw_name[..end])
            .map_err(|_| RigError::Invalid("invalid bone name encoding".into()))?
            .trim()
            .to_owned();
        if name.is_empty() || !unique.insert(name.to_ascii_lowercase()) {
            return Err(RigError::Invalid(
                "blank/duplicate skeleton bone name".into(),
            ));
        }
        let inv = matrix(raw, at + 216)?;
        let rest = matrix(raw, at + 264)?;
        let product = multiply(&inv, &rest);
        if (0..4)
            .any(|r| (0..4).any(|c| (product[r][c] - if r == c { 1.0 } else { 0.0 }).abs() > 0.001))
        {
            return Err(RigError::Invalid(format!(
                "compiled bind/inverse mismatch: {name}"
            )));
        }
        let delta = read_u32(raw, at + 572)? as i32;
        let parent = if delta < 0 {
            let p = (i as i64) + i64::from(delta);
            if p < 0 || p >= i as i64 {
                return Err(RigError::Invalid("invalid parent offset".into()));
            }
            Some(p as usize)
        } else {
            roots += 1;
            None
        };
        names.push(name);
        parents.push(parent);
        world.push(rest);
        inverse.push(inv);
    }
    if roots != 1 {
        return Err(RigError::Invalid(
            "expected one source skeleton root".into(),
        ));
    }
    Ok(CompiledSkeleton {
        controller_ids,
        names,
        parents,
        world,
        inverse,
    })
}
type InfluenceStream<'a> = (usize, usize, &'a [u8]);
fn stream<'a>(all: &'a [Chunk<'a>], kind: u32) -> Result<Option<InfluenceStream<'a>>, RigError> {
    let mut result = None;
    for chunk in all.iter().filter(|c| c.kind == STREAM) {
        let r = chunk.data;
        if r.len() < 24 {
            return Err(RigError::Limits);
        }
        let ty = read_u32(r, 4)?;
        if ty != kind {
            continue;
        }
        let count = read_u32(r, 8)? as usize;
        let stride = read_u32(r, 12)? as usize;
        if count == 0
            || stride == 0
            || count.checked_mul(stride).and_then(|n| n.checked_add(24)) != Some(r.len())
        {
            return Err(RigError::Invalid(
                "unsupported rig influence stream header".into(),
            ));
        }
        if result.replace((count, stride, &r[24..])).is_some() {
            return Err(RigError::Invalid("duplicate rig influence stream".into()));
        }
    }
    Ok(result)
}
pub fn remap_verified_weights(
    mesh: &[u8],
    vertices: usize,
    bones: usize,
) -> Result<VerifiedWeights, RigError> {
    if vertices == 0 || vertices > 250000 || bones == 0 || bones > MAX_BONES {
        return Err(RigError::Limits);
    }
    let all = chunks(mesh)?;
    let (count, stride, raw) = stream(&all, 9)?
        .ok_or_else(|| RigError::Invalid("native bone influence stream missing".into()))?;
    let (index_count, index_stride, index_raw) = stream(&all, 5)?
        .ok_or_else(|| RigError::Invalid("original index stream missing".into()))?;
    if stride != 8
        || !matches!(count,v if v==vertices || v==vertices*2)
        || !matches!(index_stride, 2 | 4)
        || index_count == 0
        || index_count > 1_500_000
    {
        return Err(RigError::Invalid(
            "unsupported native skin/index layout".into(),
        ));
    }
    let influences = if count == vertices * 2 { 8 } else { 4 };
    let mut joint_ids = vec![[0u16; 8]; vertices];
    let mut weight_bytes = vec![[0u8; 8]; vertices];
    for block in 0..influences / 4 {
        for v in 0..vertices {
            let offset = (block * vertices + v) * 8;
            for lane in 0..4 {
                joint_ids[v][block * 4 + lane] = u16::from(raw[offset + lane]);
                weight_bytes[v][block * 4 + lane] = raw[offset + 4 + lane];
            }
        }
    }
    if weight_bytes
        .iter()
        .any(|row| row.iter().map(|x| u32::from(*x)).sum::<u32>() != 255)
    {
        return Err(RigError::Invalid(
            "source skin weight bytes do not total 255".into(),
        ));
    }
    let mut indices = Vec::with_capacity(index_count);
    for i in 0..index_count {
        let at = i * index_stride;
        let id = if index_stride == 2 {
            u16::from_le_bytes(
                index_raw[at..at + 2]
                    .try_into()
                    .map_err(|_| RigError::Limits)?,
            ) as usize
        } else {
            read_u32(index_raw, at)? as usize
        };
        if id >= vertices {
            return Err(RigError::Invalid("subset index outside mesh".into()));
        }
        indices.push(id);
    }
    let mut subsets = all.iter().filter(|c| c.kind == SUBSETS);
    let table = subsets
        .next()
        .ok_or_else(|| RigError::Invalid("subset bone palette table missing".into()))?
        .data;
    if subsets.next().is_some() || table.len() < 16 {
        return Err(RigError::Limits);
    }
    let count_subsets = read_u32(table, 4)? as usize;
    if !(1..=128).contains(&count_subsets) || table.len() < 16 + count_subsets * (36 + 260) {
        return Err(RigError::Limits);
    }
    let mut covered = vec![false; vertices];
    let mut mapped = vec![[0u16; 8]; vertices];
    for i in 0..count_subsets {
        let at = 16 + i * 36;
        let first = read_u32(table, at)? as usize;
        let n = read_u32(table, at + 4)? as usize;
        let vstart = read_u32(table, at + 8)? as usize;
        let vn = read_u32(table, at + 12)? as usize;
        if n == 0
            || first.checked_add(n).is_none_or(|end| end > index_count)
            || vstart.checked_add(vn).is_none_or(|end| end > vertices)
        {
            return Err(RigError::Invalid("subset geometry ranges invalid".into()));
        }
        let pat = 16 + count_subsets * 36 + i * 260;
        let palette_count = read_u32(table, pat)? as usize;
        if !(1..=128).contains(&palette_count) {
            return Err(RigError::Limits);
        }
        let palette = (0..128)
            .map(|j| {
                let at = pat + 4 + j * 2;
                u16::from_le_bytes([table[at], table[at + 1]])
            })
            .collect::<Vec<_>>();
        if palette[..palette_count]
            .iter()
            .any(|id| usize::from(*id) >= bones)
        {
            return Err(RigError::Invalid("subset palette exceeds skeleton".into()));
        }
        for &vertex in &indices[first..first + n] {
            if vertex < vstart || vertex >= vstart + vn {
                return Err(RigError::Invalid(
                    "subset indices outside declared vertex block".into(),
                ));
            }
            let mut remapped = [0u16; 8];
            for lane in 0..influences {
                if weight_bytes[vertex][lane] != 0 {
                    let local = joint_ids[vertex][lane] as usize;
                    if local >= palette_count {
                        return Err(RigError::Invalid("local joint outside palette".into()));
                    }
                    remapped[lane] = palette[local];
                }
            }
            if covered[vertex] && mapped[vertex] != remapped {
                return Err(RigError::Invalid(
                    "conflicting palette mapping on shared vertex".into(),
                ));
            }
            mapped[vertex] = remapped;
            covered[vertex] = true;
        }
    }
    if covered.iter().any(|used| !*used) {
        return Err(RigError::Invalid(
            "mesh vertex has no verified skin palette".into(),
        ));
    }
    Ok(VerifiedWeights {
        joint_ids: mapped,
        weight_bytes,
        influences,
        subsets: count_subsets,
    })
}
fn add_view(
    binary: &mut Vec<u8>,
    doc: &mut Value,
    payload: &[u8],
    target: Option<u32>,
) -> Result<usize, RigError> {
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let at = binary.len();
    binary.extend_from_slice(payload);
    let mut v = json!({"buffer":0,"byteOffset":at,"byteLength":payload.len()});
    if let Some(target) = target {
        v["target"] = json!(target);
    }
    let views = doc["bufferViews"].as_array_mut().ok_or(RigError::Limits)?;
    views.push(v);
    Ok(views.len() - 1)
}
fn add_accessor(
    doc: &mut Value,
    view: usize,
    component: u32,
    count: usize,
    kind: &str,
    normalized: bool,
) -> Result<usize, RigError> {
    let access = doc["accessors"].as_array_mut().ok_or(RigError::Limits)?;
    let mut a = json!({"bufferView":view,"componentType":component,
                     "count":count,"type":kind});
    if normalized {
        a["normalized"] = json!(true);
    }
    access.push(a);
    Ok(access.len() - 1)
}

/// glTF animated nodes require TRS rather than a baked matrix. Decompose only
/// verified rigid (no shear/mirror/scale) bind-space transforms, or refuse the
/// animation instead of corrupting bone orientations.
pub(crate) fn rigid_trs(matrix: &Value) -> Result<([f32; 3], [f32; 4]), RigError> {
    let values = matrix
        .as_array()
        .ok_or_else(|| RigError::Invalid("Missing source bone matrix".into()))?;
    if values.len() != 16 {
        return Err(RigError::Invalid(
            "Expected a 4×4 source bone matrix".into(),
        ));
    }
    let mut m = [0.0f32; 16];
    for (dst, value) in m.iter_mut().zip(values) {
        *dst = value.as_f64().ok_or(RigError::Limits)? as f32;
        if !dst.is_finite() {
            return Err(RigError::Limits);
        }
    }
    for axis in 0..3 {
        let base = axis * 4;
        let squared = m[base..base + 3].iter().map(|x| x * x).sum::<f32>();
        if (squared - 1.0).abs() > 0.02 {
            return Err(RigError::Invalid(
                "Animated bind transform has unsupported scale".into(),
            ));
        }
    }
    for a in 0..3 {
        for b in a + 1..3 {
            let dot = (0..3).map(|c| m[a * 4 + c] * m[b * 4 + c]).sum::<f32>();
            if dot.abs() > 0.02 {
                return Err(RigError::Invalid(
                    "Animated bind transform has unsupported shear".into(),
                ));
            }
        }
    }
    let (a, b, c, d, e, f, g, h, i) = (m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]);
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if determinant < 0.98 {
        return Err(RigError::Invalid(
            "Animated bind transform is mirrored/non-rigid".into(),
        ));
    }
    let trace = a + e + i;
    let [x, y, z, w] = if trace > 0.0 {
        let t = 2.0 * (trace + 1.0).sqrt();
        [(h - f) / t, (c - g) / t, (d - b) / t, 0.25 * t]
    } else if a > e && a > i {
        let t = 2.0 * (1.0 + a - e - i).sqrt();
        [0.25 * t, (b + d) / t, (c + g) / t, (h - f) / t]
    } else if e > i {
        let t = 2.0 * (1.0 + e - a - i).sqrt();
        [(b + d) / t, 0.25 * t, (f + h) / t, (c - g) / t]
    } else {
        let t = 2.0 * (1.0 + i - a - e).sqrt();
        [(c + g) / t, (f + h) / t, 0.25 * t, (d - b) / t]
    };
    let len = (x * x + y * y + z * z + w * w).sqrt();
    if len < 0.98 || !len.is_finite() {
        return Err(RigError::Invalid(
            "Cannot recover rigid source bone quaternion".into(),
        ));
    }
    Ok(([m[12], m[13], m[14]], [x / len, y / len, z / len, w / len]))
}

#[allow(clippy::too_many_arguments)]
fn add_native_channel(
    doc: &mut Value,
    binary: &mut Vec<u8>,
    samplers: &mut Vec<Value>,
    channels: &mut Vec<Value>,
    bone_node: usize,
    path: &str,
    frames: &[f32],
    values: &[f32],
    dimensions: usize,
    start_frame: f32,
    fps: f32,
) -> Result<(), RigError> {
    if frames.is_empty() {
        return Ok(());
    }
    if frames.len() > 8192
        || values.len() != frames.len() * dimensions
        || !frames
            .iter()
            .all(|frame| frame.is_finite() && *frame >= start_frame)
        || !frames.windows(2).all(|pair| pair[0] < pair[1])
        || !values.iter().all(|v| v.is_finite())
    {
        return Err(RigError::Invalid(
            "Unsupported CAF channel timestamps or samples".into(),
        ));
    }
    let seconds = frames
        .iter()
        .map(|frame| (frame - start_frame) / fps)
        .collect::<Vec<_>>();
    if !seconds
        .iter()
        .all(|value| value.is_finite() && *value <= 1200.0)
    {
        return Err(RigError::Invalid(
            "CAF channel exceeds supported animation duration".into(),
        ));
    }
    let mut t_bytes = Vec::with_capacity(seconds.len() * 4);
    for sample in &seconds {
        t_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let time_view = add_view(binary, doc, &t_bytes, None)?;
    let time_acc = add_accessor(doc, time_view, 5126, seconds.len(), "SCALAR", false)?;
    doc["accessors"][time_acc]["min"] = json!([seconds[0]]);
    doc["accessors"][time_acc]["max"] = json!([seconds[seconds.len() - 1]]);
    let mut v_bytes = Vec::with_capacity(values.len() * 4);
    for sample in values {
        v_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let samples_view = add_view(binary, doc, &v_bytes, None)?;
    let kind = if dimensions == 4 { "VEC4" } else { "VEC3" };
    let samples_acc = add_accessor(doc, samples_view, 5126, frames.len(), kind, false)?;
    let sampler_index = samplers.len();
    samplers.push(json!({"input":time_acc, "output":samples_acc, "interpolation":"LINEAR"}));
    channels.push(json!({"sampler":sampler_index,"target":{"node":bone_node,"path":path}}));
    Ok(())
}

/// Bind strictly validated native CAF tracks to the matching original compiled
/// skeleton only. Bone CRC collisions/unmatched controllers are hard failures.
/// This is intentionally one verified non-additive CAF clip, not a guessed
/// layered DBA set or inferred animation for another character.
fn attach_native_caf(
    doc: &mut Value,
    binary: &mut Vec<u8>,
    skeleton: &CompiledSkeleton,
    first: usize,
    motion: &NativeCafMotion,
) -> Result<(), RigError> {
    if motion.additive
        || motion.controllers.is_empty()
        || motion.controllers.len() > 1024
        || !(1.0..=240.0).contains(&motion.fps)
        || !motion.start_frame.is_finite()
        || !motion.end_frame.is_finite()
        || motion.end_frame < motion.start_frame
    {
        return Err(RigError::Invalid(
            "Unsupported additive or invalid native CAF clip".into(),
        ));
    }
    let mut matching = HashMap::new();
    for (i, &crc) in skeleton.controller_ids.iter().enumerate() {
        if matching.insert(crc, i).is_some() {
            return Err(RigError::Invalid(
                "Ambiguous original skeleton bone CRC".into(),
            ));
        }
    }
    let mut seen = HashSet::new();
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    for controller in &motion.controllers {
        let bone = *matching.get(&controller.bone_crc32).ok_or_else(|| {
            RigError::Invalid(format!(
                "CAF controller CRC {:08x} has no exact skeleton match",
                controller.bone_crc32
            ))
        })?;
        if !seen.insert(bone) {
            return Err(RigError::Invalid(
                "Duplicate native CAF controller for one bone".into(),
            ));
        }
        let node = first + bone;
        let (translation, rotation) = rigid_trs(&doc["nodes"][node]["matrix"])?;
        doc["nodes"][node]
            .as_object_mut()
            .ok_or(RigError::Limits)?
            .remove("matrix");
        doc["nodes"][node]["translation"] = json!(translation);
        doc["nodes"][node]["rotation"] = json!(rotation);
        doc["nodes"][node]["scale"] = json!([1.0, 1.0, 1.0]);
        if !controller.rotation_times.is_empty() {
            let mut values = Vec::with_capacity(controller.rotations.len() * 4);
            for q in &controller.rotations {
                let len = q.iter().map(|x| x * x).sum::<f32>().sqrt();
                if !(0.95..=1.05).contains(&len) {
                    return Err(RigError::Invalid("Invalid source CAF quaternion".into()));
                }
                values.extend(q.iter().map(|x| x / len));
            }
            add_native_channel(
                doc,
                binary,
                &mut samplers,
                &mut channels,
                node,
                "rotation",
                &controller.rotation_times,
                &values,
                4,
                motion.start_frame,
                motion.fps,
            )?;
        }
        if !controller.position_times.is_empty() {
            let values = controller
                .positions
                .iter()
                .flat_map(|p| p.iter().copied())
                .collect::<Vec<_>>();
            add_native_channel(
                doc,
                binary,
                &mut samplers,
                &mut channels,
                node,
                "translation",
                &controller.position_times,
                &values,
                3,
                motion.start_frame,
                motion.fps,
            )?;
        }
    }
    if channels.is_empty() {
        return Err(RigError::Invalid(
            "Native CAF contained no verified animated channels".into(),
        ));
    }
    doc["animations"] =
        json!([{"name":"Verified original native CAF","samplers":samplers,"channels":channels}]);
    doc["extras"]["animations_embedded"] = json!(true);
    Ok(())
}

/// Build a fully skinned GLB ONLY from verified source original vertex,
/// bone-map and exact matching compiled skeleton. Preserves all raw UV0.
pub fn rigged_glb_bytes(
    mesh: &RenderMesh,
    mesh_source: &[u8],
    skeleton_source: &[u8],
) -> Result<(Vec<u8>, RigReport), RigError> {
    rigged_glb_bytes_internal(mesh, mesh_source, skeleton_source, None)
}

/// Embed one exact non-additive native CAF in a rigorously verified skin.
pub fn rigged_glb_bytes_with_caf(
    mesh: &RenderMesh,
    mesh_source: &[u8],
    skeleton_source: &[u8],
    animation: &NativeCafMotion,
) -> Result<(Vec<u8>, RigReport), RigError> {
    rigged_glb_bytes_internal(mesh, mesh_source, skeleton_source, Some(animation))
}

fn rigged_glb_bytes_internal(
    mesh: &RenderMesh,
    mesh_source: &[u8],
    skeleton_source: &[u8],
    animation: Option<&NativeCafMotion>,
) -> Result<(Vec<u8>, RigReport), RigError> {
    let historical = mesh_source.starts_with(b"CryTek\0\0");
    if historical && animation.is_some() {
        return Err(RigError::Invalid(
            "2012 DBA/CAF requires verified historical controller conversion before animation export".into(),
        ));
    }
    let (skeleton, mapped) = if historical {
        crate::legacy_rig::read_legacy_skin(
            mesh_source, skeleton_source, mesh.positions.len(),
        )?
    } else {
        let skeleton = parse_compiled_skeleton(skeleton_source)?;
        let mapped = remap_verified_weights(mesh_source, mesh.positions.len(), skeleton.names.len())?;
        (skeleton, mapped)
    };
    let source = geometry_glb_bytes(mesh)?;
    let len = u32::from_le_bytes(source[12..16].try_into().map_err(|_| RigError::Limits)?) as usize;
    let mut document: Value =
        serde_json::from_slice(source.get(20..20 + len).ok_or(RigError::Limits)?)?;
    if document["nodes"].as_array().is_none_or(|n| n.len() != 1)
        || document["skins"].is_array()
        || document["meshes"][0]["primitives"][0]["attributes"]["POSITION"].is_null()
    {
        return Err(RigError::Invalid(
            "source geometry is not one verified mesh".into(),
        ));
    }
    let bin_start = 20 + len + 8;
    let mut binary = source.get(bin_start..).ok_or(RigError::Limits)?.to_vec();
    let count = mesh.positions.len();
    let mut ids = vec![];
    let mut weights = vec![];
    for (j, w) in mapped.joint_ids.iter().zip(&mapped.weight_bytes) {
        for id in &j[..4] {
            ids.extend_from_slice(&id.to_le_bytes());
        }
        weights.extend_from_slice(&w[..4]);
    }
    let joint_view = add_view(&mut binary, &mut document, &ids, Some(34962))?;
    let weight_view = add_view(&mut binary, &mut document, &weights, Some(34962))?;
    let joint_acc = add_accessor(&mut document, joint_view, 5123, count, "VEC4", false)?;
    let weight_acc = add_accessor(&mut document, weight_view, 5121, count, "VEC4", true)?;
    document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(joint_acc);
    document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(weight_acc);
    if mapped.influences == 8 {
        let mut ids = Vec::new();
        let mut weights = Vec::new();
        for (j, w) in mapped.joint_ids.iter().zip(&mapped.weight_bytes) {
            for id in &j[4..8] {
                ids.extend_from_slice(&id.to_le_bytes());
            }
            weights.extend_from_slice(&w[4..8]);
        }
        let id_view = add_view(&mut binary, &mut document, &ids, Some(34962))?;
        let weight_view = add_view(&mut binary, &mut document, &weights, Some(34962))?;
        let id_acc = add_accessor(&mut document, id_view, 5123, count, "VEC4", false)?;
        let weight_acc = add_accessor(&mut document, weight_view, 5121, count, "VEC4", true)?;
        document["meshes"][0]["primitives"][0]["attributes"]["JOINTS_1"] = json!(id_acc);
        document["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_1"] = json!(weight_acc);
    }
    let mut binds = Vec::with_capacity(skeleton.names.len() * 64);
    for inverse in &skeleton.inverse {
        for v in column_major(inverse) {
            binds.extend_from_slice(&v.to_le_bytes());
        }
    }
    let bind_view = add_view(&mut binary, &mut document, &binds, None)?;
    let bind_acc = add_accessor(
        &mut document,
        bind_view,
        5126,
        skeleton.names.len(),
        "MAT4",
        false,
    )?;
    let first = document["nodes"].as_array().ok_or(RigError::Limits)?.len();
    for (i, name) in skeleton.names.iter().enumerate() {
        let transform = if let Some(parent) = skeleton.parents[i] {
            multiply(&skeleton.inverse[parent], &skeleton.world[i])
        } else {
            skeleton.world[i]
        };
        let node = json!({"name":name,"matrix":column_major(&transform)});
        document["nodes"]
            .as_array_mut()
            .ok_or(RigError::Limits)?
            .push(node);
    }
    let mut roots = Vec::new();
    for (i, p) in skeleton.parents.iter().enumerate() {
        if let Some(parent) = p {
            let index = first + parent;
            if document["nodes"][index]["children"].is_null() {
                document["nodes"][index]["children"] = json!([]);
            }
            document["nodes"][index]["children"]
                .as_array_mut()
                .ok_or(RigError::Limits)?
                .push(json!(first + i));
        } else {
            roots.push(first + i);
        }
    }
    if roots.is_empty() { return Err(RigError::Limits); }
    let mut skin = json!({
        "name":"Original CryEngine bind pose",
        "joints":(0..skeleton.names.len()).map(|i|first+i).collect::<Vec<_>>(),
        "inverseBindMatrices":bind_acc
    });
    if roots.len() == 1 {
        skin["skeleton"] = json!(roots[0]);
    }
    document["skins"] = json!([skin]);
    document["nodes"][0]["skin"] = json!(0);
    // Rotate BOTH skin source skeleton and mesh together, preserving
    // original CryEngine Z-up transforms rather than rotating only bones.
    let basis_index = document["nodes"].as_array().ok_or(RigError::Limits)?.len();
    document["nodes"]
        .as_array_mut()
        .ok_or(RigError::Limits)?
        .push(json!({
            "name":"Source CryEngine Z-up to Blender Y-up",
            "rotation":[-std::f32::consts::FRAC_1_SQRT_2,0,0,std::f32::consts::FRAC_1_SQRT_2],
            "children":std::iter::once(0).chain(roots.iter().copied()).collect::<Vec<_>>()
        }));
    document["scenes"][0]["nodes"] = json!([basis_index]);
    document["extras"] = json!({
        "rigging":if historical {
            "Original verified 2012 CryTek bones, four influences, and per-subset palettes"
        } else {
            "Original verified compiled bones, inverse bind matrices and subset-local bone palettes"
        },
        "animations_embedded":false
    });
    if let Some(motion) = animation {
        attach_native_caf(&mut document, &mut binary, &skeleton, first, motion)?;
    }
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    let mut json_data = serde_json::to_vec(&document)?;
    while !json_data.len().is_multiple_of(4) {
        json_data.push(b' ');
    }
    let total = 12usize
        .checked_add(8)
        .and_then(|v| v.checked_add(json_data.len()))
        .and_then(|v| v.checked_add(8))
        .and_then(|v| v.checked_add(binary.len()))
        .ok_or(RigError::Limits)?;
    if total > MAX_BYTES * 4 || total > u32::MAX as usize {
        return Err(RigError::Limits);
    }
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json_data.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json_data);
    output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&binary);
    let active = mapped
        .joint_ids
        .iter()
        .zip(&mapped.weight_bytes)
        .flat_map(|(ids, weights)| {
            (0..mapped.influences)
                .filter_map(move |i| if weights[i] > 0 { Some(ids[i]) } else { None })
        })
        .collect::<HashSet<_>>()
        .len();
    Ok((
        output,
        RigReport {
            bone_count: skeleton.names.len(),
            weighted_vertices: count,
            active_bones: active,
            subset_count: mapped.subsets,
            influences_per_vertex: mapped.influences,
            animations_included: animation.is_some(),
        },
    ))
}
pub fn export_rigged_character_glb(
    mesh_source: &Path,
    skeleton_source: &Path,
    destination: &Path,
) -> Result<RigReport, RigError> {
    for file in [mesh_source, skeleton_source] {
        let stat = fs::symlink_metadata(file)?;
        if !stat.file_type().is_file() || stat.len() > MAX_BYTES as u64 {
            return Err(RigError::Limits);
        }
    }
    let mesh = read_model(mesh_source)?;
    let mesh_data = fs::read(mesh_source)?;
    let bones = fs::read(skeleton_source)?;
    let (out, report) = rigged_glb_bytes(&mesh, &mesh_data, &bones)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                RigError::Exists
            } else {
                RigError::Io(e)
            }
        })?;
    if let Err(error) = file.write_all(&out).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(destination);
        return Err(RigError::Io(error));
    }
    Ok(report)
}
/// Attach a real source DDS diffuse to the already verified rigged GLB,
/// with the highest decoded DDS mip embedded in the GLB BIN chunk. The
/// original UV0 accessor and all skeleton/accessor data are untouched.
/// Bind uniquely resolved original DDS image layers to the verified UV0
/// without touching any skeleton, animation or geometry accessor. Images have
/// been decoded to PNG by the caller; ambiguous material slots fail upstream.
pub(crate) fn embed_verified_maps_png_glb(
    source: &[u8],
    diffuse: Option<&[u8]>,
    normal: Option<&[u8]>,
    emissive: Option<&[u8]>,
) -> Result<Vec<u8>, RigError> {
    embed_verified_material_png_glb(source, diffuse, normal, emissive, None, None)
}

/// Keep authored specular color distinct from explicit gloss/roughness.
pub(crate) fn embed_verified_material_png_glb(
    source: &[u8],
    diffuse: Option<&[u8]>,
    normal: Option<&[u8]>,
    emissive: Option<&[u8]>,
    specular: Option<&[u8]>,
    roughness: Option<&[u8]>,
) -> Result<Vec<u8>, RigError> {
    if source.len() < 28
        || &source[..4] != b"glTF"
        || [diffuse, normal, emissive, specular, roughness]
            .iter()
            .all(|map| map.is_none())
    {
        return Err(RigError::Invalid(
            "Expected a rigged GLB and source PNG map".into(),
        ));
    }
    let json_size =
        u32::from_le_bytes(source[12..16].try_into().map_err(|_| RigError::Limits)?) as usize;
    let document_end = 20usize.checked_add(json_size).ok_or(RigError::Limits)?;
    let mut document: Value =
        serde_json::from_slice(source.get(20..document_end).ok_or(RigError::Limits)?)?;
    if document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"].is_null()
        || document["skins"]
            .as_array()
            .is_none_or(|skins| skins.is_empty())
        || document["images"]
            .as_array()
            .is_some_and(|images| !images.is_empty())
        || document["buffers"][0]["byteLength"].as_u64().is_none()
    {
        return Err(RigError::Invalid(
            "Cannot bind images to an unverified rig/UV0".into(),
        ));
    }
    let bin_at = document_end.checked_add(8).ok_or(RigError::Limits)?;
    if source.get(document_end + 4..document_end + 8) != Some(b"BIN\0") {
        return Err(RigError::Invalid("Missing GLB binary chunk".into()));
    }
    let mut binary = source.get(bin_at..).ok_or(RigError::Limits)?.to_vec();
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut material = json!({
        "name":"Original authored CryEngine material slot",
        "pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}
    });
    for (role, data) in [
        ("diffuse", diffuse),
        ("normal", normal),
        ("emissive", emissive),
        ("specular", specular),
        ("roughness", roughness),
    ] {
        let Some(png) = data else { continue };
        if png.is_empty() {
            return Err(RigError::Invalid(
                "Original authored map has no PNG bytes".into(),
            ));
        }
        let view = add_view(&mut binary, &mut document, png, None)?;
        images.push(
            json!({"name":format!("Original {role} DDS"),"mimeType":"image/png","bufferView":view}),
        );
        let index = textures.len();
        textures.push(json!({"source":index}));
        match role {
            "diffuse" => {
                material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":index})
            }
            "normal" => material["normalTexture"] = json!({"index":index}),
            "emissive" => {
                material["emissiveTexture"] = json!({"index":index});
                material["emissiveFactor"] = json!([1.0, 1.0, 1.0]);
            }
            "specular" => {
                material["extensions"]["KHR_materials_specular"]["specularColorTexture"] =
                    json!({"index":index});
                material["extensions"]["KHR_materials_specular"]["specularColorFactor"] =
                    json!([1.0, 1.0, 1.0]);
                document["extensionsUsed"] = json!(["KHR_materials_specular"]);
            }
            "roughness" => {
                material["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
                    json!({"index":index});
            }
            _ => unreachable!(),
        }
    }
    document["images"] = json!(images);
    document["textures"] = json!(textures);
    document["materials"] = json!([material]);
    document["meshes"][0]["primitives"][0]["material"] = json!(0);
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    document["buffers"][0]["byteLength"] = json!(binary.len());
    let mut json_bytes = serde_json::to_vec(&document)?;
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let full = 28usize
        .checked_add(json_bytes.len())
        .and_then(|n| n.checked_add(binary.len()))
        .ok_or(RigError::Limits)?;
    if full > MAX_BYTES * 4 || full > u32::MAX as usize {
        return Err(RigError::Limits);
    }
    let mut output = Vec::with_capacity(full);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(full as u32).to_le_bytes());
    output.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json_bytes);
    output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&binary);
    Ok(output)
}

/// Exact source MTL leaf (including empty and NoDraw leaves). The caller
/// resolves each DDS through the indexed asset library, never by basename.
#[derive(Default)]
pub(crate) struct OriginalMaterialPng {
    pub name: String,
    pub no_draw: bool,
    pub diffuse: Option<Vec<u8>>,
    pub normal: Option<Vec<u8>>,
    pub emissive: Option<Vec<u8>>,
    pub specular: Option<Vec<u8>>,
    pub roughness: Option<Vec<u8>>,
}

/// Split ONE verified rigged GLB into original material-ID primitives, reusing
/// the source index buffer, original UV0 and all skin joint/weight attributes.
/// Each authored MTL leaf gets its own original texture set. A NoDraw leaf
/// remains an identifiable, fully transparent material instead of moving
/// subsequent CryEngine subset indices or deleting original geometry.
pub(crate) fn embed_original_material_slots_glb(
    source: &[u8],
    subsets: &[(u32, u32, u32)],
    slots: &[OriginalMaterialPng],
) -> Result<Vec<u8>, RigError> {
    if source.len() < 28
        || source.get(..4) != Some(b"glTF")
        || slots.is_empty()
        || slots.len() > 128
        || subsets.is_empty()
        || subsets.len() > 128
    {
        return Err(RigError::Invalid(
            "Invalid verified multi-slot FBX source".into(),
        ));
    }
    let json_size =
        u32::from_le_bytes(source[12..16].try_into().map_err(|_| RigError::Limits)?) as usize;
    let json_end = 20usize.checked_add(json_size).ok_or(RigError::Limits)?;
    let mut doc: Value = serde_json::from_slice(source.get(20..json_end).ok_or(RigError::Limits)?)?;
    if source.get(json_end + 4..json_end + 8) != Some(b"BIN\0") {
        return Err(RigError::Invalid(
            "Rigged GLB has no valid BIN chunk".into(),
        ));
    }
    let primitives = doc["meshes"][0]["primitives"]
        .as_array()
        .ok_or_else(|| RigError::Invalid("Rigged geometry has no primitives".into()))?;
    if primitives.len() != 1 || doc["skins"].as_array().is_none_or(|v| v.len() != 1) {
        return Err(RigError::Invalid(
            "Expected one verified source skinned mesh".into(),
        ));
    }
    let base = primitives[0].clone();
    for attr in ["POSITION", "TEXCOORD_0", "JOINTS_0", "WEIGHTS_0"] {
        if base["attributes"][attr].as_u64().is_none() {
            return Err(RigError::Invalid(
                "Rigged GLB has incomplete source attributes".into(),
            ));
        }
    }
    let accessor_id = base["indices"].as_u64().ok_or(RigError::Limits)? as usize;
    let index_acc = doc["accessors"].get(accessor_id).ok_or(RigError::Limits)?;
    let total = index_acc["count"].as_u64().ok_or(RigError::Limits)? as u32;
    let view = index_acc["bufferView"].as_u64().ok_or(RigError::Limits)?;
    if index_acc["componentType"] != 5125
        || index_acc["type"] != "SCALAR"
        || index_acc["byteOffset"].as_u64().unwrap_or(0) != 0
    {
        return Err(RigError::Invalid(
            "Source index buffer is not UINT32 triangle data".into(),
        ));
    }
    let mut offset = 0u32;
    let mut rewritten = Vec::with_capacity(subsets.len());
    for &(first, count, material) in subsets {
        let last = first.checked_add(count).ok_or(RigError::Limits)?;
        if first != offset
            || count == 0
            || !count.is_multiple_of(3)
            || last > total
            || material == 255
            || material as usize >= slots.len()
        {
            return Err(RigError::Invalid(
                "Unverified CryEngine subset material index".into(),
            ));
        }
        let mut primitive = base.clone();
        let accessors = doc["accessors"].as_array_mut().ok_or(RigError::Limits)?;
        let sub = accessors.len();
        accessors.push(json!({
            "bufferView":view, "byteOffset":first as u64 * 4,
            "componentType":5125, "count":count, "type":"SCALAR"
        }));
        primitive["indices"] = json!(sub);
        primitive["material"] = json!(material);
        rewritten.push(primitive);
        offset = last;
    }
    if offset != total {
        return Err(RigError::Invalid(
            "CDF multi-material subsets do not cover mesh".into(),
        ));
    }

    let mut binary = source.get(json_end + 8..).ok_or(RigError::Limits)?.to_vec();
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut materials = Vec::with_capacity(slots.len());
    let mut any_specular = false;
    for (slot_index, slot) in slots.iter().enumerate() {
        if slot.name.is_empty() || slot.name.len() > 256 || slot.name.chars().any(char::is_control)
        {
            return Err(RigError::Invalid("Unsafe source MTL leaf name".into()));
        }
        let mut material = json!({
            "name": format!("CryEngine slot {slot_index:03} - {}", slot.name),
            "doubleSided":true,
            "pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}
        });
        if slot.no_draw {
            material["alphaMode"] = json!("BLEND");
            material["pbrMetallicRoughness"]["baseColorFactor"] = json!([1.0, 1.0, 1.0, 0.0]);
        }
        for (role, png) in [
            ("diffuse", slot.diffuse.as_deref()),
            ("normal", slot.normal.as_deref()),
            ("emissive", slot.emissive.as_deref()),
            ("specular", slot.specular.as_deref()),
            ("roughness", slot.roughness.as_deref()),
        ] {
            let Some(bytes) = png else { continue };
            if bytes.len() < 8 || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
                return Err(RigError::Invalid(
                    "Authored multi-slot map is not PNG".into(),
                ));
            }
            let image_view = add_view(&mut binary, &mut doc, bytes, None)?;
            let image_index = images.len();
            images.push(json!({
                "name": format!("CryEngine {slot_index} {role}"),
                "mimeType":"image/png", "bufferView":image_view
            }));
            let texture_index = textures.len();
            textures.push(json!({"source":image_index}));
            match role {
                "diffuse" => {
                    material["pbrMetallicRoughness"]["baseColorTexture"] =
                        json!({"index":texture_index})
                }
                "normal" => material["normalTexture"] = json!({"index":texture_index}),
                "emissive" => {
                    material["emissiveTexture"] = json!({"index":texture_index});
                    material["emissiveFactor"] = json!([1., 1., 1.]);
                }
                "specular" => {
                    any_specular = true;
                    material["extensions"]["KHR_materials_specular"]["specularColorTexture"] =
                        json!({"index":texture_index});
                    material["extensions"]["KHR_materials_specular"]["specularColorFactor"] =
                        json!([1., 1., 1.]);
                }
                "roughness" => {
                    material["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
                        json!({"index":texture_index})
                }
                _ => unreachable!(),
            }
        }
        materials.push(material);
    }
    if any_specular {
        doc["extensionsUsed"] = json!(["KHR_materials_specular"]);
    }
    doc["meshes"][0]["primitives"] = json!(rewritten);
    doc["materials"] = json!(materials);
    doc["images"] = json!(images);
    doc["textures"] = json!(textures);
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    doc["buffers"][0]["byteLength"] = json!(binary.len());
    let mut json_blob = serde_json::to_vec(&doc)?;
    while !json_blob.len().is_multiple_of(4) {
        json_blob.push(b' ');
    }
    let total = 28usize
        .checked_add(json_blob.len())
        .and_then(|n| n.checked_add(binary.len()))
        .ok_or(RigError::Limits)?;
    if total > MAX_BYTES * 4 || total > u32::MAX as usize {
        return Err(RigError::Limits);
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_blob.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_blob);
    out.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&binary);
    Ok(out)
}

fn embed_diffuse_png_glb(source: &[u8], png: &[u8]) -> Result<Vec<u8>, RigError> {
    embed_verified_maps_png_glb(source, Some(png), None, None)
}

/// Produce the same verified skinned FBX as the ordinary export, but embed an
/// explicitly selected original diffuse DDS if provided. No filenames are
/// guessed; multi-material / normal / emissive maps require authored .mtl
/// resolution and remain outside this single-slot export.
pub fn export_rigged_character_fbx_textured(
    mesh_source: &Path,
    skeleton_source: &Path,
    blender: &Path,
    diffuse: Option<&Path>,
    destination: &Path,
) -> Result<(RigReport, FbxReport), RigError> {
    export_rigged_character_fbx_textured_animated(
        mesh_source,
        skeleton_source,
        blender,
        diffuse,
        None,
        destination,
    )
}

/// Strict opt-in CAF: do not claim embedded animations if source bone CRCs
/// cannot be paired uniquely to the original compiled skeleton.
pub fn export_rigged_character_fbx_textured_animated(
    mesh_source: &Path,
    skeleton_source: &Path,
    blender: &Path,
    diffuse: Option<&Path>,
    native_caf: Option<&Path>,
    destination: &Path,
) -> Result<(RigReport, FbxReport), RigError> {
    for file in [mesh_source, skeleton_source] {
        let stat = fs::symlink_metadata(file)?;
        if !stat.file_type().is_file() || stat.len() > MAX_BYTES as u64 {
            return Err(RigError::Limits);
        }
    }
    let mesh = read_model(mesh_source)?;
    let mesh_data = fs::read(mesh_source)?;
    let skeleton_data = fs::read(skeleton_source)?;
    let motion = native_caf
        .map(|file| {
            crate::native_caf::read_native_caf(file)
                .map_err(|e| RigError::Invalid(format!("Original native CAF: {e}")))
        })
        .transpose()?;
    let (mut output, report) = if let Some(motion) = &motion {
        rigged_glb_bytes_with_caf(&mesh, &mesh_data, &skeleton_data, motion)?
    } else {
        rigged_glb_bytes(&mesh, &mesh_data, &skeleton_data)?
    };
    if let Some(texture_source) = diffuse {
        let (_info, rgba) = crate::dds::decode_dds_rgba(texture_source)
            .map_err(|e| RigError::Invalid(format!("Source diffuse DDS: {e}")))?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| RigError::Invalid(format!("Embedded DDS PNG: {e}")))?;
        output = embed_diffuse_png_glb(&output, &png)?;
    }
    let temp = tempfile::tempdir()?;
    let glb = temp.path().join("verified-textured-rig.glb");
    fs::write(&glb, &output)?;
    let exported = convert_glb_to_fbx(
        blender,
        &glb,
        destination,
        AnimationPolicy {
            embed_verified_animations: motion.is_some(),
        },
    )?;
    Ok((report, exported))
}

pub fn export_rigged_character_fbx(
    mesh_source: &Path,
    skeleton_source: &Path,
    blender: &Path,
    destination: &Path,
) -> Result<(RigReport, FbxReport), RigError> {
    export_rigged_character_fbx_textured(mesh_source, skeleton_source, blender, None, destination)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn identity_record(name: &str) -> Vec<u8> {
        let mut data = vec![0u8; RECORD];
        data[..4].copy_from_slice(&crc32fast::hash(name.as_bytes()).to_le_bytes());
        for base in [216, 264] {
            for r in 0..3 {
                data[base + (r * 4 + r) * 4..base + (r * 4 + r) * 4 + 4]
                    .copy_from_slice(&1f32.to_le_bytes());
            }
        }
        data[312..312 + name.len()].copy_from_slice(name.as_bytes());
        data
    }
    fn sample_skeleton() -> Vec<u8> {
        let mut source = vec![0u8; 32];
        source[..8].copy_from_slice(b"CrChF\x07\0\0");
        source[8..12].copy_from_slice(&1u32.to_le_bytes());
        source[12..16].copy_from_slice(&16u32.to_le_bytes());
        source[16..20].copy_from_slice(&BONES.to_le_bytes());
        source[20..24].copy_from_slice(&1u32.to_le_bytes());
        source[24..28].copy_from_slice(&(32 + RECORD as u32).to_le_bytes());
        source[28..32].copy_from_slice(&32u32.to_le_bytes());
        source.extend_from_slice(&[0u8; 32]);
        source.extend_from_slice(&identity_record("Root"));
        source
    }

    #[test]
    fn compiled_controller_ids_are_preserved_and_duplicates_rejected() {
        let mut raw = sample_skeleton();
        raw[64..68].copy_from_slice(&0x12345678u32.to_le_bytes());
        let skeleton = parse_compiled_skeleton(&raw).unwrap();
        assert_eq!(skeleton.controller_ids, vec![0x12345678]);
        assert_eq!(skeleton.names, vec!["Root"]);
        let mut child = identity_record("Child");
        child[..4].copy_from_slice(&0x12345678u32.to_le_bytes());
        child[572..576].copy_from_slice(&(-1i32).to_le_bytes());
        raw.extend(child);
        raw[24..28].copy_from_slice(&(32 + 2 * RECORD as u32).to_le_bytes());
        assert!(
            matches!(parse_compiled_skeleton(&raw), Err(RigError::Invalid(message)) if message.contains("Duplicate compiled controller"))
        );
    }

    #[test]
    fn verified_native_caf_becomes_original_skeleton_animation_channels() {
        use crate::native_caf::{CafController, NativeCafMotion};
        let skeleton = parse_compiled_skeleton(&sample_skeleton()).unwrap();
        let mut doc = json!({
            "nodes":[{}, {"name":"Root","matrix":[
                1.0,0.0,0.0,0.0,
                0.0,1.0,0.0,0.0,
                0.0,0.0,1.0,0.0,
                0.0,0.0,0.0,1.0
            ]}],
            "bufferViews":[], "accessors":[], "extras":{"animations_embedded":false}
        });
        let controller = CafController {
            bone_crc32: crc32fast::hash(b"Root"),
            rotations: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]],
            rotation_times: vec![0.0, 30.0],
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            position_times: vec![0.0, 30.0],
        };
        let motion = NativeCafMotion {
            schema: 1,
            controllers: vec![controller],
            start_frame: 0.0,
            end_frame: 30.0,
            fps: 30.0,
            additive: false,
            cyclic: false,
        };
        let mut binary = Vec::new();
        attach_native_caf(&mut doc, &mut binary, &skeleton, 1, &motion).unwrap();
        assert_eq!(
            doc["animations"][0]["channels"].as_array().unwrap().len(),
            2
        );
        assert_eq!(doc["animations"][0]["channels"][0]["target"]["node"], 1);
        assert!(doc["nodes"][1]["matrix"].is_null());
        assert!(doc["extras"]["animations_embedded"].as_bool().unwrap());
        assert!(!binary.is_empty());
        let mut bad = motion.clone();
        bad.controllers[0].bone_crc32 = 0x1234;
        let mut copy = json!({"nodes":[{}, {"matrix":[
            1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,
            0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0
        ]}],"bufferViews":[],"accessors":[]});
        assert!(attach_native_caf(&mut copy, &mut Vec::new(), &skeleton, 1, &bad).is_err());
    }

    #[test]
    fn embeds_png_bytes_without_changing_authored_uv_accessor() {
        let doc = json!({
            "meshes":[{"primitives":[{"attributes":{"TEXCOORD_0":3}}]}],
            "skins":[{"joints":[0]}],
            "buffers":[{"byteLength":4}],
            "bufferViews":[],
            "images":[]
        });
        let mut encoded = serde_json::to_vec(&doc).unwrap();
        while !encoded.len().is_multiple_of(4) {
            encoded.push(b' ');
        }
        let size = 28 + encoded.len() + 4;
        let mut input = Vec::new();
        input.extend_from_slice(b"glTF");
        input.extend_from_slice(&2u32.to_le_bytes());
        input.extend_from_slice(&(size as u32).to_le_bytes());
        input.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        input.extend_from_slice(b"JSON");
        input.extend_from_slice(&encoded);
        input.extend_from_slice(&4u32.to_le_bytes());
        input.extend_from_slice(b"BIN\0");
        input.extend_from_slice(&[1, 2, 3, 4]);
        let image = b"synthetic-png-payload";
        let output = embed_diffuse_png_glb(&input, image).unwrap();
        assert_eq!(
            u32::from_le_bytes(output[8..12].try_into().unwrap()) as usize,
            output.len()
        );
        let json_len = u32::from_le_bytes(output[12..16].try_into().unwrap()) as usize;
        let result: Value = serde_json::from_slice(&output[20..20 + json_len]).unwrap();
        assert_eq!(
            result["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"],
            3
        );
        assert_eq!(result["images"][0]["mimeType"], "image/png");
        assert_eq!(
            result["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
        let off = 20 + json_len + 8 + 4;
        assert_eq!(&output[off..off + image.len()], image);
        assert_eq!(&input[input.len() - 4..], &[1, 2, 3, 4]);
        let full = embed_verified_maps_png_glb(
            &input,
            Some(b"diffuse"),
            Some(b"normal"),
            Some(b"emissive"),
        )
        .unwrap();
        let n = u32::from_le_bytes(full[12..16].try_into().unwrap()) as usize;
        let result: Value = serde_json::from_slice(&full[20..20 + n]).unwrap();
        assert_eq!(result["images"].as_array().unwrap().len(), 3);
        assert_eq!(result["materials"][0]["normalTexture"]["index"], 1);
        assert_eq!(result["materials"][0]["emissiveTexture"]["index"], 2);
        assert_eq!(
            result["materials"][0]["emissiveFactor"],
            json!([1.0, 1.0, 1.0])
        );
        assert_eq!(
            result["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"],
            3
        );
        assert!(embed_verified_maps_png_glb(&input, None, None, None).is_err());
        let extended = embed_verified_material_png_glb(
            &input,
            None,
            None,
            None,
            Some(b"specular"),
            Some(b"roughness"),
        )
        .unwrap();
        let length = u32::from_le_bytes(extended[12..16].try_into().unwrap()) as usize;
        let rendered: Value = serde_json::from_slice(&extended[20..20 + length]).unwrap();
        assert_eq!(rendered["extensionsUsed"][0], "KHR_materials_specular");
        assert_eq!(
            rendered["materials"][0]["extensions"]["KHR_materials_specular"]["specularColorTexture"]
                ["index"],
            0
        );
        assert_eq!(
            rendered["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"],
            1
        );
        assert!(rendered["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"].is_null());
        assert_eq!(
            rendered["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"],
            3
        );
    }
    #[test]
    fn original_submaterial_primitives_keep_exact_ids_and_skin_accessors() {
        let mesh = RenderMesh {
            info: crate::model::ModelInfo {
                format: "synthetic".into(),
                version: 7,
                chunk_count: 0,
                vertex_count: 3,
                triangle_count: 2,
                original_uv0: true,
                original_normals: false,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2, 0, 2, 1],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: None,
        };
        let geometry = crate::glb::geometry_glb_bytes(&mesh).unwrap();
        let json_len = u32::from_le_bytes(geometry[12..16].try_into().unwrap()) as usize;
        let mut doc: Value = serde_json::from_slice(&geometry[20..20 + json_len]).unwrap();
        // Structural fixture only. This is not an actual native weighted rig:
        // the production exporter generates real JOINTS/WEIGHTS from CrChF.
        doc["skins"] = json!([{"joints":[0]}]);
        doc["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(0);
        doc["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(0);
        let binary = &geometry[20 + json_len + 8..];
        let mut json_bytes = serde_json::to_vec(&doc).unwrap();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        let mut source = Vec::new();
        source.extend_from_slice(b"glTF");
        source.extend_from_slice(&2u32.to_le_bytes());
        source.extend_from_slice(&((28 + json_bytes.len() + binary.len()) as u32).to_le_bytes());
        source.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        source.extend_from_slice(b"JSON");
        source.extend_from_slice(&json_bytes);
        source.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        source.extend_from_slice(b"BIN\0");
        source.extend_from_slice(binary);
        let slots = vec![
            OriginalMaterialPng {
                name: "hidden".into(),
                no_draw: true,
                ..Default::default()
            },
            OriginalMaterialPng {
                name: "armor".into(),
                diffuse: Some(b"\x89PNG\r\n\x1a\n".to_vec()),
                ..Default::default()
            },
        ];
        let out =
            embed_original_material_slots_glb(&source, &[(0, 3, 0), (3, 3, 1)], &slots).unwrap();
        let len = u32::from_le_bytes(out[12..16].try_into().unwrap()) as usize;
        let result: Value = serde_json::from_slice(&out[20..20 + len]).unwrap();
        let primitives = result["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), 2);
        assert_eq!(primitives[0]["material"], 0);
        assert_eq!(primitives[1]["material"], 1);
        assert_eq!(primitives[0]["attributes"]["JOINTS_0"], 0);
        assert_eq!(primitives[1]["attributes"]["TEXCOORD_0"], 1);
        assert_eq!(result["materials"][0]["alphaMode"], "BLEND");
        assert_eq!(
            result["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
        assert_eq!(
            result["accessors"][primitives[1]["indices"].as_u64().unwrap() as usize]["byteOffset"],
            12
        );
        assert!(
            embed_original_material_slots_glb(&source, &[(0, 3, 0), (6, 3, 1)], &slots).is_err()
        );
        assert!(
            embed_original_material_slots_glb(&source, &[(0, 3, 0), (3, 3, 255)], &slots).is_err()
        );
    }

    #[test]
    fn verifies_inverse_bind_and_unchanged_hierarchy() {
        let raw = sample_skeleton();
        let bones = parse_compiled_skeleton(&raw).unwrap();
        assert_eq!(bones.names, ["Root"]);
        assert_eq!(bones.parents, [None]);
        let mut bad = raw;
        let at = 32 + 32 + 264;
        bad[at..at + 4].copy_from_slice(&2f32.to_le_bytes());
        assert!(parse_compiled_skeleton(&bad).is_err());
    }
    #[test]
    fn refuses_no_subset_palette_without_guessing_bone_ids() {
        let raw = sample_skeleton();
        let bones = parse_compiled_skeleton(&raw).unwrap();
        assert!(remap_verified_weights(&raw, 3, bones.names.len()).is_err());
    }
}
