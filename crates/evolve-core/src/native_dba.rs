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
        if kind & 0xffff == 0x100d && kind >> 16 == 0x905 {
            if controller.replace((offset, size)).is_some() {
                return Err(NativeDbaError::Invalid(
                    "Multiple DBA Controller_905 chunks".into(),
                ));
            }
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
