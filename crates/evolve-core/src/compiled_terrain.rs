//! Read-only decoding of the engine-compiled `terrain/terrain.dat` (version
//! 25) shipped in Stage 2 `Game/Levels/<map>/level.pak`.
//!
//! Maps ported from Legacy ship no editor `Heightmap.dat`, `VegetationMap.dat`
//! or brush layers; their heightmap, brushes and vegetation exist only here.
//! The layout was verified against every shipped Stage 2 map and, for the
//! Tutorial, against its editor sources: all 268,213 height samples agree with
//! `Heightmap.dat` within one 0.05 m step, every top-level `.lyr` brush matches
//! a brush record's position, model and rotation × scale, and every vegetation
//! record matches a `VegetationMap.dat` instance's position, model and scale.
//!
//! File layout (little endian):
//! - header: version u8 (25), 3 flag bytes, file size u32, heightmap size in
//!   units, unit size (m), sector size (m), sector table size, Z ratio, ocean level
//! - vegetation groups: count, then 352-byte records (256-byte CGF path + params)
//! - brush CGF table and material table: count, then 256-byte names
//! - terrain quadtree, depth first: 44-byte node header (version 7, AABB,
//!   height offset, range, sample side, palette size), side² u16 samples
//!   (X-major; height = offset + (raw >> 4) × 0.05 m, raw & 15 = surface,
//!   15 = hole), five geometry-error floats, the padded surface palette, then
//!   four children until the node is one sector
//! - object octree, depth first: 32-byte node header (version 5, child mask,
//!   AABB, object block size), the block's render-node records, then children
use crate::scene::Transform;
use crate::terrain::{TerrainError, TerrainInfo, TerrainMesh, height_grid_mesh};
use std::fs;
use std::path::Path;

pub const COMPILED_TERRAIN_VERSION: u8 = 25;
const MAX_FILE: usize = 256 * 1024 * 1024;
const MAX_UNITS: u32 = 4096;
const MAX_TABLE: usize = 65_536;
const MAX_OCTREE_DEPTH: usize = 32;
/// Height of one 12-bit sample step; the low four bits are the surface index.
const HEIGHT_STEP: f32 = 0.05;
const HOLE_SURFACE: u16 = 15;

const BRUSH: u32 = 0x1A;
const VEGETATION: u32 = 0x02;
const MERGED_MESH: u32 = 0x17;

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledBrush {
    /// Index into [`CompiledTerrain::models`].
    pub model: usize,
    /// Index into [`CompiledTerrain::materials`], when the record names one.
    pub material: Option<usize>,
    pub layer: u16,
    /// Row-major 3×4 world matrix: rotation × scale, translation in column 3.
    pub matrix: [f32; 12],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledVegetation {
    /// Index into [`CompiledTerrain::vegetation_groups`].
    pub group: usize,
    pub layer: u16,
    pub position: [f32; 3],
    pub scale: f32,
    /// Rotation about Z, X and Y in 1/256 turns.
    pub angles: [u8; 3],
}

#[derive(Clone, Debug)]
pub struct CompiledTerrain {
    pub version: u8,
    /// Heightmap side in units; the sample grid has `units + 1` points per side.
    pub units: u32,
    pub unit_size: f32,
    pub sector_size: u32,
    pub ocean_level: f32,
    /// `(units + 1)²` heights, X-major like `Heightmap.dat`.
    pub heights: Vec<f32>,
    /// One flag per sample: the quad starting here is a terrain hole.
    pub holes: Vec<bool>,
    pub vegetation_groups: Vec<String>,
    pub models: Vec<String>,
    pub materials: Vec<String>,
    pub brushes: Vec<CompiledBrush>,
    pub vegetation: Vec<CompiledVegetation>,
    /// Merged-mesh cells reference per-sector instance files kept elsewhere.
    pub merged_mesh_cells: usize,
    /// Decals, roads, water volumes, lights and similar records (framed, not decoded).
    pub other_records: usize,
    /// Octree blocks whose remainder used an unknown record type and was skipped.
    pub skipped_blocks: usize,
}

struct Reader<'a> {
    data: &'a [u8],
}

impl Reader<'_> {
    fn bytes(&self, at: usize, len: usize) -> Result<&[u8], TerrainError> {
        at.checked_add(len)
            .and_then(|end| self.data.get(at..end))
            .ok_or_else(|| TerrainError::Format(format!("terrain.dat truncated at {at}")))
    }
    fn u16(&self, at: usize) -> Result<u16, TerrainError> {
        let b = self.bytes(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, at: usize) -> Result<u32, TerrainError> {
        let b = self.bytes(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&self, at: usize) -> Result<i32, TerrainError> {
        Ok(self.u32(at)? as i32)
    }
    fn f32(&self, at: usize) -> Result<f32, TerrainError> {
        Ok(f32::from_bits(self.u32(at)?))
    }
    fn name(&self, at: usize) -> Result<String, TerrainError> {
        let raw = self.bytes(at, 256)?;
        let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).replace('\\', "/"))
    }
    fn table(&self, at: usize, stride: usize) -> Result<(Vec<String>, usize), TerrainError> {
        let count = self.u32(at)? as usize;
        if count > MAX_TABLE {
            return Err(TerrainError::Limits);
        }
        let mut names = Vec::with_capacity(count);
        for index in 0..count {
            names.push(self.name(at + 4 + index * stride)?);
        }
        Ok((names, at + 4 + count * stride))
    }
}

fn format(message: impl Into<String>) -> TerrainError {
    TerrainError::Format(message.into())
}

pub fn read_compiled_terrain(path: &Path) -> Result<CompiledTerrain, TerrainError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE as u64 {
        return Err(TerrainError::Limits);
    }
    decode_compiled_terrain(&fs::read(path)?)
}

pub fn decode_compiled_terrain(data: &[u8]) -> Result<CompiledTerrain, TerrainError> {
    if data.len() > MAX_FILE || data.len() < 36 {
        return Err(TerrainError::Limits);
    }
    let r = Reader { data };
    if data[0] != COMPILED_TERRAIN_VERSION {
        return Err(format(format!(
            "unsupported compiled terrain version {}",
            data[0]
        )));
    }
    if r.u32(4)? as usize != data.len() {
        return Err(format(
            "compiled terrain size field does not match the file",
        ));
    }
    let units = r.u32(8)?;
    let unit = r.u32(12)?;
    let sector = r.u32(16)?;
    let ocean_level = r.f32(28)?;
    if !(16..=MAX_UNITS).contains(&units)
        || !units.is_power_of_two()
        || unit == 0
        || !unit.is_power_of_two()
        || unit > 64
        || sector < unit * 2
        || !sector.is_power_of_two()
        || sector > units * unit
        || !ocean_level.is_finite()
    {
        return Err(TerrainError::Limits);
    }
    let (vegetation_groups, at) = r.table(32, 352)?;
    let (models, at) = r.table(at, 256)?;
    let (materials, at) = r.table(at, 256)?;

    let side = units as usize + 1;
    let mut terrain = CompiledTerrain {
        version: data[0],
        units,
        unit_size: unit as f32,
        sector_size: sector,
        ocean_level,
        heights: vec![f32::NAN; side * side],
        holes: vec![false; side * side],
        vegetation_groups,
        models,
        materials,
        brushes: Vec::new(),
        vegetation: Vec::new(),
        merged_mesh_cells: 0,
        other_records: 0,
        skipped_blocks: 0,
    };
    let at = terrain_node(&r, at, units * unit, &mut terrain)?;
    if terrain.heights.iter().any(|h| !h.is_finite()) {
        return Err(format(
            "terrain quadtree does not cover every height sample",
        ));
    }
    let end = octree_node(&r, at, 0, &mut terrain)?;
    if end != data.len() {
        return Err(format(format!(
            "object octree ended at {end} of {} bytes",
            data.len()
        )));
    }
    Ok(terrain)
}

fn terrain_node(
    r: &Reader<'_>,
    at: usize,
    size: u32,
    out: &mut CompiledTerrain,
) -> Result<usize, TerrainError> {
    if r.u16(at)? != 7 {
        return Err(format(format!("terrain node version at {at} is not 7")));
    }
    let min_x = r.f32(at + 4)?;
    let min_y = r.f32(at + 8)?;
    let offset = r.f32(at + 28)?;
    let n = r.u32(at + 36)? as usize;
    let palette = r.u32(at + 40)? as usize;
    if n > 257 || palette > 256 || !offset.is_finite() {
        return Err(TerrainError::Limits);
    }
    let samples_at = at + 44;
    let mut next = (samples_at + n * n * 2 + 3) & !3;
    next += 20 + ((palette + 3) & !3);
    r.bytes(samples_at, next - samples_at)?;
    if size > out.sector_size {
        for _ in 0..4 {
            next = terrain_node(r, next, size / 2, out)?;
        }
        return Ok(next);
    }
    // One sector: write its samples into the shared grid.
    let unit = out.unit_size;
    let cells = (out.sector_size as f32 / unit) as usize;
    if n < 2 || !cells.is_multiple_of(n - 1) {
        return Err(format(format!(
            "sector sample side {n} at {at} is unsupported"
        )));
    }
    let x0 = min_x / unit;
    let y0 = min_y / unit;
    if x0.fract() != 0.0 || y0.fract() != 0.0 || x0 < 0.0 || y0 < 0.0 {
        return Err(format(format!(
            "sector origin at {at} is not on the unit grid"
        )));
    }
    let (x0, y0) = (x0 as usize, y0 as usize);
    let side = out.units as usize + 1;
    let units = out.units as usize;
    if x0 + cells > units || y0 + cells > units || x0 % cells != 0 || y0 % cells != 0 {
        return Err(format(format!("sector at {at} lies outside the heightmap")));
    }
    let step = cells / (n - 1);
    let raw = |i: usize, j: usize| r.u16(samples_at + (i * n + j) * 2);
    let height = |value: u16| offset + f32::from(value >> 4) * HEIGHT_STEP;
    for lx in 0..=cells {
        for ly in 0..=cells {
            let (fx, fy) = (lx as f32 / step as f32, ly as f32 / step as f32);
            let (i0, j0) = ((lx / step).min(n - 1), (ly / step).min(n - 1));
            let (i1, j1) = ((i0 + 1).min(n - 1), (j0 + 1).min(n - 1));
            let (tx, ty) = (fx - i0 as f32, fy - j0 as f32);
            let h00 = height(raw(i0, j0)?);
            let h10 = height(raw(i1, j0)?);
            let h01 = height(raw(i0, j1)?);
            let h11 = height(raw(i1, j1)?);
            let value =
                (h00 * (1.0 - tx) + h10 * tx) * (1.0 - ty) + (h01 * (1.0 - tx) + h11 * tx) * ty;
            let nearest = raw(
                ((fx + 0.5) as usize).min(n - 1),
                ((fy + 0.5) as usize).min(n - 1),
            )?;
            let index = (x0 + lx) * side + (y0 + ly);
            out.heights[index] = value;
            if lx < cells && ly < cells {
                out.holes[index] = nearest & 15 == HOLE_SURFACE;
            }
        }
    }
    Ok(next)
}

fn octree_node(
    r: &Reader<'_>,
    at: usize,
    depth: usize,
    out: &mut CompiledTerrain,
) -> Result<usize, TerrainError> {
    if depth > MAX_OCTREE_DEPTH {
        return Err(TerrainError::Limits);
    }
    if r.u16(at)? != 5 {
        return Err(format(format!(
            "object octree node version at {at} is not 5"
        )));
    }
    let mask = r.u16(at + 2)?;
    let size = r.i32(at + 28)?;
    let start = at + 32;
    let end = start
        .checked_add(usize::try_from(size).map_err(|_| TerrainError::Limits)?)
        .ok_or(TerrainError::Limits)?;
    r.bytes(start, end - start)?;
    object_block(r, start, end, out)?;
    let mut next = end;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            next = octree_node(r, next, depth + 1, out)?;
        }
    }
    Ok(next)
}

fn record_size(r: &Reader<'_>, at: usize, kind: u32) -> Result<Option<usize>, TerrainError> {
    let count = |offset: usize| -> Result<usize, TerrainError> {
        let value = r.u32(at + offset)? as usize;
        if value > 1 << 20 {
            return Err(TerrainError::Limits);
        }
        Ok(value)
    };
    Ok(Some(match kind {
        VEGETATION => 64,
        0x07 => 120,
        0x0C => 92,
        0x16 => 108,
        BRUSH => 104,
        MERGED_MESH => 68 + 8 * count(40)?,
        0x0B => 72 + 12 * count(40)?,
        0x19 => 244 + 12 * (count(124)? + count(136)?),
        _ => return Ok(None),
    }))
}

fn object_block(
    r: &Reader<'_>,
    mut at: usize,
    end: usize,
    out: &mut CompiledTerrain,
) -> Result<(), TerrainError> {
    while at < end {
        let kind = r.u32(at)?;
        let Some(size) = record_size(r, at, kind)? else {
            out.skipped_blocks += 1;
            return Ok(());
        };
        if at + size > end {
            return Err(format(format!(
                "render node record at {at} overruns its block"
            )));
        }
        match kind {
            BRUSH => {
                let model = r.u16(at + 36)? as usize;
                let mut matrix = [0f32; 12];
                for (index, value) in matrix.iter_mut().enumerate() {
                    *value = r.f32(at + 40 + index * 4)?;
                }
                let material = r.i32(at + 92)?;
                if model < out.models.len() && matrix.iter().all(|v| v.is_finite()) {
                    out.brushes.push(CompiledBrush {
                        model,
                        material: usize::try_from(material)
                            .ok()
                            .filter(|index| *index < out.materials.len()),
                        layer: r.u16(at + 28)?,
                        matrix,
                    });
                }
            }
            VEGETATION => {
                let group = r.u16(at + 36)? as usize;
                let position = [r.f32(at + 40)?, r.f32(at + 44)?, r.f32(at + 48)?];
                let scale = r.f32(at + 52)?;
                let angles = r.bytes(at + 57, 3)?;
                if group < out.vegetation_groups.len()
                    && position.iter().all(|v| v.is_finite())
                    && scale.is_finite()
                    && scale > 0.0
                {
                    out.vegetation.push(CompiledVegetation {
                        group,
                        layer: r.u16(at + 28)?,
                        position,
                        scale,
                        angles: [angles[0], angles[1], angles[2]],
                    });
                }
            }
            MERGED_MESH => out.merged_mesh_cells += 1,
            _ => out.other_records += 1,
        }
        at += size;
    }
    Ok(())
}

impl CompiledBrush {
    /// Decompose the world matrix into position, rotation and per-axis scale.
    /// A mirrored matrix keeps its handedness as a negative X scale.
    pub fn transform(&self) -> Transform {
        let m = &self.matrix;
        let column = |c: usize| [m[c], m[4 + c], m[8 + c]];
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut axes = [column(0), column(1), column(2)];
        let mut scale = axes.map(length);
        let det = axes[0][0] * (axes[1][1] * axes[2][2] - axes[2][1] * axes[1][2])
            - axes[1][0] * (axes[0][1] * axes[2][2] - axes[2][1] * axes[0][2])
            + axes[2][0] * (axes[0][1] * axes[1][2] - axes[1][1] * axes[0][2]);
        if det < 0.0 {
            scale[0] = -scale[0];
        }
        for (axis, s) in axes.iter_mut().zip(scale) {
            if s.abs() > f32::EPSILON {
                for value in axis.iter_mut() {
                    *value /= s;
                }
            }
        }
        // Rotation matrix r[row][col] from the normalised columns.
        let r = |row: usize, col: usize| axes[col][row];
        let trace = r(0, 0) + r(1, 1) + r(2, 2);
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [
                s / 4.0,
                (r(2, 1) - r(1, 2)) / s,
                (r(0, 2) - r(2, 0)) / s,
                (r(1, 0) - r(0, 1)) / s,
            ]
        } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
            let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
            [
                (r(2, 1) - r(1, 2)) / s,
                s / 4.0,
                (r(0, 1) + r(1, 0)) / s,
                (r(0, 2) + r(2, 0)) / s,
            ]
        } else if r(1, 1) > r(2, 2) {
            let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
            [
                (r(0, 2) - r(2, 0)) / s,
                (r(0, 1) + r(1, 0)) / s,
                s / 4.0,
                (r(1, 2) + r(2, 1)) / s,
            ]
        } else {
            let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
            [
                (r(1, 0) - r(0, 1)) / s,
                (r(0, 2) + r(2, 0)) / s,
                (r(1, 2) + r(2, 1)) / s,
                s / 4.0,
            ]
        };
        let norm = q
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt()
            .max(f32::EPSILON);
        Transform {
            position: [m[3], m[7], m[11]],
            quaternion: q.map(|v| v / norm),
            scale,
        }
    }
}

impl CompiledVegetation {
    /// Uniform scale and the record's Z, X and Y angles (1/256 turns).
    pub fn transform(&self) -> Transform {
        let turn = |byte: u8| f32::from(byte) * std::f32::consts::TAU / 256.0;
        let (z, x, y) = (
            turn(self.angles[0]),
            turn(self.angles[1]),
            turn(self.angles[2]),
        );
        let (sx, cx) = (x * 0.5).sin_cos();
        let (sy, cy) = (y * 0.5).sin_cos();
        let (sz, cz) = (z * 0.5).sin_cos();
        Transform {
            position: self.position,
            quaternion: [
                cx * cy * cz + sx * sy * sz,
                sx * cy * cz - cx * sy * sz,
                cx * sy * cz + sx * cy * sz,
                cx * cy * sz - sx * sy * cz,
            ],
            scale: [self.scale; 3],
        }
    }
}

impl CompiledTerrain {
    /// Grid mesh in world metres, omitting hole quads. UVs span the whole map.
    pub fn mesh(&self) -> Result<TerrainMesh, TerrainError> {
        let side = self.units + 1;
        let info = TerrainInfo {
            width: side,
            height: side,
            unit: self.unit_size,
            max_height: self.heights.iter().copied().fold(0.0, f32::max),
            decoded_samples: self.heights.len(),
            stride: 1,
        };
        let s = side as usize;
        height_grid_mesh(
            info,
            |x, y| self.heights[x as usize * s + y as usize],
            |x, y| self.holes[x as usize * s + y as usize],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, value: &str) {
        let mut raw = [0u8; 256];
        raw[..value.len()].copy_from_slice(value.as_bytes());
        out.extend_from_slice(&raw);
    }

    fn f(out: &mut Vec<u8>, values: &[f32]) {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn u(out: &mut Vec<u8>, values: &[u32]) {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    /// A 16×16-unit map of one 16 m sector: 3×3 samples, one hole sample.
    fn fixture() -> Vec<u8> {
        let mut d = vec![25, 0, 2, 1];
        u(&mut d, &[0, 16, 1, 16, 1]);
        f(&mut d, &[1.0 / 64.0, 0.0]);
        u(&mut d, &[1]);
        name(&mut d, "objects/natural/trees/fir.cgf");
        d.extend_from_slice(&[0u8; 96]);
        u(&mut d, &[1]);
        name(&mut d, "objects/natural/rocks/rock.cgf");
        u(&mut d, &[1]);
        name(&mut d, "materials/rock");
        // Terrain leaf: version 7, no-hole flag, AABB, offset 10, range, 3×3, palette 1.
        d.extend_from_slice(&[7, 0, 0, 0]);
        f(&mut d, &[0.0, 0.0, 7.0, 16.0, 16.0, 30.0, 10.0, 0.0]);
        u(&mut d, &[3, 1]);
        for raw in [0u16, 160, 320, 0, 160, 320, 0, 160, 15] {
            d.extend_from_slice(&raw.to_le_bytes());
        }
        d.extend_from_slice(&[0, 0]);
        f(&mut d, &[0.0; 5]);
        d.extend_from_slice(&[4, 0xde, 0xde, 0xde]);
        // Octree root: version 5, no children, one brush and one vegetation record.
        let mut block = Vec::new();
        u(&mut block, &[BRUSH]);
        f(&mut block, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        u(&mut block, &[8, 0]);
        block.extend_from_slice(&[0, 0, 100, 100]);
        f(
            &mut block,
            &[0.0, -2.0, 0.0, 5.0, 2.0, 0.0, 0.0, 6.0, 0.0, 0.0, 2.0, 7.0],
        );
        u(&mut block, &[0, 0, 0, 0]);
        u(&mut block, &[VEGETATION]);
        f(&mut block, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        u(&mut block, &[0, 0]);
        block.extend_from_slice(&[0, 0, 100, 100]);
        f(&mut block, &[3.0, 4.0, 12.0, 1.5]);
        block.extend_from_slice(&[0, 64, 0, 0, 0x7f, 0x7f, 0, 0]);
        d.extend_from_slice(&[5, 0, 0, 0]);
        f(&mut d, &[0.0, 0.0, 0.0, 16.0, 16.0, 16.0]);
        u(&mut d, &[block.len() as u32]);
        d.extend_from_slice(&block);
        let size = d.len() as u32;
        d[4..8].copy_from_slice(&size.to_le_bytes());
        d
    }

    #[test]
    fn decodes_heights_holes_brushes_and_vegetation() {
        let t = decode_compiled_terrain(&fixture()).unwrap();
        assert_eq!((t.units, t.sector_size), (16, 16));
        let side = 17;
        // raw 160 >> 4 = 10 steps of 0.05 m above the 10 m offset.
        assert_eq!(t.heights[0], 10.0);
        assert!((t.heights[8 * side] - 10.0).abs() < 1e-6);
        assert!((t.heights[8] - 10.5).abs() < 1e-6);
        assert!((t.heights[16] - 11.0).abs() < 1e-6);
        assert!((t.heights[4] - 10.25).abs() < 1e-6, "interpolated");
        assert!(!t.holes[0]);
        assert!(t.holes[15 * side + 15], "sample (2,2) carries surface 15");
        assert_eq!(t.models, vec!["objects/natural/rocks/rock.cgf"]);
        assert_eq!(t.brushes.len(), 1);
        assert_eq!(t.brushes[0].layer, 8);
        assert_eq!(t.brushes[0].material, Some(0));
        assert_eq!(t.brushes[0].matrix[3], 5.0);
        // 90° about Z with uniform scale 2.
        let tf = t.brushes[0].transform();
        assert_eq!(tf.position, [5.0, 6.0, 7.0]);
        assert_eq!(tf.scale, [2.0, 2.0, 2.0]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        for (got, want) in tf.quaternion.iter().zip([half, 0.0, 0.0, half]) {
            assert!((got - want).abs() < 1e-6, "{:?}", tf.quaternion);
        }
        let mut mirrored = t.brushes[0].clone();
        mirrored.matrix[0] = -mirrored.matrix[0];
        mirrored.matrix[4] = -mirrored.matrix[4];
        mirrored.matrix[8] = -mirrored.matrix[8];
        assert_eq!(mirrored.transform().scale[0], -2.0);
        let veg = t.vegetation[0].transform();
        assert!((veg.quaternion[0] - half).abs() < 1e-6 && (veg.quaternion[3] - half).abs() < 1e-6);
        assert_eq!(t.vegetation.len(), 1);
        assert_eq!(t.vegetation[0].position, [3.0, 4.0, 12.0]);
        assert_eq!(t.vegetation[0].angles, [64, 0, 0]);
        let mesh = t.mesh().unwrap();
        assert_eq!(mesh.vertices.len(), side * side);
        assert!(mesh.indices.len() < 16 * 16 * 6, "hole quads are omitted");
    }

    #[test]
    fn refuses_wrong_version_size_and_truncation() {
        let good = fixture();
        let mut bad = good.clone();
        bad[0] = 24;
        assert!(decode_compiled_terrain(&bad).is_err());
        let mut bad = good.clone();
        bad[4] ^= 1;
        assert!(decode_compiled_terrain(&bad).is_err());
        assert!(decode_compiled_terrain(&good[..good.len() - 8]).is_err());
    }
}
