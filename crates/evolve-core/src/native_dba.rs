//! Bounded read-only CryEngine DBA Controller_905 structural inspection.
//! Does NOT claim compressed native DBA playback: key-frame compression,
//! shared tracks and individual animation directories must be decoded and
//! compared against real Stage 2 data before exposing controller samples.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const MAX_DBA_BYTES: usize = 512 * 1024 * 1024;
const MAX_CHUNKS: usize = 10_000;
const MAX_TRACKS: usize = 65_536;

#[derive(Debug, thiserror::Error)]
pub enum NativeDbaError {
    #[error("Unsupported or malformed DBA: {0}")]
    Invalid(String),
    #[error("File access: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeDbaInspection {
    pub controller_version: u16,
    pub controller_chunks: usize,
    pub position_tracks: usize,
    pub rotation_tracks: usize,
    pub time_tracks: usize,
    pub declared_animations: usize,
    pub inplace_streaming: bool,
    /// Decoding packed controller tracks is not implemented. Never expose
    /// database metadata as if these were playable original CAF clips.
    pub playable_animation_count: usize,
}

fn word(data: &[u8], at: usize) -> Result<u32, NativeDbaError> {
    let raw = data
        .get(
            at..at
                .checked_add(4)
                .ok_or_else(|| NativeDbaError::Invalid("DBA offset overflow".into()))?,
        )
        .ok_or_else(|| NativeDbaError::Invalid("Truncated DBA".into()))?;
    Ok(u32::from_le_bytes(raw.try_into().map_err(|_| {
        NativeDbaError::Invalid("DBA word length invalid".into())
    })?))
}

/// Only the known CrChF v7 chunk-table layout and Controller_905 metadata
/// are inspected. Unknown packed formats fail instead of guessing offsets.
pub fn inspect_native_dba_bytes(data: &[u8]) -> Result<NativeDbaInspection, NativeDbaError> {
    if data.len() < 32 || data.len() > MAX_DBA_BYTES || data.get(..8) != Some(b"CrChF\x07\0\0") {
        return Err(NativeDbaError::Invalid(
            "Unsupported DBA magic or size".into(),
        ));
    }
    let count = word(data, 8)? as usize;
    let table = word(data, 12)? as usize;
    let end = table
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| NativeDbaError::Invalid("DBA chunk-table overflow".into()))?,
        )
        .ok_or_else(|| NativeDbaError::Invalid("DBA directory overflow".into()))?;
    if count == 0 || count > MAX_CHUNKS || table < 16 || end > data.len() {
        return Err(NativeDbaError::Invalid("Invalid DBA chunk-table".into()));
    }
    let mut seen = Vec::new();
    let mut controller = None;
    for i in 0..count {
        let at = table + i * 16;
        let kind = word(data, at)?;
        let size = word(data, at + 8)? as usize;
        let offset = word(data, at + 12)? as usize;
        let finish = offset
            .checked_add(size)
            .ok_or_else(|| NativeDbaError::Invalid("DBA chunk length overflow".into()))?;
        if size == 0 || offset < end || finish > data.len() {
            return Err(NativeDbaError::Invalid("Invalid DBA chunk bounds".into()));
        }
        seen.push((offset, finish));
        if kind & 0xffff == 0x100d
            && kind >> 16 == 0x905
            && controller.replace((offset, size)).is_some()
        {
            return Err(NativeDbaError::Invalid(
                "Multiple DBA Controller_905 chunks".into(),
            ));
        }
    }
    seen.sort_unstable();
    if seen.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(NativeDbaError::Invalid("Overlapping DBA chunks".into()));
    }
    let (at, size) = controller
        .ok_or_else(|| NativeDbaError::Invalid("Controller_905 DBA metadata not found".into()))?;
    if size < 120 {
        return Err(NativeDbaError::Invalid(
            "Truncated DBA controller header".into(),
        ));
    }
    let position_tracks = word(data, at)? as usize;
    let rotation_tracks = word(data, at + 4)? as usize;
    let time_tracks = word(data, at + 8)? as usize;
    let animations = word(data, at + 12)? as usize;
    if position_tracks > MAX_TRACKS
        || rotation_tracks > MAX_TRACKS
        || time_tracks > MAX_TRACKS
        || animations == 0
        || animations > MAX_TRACKS
    {
        return Err(NativeDbaError::Invalid("Unbounded DBA track counts".into()));
    }
    // Controller_905: 16-byte header; track length u16 arrays, 7 time /
    // 9 position / 9 rotation format-count u32 arrays; then signed
    // time/position/rotation offsets, with an extra rotation terminator.
    let required = 16usize
        .checked_add(
            time_tracks
                .checked_mul(6)
                .ok_or_else(|| NativeDbaError::Invalid("DBA time table overflow".into()))?,
        )
        .and_then(|v| v.checked_add(position_tracks.checked_mul(6)?))
        .and_then(|v| v.checked_add(rotation_tracks.checked_mul(6)?))
        .and_then(|v| v.checked_add(28 + 36 + 36 + 4))
        .ok_or_else(|| NativeDbaError::Invalid("DBA table overflow".into()))?;
    if required > size {
        return Err(NativeDbaError::Invalid(
            "Incomplete DBA track tables".into(),
        ));
    }
    // Signed time track offsets identify in-place streaming mode. Do not
    // parse keyframe contents without validating their storage format.
    let time_offsets_at =
        at + 16 + 2 * time_tracks + 28 + 2 * position_tracks + 36 + 2 * rotation_tracks + 36;
    let first_time_offset = if time_tracks > 0 {
        Some(word(data, time_offsets_at)? as i32)
    } else {
        None
    };
    Ok(NativeDbaInspection {
        controller_version: 0x905,
        controller_chunks: 1,
        position_tracks,
        rotation_tracks,
        time_tracks,
        declared_animations: animations,
        inplace_streaming: first_time_offset.is_some_and(|offset| offset < 0),
        playable_animation_count: 0,
    })
}


#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbaAnimationRecord {
    pub name: String,
    pub controller_count: usize,
    pub additive: bool,
    pub cyclic: bool,
}

/// Read the named Controller_905 animation directory and its source
/// controller references without decoding compressed positions/quaternions.
/// Both standard (tracks before names) and in-place (names before tracks)
/// storage have a distinct ordering. This function refuses corrupt offsets
/// or unknown track references rather than making fake idle poses.
pub fn index_native_dba_bytes(data: &[u8]) -> Result<Vec<DbaAnimationRecord>, NativeDbaError> {
    let summary = inspect_native_dba_bytes(data)?;
    let count = word(data, 8)? as usize;
    let table = word(data, 12)? as usize;
    let mut chunk = None;
    for i in 0..count {
        let at = table + i * 16;
        let kind = word(data, at)?;
        if kind & 0xffff == 0x100d && kind >> 16 == 0x905 {
            chunk = Some((word(data, at + 12)? as usize, word(data, at + 8)? as usize));
        }
    }
    let (start, size) =
        chunk.ok_or_else(|| NativeDbaError::Invalid("Missing Controller_905".into()))?;
    let end = start.checked_add(size).ok_or_else(|| NativeDbaError::Invalid(
        "Controller_905 boundary overflow".into()
    ))?;
    let nt = summary.time_tracks;
    let np = summary.position_tracks;
    let nr = summary.rotation_tracks;
    let mut cursor = start + 16;
    let mut advance = |bytes: usize| -> Result<usize, NativeDbaError> {
        let old = cursor;
        cursor = old.checked_add(bytes).ok_or_else(|| NativeDbaError::Invalid(
            "DBA track-table offset overflow".into()
        ))?;
        if cursor > end {
            return Err(NativeDbaError::Invalid("Truncated DBA track table".into()));
        }
        Ok(old)
    };
    advance(nt * 2)?;
    let time_formats = advance(7 * 4)?;
    advance(np * 2)?;
    let position_formats = advance(9 * 4)?;
    advance(nr * 2)?;
    let rotation_formats = advance(9 * 4)?;
    let time_offsets = advance(nt * 4)?;
    let pos_offsets = advance(np * 4)?;
    let rot_offsets = advance((nr + 1) * 4)?;
    // Verify all declared format bins match the source track counts,
    // including cases with no tracks in a particular channel.
    for (base, bins, expected) in [
        (time_formats, 7usize, nt),
        (position_formats, 9usize, np),
        (rotation_formats, 9usize, nr),
    ] {
        let total = (0..bins).try_fold(0usize, |n, i| {
            n.checked_add(word(data, base + i * 4)? as usize)
                .ok_or_else(|| NativeDbaError::Invalid("DBA format-count overflow".into()))
        })?;
        if total != expected {
            return Err(NativeDbaError::Invalid(
                "DBA compression format counts do not match track lengths".into(),
            ));
        }
    }
    // The final rotation offset is always the track data extent, even
    // when the DBA does not contain any rotation tracks.
    let signed_size = word(data, rot_offsets + nr * 4)? as i32;
    let extent = if summary.inplace_streaming {
        signed_size.checked_neg()
    } else {
        Some(signed_size)
    }
    .filter(|n| *n >= 0)
    .ok_or_else(|| NativeDbaError::Invalid("Invalid DBA track-data extent".into()))?
        as usize;
    for (at, num) in [(time_offsets, nt), (pos_offsets, np), (rot_offsets, nr)] {
        for i in 0..num {
            let offset = word(data, at + 4 * i)? as i32;
            let valid = if summary.inplace_streaming {
                offset <= 0 && i64::from(offset) >= -(extent as i64)
            } else {
                offset >= 0 && (offset as usize) <= extent
            };
            if !valid {
                return Err(NativeDbaError::Invalid("Invalid DBA shared-track offset".into()));
            }
        }
    }
    if summary.inplace_streaming {
        let pad = word(data, advance(4)?)? as usize;
        if pad > size {
            return Err(NativeDbaError::Invalid("Unbounded streaming DBA padding".into()));
        }
        advance(pad)?;
    }
    // CryEngine's Controller_905 uses four-byte alignment after the offset
    // arrays (and optional streaming padding).
    cursor = cursor.checked_add(3).ok_or_else(|| NativeDbaError::Invalid(
        "DBA alignment overflow".into()
    ))? & !3;
    if cursor > end {
        return Err(NativeDbaError::Invalid("DBA alignment outside chunk".into()));
    }
    let mut names_at = cursor;
    if !summary.inplace_streaming {
        names_at = names_at.checked_add(extent).ok_or_else(|| NativeDbaError::Invalid(
            "DBA track extent overflows chunk".into()
        ))?;
    }
    if names_at > end {
        return Err(NativeDbaError::Invalid("DBA tracks exceed chunk".into()));
    }
    // MotionParams905 contains fixed-size (132-byte) motion metadata.
    const MOTION_PARAMS: usize = 132;
    const MAX_NAME: usize = 512;
    let mut records = Vec::with_capacity(summary.declared_animations.min(4096));
    let mut position = names_at;
    let mut all_controller_counts = Vec::new();
    for _ in 0..summary.declared_animations {
        if position + 2 > end {
            return Err(NativeDbaError::Invalid("Truncated DBA animation name".into()));
        }
        let name_length = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        if name_length == 0 || name_length > MAX_NAME || position + name_length > end {
            return Err(NativeDbaError::Invalid("Unbounded DBA animation name".into()));
        }
        let name = std::str::from_utf8(&data[position..position + name_length])
            .map_err(|_| NativeDbaError::Invalid("Invalid DBA animation name".into()))?;
        if name.chars().any(char::is_control) {
            return Err(NativeDbaError::Invalid("Control character in DBA animation name".into()));
        }
        position += name_length;
        if position + MOTION_PARAMS + 2 > end {
            return Err(NativeDbaError::Invalid("Truncated DBA motion params".into()));
        }
        let flags = word(data, position)?;
        position += MOTION_PARAMS;
        let foot_count = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        if position + foot_count + 2 > end {
            return Err(NativeDbaError::Invalid("Truncated DBA footplant track".into()));
        }
        position += foot_count;
        let controllers = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        if controllers == 0 || controllers > 4096 {
            return Err(NativeDbaError::Invalid("Unsupported DBA controller count".into()));
        }
        if summary.inplace_streaming {
            if position + 4 > end {
                return Err(NativeDbaError::Invalid("Truncated streaming controller pointer".into()));
            }
            position += 4;
        } else {
            position = parse_dba_controllers(data, position, end, controllers, nt, np, nr)?;
        }
        all_controller_counts.push(controllers);
        records.push(DbaAnimationRecord {
            name: name.to_owned(),
            controller_count: controllers,
            additive: flags & 1 != 0,
            cyclic: flags & 2 != 0,
        });
    }
    if summary.inplace_streaming {
        // Streaming DBA controller headers form a single contiguous block
        // immediately after its named animation directory.
        for count in all_controller_counts {
            position = parse_dba_controllers(data, position, end, count, nt, np, nr)?;
        }
        let aligned = position.checked_add(3)
            .ok_or_else(|| NativeDbaError::Invalid("DBA alignment overflow".into()))? & !3;
        if aligned.checked_add(extent).is_none_or(|n| n > end) {
            return Err(NativeDbaError::Invalid("DBA streaming track block is truncated".into()));
        }
    }
    Ok(records)
}

fn parse_dba_controllers(
    bytes: &[u8],
    start: usize,
    end: usize,
    count: usize,
    time_tracks: usize,
    position_tracks: usize,
    rotation_tracks: usize,
) -> Result<usize, NativeDbaError> {
    let finish = start.checked_add(count.checked_mul(20).ok_or_else(|| NativeDbaError::Invalid(
        "DBA controller table overflow".into()
    ))?).ok_or_else(|| NativeDbaError::Invalid("DBA controller table overflow".into()))?;
    if finish > end {
        return Err(NativeDbaError::Invalid("Truncated DBA controller table".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for i in 0..count {
        let at = start + i * 20;
        let crc = word(bytes, at)?;
        if !seen.insert(crc) {
            return Err(NativeDbaError::Invalid("Repeated DBA controller CRC".into()));
        }
        let index = |n| word(bytes, at + n).map(|v| v as i32);
        let pos_time = index(4)?;
        let pos = index(8)?;
        let rot_time = index(12)?;
        let rot = index(16)?;
        for (value, bound) in [
            (pos_time, time_tracks),
            (pos, position_tracks),
            (rot_time, time_tracks),
            (rot, rotation_tracks),
        ] {
            if value != -1 && (value < 0 || value as usize >= bound) {
                return Err(NativeDbaError::Invalid("DBA controller track index out of range".into()));
            }
        }
        if (pos_time == -1) != (pos == -1) || (rot_time == -1) != (rot == -1) {
            return Err(NativeDbaError::Invalid("Incomplete DBA controller channel".into()));
        }
        if pos == -1 && rot == -1 {
            return Err(NativeDbaError::Invalid("DBA controller has no source channels".into()));
        }
    }
    Ok(finish)
}

pub fn inspect_native_dba_file(path: &Path) -> Result<NativeDbaInspection, NativeDbaError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() as usize > MAX_DBA_BYTES {
        return Err(NativeDbaError::Invalid(
            "Unsafe or oversized native DBA".into(),
        ));
    }
    inspect_native_dba_bytes(&fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_bounded_controller_905_without_claiming_playback() {
        let mut data = vec![0u8; 32 + 142];
        data[..8].copy_from_slice(b"CrChF\x07\0\0");
        data[8..12].copy_from_slice(&1u32.to_le_bytes());
        data[12..16].copy_from_slice(&16u32.to_le_bytes());
        data[16..20].copy_from_slice(&0x0905100du32.to_le_bytes());
        data[24..28].copy_from_slice(&142u32.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());
        data[32..36].copy_from_slice(&1u32.to_le_bytes());
        data[36..40].copy_from_slice(&1u32.to_le_bytes());
        data[40..44].copy_from_slice(&1u32.to_le_bytes());
        data[44..48].copy_from_slice(&2u32.to_le_bytes());
        let found = inspect_native_dba_bytes(&data).unwrap();
        assert_eq!(found.controller_version, 0x905);
        assert_eq!(found.declared_animations, 2);
        assert!(!found.inplace_streaming);
        assert_eq!(found.playable_animation_count, 0);
        // Offset first-time-track is at 16+2+28+2+36+2+36 = 122
        data[32 + 122..32 + 126].copy_from_slice(&(-12i32).to_le_bytes());
        assert!(inspect_native_dba_bytes(&data).unwrap().inplace_streaming);
    }

    fn controller905_named_sample(streaming: bool) -> Vec<u8> {
        // Synthetic directory fixture only; its track payload is deliberately
        // not decoded into animation curves.
        let nt = 1usize;
        let np = 1usize;
        let nr = 1usize;
        let base = 32usize;
        let mut chunk = vec![0u8; 138];
        let mut put = |at: usize, value: u32| {
            chunk[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(0, np as u32);
        put(4, nr as u32);
        put(8, nt as u32);
        put(12, 1); // one original named animation
        put(16 + 2, 1); // key-time format eF32
        put(16 + 2 + 28 + 2, 1); // position format 0
        put(16 + 2 + 28 + 2 + 36 + 2, 1); // rotation format 0
        let offsets = 16 + 2 + 28 + 2 + 36 + 2 + 36;
        if streaming {
            put(offsets, (-24i32) as u32);
            put(offsets + 4, (-16i32) as u32);
            put(offsets + 8, (-8i32) as u32);
            put(offsets + 12, (-24i32) as u32);
            chunk.extend_from_slice(&0u32.to_le_bytes()); // streaming pad
        } else {
            put(offsets, 0);
            put(offsets + 4, 8);
            put(offsets + 8, 16);
            put(offsets + 12, 24);
        }
        while !(base + chunk.len()).is_multiple_of(4) {
            chunk.push(0);
        }
        let mut clip = Vec::new();
        clip.extend_from_slice(&4u16.to_le_bytes());
        clip.extend_from_slice(b"Idle");
        clip.extend_from_slice(&vec![0u8; 132]); // motion parameters
        clip.extend_from_slice(&0u16.to_le_bytes()); // footplant count
        clip.extend_from_slice(&1u16.to_le_bytes()); // controller count
        if streaming {
            clip.extend_from_slice(&0i32.to_le_bytes()); // streaming header pointer
        }
        let mut controller = Vec::new();
        controller.extend_from_slice(&12345u32.to_le_bytes());
        controller.extend_from_slice(&0i32.to_le_bytes()); // pos time track
        controller.extend_from_slice(&0i32.to_le_bytes()); // position track
        controller.extend_from_slice(&0i32.to_le_bytes()); // rot time track
        controller.extend_from_slice(&0i32.to_le_bytes()); // rotation track
        if streaming {
            chunk.extend_from_slice(&clip);
            chunk.extend_from_slice(&controller);
            while !(base + chunk.len()).is_multiple_of(4) {
                chunk.push(0);
            }
            chunk.extend_from_slice(&[0u8; 24]);
        } else {
            chunk.extend_from_slice(&[0u8; 24]);
            chunk.extend_from_slice(&clip);
            chunk.extend_from_slice(&controller);
        }
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(b"CrChF\x07\0\0");
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&16u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&0x0905100du32.to_le_bytes());
        bytes[24..28].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
        bytes[28..32].copy_from_slice(&(base as u32).to_le_bytes());
        bytes.extend_from_slice(&chunk);
        bytes
    }

    #[test]
    fn indexes_standard_and_inplace_dba_directories_without_fake_playback() {
        for streaming in [false, true] {
            let original = controller905_named_sample(streaming);
            let inspection = inspect_native_dba_bytes(&original).unwrap();
            assert_eq!(inspection.inplace_streaming, streaming);
            assert_eq!(inspection.playable_animation_count, 0);
            let entries = index_native_dba_bytes(&original).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].name, "Idle");
            assert_eq!(entries[0].controller_count, 1);
            assert!(!entries[0].additive);
            assert!(!entries[0].cyclic);
            let mut corrupted = original.clone();
            let wrong = (55u32).to_le_bytes();
            let controller_at = if streaming {
                // Inplace controller table immediately after named record.
                let track = corrupted.len() - 24;
                track - 20
            } else {
                corrupted.len() - 20
            };
            corrupted[controller_at + 8..controller_at + 12].copy_from_slice(&wrong);
            assert!(index_native_dba_bytes(&corrupted).is_err());
        }
    }

    #[test]
    fn rejects_truncated_and_unknown_compressed_database() {
        let mut data = vec![0; 174];
        assert!(inspect_native_dba_bytes(&data).is_err());
        data[..8].copy_from_slice(b"CrChF\x07\0\0");
        data[8..12].copy_from_slice(&1u32.to_le_bytes());
        data[12..16].copy_from_slice(&16u32.to_le_bytes());
        data[16..20].copy_from_slice(&0x0904100du32.to_le_bytes());
        data[24..28].copy_from_slice(&142u32.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());
        assert!(inspect_native_dba_bytes(&data).is_err());
    }
}
