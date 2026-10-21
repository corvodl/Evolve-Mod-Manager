//! Validated read-only CryTek/CrChF vertex+triangle stream decoding.
//! Unsupported stream layouts are NOT coerced to plausible-looking meshes;
//! skeletal deformation and node transforms require independent validation.
use half::f16;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self};
use std::io::{self, Write};
use std::path::Path;

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_CHUNKS: usize = 100000;
const MAX_VERTICES: usize = 250000;
const MAX_INDICES: usize = 1_500_000;
const CRCHF_STREAM: u32 = 0x08001016;
const CRYTEK_STREAM: u32 = 0xCCCC0016;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("Invalid CryEngine model stream: {0}")]
    Format(String),
    #[error("Untrusted chunk sizes or geometry exceeds safe limits")]
    Limits,
    #[error("Refusing to overwrite an existing model export")]
    OutputExists,
    #[error("File error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub format: String,
    pub version: u32,
    pub chunk_count: usize,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub original_uv0: bool,
    pub original_normals: bool,
}

#[derive(Clone, Debug)]
pub struct RenderMesh {
    pub info: ModelInfo,
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub uv0: Option<Vec<[f32; 2]>>,
    pub normals: Option<Vec<[f32; 3]>>,
}

fn u32at(data: &[u8], at: usize) -> Result<u32, ModelError> {
    let row = data
        .get(at..at + 4)
        .ok_or_else(|| ModelError::Format("truncated chunk data".into()))?;
    Ok(u32::from_le_bytes([row[0], row[1], row[2], row[3]]))
}
fn endian_u32(data: &[u8], at: usize, big: bool) -> Result<u32, ModelError> {
    let v = u32at(data, at)?;
    Ok(if big { v.swap_bytes() } else { v })
}
fn endian_f32(data: &[u8], at: usize, big: bool) -> Result<f32, ModelError> {
    Ok(f32::from_bits(endian_u32(data, at, big)?))
}

#[derive(Clone, Copy)]
struct Stream {
    offset: usize,
    count: usize,
    stride: usize,
    big_endian: bool,
}
type ModelChunks = (String, u32, Vec<(u32, usize, usize, bool)>);
fn validate_chunks(data: &[u8]) -> Result<ModelChunks, ModelError> {
    if data.len() > MAX_BYTES {
        return Err(ModelError::Limits);
    }
    let mut out = Vec::new();
    let (format, version, end);
    if data.starts_with(b"CrChF") {
        if data.len() < 32 {
            return Err(ModelError::Format("truncated CrChF header".into()));
        }
        version = u32::from_le_bytes([data[5], data[6], data[7], 0]);
        if version != 7 {
            return Err(ModelError::Format("unsupported CrChF version".into()));
        }
        let count = u32at(data, 8)? as usize;
        let start = u32at(data, 12)? as usize;
        end = start
            .checked_add(count.checked_mul(16).ok_or(ModelError::Limits)?)
            .ok_or(ModelError::Limits)?;
        if count == 0 || count > MAX_CHUNKS || start < 16 || end > data.len() {
            return Err(ModelError::Limits);
        }
        let mut ids = HashSet::new();
        for i in 0..count {
            let at = start + i * 16;
            let ty = u32at(data, at)?;
            let id = u32at(data, at + 4)?;
            let len = u32at(data, at + 8)? as usize;
            let offset = u32at(data, at + 12)? as usize;
            if !ids.insert(id)
                || len == 0
                || offset < end
                || offset.checked_add(len).is_none_or(|n| n > data.len())
            {
                return Err(ModelError::Format("invalid CrChF chunk table".into()));
            }
            out.push((ty, offset, len, false));
        }
        format = "CrChF v7".to_string();
    } else if data.starts_with(b"CryTek\0\0") {
        if data.len() < 28 {
            return Err(ModelError::Format("truncated CryTek header".into()));
        }
        let kind = u32at(data, 8)?;
        if kind != 0xffff0000 {
            return Err(ModelError::Format(
                "CryTek file is not render geometry".into(),
            ));
        }
        version = u32at(data, 12)?;
        if version != 0x745 {
            return Err(ModelError::Format(
                "unsupported legacy geometry version".into(),
            ));
        }
        let table = u32at(data, 16)? as usize;
        let count = u32at(data, table)? as usize;
        let start = table.checked_add(4).ok_or(ModelError::Limits)?;
        let end20 = start
            .checked_add(count.checked_mul(20).ok_or(ModelError::Limits)?)
            .ok_or(ModelError::Limits)?;
        // 2012 CryTek releases commonly store a 16-byte chunk directory
        // after the payloads. Checking its bounds as a 20-byte table first
        // rejected valid historical meshes before the legacy branch ran.
        let legacy_terminal =
            start.checked_add(count.checked_mul(16).ok_or(ModelError::Limits)?) == Some(data.len());
        if count == 0
            || count > MAX_CHUNKS
            || table < 20
            || (!legacy_terminal && end20 > data.len())
        {
            return Err(ModelError::Limits);
        }
        end = if legacy_terminal {
            start + count * 16
        } else {
            end20
        };
        let mut ids = HashSet::new();
        if legacy_terminal {
            let mut offsets = Vec::with_capacity(count);
            let mut rows = Vec::with_capacity(count);
            for i in 0..count {
                let at = start + i * 16;
                let ty = u32at(data, at)?;
                let position = u32at(data, at + 8)? as usize;
                let id = u32at(data, at + 12)?;
                if !ids.insert(id) || position < 20 || position >= table {
                    return Err(ModelError::Limits);
                }
                offsets.push(position);
                rows.push((ty, position));
            }
            offsets.sort_unstable();
            if offsets.windows(2).any(|w| w[0] == w[1]) {
                return Err(ModelError::Limits);
            }
            for (ty, pos) in rows {
                let upper = offsets.iter().copied().find(|v| *v > pos).unwrap_or(table);
                out.push((ty, pos, upper - pos, true));
            }
        } else {
            for i in 0..count {
                let at = start + i * 20;
                let ty = u32at(data, at)?;
                let position = u32at(data, at + 8)? as usize;
                let id = u32at(data, at + 12)?;
                let len = u32at(data, at + 16)? as usize;
                if !ids.insert(id)
                    || len == 0
                    || position < end
                    || position.checked_add(len).is_none_or(|n| n > data.len())
                {
                    return Err(ModelError::Limits);
                }
                out.push((ty, position, len, true));
            }
        }
        format = "CryTek 0x745".to_string();
    } else {
        return Err(ModelError::Format("unknown mesh magic".into()));
    }
    let mut ranges = out
        .iter()
        .map(|(_, at, len, _)| (*at, *at + *len))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(ModelError::Format("overlapping chunks".into()));
    }
    Ok((format, version, out))
}

pub fn parse_model_bytes(data: &[u8]) -> Result<RenderMesh, ModelError> {
    let (format, version, chunks) = validate_chunks(data)?;
    let legacy = format.starts_with("CryTek");
    let mut streams: HashMap<u32, Stream> = HashMap::new();
    for (ty, offset, size, _) in &chunks {
        let relevant = if legacy {
            *ty == CRYTEK_STREAM
        } else {
            *ty == CRCHF_STREAM
        };
        if !relevant {
            continue;
        }
        let header_size = if legacy { 40 } else { 24 };
        if *size < header_size {
            return Err(ModelError::Format("truncated stream header".into()));
        }
        let swapped = legacy && u32at(data, *offset + 4)? == 0x80000800;
        if legacy
            && (u32at(data, *offset)? != *ty
                || !matches!(u32at(data, *offset + 4)?, 0x800 | 0x80000800)
                || u32at(data, *offset + 8)? as usize != *offset)
        {
            return Err(ModelError::Format(
                "legacy stream header/table mismatch".into(),
            ));
        }
        let base = offset + if legacy { 16 } else { 0 };
        let stream_type = endian_u32(data, base + 4, swapped)?;
        if !matches!(stream_type, 0 | 1 | 2 | 5 | 15) {
            continue;
        }
        let count = endian_u32(data, base + 8, swapped)? as usize;
        let stride = endian_u32(data, base + 12, swapped)? as usize;
        let a = endian_u32(data, base + 16, swapped)?;
        let b = endian_u32(data, base + 20, swapped)?;
        if count == 0
            || stride == 0
            || (legacy && (endian_u32(data, base, swapped)? != 0 || a != 0 || b != 0))
            || count
                .checked_mul(stride)
                .and_then(|v| v.checked_add(header_size))
                != Some(*size)
        {
            return Err(ModelError::Format(
                "invalid geometry stream dimensions".into(),
            ));
        }
        if streams
            .insert(
                stream_type,
                Stream {
                    offset: offset + header_size,
                    count,
                    stride,
                    big_endian: swapped,
                },
            )
            .is_some()
        {
            return Err(ModelError::Format(
                "ambiguous duplicate vertex/index stream".into(),
            ));
        }
    }
    let packed = !streams.contains_key(&0);
    let positions_stream = streams
        .get(&(if packed { 15 } else { 0 }))
        .ok_or_else(|| ModelError::Format("missing position stream".into()))?;
    let indices_stream = streams
        .get(&5)
        .ok_or_else(|| ModelError::Format("missing triangle stream".into()))?;
    let nv = positions_stream.count;
    let ni = indices_stream.count;
    if !(3..=MAX_VERTICES).contains(&nv)
        || !(3..=MAX_INDICES).contains(&ni)
        || ni % 3 != 0
        || !matches!(indices_stream.stride, 2)
    {
        return Err(ModelError::Limits);
    }
    if !matches!(
        (packed, positions_stream.stride),
        (true, 16) | (false, 8) | (false, 12)
    ) {
        return Err(ModelError::Format("unknown vertex encoding".into()));
    }
    let mut positions = Vec::with_capacity(nv);
    let mut packed_uv = Vec::new();
    for i in 0..nv {
        let at = positions_stream.offset + i * positions_stream.stride;
        let v = if packed {
            let x = f16::from_bits(u16::from_le_bytes(
                data[at..at + 2]
                    .try_into()
                    .map_err(|_| ModelError::Limits)?,
            ))
            .to_f32();
            let y = f16::from_bits(u16::from_le_bytes(
                data[at + 2..at + 4]
                    .try_into()
                    .map_err(|_| ModelError::Limits)?,
            ))
            .to_f32();
            let z = f16::from_bits(u16::from_le_bytes(
                data[at + 4..at + 6]
                    .try_into()
                    .map_err(|_| ModelError::Limits)?,
            ))
            .to_f32();
            let u = f16::from_bits(u16::from_le_bytes(
                data[at + 12..at + 14]
                    .try_into()
                    .map_err(|_| ModelError::Limits)?,
            ))
            .to_f32();
            let v = f16::from_bits(u16::from_le_bytes(
                data[at + 14..at + 16]
                    .try_into()
                    .map_err(|_| ModelError::Limits)?,
            ))
            .to_f32();
            packed_uv.push([u, v]);
            [x, y, z]
        } else if positions_stream.stride == 8 {
            let mut v = [0f32; 4];
            for j in 0..4 {
                v[j] = f16::from_bits(u16::from_le_bytes(
                    data[at + 2 * j..at + 2 * j + 2]
                        .try_into()
                        .map_err(|_| ModelError::Limits)?,
                ))
                .to_f32();
            }
            if !v[3].is_finite() || (v[3] - 1.0).abs() > 0.01 {
                return Err(ModelError::Format(
                    "packed 4th vertex component not 1".into(),
                ));
            }
            [v[0], v[1], v[2]]
        } else {
            [
                endian_f32(data, at, positions_stream.big_endian)?,
                endian_f32(data, at + 4, positions_stream.big_endian)?,
                endian_f32(data, at + 8, positions_stream.big_endian)?,
            ]
        };
        if v.iter()
            .any(|value| !value.is_finite() || value.abs() > 1e7)
        {
            return Err(ModelError::Format("invalid vertex coordinates".into()));
        }
        positions.push(v);
    }
    let indices = (0..ni)
        .map(|i| {
            let at = indices_stream.offset + i * 2;
            if indices_stream.big_endian {
                u16::from_be_bytes([data[at], data[at + 1]]) as u32
            } else {
                u16::from_le_bytes([data[at], data[at + 1]]) as u32
            }
        })
        .collect::<Vec<_>>();
    if indices.iter().any(|v| *v >= nv as u32) {
        return Err(ModelError::Format("triangle index out of range".into()));
    }
    let uv0 = if let Some(stream) = streams.get(&2) {
        if stream.count != nv || stream.stride != 8 {
            return Err(ModelError::Format(
                "original UV0 vertex count or width mismatch".into(),
            ));
        }
        Some(
            (0..nv)
                .map(|i| {
                    let at = stream.offset + i * 8;
                    [
                        endian_f32(data, at, stream.big_endian).unwrap_or(f32::NAN),
                        endian_f32(data, at + 4, stream.big_endian).unwrap_or(f32::NAN),
                    ]
                })
                .collect::<Vec<_>>(),
        )
    } else if packed {
        Some(packed_uv)
    } else {
        None
    };
    if uv0.as_ref().is_some_and(|v| {
        v.iter()
            .any(|xy| xy.iter().any(|c| !c.is_finite() || c.abs() > 1e4))
    }) {
        return Err(ModelError::Format("invalid UV0 values".into()));
    }
    let normals = if let Some(stream) = streams.get(&1) {
        if stream.count == nv && stream.stride == 12 {
            let decoded = (0..nv)
                .map(|i| {
                    let at = stream.offset + i * 12;
                    [
                        endian_f32(data, at, stream.big_endian).unwrap_or(f32::NAN),
                        endian_f32(data, at + 4, stream.big_endian).unwrap_or(f32::NAN),
                        endian_f32(data, at + 8, stream.big_endian).unwrap_or(f32::NAN),
                    ]
                })
                .collect::<Vec<_>>();
            if decoded.iter().all(|v| {
                let len = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                (0.5..1.5).contains(&len)
            }) {
                Some(decoded)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let bounds = (0..3)
        .map(|i| {
            let lo = positions.iter().map(|v| v[i]).fold(f32::INFINITY, f32::min);
            let hi = positions
                .iter()
                .map(|v| v[i])
                .fold(f32::NEG_INFINITY, f32::max);
            hi - lo
        })
        .fold(0.0f32, f32::max);
    if bounds <= 1e-7 {
        return Err(ModelError::Format("degenerate mesh bounds".into()));
    }
    let info = ModelInfo {
        format,
        version,
        chunk_count: chunks.len(),
        vertex_count: nv,
        triangle_count: ni / 3,
        original_uv0: uv0.is_some(),
        original_normals: normals.is_some(),
    };
    Ok(RenderMesh {
        info,
        positions,
        indices,
        uv0,
        normals,
    })
}

/// Whether a verified original CryEngine SUBSETS table was provided.
/// A standalone mesh with multiple MTL leaves but no source table must not
/// paint every triangle with slot zero or guess equipment material IDs.
pub fn has_original_mesh_subset_table(data: &[u8]) -> Result<bool, ModelError> {
    let (_format, _version, chunks) = validate_chunks(data)?;
    Ok(chunks
        .iter()
        .any(|(kind, _, _, _)| matches!(*kind, 0x08001017 | 0xCCCC0017)))
}

/// Verified CryEngine CrChF v7 triangle ranges and the source MTL leaf ID.
/// Each range retains original index ordering, including sparse IDs such as
/// Kala's 11th equipment/emission slot. The sentinel 255 is untextured.
/// Legacy meshes without an original subset table keep slot zero.
pub fn original_mesh_subsets(
    data: &[u8],
    index_count: usize,
) -> Result<Vec<(u32, u32, u32)>, ModelError> {
    if index_count == 0 || index_count > MAX_INDICES || !index_count.is_multiple_of(3) {
        return Err(ModelError::Limits);
    }
    let (format, _, chunks) = validate_chunks(data)?;
    let legacy = format.starts_with("CryTek");
    let subset_kind = if legacy { 0xCCCC0017 } else { 0x08001017 };
    let mut sources = chunks.iter().filter(|(kind, _, _, _)| *kind == subset_kind);
    let Some((_, offset, size, _)) = sources.next() else {
        return Ok(vec![(0, index_count as u32, 0)]);
    };
    if sources.next().is_some() {
        return Err(ModelError::Format("ambiguous original subset table".into()));
    }
    // Historical CryTek 0x745 sections carry their own kind/version/offset/id
    // header, then flags and count at +16/+20 and 36-byte subset records
    // starting at +32. CrChF v7 has a different 16-byte table layout.
    let swapped = legacy && u32at(data, *offset + 4)? == 0x80000800;
    let (count, row_offset) = if legacy {
        if *size < 68
            || u32at(data, *offset)? != subset_kind
            || !matches!(u32at(data, *offset + 4)?, 0x800 | 0x80000800)
            || u32at(data, *offset + 8)? as usize != *offset
        {
            return Err(ModelError::Format(
                "unsupported historical CryTek subset header".into(),
            ));
        }
        let flags = endian_u32(data, *offset + 16, swapped)?;
        if !matches!(flags, 0 | 2)
            || endian_u32(data, *offset + 24, swapped)? != 0
            || endian_u32(data, *offset + 28, swapped)? != 0
        {
            return Err(ModelError::Format("unknown historical subset flags".into()));
        }
        (endian_u32(data, *offset + 20, swapped)? as usize, 32usize)
    } else {
        if *size < 16 {
            return Err(ModelError::Format("truncated CrChF subset table".into()));
        }
        (u32at(data, *offset + 4)? as usize, 16usize)
    };
    if !(1..=128).contains(&count) || row_offset.checked_add(count * 36).is_none_or(|n| n > *size) {
        return Err(ModelError::Limits);
    }
    let legacy_vertex_count = if legacy {
        Some(parse_model_bytes(data)?.positions.len())
    } else {
        None
    };
    let mut result = Vec::with_capacity(count + 1);
    let mut cursor = 0usize;
    for row in 0..count {
        let at = *offset + row_offset + row * 36;
        let first = endian_u32(data, at, swapped)? as usize;
        let n = endian_u32(data, at + 4, swapped)? as usize;
        let material = endian_u32(data, at + 16, swapped)?;
        if first != cursor
            || n == 0
            || !first.is_multiple_of(3)
            || !n.is_multiple_of(3)
            || first.checked_add(n).is_none_or(|end| end > index_count)
            || material >= 128
        {
            return Err(ModelError::Format(
                "source mesh subset geometry/material ranges are invalid".into(),
            ));
        }
        if legacy {
            let first_vertex = endian_u32(data, at + 8, swapped)? as usize;
            let vertex_count = endian_u32(data, at + 12, swapped)? as usize;
            let nv = legacy_vertex_count.unwrap_or_default();
            if first_vertex
                .checked_add(vertex_count)
                .is_none_or(|end| end > nv)
            {
                return Err(ModelError::Format(
                    "historical material subset has invalid vertex range".into(),
                ));
            }
        }
        result.push((first as u32, n as u32, material));
        cursor += n;
    }
    if legacy && cursor != index_count {
        return Err(ModelError::Format(
            "historical subset index coverage is incomplete".into(),
        ));
    }
    if cursor < index_count {
        result.push((cursor as u32, (index_count - cursor) as u32, 255));
    }
    Ok(result)
}

pub fn read_model(path: &Path) -> Result<RenderMesh, ModelError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_BYTES as u64 {
        return Err(ModelError::Limits);
    }
    parse_model_bytes(&fs::read(path)?)
}

/// Creates a Blender-importable geometric OBJ; does not claim skeletal FBX
/// equivalence or fabricate missing UV/material/skeleton information.
pub fn export_model_obj(mesh: &RenderMesh, target: &Path) -> Result<(), ModelError> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                ModelError::OutputExists
            } else {
                ModelError::Io(e)
            }
        })?;
    let result = (|| -> Result<(), ModelError> {
        writeln!(output, "# Original CryEngine mesh positions and raw UV0")?;
        for v in &mesh.positions {
            writeln!(output, "v {} {} {}", v[0], v[1], v[2])?;
        }
        if let Some(uvs) = &mesh.uv0 {
            for uv in uvs {
                writeln!(output, "vt {} {}", uv[0], uv[1])?;
            }
        }
        if let Some(normals) = &mesh.normals {
            for n in normals {
                writeln!(output, "vn {} {} {}", n[0], n[1], n[2])?;
            }
        }
        for tri in mesh.indices.as_chunks::<3>().0.iter() {
            let mut corners = Vec::with_capacity(3);
            for idx in tri {
                let i = idx + 1;
                let corner = match (mesh.uv0.is_some(), mesh.normals.is_some()) {
                    (true, true) => format!("{i}/{i}/{i}"),
                    (true, false) => format!("{i}/{i}"),
                    (false, true) => format!("{i}//{i}"),
                    (false, false) => i.to_string(),
                };
                corners.push(corner);
            }
            writeln!(output, "f {} {} {}", corners[0], corners[1], corners[2])?;
        }
        output.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        drop(output);
        let _ = fs::remove_file(target);
    }
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> Vec<u8> {
        let streams = [(0u32, 3u32, 12u32), (5u32, 3u32, 2u32), (2u32, 3u32, 8u32)];
        let mut data = vec![0u8; 16 + streams.len() * 16];
        data[..8].copy_from_slice(b"CrChF\x07\0\0");
        data[8..12].copy_from_slice(&(streams.len() as u32).to_le_bytes());
        data[12..16].copy_from_slice(&16u32.to_le_bytes());
        for (i, (ty, count, stride)) in streams.iter().enumerate() {
            let offset = data.len();
            let mut chunk = vec![0u8; 24];
            chunk[4..8].copy_from_slice(&ty.to_le_bytes());
            chunk[8..12].copy_from_slice(&count.to_le_bytes());
            chunk[12..16].copy_from_slice(&stride.to_le_bytes());
            let payload = match ty {
                0 => [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
                    .iter()
                    .flat_map(|v| v.iter().flat_map(|c: &f32| c.to_le_bytes()))
                    .collect::<Vec<_>>(),
                5 => [0u16, 1, 2].iter().flat_map(|x| x.to_le_bytes()).collect(),
                _ => [[0f32, 0f32], [1., 0.], [0., 1.]]
                    .iter()
                    .flat_map(|v| v.iter().flat_map(|c| c.to_le_bytes()))
                    .collect(),
            };
            chunk.extend_from_slice(&payload);
            let at = 16 + i * 16;
            data[at..at + 4].copy_from_slice(&CRCHF_STREAM.to_le_bytes());
            data[at + 4..at + 8].copy_from_slice(&(i as u32).to_le_bytes());
            data[at + 8..at + 12].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            data[at + 12..at + 16].copy_from_slice(&(offset as u32).to_le_bytes());
            data.extend_from_slice(&chunk);
        }
        data
    }
    // The pre-Stage-2 2012 client uses CryTek 0x745 and legacy 16-byte
    // terminal chunk records, not the CrChF v7 chunk table.
    fn legacy_2012_fixture() -> Vec<u8> {
        let positions = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<_>>();
        let indices = [0u16, 1, 2]
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect::<Vec<_>>();
        let uv = [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]]
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<_>>();
        let streams = [(0u32, 12u32, positions), (5, 2, indices), (2, 8, uv)];
        let mut data = vec![0u8; 20];
        data[..8].copy_from_slice(b"CryTek\0\0");
        data[8..12].copy_from_slice(&0xffff0000u32.to_le_bytes());
        data[12..16].copy_from_slice(&0x745u32.to_le_bytes());
        let mut records = Vec::new();
        for (id, (stream_type, stride, values)) in streams.into_iter().enumerate() {
            let offset = data.len();
            let mut chunk = vec![0u8; 40];
            chunk[0..4].copy_from_slice(&CRYTEK_STREAM.to_le_bytes());
            chunk[4..8].copy_from_slice(&0x800u32.to_le_bytes());
            chunk[8..12].copy_from_slice(&(offset as u32).to_le_bytes());
            chunk[20..24].copy_from_slice(&stream_type.to_le_bytes());
            chunk[24..28].copy_from_slice(&3u32.to_le_bytes());
            chunk[28..32].copy_from_slice(&stride.to_le_bytes());
            data.extend_from_slice(&chunk);
            data.extend_from_slice(&values);
            records.push((offset, id));
        }
        let table = data.len();
        data[16..20].copy_from_slice(&(table as u32).to_le_bytes());
        data.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (offset, id) in records {
            data.extend_from_slice(&CRYTEK_STREAM.to_le_bytes());
            data.extend_from_slice(&0x800u32.to_le_bytes());
            data.extend_from_slice(&(offset as u32).to_le_bytes());
            data.extend_from_slice(&(id as u32).to_le_bytes());
        }
        data
    }

    #[test]
    fn historical_crytek_subsets_preserve_nonzero_material_slots() {
        // Synthetic 0x745 geometry with authored legacy material index 10,
        // mirroring old NoDraw skeleton leaves before a visible body.
        let mut bytes = legacy_2012_fixture();
        let old_table = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        bytes.truncate(old_table);
        let offset = bytes.len();
        let mut subset = vec![0u8; 68];
        subset[..4].copy_from_slice(&0xCCCC0017u32.to_le_bytes());
        subset[4..8].copy_from_slice(&0x800u32.to_le_bytes());
        subset[8..12].copy_from_slice(&(offset as u32).to_le_bytes());
        subset[12..16].copy_from_slice(&99u32.to_le_bytes());
        subset[16..20].copy_from_slice(&2u32.to_le_bytes());
        subset[20..24].copy_from_slice(&1u32.to_le_bytes());
        subset[36..40].copy_from_slice(&3u32.to_le_bytes());
        subset[44..48].copy_from_slice(&3u32.to_le_bytes());
        subset[48..52].copy_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&subset);
        let table = bytes.len();
        bytes[16..20].copy_from_slice(&(table as u32).to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        let mut old = legacy_2012_fixture();
        old.drain(..old_table + 4);
        bytes.extend_from_slice(&old[..3 * 16]);
        bytes.extend_from_slice(&0xCCCC0017u32.to_le_bytes());
        bytes.extend_from_slice(&0x800u32.to_le_bytes());
        bytes.extend_from_slice(&(offset as u32).to_le_bytes());
        bytes.extend_from_slice(&99u32.to_le_bytes());
        assert!(has_original_mesh_subset_table(&bytes).unwrap());
        assert_eq!(original_mesh_subsets(&bytes, 3).unwrap(), vec![(0, 3, 10)]);
        let mut console = bytes.clone();
        // The historical CGF subset table shares the same original material
        // index semantics with the old CHR table but uses big-endian fields.
        convert_2012_streams_to_console(&mut console);
        console[offset + 4..offset + 8].copy_from_slice(&0x80000800u32.to_le_bytes());
        for at in (offset + 16..offset + 32).step_by(4) {
            let value = u32at(&console, at).unwrap();
            console[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }
        for at in (offset + 32..offset + 32 + 20).step_by(4) {
            let value = u32at(&console, at).unwrap();
            console[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }
        assert_eq!(
            original_mesh_subsets(&console, 3).unwrap(),
            vec![(0, 3, 10)]
        );
        bytes[offset + 48] = 200;
        assert!(original_mesh_subsets(&bytes, 3).is_err());
    }

    // Synthetic copy of the stream endian variant used by historical 2012
    // CGFs; the 20-byte outer chunk directory remains little-endian while
    // the geometry stream payload and dimensions are big-endian.
    fn convert_2012_streams_to_console(data: &mut [u8]) {
        let table = u32at(data, 16).unwrap() as usize;
        let count = u32at(data, table).unwrap() as usize;
        let record_size = if table + 4 + count * 16 == data.len() {
            16
        } else {
            20
        };
        for row in 0..count {
            let at = table + 4 + row * record_size;
            if u32at(data, at).unwrap() != CRYTEK_STREAM {
                continue;
            }
            let offset = u32at(data, at + 8).unwrap() as usize;
            let kind = u32at(data, offset + 20).unwrap();
            let n = u32at(data, offset + 24).unwrap() as usize;
            let stride = u32at(data, offset + 28).unwrap() as usize;
            for field in 0..6 {
                let pos = offset + 16 + field * 4;
                let original = u32at(data, pos).unwrap();
                data[pos..pos + 4].copy_from_slice(&original.to_be_bytes());
            }
            match kind {
                0..=2 => {
                    for pos in (offset + 40..offset + 40 + n * stride).step_by(4) {
                        let original = u32at(data, pos).unwrap();
                        data[pos..pos + 4].copy_from_slice(&original.to_be_bytes());
                    }
                }
                5 => {
                    for pos in (offset + 40..offset + 40 + n * stride).step_by(2) {
                        data.swap(pos, pos + 1);
                    }
                }
                _ => {}
            }
            data[offset + 4..offset + 8].copy_from_slice(&0x80000800u32.to_le_bytes());
        }
    }

    #[test]
    fn historical_console_2012_cgf_coordinates_indices_and_uvs_decode() {
        let mut raw = legacy_2012_fixture();
        convert_2012_streams_to_console(&mut raw);
        let mesh = parse_model_bytes(&raw).unwrap();
        assert_eq!(mesh.info.format, "CryTek 0x745");
        assert_eq!(
            mesh.positions,
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        );
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.uv0.unwrap()[2], [0., 1.]);
        assert_eq!(original_mesh_subsets(&raw, 3).unwrap(), vec![(0, 3, 0)]);
        // A malformed console stream must still fail instead of displaying
        // plausibly shaped but incorrect geometry.
        let table = u32at(&raw, 16).unwrap() as usize;
        let position = u32at(&raw, table + 4 + 8).unwrap() as usize;
        raw[position + 40..position + 44].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(parse_model_bytes(&raw).is_err());
    }

    #[test]
    fn historical_2012_crytek_geometry_remains_previewable() {
        let source = legacy_2012_fixture();
        let mesh = parse_model_bytes(&source).unwrap();
        assert_eq!(mesh.info.format, "CryTek 0x745");
        assert_eq!(mesh.info.version, 0x745);
        assert_eq!(mesh.info.vertex_count, 3);
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert_eq!(mesh.uv0.as_ref().unwrap()[2], [0.0, 1.0]);
        // A 2012 mesh lacking modern material-subset data must not be
        // rejected by the CDF viewport's per-material draw-range path.
        assert_eq!(original_mesh_subsets(&source, 3).unwrap(), vec![(0, 3, 0)]);
    }

    #[test]
    fn decodes_original_triangle_uvs() {
        let mesh = parse_model_bytes(&fixture()).unwrap();
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert_eq!(mesh.uv0.as_ref().unwrap()[2], [0., 1.]);
        assert_eq!(mesh.info.triangle_count, 1);
    }
    #[test]
    fn rejects_out_of_bounds_chunks() {
        let mut bytes = fixture();
        bytes[16 + 12..16 + 16].copy_from_slice(&99999999u32.to_le_bytes());
        assert!(parse_model_bytes(&bytes).is_err());
    }
    #[test]
    fn writes_obj_only_once() {
        let mesh = parse_model_bytes(&fixture()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let obj = dir.path().join("test.obj");
        export_model_obj(&mesh, &obj).unwrap();
        let saved = fs::read_to_string(&obj).unwrap();
        assert!(saved.contains("f 1/1 2/2 3/3"));
        assert!(matches!(
            export_model_obj(&mesh, &obj),
            Err(ModelError::OutputExists)
        ));
    }
}
