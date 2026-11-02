//! Bounded read-only CryEngine DBA Controller_905 decoding. A database holds a
//! named directory of clips whose controllers index shared time, position and
//! rotation tracks; one clip decodes to the same motion as a loose CAF.
use crate::native_caf::{CafController, NativeCafMotion, quaternion, rotation_size};
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
    /// Inspection reads headers only; `decode_native_dba_clip` decodes tracks.
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
pub fn index_native_dba_bytes(data: &[u8]) -> Result<Vec<DbaAnimationRecord>, NativeDbaError> {
    Ok(parse_directory(data)?
        .clips
        .into_iter()
        .map(|clip| DbaAnimationRecord {
            name: clip.name,
            controller_count: clip.controllers.len(),
            additive: clip.flags & 1 != 0,
            cyclic: clip.flags & 2 != 0,
        })
        .collect())
}

#[derive(Clone, Copy)]
struct DbaController {
    crc: u32,
    position_times: i32,
    position: i32,
    rotation_times: i32,
    rotation: i32,
}

struct DbaClip {
    name: String,
    flags: u32,
    ticks_per_frame: i32,
    seconds_per_tick: f32,
    start: i32,
    end: i32,
    controllers: Vec<DbaController>,
}

/// One track family: per-track key counts, the number of tracks stored in each
/// compression format (tracks are grouped by format), and signed data offsets.
struct TrackTable {
    lengths: usize,
    formats: usize,
    offsets: usize,
}

impl TrackTable {
    fn length(&self, data: &[u8], track: usize) -> usize {
        usize::from(u16::from_le_bytes([
            data[self.lengths + track * 2],
            data[self.lengths + track * 2 + 1],
        ]))
    }
    fn format(&self, data: &[u8], track: usize, bins: usize) -> Result<usize, NativeDbaError> {
        let mut first = 0usize;
        for bin in 0..bins {
            first += word(data, self.formats + bin * 4)? as usize;
            if track < first {
                return Ok(bin);
            }
        }
        Err(NativeDbaError::Invalid("DBA track has no format".into()))
    }
}

struct DbaDirectory {
    clips: Vec<DbaClip>,
    times: TrackTable,
    positions: TrackTable,
    rotations: TrackTable,
    /// Track offsets are relative to this file position: the block start for
    /// standard storage, the block end (negative offsets) for in-place storage.
    anchor: i64,
    tracks_end: usize,
}

impl DbaDirectory {
    fn track_at(
        &self,
        data: &[u8],
        table: &TrackTable,
        track: usize,
    ) -> Result<usize, NativeDbaError> {
        usize::try_from(self.anchor + i64::from(word(data, table.offsets + track * 4)? as i32))
            .map_err(|_| NativeDbaError::Invalid("DBA track before file start".into()))
    }

    fn key_times(&self, data: &[u8], track: usize) -> Result<Vec<f32>, NativeDbaError> {
        const F32: usize = 0;
        const U16: usize = 1;
        const U8: usize = 2;
        const BITSET: usize = 6;
        let at = self.track_at(data, &self.times, track)?;
        let length = self.times.length(data, track);
        let format = self.times.format(data, track, 7)?;
        let stride = match format {
            F32 => 4,
            U16 | BITSET => 2,
            U8 => 1,
            // Start/stop time formats occur in no shipped database.
            _ => {
                return Err(NativeDbaError::Invalid(
                    "Unsupported DBA key time format".into(),
                ));
            }
        };
        let raw = at
            .checked_add(length * stride)
            .filter(|end| *end <= self.tracks_end)
            .and_then(|end| data.get(at..end))
            .ok_or_else(|| NativeDbaError::Invalid("DBA key time track out of bounds".into()))?;
        let half = |index: usize| u16::from_le_bytes([raw[index * 2], raw[index * 2 + 1]]);
        let times = match format {
            F32 => raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| f32::from_le_bytes(*v))
                .collect::<Vec<_>>(),
            U16 => (0..length).map(|i| f32::from(half(i))).collect(),
            U8 => raw.iter().map(|v| f32::from(*v)).collect(),
            _ => {
                // u16 first frame, last frame, key count, then one bit per
                // frame (low bit first) marking which frames carry a key.
                if length < 3 {
                    return Err(NativeDbaError::Invalid("Truncated DBA key bitset".into()));
                }
                let first = u32::from(half(0));
                let count = usize::from(half(2));
                let mut keys = Vec::with_capacity(count);
                for index in 0..(length - 3) * 16 {
                    if half(3 + index / 16) >> (index % 16) & 1 != 0 {
                        keys.push((first + index as u32) as f32);
                    }
                }
                if keys.len() != count {
                    return Err(NativeDbaError::Invalid(
                        "DBA key bitset does not match its key count".into(),
                    ));
                }
                keys
            }
        };
        if times.iter().any(|t| !t.is_finite()) || times.windows(2).any(|t| t[1] < t[0]) {
            return Err(NativeDbaError::Invalid("Nonmonotonic DBA key times".into()));
        }
        Ok(times)
    }

    fn rotation_keys(&self, data: &[u8], track: usize) -> Result<Vec<[f32; 4]>, NativeDbaError> {
        let at = self.track_at(data, &self.rotations, track)?;
        let format = self.rotations.format(data, track, 9)?;
        let width = rotation_size(format as u8)
            .ok_or_else(|| NativeDbaError::Invalid("Unsupported DBA rotation format".into()))?;
        let length = self.rotations.length(data, track);
        if at + length * width > self.tracks_end {
            return Err(NativeDbaError::Invalid(
                "DBA rotation track out of bounds".into(),
            ));
        }
        (0..length)
            .map(|i| {
                quaternion(data, at + i * width, format as u8)
                    .map_err(|e| NativeDbaError::Invalid(e.to_string()))
            })
            .collect()
    }

    fn position_keys(&self, data: &[u8], track: usize) -> Result<Vec<[f32; 3]>, NativeDbaError> {
        let at = self.track_at(data, &self.positions, track)?;
        if !matches!(self.positions.format(data, track, 9)?, 0 | 2) {
            return Err(NativeDbaError::Invalid(
                "Unsupported DBA position format".into(),
            ));
        }
        let length = self.positions.length(data, track);
        if at + length * 12 > self.tracks_end {
            return Err(NativeDbaError::Invalid(
                "DBA position track out of bounds".into(),
            ));
        }
        (0..length)
            .map(|i| {
                let read = |axis: usize| word(data, at + i * 12 + axis * 4).map(f32::from_bits);
                let xyz = [read(0)?, read(1)?, read(2)?];
                if xyz.iter().any(|v| !v.is_finite() || v.abs() > 10000.) {
                    return Err(NativeDbaError::Invalid("Invalid DBA bone position".into()));
                }
                Ok(xyz)
            })
            .collect()
    }
}

/// Decode one named clip from a Controller_905 database into the motion a loose
/// CAF would produce. The name comparison ignores case, as CryEngine paths do.
pub fn decode_native_dba_clip(data: &[u8], name: &str) -> Result<NativeCafMotion, NativeDbaError> {
    let directory = parse_directory(data)?;
    let clip = directory
        .clips
        .iter()
        .find(|clip| clip.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| NativeDbaError::Invalid(format!("DBA has no clip named {name}")))?;
    let mut controllers = Vec::with_capacity(clip.controllers.len());
    for controller in &clip.controllers {
        let mut decoded = CafController {
            bone_crc32: controller.crc,
            rotation_times: Vec::new(),
            rotations: Vec::new(),
            position_times: Vec::new(),
            positions: Vec::new(),
        };
        if controller.rotation >= 0 {
            decoded.rotation_times =
                directory.key_times(data, controller.rotation_times as usize)?;
            decoded.rotations = directory.rotation_keys(data, controller.rotation as usize)?;
        }
        if controller.position >= 0 {
            decoded.position_times =
                directory.key_times(data, controller.position_times as usize)?;
            decoded.positions = directory.position_keys(data, controller.position as usize)?;
        }
        if decoded.rotation_times.len() != decoded.rotations.len()
            || decoded.position_times.len() != decoded.positions.len()
        {
            return Err(NativeDbaError::Invalid(
                "DBA key times do not match their track length".into(),
            ));
        }
        controllers.push(decoded);
    }
    let (mut start, mut end, mut fps) = (clip.start as f32, clip.end as f32, 30.0);
    if clip.ticks_per_frame > 0 && clip.seconds_per_tick > 0.0 && clip.seconds_per_tick < 1.0 {
        fps = 1.0 / (clip.ticks_per_frame as f32 * clip.seconds_per_tick);
        start /= clip.ticks_per_frame as f32;
        end /= clip.ticks_per_frame as f32;
    }
    Ok(NativeCafMotion {
        schema: 1,
        controllers,
        start_frame: start,
        end_frame: end,
        fps,
        additive: clip.flags & 1 != 0,
        cyclic: clip.flags & 2 != 0,
    })
}

/// Both standard (tracks before names) and in-place (names before tracks)
/// storage have a distinct ordering. This function refuses corrupt offsets
/// or unknown track references rather than making fake idle poses.
fn parse_directory(data: &[u8]) -> Result<DbaDirectory, NativeDbaError> {
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
    let end = start
        .checked_add(size)
        .ok_or_else(|| NativeDbaError::Invalid("Controller_905 boundary overflow".into()))?;
    let nt = summary.time_tracks;
    let np = summary.position_tracks;
    let nr = summary.rotation_tracks;
    let mut cursor = start + 16;
    let mut advance = |bytes: usize| -> Result<usize, NativeDbaError> {
        let old = cursor;
        cursor = old
            .checked_add(bytes)
            .ok_or_else(|| NativeDbaError::Invalid("DBA track-table offset overflow".into()))?;
        if cursor > end {
            return Err(NativeDbaError::Invalid("Truncated DBA track table".into()));
        }
        Ok(old)
    };
    let time_lengths = advance(nt * 2)?;
    let time_formats = advance(7 * 4)?;
    let position_lengths = advance(np * 2)?;
    let position_formats = advance(9 * 4)?;
    let rotation_lengths = advance(nr * 2)?;
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
                return Err(NativeDbaError::Invalid(
                    "Invalid DBA shared-track offset".into(),
                ));
            }
        }
    }
    if summary.inplace_streaming {
        let pad = word(data, advance(4)?)? as usize;
        if pad > size {
            return Err(NativeDbaError::Invalid(
                "Unbounded streaming DBA padding".into(),
            ));
        }
        advance(pad)?;
    }
    // CryEngine's Controller_905 uses four-byte alignment after the offset
    // arrays (and optional streaming padding).
    cursor = cursor
        .checked_add(3)
        .ok_or_else(|| NativeDbaError::Invalid("DBA alignment overflow".into()))?
        & !3;
    if cursor > end {
        return Err(NativeDbaError::Invalid(
            "DBA alignment outside chunk".into(),
        ));
    }
    let block_start = cursor;
    let mut names_at = cursor;
    if !summary.inplace_streaming {
        names_at = names_at
            .checked_add(extent)
            .ok_or_else(|| NativeDbaError::Invalid("DBA track extent overflows chunk".into()))?;
    }
    if names_at > end {
        return Err(NativeDbaError::Invalid("DBA tracks exceed chunk".into()));
    }
    // MotionParams905 contains fixed-size (132-byte) motion metadata.
    const MOTION_PARAMS: usize = 132;
    const MAX_NAME: usize = 512;
    let mut clips = Vec::with_capacity(summary.declared_animations.min(4096));
    let mut position = names_at;
    for _ in 0..summary.declared_animations {
        if position + 2 > end {
            return Err(NativeDbaError::Invalid(
                "Truncated DBA animation name".into(),
            ));
        }
        let name_length = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        if name_length == 0 || name_length > MAX_NAME || position + name_length > end {
            return Err(NativeDbaError::Invalid(
                "Unbounded DBA animation name".into(),
            ));
        }
        let name = std::str::from_utf8(&data[position..position + name_length])
            .map_err(|_| NativeDbaError::Invalid("Invalid DBA animation name".into()))?;
        if name.chars().any(char::is_control) {
            return Err(NativeDbaError::Invalid(
                "Control character in DBA animation name".into(),
            ));
        }
        position += name_length;
        if position + MOTION_PARAMS + 2 > end {
            return Err(NativeDbaError::Invalid(
                "Truncated DBA motion params".into(),
            ));
        }
        let flags = word(data, position)?;
        let ticks_per_frame = word(data, position + 8)? as i32;
        let seconds_per_tick = f32::from_bits(word(data, position + 12)?);
        let clip_start = word(data, position + 16)? as i32;
        let clip_end = word(data, position + 20)? as i32;
        position += MOTION_PARAMS;
        let foot_count = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        if position + foot_count + 2 > end {
            return Err(NativeDbaError::Invalid(
                "Truncated DBA footplant track".into(),
            ));
        }
        position += foot_count;
        let controllers = u16::from_le_bytes([data[position], data[position + 1]]) as usize;
        position += 2;
        // Real databases hold a few empty clips, which simply show the bind pose.
        if controllers > 4096 {
            return Err(NativeDbaError::Invalid(
                "Unsupported DBA controller count".into(),
            ));
        }
        let mut table = Vec::new();
        if summary.inplace_streaming {
            if position + 4 > end {
                return Err(NativeDbaError::Invalid(
                    "Truncated streaming controller pointer".into(),
                ));
            }
            position += 4;
        } else {
            table = parse_dba_controllers(data, position, end, controllers, nt, np, nr)?;
            position += controllers * 20;
        }
        clips.push((
            DbaClip {
                name: name.to_owned(),
                flags,
                ticks_per_frame,
                seconds_per_tick,
                start: clip_start,
                end: clip_end,
                controllers: table,
            },
            controllers,
        ));
    }
    let mut anchor = block_start as i64;
    let mut tracks_end = block_start + extent;
    if summary.inplace_streaming {
        // Streaming DBA controller headers form a single contiguous block
        // immediately after its named animation directory.
        for (clip, count) in &mut clips {
            clip.controllers = parse_dba_controllers(data, position, end, *count, nt, np, nr)?;
            position += *count * 20;
        }
        let aligned = position
            .checked_add(3)
            .ok_or_else(|| NativeDbaError::Invalid("DBA alignment overflow".into()))?
            & !3;
        if aligned.checked_add(extent).is_none_or(|n| n > end) {
            return Err(NativeDbaError::Invalid(
                "DBA streaming track block is truncated".into(),
            ));
        }
        tracks_end = aligned + extent;
        anchor = tracks_end as i64;
    }
    if tracks_end > end {
        return Err(NativeDbaError::Invalid("DBA tracks exceed chunk".into()));
    }
    let table = |lengths, formats, offsets| TrackTable {
        lengths,
        formats,
        offsets,
    };
    Ok(DbaDirectory {
        clips: clips.into_iter().map(|(clip, _)| clip).collect(),
        times: table(time_lengths, time_formats, time_offsets),
        positions: table(position_lengths, position_formats, pos_offsets),
        rotations: table(rotation_lengths, rotation_formats, rot_offsets),
        anchor,
        tracks_end,
    })
}

fn parse_dba_controllers(
    bytes: &[u8],
    start: usize,
    end: usize,
    count: usize,
    time_tracks: usize,
    position_tracks: usize,
    rotation_tracks: usize,
) -> Result<Vec<DbaController>, NativeDbaError> {
    let finish = start
        .checked_add(
            count
                .checked_mul(20)
                .ok_or_else(|| NativeDbaError::Invalid("DBA controller table overflow".into()))?,
        )
        .ok_or_else(|| NativeDbaError::Invalid("DBA controller table overflow".into()))?;
    if finish > end {
        return Err(NativeDbaError::Invalid(
            "Truncated DBA controller table".into(),
        ));
    }
    let mut seen = std::collections::HashMap::new();
    let mut controllers = Vec::with_capacity(count);
    for i in 0..count {
        let at = start + i * 20;
        let crc = word(bytes, at)?;
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
                return Err(NativeDbaError::Invalid(
                    "DBA controller track index out of range".into(),
                ));
            }
        }
        if (pos_time == -1) != (pos == -1) || (rot_time == -1) != (rot == -1) {
            return Err(NativeDbaError::Invalid(
                "Incomplete DBA controller channel".into(),
            ));
        }
        if pos == -1 && rot == -1 {
            return Err(NativeDbaError::Invalid(
                "DBA controller has no source channels".into(),
            ));
        }
        // Val and Griffin ship clips that list a controller twice with the same
        // tracks. Only a repeat that disagrees is a corrupt directory.
        let channels = [pos_time, pos, rot_time, rot];
        match seen.insert(crc, channels) {
            None => {}
            Some(previous) if previous == channels => continue,
            Some(_) => {
                return Err(NativeDbaError::Invalid(
                    "Conflicting repeated DBA controller CRC".into(),
                ));
            }
        }
        controllers.push(DbaController {
            crc,
            position_times: pos_time,
            position: pos,
            rotation_times: rot_time,
            rotation: rot,
        });
    }
    Ok(controllers)
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
        clip.extend_from_slice(&[0u8; 132]); // motion parameters
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

    /// In-place database: one clip, with one bone keyed on frames 0 and 3 by a
    /// bitset, rotated by a float quaternion and moved by a float position.
    fn decodable_database(second_controller: Option<[i32; 4]>) -> Vec<u8> {
        let mut tracks = Vec::new();
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0] {
            tracks.extend_from_slice(&value.to_le_bytes());
        }
        for value in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            tracks.extend_from_slice(&value.to_le_bytes());
        }
        for half in [0u16, 3, 2, 0b1001] {
            tracks.extend_from_slice(&half.to_le_bytes());
        }
        let extent = tracks.len() as i32;
        let mut chunk = Vec::new();
        for count in [1u32, 1, 1, 1] {
            chunk.extend_from_slice(&count.to_le_bytes());
        }
        let bins = |slot: usize, size: usize| {
            let mut out = vec![0u8; size * 4];
            out[slot * 4] = 1;
            out
        };
        chunk.extend_from_slice(&4u16.to_le_bytes()); // time track length in u16s
        chunk.extend(bins(6, 7)); // bitset key times
        chunk.extend_from_slice(&2u16.to_le_bytes());
        chunk.extend(bins(2, 9)); // uncompressed positions
        chunk.extend_from_slice(&2u16.to_le_bytes());
        chunk.extend(bins(1, 9)); // uncompressed quaternions
        for offset in [-8i32, -32, -extent, -extent] {
            chunk.extend_from_slice(&offset.to_le_bytes());
        }
        chunk.extend_from_slice(&0u32.to_le_bytes()); // streaming pad
        while !(32 + chunk.len()).is_multiple_of(4) {
            chunk.push(0);
        }
        let name = b"anims/run.caf";
        chunk.extend_from_slice(&(name.len() as u16).to_le_bytes());
        chunk.extend_from_slice(name);
        let mut params = [0u8; 132];
        params[..4].copy_from_slice(&2u32.to_le_bytes()); // cyclic
        params[8..12].copy_from_slice(&1u32.to_le_bytes());
        params[12..16].copy_from_slice(&(1.0f32 / 30.0).to_le_bytes());
        params[20..24].copy_from_slice(&3u32.to_le_bytes());
        chunk.extend_from_slice(&params);
        chunk.extend_from_slice(&0u16.to_le_bytes()); // footplants
        let count = 1 + u16::from(second_controller.is_some());
        chunk.extend_from_slice(&count.to_le_bytes());
        chunk.extend_from_slice(&0i32.to_le_bytes()); // streaming pointer
        chunk.extend_from_slice(&77u32.to_le_bytes());
        for index in [0i32; 4] {
            chunk.extend_from_slice(&index.to_le_bytes());
        }
        if let Some(channels) = second_controller {
            chunk.extend_from_slice(&77u32.to_le_bytes());
            for index in channels {
                chunk.extend_from_slice(&index.to_le_bytes());
            }
        }
        while !(32 + chunk.len()).is_multiple_of(4) {
            chunk.push(0);
        }
        chunk.extend_from_slice(&tracks);
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(b"CrChF\x07\0\0");
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&16u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&0x0905100du32.to_le_bytes());
        bytes[24..28].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
        bytes[28..32].copy_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&chunk);
        bytes
    }

    #[test]
    fn decodes_bitset_times_and_shared_tracks_into_a_playable_motion() {
        let data = decodable_database(None);
        let motion = decode_native_dba_clip(&data, "ANIMS/Run.caf").unwrap();
        assert!(motion.cyclic && !motion.additive);
        assert!((motion.fps - 30.0).abs() < 0.01);
        assert_eq!(motion.end_frame, 3.0);
        let track = &motion.controllers[0];
        assert_eq!(track.bone_crc32, 77);
        assert_eq!(track.rotation_times, [0.0, 3.0]);
        assert_eq!(track.rotations, [[0., 0., 0., 1.], [0., 0., 1., 0.]]);
        assert_eq!(track.positions, [[1., 2., 3.], [4., 5., 6.]]);
        let moved = motion.sample_seconds(0.05).unwrap()[0].translation.unwrap();
        assert!((moved[0] - 2.5).abs() < 1e-3 && (moved[2] - 4.5).abs() < 1e-3);
        assert!(decode_native_dba_clip(&data, "anims/walk.caf").is_err());
    }

    #[test]
    fn identical_repeated_controller_is_kept_once_and_a_conflict_is_refused() {
        let repeat = decodable_database(Some([0; 4]));
        let motion = decode_native_dba_clip(&repeat, "anims/run.caf").unwrap();
        assert_eq!(motion.controllers.len(), 1);
        let conflict = decodable_database(Some([0, 0, -1, -1]));
        assert!(index_native_dba_bytes(&conflict).is_err());
    }

    /// Set EVOLVE_BENCH_LIBRARY to an imported library to decode everything it holds.
    #[test]
    #[ignore]
    fn decodes_every_clip_of_every_imported_database() {
        let library = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let mut pending = fs::read_dir(&library)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("AnimationsDBA.pak__"))
            })
            .collect::<Vec<_>>();
        let (mut databases, mut clips) = (0, 0);
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder).unwrap().filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "dba") {
                    let data = fs::read(&path).unwrap();
                    let directory = parse_directory(&data)
                        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                    databases += 1;
                    for clip in &directory.clips {
                        let motion = decode_native_dba_clip(&data, &clip.name)
                            .unwrap_or_else(|error| panic!("{}: {error}", clip.name));
                        assert!(motion.duration_seconds().is_finite() && motion.fps > 1.0);
                        motion
                            .sample_seconds(motion.duration_seconds() * 0.5)
                            .unwrap();
                        clips += 1;
                    }
                }
            }
        }
        println!("decoded {clips} clips from {databases} databases");
        assert!(clips > 0);
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
