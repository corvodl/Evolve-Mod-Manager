//! Original 2012 CryTek 0x745 bone and subset-palette reader.
//! Methodology follows the private manager's validated historical rig path:
//! never synthesize influences or retarget incompatible weighted bind poses.
use crate::rig::{CompiledSkeleton, RigError, VerifiedWeights};
use std::collections::{HashMap, HashSet};

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_BONES: usize = 1024;
const BONE: u32 = 0xACDC0000;
const STREAM: u32 = 0xCCCC0016;
const SUBSETS: u32 = 0xCCCC0017;
const RECORD: usize = 584;

#[derive(Clone, Copy)]
struct Chunk<'a> {
    kind: u32,
    data: &'a [u8],
}
fn invalid(message: &str) -> RigError {
    RigError::Invalid(message.into())
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, RigError> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or(RigError::Limits)?
            .try_into()
            .map_err(|_| RigError::Limits)?,
    ))
}
fn f32_at(b: &[u8], at: usize) -> Result<f32, RigError> {
    Ok(f32::from_bits(u32_at(b, at)?))
}
fn chunks(data: &[u8]) -> Result<Vec<Chunk<'_>>, RigError> {
    if data.len() < 24
        || data.len() > MAX_BYTES
        || data.get(..8) != Some(b"CryTek\0\0")
        || u32_at(data, 8)? != 0xFFFF0000
        || u32_at(data, 12)? != 0x745
    {
        return Err(invalid("Expected 2012 CryTek 0x745 render geometry"));
    }
    let table = u32_at(data, 16)? as usize;
    if table < 20 || table > data.len().saturating_sub(4) {
        return Err(RigError::Limits);
    }
    let count = u32_at(data, table)? as usize;
    if count == 0 || count > 10_000 {
        return Err(RigError::Limits);
    }
    let start = table.checked_add(4).ok_or(RigError::Limits)?;
    // Scorpid-era 2012 CryTek models can use the terminal FOUR-dword chunk
    // directory. Derive its chunk sizes only from strictly unique offsets;
    // never interpret a partially written five-dword directory as four.
    // This mirrors the established private historical mesh parser.
    let terminal_four =
        start.checked_add(count.checked_mul(16).ok_or(RigError::Limits)?) == Some(data.len());
    let entry_size = if terminal_four { 16 } else { 20 };
    let end = start
        .checked_add(count.checked_mul(entry_size).ok_or(RigError::Limits)?)
        .ok_or(RigError::Limits)?;
    if end > data.len() {
        return Err(invalid("Incomplete historical skeletal chunk directory"));
    }
    let mut records = Vec::with_capacity(count);
    let mut ids = HashSet::new();
    let mut offsets = HashSet::new();
    for i in 0..count {
        let at = start + i * entry_size;
        let kind = u32_at(data, at)?;
        let version = u32_at(data, at + 4)?;
        let offset = u32_at(data, at + 8)? as usize;
        let id = u32_at(data, at + 12)?;
        if !ids.insert(id) || !offsets.insert(offset) || offset < 20 || offset >= table {
            return Err(invalid("Historical bone/stream chunk table mismatch"));
        }
        let declared_size = if terminal_four {
            None
        } else {
            Some(u32_at(data, at + 16)? as usize)
        };
        records.push((kind, version, offset, id, declared_size));
    }
    let mut sorted = records.iter().map(|r| r.2).collect::<Vec<_>>();
    sorted.sort_unstable();
    let mut out = Vec::with_capacity(count);
    let mut ranges = Vec::with_capacity(count);
    for (kind, version, offset, id, declared_size) in records {
        let upper = if let Some(size) = declared_size {
            offset.checked_add(size).ok_or(RigError::Limits)?
        } else {
            let index = sorted
                .binary_search(&offset)
                .map_err(|_| RigError::Limits)?;
            sorted.get(index + 1).copied().unwrap_or(table)
        };
        if upper < offset + 16
            || upper > table
            || data.get(offset..offset + 4) != Some(&kind.to_le_bytes())
            || u32_at(data, offset + 4)? != version
            || u32_at(data, offset + 8)? as usize != offset
            || u32_at(data, offset + 12)? != id
        {
            return Err(invalid("Historical bone/stream chunk table mismatch"));
        }
        out.push(Chunk {
            kind,
            data: &data[offset..upper],
        });
        ranges.push((offset, upper));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|p| p[0].1 > p[1].0) {
        return Err(invalid("Overlapping historical skeletal chunks"));
    }
    Ok(out)
}
fn single<'a>(list: &'a [Chunk<'a>], kind: u32) -> Result<&'a [u8], RigError> {
    let mut found = list.iter().filter(|v| v.kind == kind);
    let part = found
        .next()
        .ok_or_else(|| invalid("Missing historical rig chunk"))?;
    if found.next().is_some() {
        return Err(invalid("Ambiguous historical rig chunk"));
    }
    Ok(part.data)
}
fn matrix(data: &[u8], at: usize) -> Result<[[f32; 4]; 4], RigError> {
    let mut m = [[0.0f32; 4]; 4];
    for (r, row) in m.iter_mut().enumerate().take(3) {
        for (c, value) in row.iter_mut().enumerate() {
            *value = f32_at(data, at + 4 * (r * 4 + c))?;
            if !value.is_finite() || value.abs() > 1e7 {
                return Err(invalid("Historical bone contains a non-finite transform"));
            }
        }
    }
    m[3] = [0.0, 0.0, 0.0, 1.0];
    Ok(m)
}
fn multiply(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}
fn same_rest(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> bool {
    (0..4).all(|r| {
        (0..4).all(|c| {
            let x = a[r][c];
            let y = b[r][c];
            (x - y).abs() <= 0.005 + y.abs() * 0.0001
        })
    })
}
pub fn parse_legacy_skeleton(source: &[u8]) -> Result<CompiledSkeleton, RigError> {
    let parts = chunks(source)?;
    let raw = single(&parts, BONE)?;
    if raw.len() < 48 || !(raw.len() - 48).is_multiple_of(RECORD) {
        return Err(invalid("Historical bones are not 48 + N*584 bytes"));
    }
    let count = (raw.len() - 48) / RECORD;
    if !(1..=MAX_BONES).contains(&count) {
        return Err(RigError::Limits);
    }
    let mut names = Vec::with_capacity(count);
    let mut parents = Vec::with_capacity(count);
    let mut world = Vec::with_capacity(count);
    let mut inverse = Vec::with_capacity(count);
    let mut labels = HashSet::new();
    for i in 0..count {
        let at = 48 + i * RECORD;
        let raw_name = raw.get(at + 312..at + 568).ok_or(RigError::Limits)?;
        let end = raw_name
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(|| invalid("Unterminated 2012 bone name"))?;
        let mut name = std::str::from_utf8(&raw_name[..end])
            .map_err(|_| invalid("Invalid 2012 bone name encoding"))?
            .trim()
            .to_owned();
        if name.is_empty() {
            return Err(invalid("Blank 2012 bone name"));
        }
        if !labels.insert(name.to_ascii_lowercase()) {
            name.push_str(&format!("__source_joint_{i}"));
            if !labels.insert(name.to_ascii_lowercase()) {
                return Err(invalid("Duplicate 2012 bone identity"));
            }
        }
        let inv = matrix(raw, at + 216)?;
        let rest = matrix(raw, at + 264)?;
        let pair = multiply(&inv, &rest);
        if (0..4)
            .any(|r| (0..4).any(|c| (pair[r][c] - if r == c { 1.0 } else { 0.0 }).abs() > 0.001))
        {
            return Err(invalid(
                "2012 inverse bind matrix disagrees with original rest pose",
            ));
        }
        let delta = u32_at(raw, at + 572)? as i32;
        let parent = if delta < 0 {
            let index = (i as i64) + (delta as i64);
            if index < 0 || index >= i as i64 {
                return Err(invalid("2012 bone parent escapes hierarchy"));
            }
            Some(index as usize)
        } else {
            None
        };
        names.push(name);
        parents.push(parent);
        world.push(rest);
        inverse.push(inv);
    }
    if !parents.iter().any(Option::is_none) {
        return Err(invalid("2012 bone hierarchy has no root"));
    }
    Ok(CompiledSkeleton {
        // Historical controllers are not verified as Stage 2 CAF CRCs.
        // Do not hand these IDs to the native Stage 2 motion decoder.
        controller_ids: Vec::new(),
        names,
        parents,
        world,
        inverse,
    })
}
#[derive(Clone, Copy)]
struct StreamData<'a> {
    count: usize,
    stride: usize,
    data: &'a [u8],
}
fn streams<'a>(parts: &'a [Chunk<'a>]) -> Result<HashMap<u32, StreamData<'a>>, RigError> {
    let mut result = HashMap::new();
    for p in parts.iter().filter(|p| p.kind == STREAM) {
        if p.data.len() < 40
            || u32_at(p.data, 16)? != 0
            || u32_at(p.data, 32)? != 0
            || u32_at(p.data, 36)? != 0
        {
            return Err(invalid("Unknown 2012 vertex stream encoding"));
        }
        let kind = u32_at(p.data, 20)?;
        let count = u32_at(p.data, 24)? as usize;
        let stride = u32_at(p.data, 28)? as usize;
        if count == 0
            || stride == 0
            || count.checked_mul(stride).and_then(|n| n.checked_add(40)) != Some(p.data.len())
        {
            return Err(invalid("Invalid 2012 vertex stream dimensions"));
        }
        if result
            .insert(
                kind,
                StreamData {
                    count,
                    stride,
                    data: &p.data[40..],
                },
            )
            .is_some()
        {
            return Err(invalid("Duplicate 2012 vertex/weight stream"));
        }
    }
    Ok(result)
}
/// Return a verified source skeleton and original four-influence data.
/// Use the master skeleton only if every ACTIVE weighted joint agrees in
/// identity, ancestor chain and rest pose. Otherwise use the body's own
/// complete bind skeleton, never a guessed root influence.
pub fn read_legacy_skin(
    body: &[u8],
    master: &[u8],
    vertices: usize,
) -> Result<(CompiledSkeleton, VerifiedWeights), RigError> {
    if !(3..=250_000).contains(&vertices) {
        return Err(RigError::Limits);
    }
    let body_skeleton = parse_legacy_skeleton(body)?;
    let master_skeleton = parse_legacy_skeleton(master)?;
    let parts = chunks(body)?;
    let streams = streams(&parts)?;
    let positions = streams
        .get(&0)
        .ok_or_else(|| invalid("Missing 2012 position stream"))?;
    let weights_stream = streams
        .get(&9)
        .ok_or_else(|| invalid("Missing 2012 four-weight stream"))?;
    let indices = streams
        .get(&5)
        .ok_or_else(|| invalid("Missing 2012 triangle stream"))?;
    if positions.count != vertices
        || weights_stream.count != vertices
        || weights_stream.stride != 8
        || indices.stride != 2
        || indices.count < 3
        || !indices.count.is_multiple_of(3)
    {
        return Err(invalid(
            "2012 rig vertex/triangle counts do not match geometry",
        ));
    }
    let mut local = Vec::with_capacity(vertices);
    let mut weights = Vec::with_capacity(vertices);
    for bytes in weights_stream.data.as_chunks::<8>().0 {
        let w = [bytes[4], bytes[5], bytes[6], bytes[7]];
        if w.iter().map(|v| u16::from(*v)).sum::<u16>() != 255 {
            return Err(invalid("2012 weights do not sum to exactly 255"));
        }
        local.push([bytes[0], bytes[1], bytes[2], bytes[3]]);
        weights.push([w[0], w[1], w[2], w[3], 0, 0, 0, 0]);
    }
    let mut triangles = Vec::with_capacity(indices.count);
    for chunk in indices.data.as_chunks::<2>().0 {
        let id = u16::from_le_bytes([chunk[0], chunk[1]]) as usize;
        if id >= vertices {
            return Err(invalid("2012 triangle escapes vertices"));
        }
        triangles.push(id);
    }
    let subset = single(&parts, SUBSETS)?;
    if subset.len() < 68 || !matches!(u32_at(subset, 16)?, 0 | 2) {
        return Err(invalid("Unverified 2012 subset/palette header"));
    }
    let n = u32_at(subset, 20)? as usize;
    if !(1..=128).contains(&n)
        || u32_at(subset, 24)? != 0
        || u32_at(subset, 28)? != 0
        || subset.len() != 32 + n * (36 + 260)
    {
        return Err(invalid("Unverified 2012 subset/palette size"));
    }
    let mut joint_ids = vec![[0u16; 8]; vertices];
    let mut seen = vec![false; vertices];
    let mut cursor = 0usize;
    for i in 0..n {
        let at = 32 + 36 * i;
        let first = u32_at(subset, at)? as usize;
        let count = u32_at(subset, at + 4)? as usize;
        let start_vertex = u32_at(subset, at + 8)? as usize;
        let length = u32_at(subset, at + 12)? as usize;
        if first != cursor
            || count == 0
            || !count.is_multiple_of(3)
            || first.checked_add(count).is_none_or(|x| x > triangles.len())
            || start_vertex
                .checked_add(length)
                .is_none_or(|x| x > vertices)
        {
            return Err(invalid("2012 subset geometry coverage invalid"));
        }
        cursor += count;
        let palette_at = 32 + n * 36 + 260 * i;
        let palette_count = u32_at(subset, palette_at)? as usize;
        if !(1..=128).contains(&palette_count) {
            return Err(invalid("2012 palette has invalid length"));
        }
        let mut palette = Vec::with_capacity(palette_count);
        for k in 0..palette_count {
            let at = palette_at + 4 + k * 2;
            let id = u16::from_le_bytes([
                *subset.get(at).ok_or(RigError::Limits)?,
                *subset.get(at + 1).ok_or(RigError::Limits)?,
            ]);
            if id as usize >= body_skeleton.names.len() {
                return Err(invalid("2012 palette escapes body bones"));
            }
            palette.push(id);
        }
        for index in &triangles[first..cursor] {
            let v = *index;
            if v < start_vertex || v >= start_vertex + length {
                return Err(invalid("2012 palette vertex range mismatch"));
            }
            let mut resolved = [0u16; 8];
            for joint in 0..4 {
                if weights[v][joint] > 0 {
                    let local_id = local[v][joint] as usize;
                    resolved[joint] = *palette
                        .get(local_id)
                        .ok_or_else(|| invalid("Active 2012 local joint escapes subset palette"))?;
                }
            }
            if seen[v] && resolved != joint_ids[v] {
                return Err(invalid("2012 overlapping subset bone palettes disagree"));
            }
            joint_ids[v] = resolved;
            seen[v] = true;
        }
    }
    if cursor != triangles.len() || seen.iter().any(|flag| !flag) {
        return Err(invalid("2012 palettes fail full mesh vertex coverage"));
    }
    let active = joint_ids
        .iter()
        .zip(&weights)
        .flat_map(|(ids, w)| (0..4).filter_map(|k| (w[k] > 0).then_some(ids[k] as usize)))
        .collect::<HashSet<_>>();
    if active.is_empty() {
        return Err(invalid("No weighted 2012 bone influences"));
    }
    let master_ids = master_skeleton
        .names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_ascii_lowercase(), i))
        .collect::<HashMap<_, _>>();
    let body_names = body_skeleton
        .names
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut remap = vec![0u16; body_skeleton.names.len()];
    let mut compatible = true;
    for (i, name) in body_skeleton.names.iter().enumerate() {
        let Some(&target) = master_ids.get(&name.to_ascii_lowercase()) else {
            if active.contains(&i) {
                compatible = false;
            }
            continue;
        };
        if target > u16::MAX as usize {
            return Err(RigError::Limits);
        }
        remap[i] = target as u16;
        if !active.contains(&i) {
            continue;
        }
        let original_parent =
            body_skeleton.parents[i].map(|p| body_skeleton.names[p].to_ascii_lowercase());
        let mut target_parent = master_skeleton.parents[target];
        while let Some(index) = target_parent {
            if body_names.contains(&master_skeleton.names[index].to_ascii_lowercase()) {
                break;
            }
            target_parent = master_skeleton.parents[index];
        }
        let found_parent = target_parent.map(|j| master_skeleton.names[j].to_ascii_lowercase());
        if original_parent != found_parent
            || !same_rest(&body_skeleton.world[i], &master_skeleton.world[target])
        {
            compatible = false;
        }
    }
    if compatible {
        for (ids, weight) in joint_ids.iter_mut().zip(&weights) {
            for i in 0..4 {
                ids[i] = if weight[i] == 0 {
                    0
                } else {
                    remap[ids[i] as usize]
                };
            }
        }
    }
    Ok((
        if compatible {
            master_skeleton
        } else {
            body_skeleton
        },
        VerifiedWeights {
            joint_ids,
            weight_bytes: weights,
            influences: 4,
            subsets: n,
        },
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn put_u32(buf: &mut [u8], at: usize, value: u32) {
        buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn sample_bones() -> Vec<u8> {
        let mut raw = vec![0; 48 + 2 * RECORD];
        for (index, name) in ["root", "arm"].iter().enumerate() {
            let at = 48 + index * RECORD;
            raw[at + 312..at + 312 + name.len()].copy_from_slice(name.as_bytes());
            for base in [216, 264] {
                for dimension in 0..3 {
                    raw[at + base + (dimension * 4 + dimension) * 4..]
                        .get_mut(..4)
                        .unwrap()
                        .copy_from_slice(&1f32.to_le_bytes());
                }
            }
            put_u32(&mut raw, at + 572, if index == 0 { 0 } else { u32::MAX });
        }
        raw
    }

    fn stream_chunk(ty: u32, count: u32, stride: u32, payload: &[u8]) -> Vec<u8> {
        let mut result = vec![0u8; 40];
        put_u32(&mut result, 20, ty);
        put_u32(&mut result, 24, count);
        put_u32(&mut result, 28, stride);
        result.extend_from_slice(payload);
        result
    }

    fn original_container(chunks: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
        let mut bytes = vec![0u8; 20];
        bytes[..8].copy_from_slice(b"CryTek\0\0");
        put_u32(&mut bytes, 8, 0xFFFF0000);
        put_u32(&mut bytes, 12, 0x745);
        let mut records = Vec::new();
        for (id, (kind, mut raw)) in chunks.into_iter().enumerate() {
            let offset = bytes.len();
            put_u32(&mut raw, 0, kind);
            put_u32(&mut raw, 4, 0x800);
            put_u32(&mut raw, 8, offset as u32);
            put_u32(&mut raw, 12, (id + 1) as u32);
            records.push((kind, offset, (id + 1) as u32, raw.len()));
            bytes.extend_from_slice(&raw);
        }
        let table = bytes.len();
        put_u32(&mut bytes, 16, table as u32);
        bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (kind, offset, id, size) in records {
            for value in [kind, 0x800, offset as u32, id, size as u32] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    fn sample_body() -> Vec<u8> {
        let positions = [[0f32, 0f32, 0f32], [1f32, 0f32, 0f32], [0f32, 1f32, 0f32]]
            .into_iter()
            .flat_map(|pos| pos.into_iter().flat_map(f32::to_le_bytes))
            .collect::<Vec<_>>();
        let indices = [0u16, 1, 2]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let weights = [
            [0u8, 0, 0, 0, 255, 0, 0, 0],
            [1u8, 0, 0, 0, 255, 0, 0, 0],
            [1u8, 0, 0, 0, 255, 0, 0, 0],
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let mut palette = vec![0u8; 32 + 36 + 260];
        put_u32(&mut palette, 20, 1);
        put_u32(&mut palette, 32 + 4, 3);
        put_u32(&mut palette, 32 + 12, 3);
        let at = 32 + 36;
        put_u32(&mut palette, at, 2);
        palette[at + 4..at + 8].copy_from_slice(&[0, 0, 1, 0]);
        original_container(vec![
            (BONE, sample_bones()),
            (STREAM, stream_chunk(0, 3, 12, &positions)),
            (STREAM, stream_chunk(5, 3, 2, &indices)),
            (STREAM, stream_chunk(9, 3, 8, &weights)),
            (SUBSETS, palette),
        ])
    }

    #[test]
    fn original_2012_palettes_produce_real_four_weight_joints() {
        let body = sample_body();
        let master = original_container(vec![(BONE, sample_bones())]);
        let (skeleton, weights) = read_legacy_skin(&body, &master, 3).unwrap();
        assert_eq!(skeleton.names, ["root", "arm"]);
        assert_eq!(skeleton.parents, [None, Some(0)]);
        assert_eq!(weights.influences, 4);
        assert_eq!(weights.joint_ids.len(), 3);
        assert_eq!(weights.joint_ids[1][0], 1);
        assert_eq!(weights.weight_bytes[1][0], 255);
        assert_eq!(weights.subsets, 1);
    }

    /// The 2012 private parser accepts this terminal four-dword table.
    /// Its offsets are verified against embedded chunk headers and the
    /// inferred sizes must form non-overlapping ranges ending at the table.
    fn terminal_four_directory(mut data: Vec<u8>) -> Vec<u8> {
        let table = u32_at(&data, 16).unwrap() as usize;
        let count = u32_at(&data, table).unwrap() as usize;
        let entries = (0..count)
            .map(|i| {
                let at = table + 4 + i * 20;
                data[at..at + 16].to_vec()
            })
            .collect::<Vec<_>>();
        data.truncate(table + 4);
        for entry in entries {
            data.extend_from_slice(&entry);
        }
        data
    }

    #[test]
    fn terminal_four_dword_2012_chunk_directory_preserves_real_weights() {
        let body = terminal_four_directory(sample_body());
        let master = terminal_four_directory(original_container(vec![(BONE, sample_bones())]));
        let (skeleton, weights) = read_legacy_skin(&body, &master, 3).unwrap();
        assert_eq!(skeleton.names, ["root", "arm"]);
        assert_eq!(weights.joint_ids[1][0], 1);
        assert_eq!(weights.weight_bytes[1][0], 255);
        assert_eq!(weights.subsets, 1);
    }

    #[test]
    fn rejects_forged_or_duplicate_terminal_chunk_offsets() {
        let mut body = terminal_four_directory(sample_body());
        let table = u32_at(&body, 16).unwrap() as usize;
        // Duplicate the first chunk offset: never infer overlapping sizes.
        let first_offset = u32_at(&body, table + 4 + 8).unwrap();
        put_u32(&mut body, table + 4 + 16 + 8, first_offset);
        assert!(parse_legacy_skeleton(&body).is_err());

        let mut original = terminal_four_directory(sample_body());
        let table = u32_at(&original, 16).unwrap() as usize;
        let id = u32_at(&original, table + 4 + 12).unwrap();
        put_u32(&mut original, table + 4 + 12, id + 900);
        assert!(parse_legacy_skeleton(&original).is_err());
    }

    #[test]
    fn rejects_wrong_legacy_header_without_fake_skin() {
        assert!(parse_legacy_skeleton(b"CrChF\x07\0\0").is_err());
        assert!(read_legacy_skin(b"not a model", b"not a skeleton", 3).is_err());
    }
}
