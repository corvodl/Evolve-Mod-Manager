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
//!   AABB, object block size), the block's render-node records, then children.
//!   Every record starts with its type, an AABB and the layer id (u16 at +28).
//!   Two record types come in two sizes, so a block is framed as the only
//!   sequence of record sizes that ends exactly at the block end.
//!
//! Decals (0x07), roads (0x0B) and water volumes (0x19) were matched against
//! every Tutorial `.lyr` Decal, Road and WaterVolume: positions, materials,
//! sort priorities, projection depth and road texture lengths agree.
//!
//! `terrain/merged_meshes_sectors/sector_X_Y_Z.dat` holds merged grass and
//! small rocks: blocks of magic `0xCAFEBAB6`, cell X, Y, Z (16 m cells),
//! vegetation group, count, then 12-byte instances: u16 X, Y, Z fractions of
//! the cell, scale byte / 64, then a signed-byte quaternion X, Y, Z, W / 127.
//! 11,660 of Tutorial's 12,182 merged instances match a `VegetationMap.dat`
//! instance's position and model exactly; the rest are ambiguous.
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
/// [`CompiledTerrain::surfaces`] value of a hole sample.
pub const HOLE_ID: u8 = 255;

const BRUSH: u32 = 0x1A;
const VEGETATION: u32 = 0x02;
const MERGED_MESH: u32 = 0x17;
const DECAL: u32 = 0x07;
const ROAD: u32 = 0x0B;
const WATER: u32 = 0x19;
/// Sizes a fixed-size record type may have; see the module documentation.
const RECORD_SIZES: &[(u32, &[usize])] = &[
    (VEGETATION, &[64]),
    (DECAL, &[120]),
    (0x0C, &[92, 68]),
    (0x16, &[108, 92]),
    (BRUSH, &[104]),
];
const MERGED_MAGIC: u32 = 0xCAFE_BAB6;
const MERGED_CELL: f32 = 16.0;
const MAX_MERGED_INSTANCES: usize = 4_000_000;

/// A deferred decal projected onto terrain and/or static geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledDecal {
    /// Index into [`CompiledTerrain::materials`].
    pub material: Option<usize>,
    pub layer: u16,
    pub position: [f32; 3],
    /// Row-major rotation; its columns are the decal's X, Y and Z axes.
    pub rotation: [f32; 9],
    /// Uniform size, as the editor's Scale.
    pub scale: f32,
    /// ProjectionDepth, 0..1.
    pub depth: f32,
    pub sort_priority: u32,
    /// ProjectionType 2: terrain only.
    pub terrain_only: bool,
}

/// A road as the engine baked it: a left/right vertex pair per cross-section.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledRoad {
    pub material: Option<usize>,
    pub layer: u16,
    pub sort_priority: i32,
    /// Texture V at the road end: its length over TileLength.
    pub texture_end: f32,
    pub vertices: Vec<[f32; 3]>,
}

/// A water volume's surface polygon, or one quad of a river.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledWater {
    pub material: Option<usize>,
    pub layer: u16,
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    pub points: Vec<[f32; 3]>,
}

/// One merged grass/rock instance from `merged_meshes_sectors`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MergedMeshInstance {
    /// Index into [`CompiledTerrain::vegetation_groups`].
    pub group: usize,
    pub position: [f32; 3],
    pub scale: f32,
    /// Normalized quaternion, CryEngine order w, x, y, z.
    pub rotation: [f32; 4],
}

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
    /// Each sample's SurfaceTypeID (the map's `leveldata.xml` SurfaceTypes),
    /// or [`HOLE_ID`].
    pub surfaces: Vec<u8>,
    pub vegetation_groups: Vec<String>,
    pub models: Vec<String>,
    pub materials: Vec<String>,
    pub brushes: Vec<CompiledBrush>,
    pub vegetation: Vec<CompiledVegetation>,
    pub decals: Vec<CompiledDecal>,
    pub roads: Vec<CompiledRoad>,
    pub water: Vec<CompiledWater>,
    /// Merged-mesh cells reference per-sector instance files kept elsewhere.
    pub merged_mesh_cells: usize,
    /// Lights, fog volumes and similar records (framed, not decoded).
    pub other_records: usize,
    /// Octree blocks no known record sizes could frame; they were skipped.
    pub skipped_blocks: usize,
    /// Where each of [`Self::brushes`] is stored, for deleting it.
    pub brush_records: Vec<RecordSpan>,
    /// Where each of [`Self::vegetation`] is stored, for deleting it.
    pub vegetation_records: Vec<RecordSpan>,
    /// Offset of the object octree's root node header.
    pub object_octree: usize,
}

/// A render-node record's bytes and the octree node whose block holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordSpan {
    /// Offset of the owning octree node's header; its block size is at +28.
    pub node: usize,
    pub at: usize,
    pub size: usize,
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
        surfaces: vec![HOLE_ID; side * side],
        vegetation_groups,
        models,
        materials,
        brushes: Vec::new(),
        vegetation: Vec::new(),
        decals: Vec::new(),
        roads: Vec::new(),
        water: Vec::new(),
        merged_mesh_cells: 0,
        other_records: 0,
        skipped_blocks: 0,
        brush_records: Vec::new(),
        vegetation_records: Vec::new(),
        object_octree: 0,
    };
    let at = terrain_node(&r, at, units * unit, &mut terrain)?;
    if terrain.heights.iter().any(|h| !h.is_finite()) {
        return Err(format(
            "terrain quadtree does not cover every height sample",
        ));
    }
    terrain.object_octree = at;
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
    let palette_at = ((samples_at + n * n * 2 + 3) & !3) + 20;
    let next = palette_at + ((palette + 3) & !3);
    r.bytes(samples_at, next - samples_at)?;
    let palette = r.bytes(palette_at, palette)?;
    if size > out.sector_size {
        let mut next = next;
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
            // Bits past the palette's end mean its last entry.
            out.surfaces[index] = match nearest & 15 {
                HOLE_SURFACE => HOLE_ID,
                bits => palette
                    .get(usize::from(bits).min(palette.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(HOLE_ID),
            };
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
    object_block(r, at, start, end, out)?;
    let mut next = end;
    for bit in 0..8 {
        if mask & (1 << bit) != 0 {
            next = octree_node(r, next, depth + 1, out)?;
        }
    }
    Ok(next)
}

/// Every size the record at `at` may have; empty for an unknown type.
fn record_sizes(r: &Reader<'_>, at: usize, end: usize) -> Vec<usize> {
    let Ok(kind) = r.u32(at) else {
        return Vec::new();
    };
    if let Some((_, sizes)) = RECORD_SIZES.iter().find(|(k, _)| *k == kind) {
        return sizes.to_vec();
    }
    // A count read past the block only rules this size out.
    let count = |offset: usize| {
        r.u32(at + offset)
            .ok()
            .map(|value| value as usize)
            .filter(|value| *value <= (end - at) / 8)
    };
    let size = match kind {
        MERGED_MESH => count(40).map(|n| 68 + 8 * n),
        ROAD => count(40).map(|n| 72 + 12 * n),
        WATER => count(124).zip(count(136)).map(|(a, b)| 244 + 12 * (a + b)),
        _ => None,
    };
    size.into_iter().collect()
}

/// Record offsets of a block: a sequence of record sizes ending exactly at
/// `end`, found breadth first over the alternatives.
fn frame_block(r: &Reader<'_>, start: usize, end: usize) -> Option<Vec<usize>> {
    let mut previous = std::collections::HashMap::from([(start, start)]);
    let mut frontier = vec![start];
    while !frontier.is_empty() && !previous.contains_key(&end) {
        let mut next = Vec::new();
        for at in frontier {
            for size in record_sizes(r, at, end) {
                let to = at + size;
                if size > 0 && to <= end && !previous.contains_key(&to) {
                    previous.insert(to, at);
                    next.push(to);
                }
            }
        }
        frontier = next;
    }
    previous.contains_key(&end).then(|| {
        let mut offsets = Vec::new();
        let mut at = end;
        while at != start {
            at = previous[&at];
            offsets.push(at);
        }
        offsets.reverse();
        offsets
    })
}

fn read_points(r: &Reader<'_>, at: usize, count: usize) -> Result<Vec<[f32; 3]>, TerrainError> {
    (0..count)
        .map(|i| {
            let p = at + i * 12;
            Ok([r.f32(p)?, r.f32(p + 4)?, r.f32(p + 8)?])
        })
        .collect()
}

fn object_block(
    r: &Reader<'_>,
    node: usize,
    start: usize,
    end: usize,
    out: &mut CompiledTerrain,
) -> Result<(), TerrainError> {
    if start == end {
        return Ok(());
    }
    let Some(offsets) = frame_block(r, start, end) else {
        out.skipped_blocks += 1;
        return Ok(());
    };
    let materials = out.materials.len();
    let material = |index: i32| usize::try_from(index).ok().filter(|i| *i < materials);
    for (position, &at) in offsets.iter().enumerate() {
        let span = RecordSpan {
            node,
            at,
            size: offsets.get(position + 1).copied().unwrap_or(end) - at,
        };
        let kind = r.u32(at)?;
        let layer = r.u16(at + 28)?;
        match kind {
            BRUSH => {
                let model = r.u16(at + 36)? as usize;
                let mut matrix = [0f32; 12];
                for (index, value) in matrix.iter_mut().enumerate() {
                    *value = r.f32(at + 40 + index * 4)?;
                }
                if model < out.models.len() && matrix.iter().all(|v| v.is_finite()) {
                    out.brushes.push(CompiledBrush {
                        model,
                        material: material(r.i32(at + 92)?),
                        layer,
                        matrix,
                    });
                    out.brush_records.push(span);
                }
            }
            DECAL => {
                let position = [r.f32(at + 44)?, r.f32(at + 48)?, r.f32(at + 52)?];
                let mut rotation = [0f32; 9];
                for (index, value) in rotation.iter_mut().enumerate() {
                    *value = r.f32(at + 68 + index * 4)?;
                }
                let scale = r.f32(at + 104)?;
                if position.iter().chain(&rotation).all(|v| v.is_finite())
                    && scale.is_finite()
                    && scale > 0.0
                {
                    out.decals.push(CompiledDecal {
                        material: material(r.i32(at + 108)?),
                        layer,
                        position,
                        rotation,
                        scale,
                        depth: 1.0 - f32::from(r.bytes(at + 43, 1)?[0]) / 255.0,
                        sort_priority: r.u32(at + 112)?,
                        terrain_only: r.bytes(at + 40, 1)?[0] == 2,
                    });
                }
            }
            ROAD => {
                let count = r.u32(at + 40)? as usize;
                let vertices = read_points(r, at + 72, count)?;
                let texture_end = r.f32(at + 56)?;
                if count >= 4
                    && count.is_multiple_of(2)
                    && texture_end.is_finite()
                    && vertices.iter().flatten().all(|v| v.is_finite())
                {
                    out.roads.push(CompiledRoad {
                        material: material(r.i32(at + 48)?),
                        layer,
                        sort_priority: r.i32(at + 44)?,
                        texture_end,
                        vertices,
                    });
                }
            }
            WATER => {
                let count = r.u32(at + 124)? as usize;
                let points = read_points(r, at + 244, count)?;
                let fog_density = r.f32(at + 56)?;
                let fog_color = [r.f32(at + 60)?, r.f32(at + 64)?, r.f32(at + 68)?];
                if count >= 3
                    && points.iter().flatten().all(|v| v.is_finite())
                    && fog_density.is_finite()
                    && fog_color.iter().all(|v| v.is_finite())
                {
                    out.water.push(CompiledWater {
                        material: material(r.i32(at + 52)?),
                        layer,
                        fog_density,
                        fog_color,
                        points,
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
                        layer,
                        position,
                        scale,
                        angles: [angles[0], angles[1], angles[2]],
                    });
                    out.vegetation_records.push(span);
                }
            }
            MERGED_MESH => out.merged_mesh_cells += 1,
            _ => out.other_records += 1,
        }
    }
    Ok(())
}

/// Which decoded placement of a `terrain.dat` to delete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledPlacement {
    /// Index into [`CompiledTerrain::brushes`].
    Brush(usize),
    /// Index into [`CompiledTerrain::vegetation`].
    Vegetation(usize),
}

/// Remove one brush or vegetation placement from `terrain.dat`: its record's
/// bytes leave the octree block that holds it and that block's size shrinks
/// to match. Nothing else refers to records, so every other byte is kept.
/// The result must decode with exactly that one placement gone.
pub fn delete_compiled_placement(
    source: &[u8],
    placement: CompiledPlacement,
) -> Result<Vec<u8>, TerrainError> {
    let terrain = decode_compiled_terrain(source)?;
    let span = match placement {
        CompiledPlacement::Brush(index) => terrain.brush_records.get(index),
        CompiledPlacement::Vegetation(index) => terrain.vegetation_records.get(index),
    }
    .copied()
    .ok_or_else(|| format("That terrain.dat placement no longer exists"))?;
    let r = Reader { data: source };
    let block = r.u32(span.node + 28)? as usize;
    let size = span.size;
    if size == 0 || block < size {
        return Err(TerrainError::Limits);
    }
    let mut output = Vec::with_capacity(source.len() - size);
    output.extend_from_slice(&source[..span.at]);
    output.extend_from_slice(&source[span.at + size..]);
    output[span.node + 28..span.node + 32].copy_from_slice(&((block - size) as u32).to_le_bytes());
    let total = u32::try_from(output.len()).map_err(|_| TerrainError::Limits)?;
    output[4..8].copy_from_slice(&total.to_le_bytes());
    let check = decode_compiled_terrain(&output)?;
    let removed = match placement {
        CompiledPlacement::Brush(index) => {
            let mut expected = terrain.brushes.clone();
            expected.remove(index);
            check.brushes == expected && check.vegetation == terrain.vegetation
        }
        CompiledPlacement::Vegetation(index) => {
            let mut expected = terrain.vegetation.clone();
            expected.remove(index);
            check.vegetation == expected && check.brushes == terrain.brushes
        }
    };
    if !removed || check.heights != terrain.heights || check.skipped_blocks != 0 {
        return Err(format(
            "terrain.dat did not decode to exactly one fewer placement",
        ));
    }
    Ok(output)
}

/// Copy one brush or vegetation placement of a `terrain.dat` to a new world
/// position. The copy is the original record with its position and bounding
/// box moved, appended to the octree root's block: the root spans the whole
/// map cube and already holds records that fit no smaller cell. The position
/// must stay inside that cube, and the result must decode to exactly one
/// more placement, equal to the original apart from where it is.
pub fn copy_compiled_placement(
    source: &[u8],
    placement: CompiledPlacement,
    position: [f32; 3],
) -> Result<Vec<u8>, TerrainError> {
    let terrain = decode_compiled_terrain(source)?;
    let span = match placement {
        CompiledPlacement::Brush(index) => terrain.brush_records.get(index),
        CompiledPlacement::Vegetation(index) => terrain.vegetation_records.get(index),
    }
    .copied()
    .ok_or_else(|| format("That terrain.dat placement no longer exists"))?;
    let r = Reader { data: source };
    let root = terrain.object_octree;
    let cube: [f32; 6] = std::array::from_fn(|i| r.f32(root + 4 + i * 4).unwrap_or(f32::NAN));
    if position
        .iter()
        .enumerate()
        .any(|(i, v)| !v.is_finite() || *v < cube[i] || *v > cube[i + 3])
    {
        return Err(format(format!(
            "A terrain.dat placement must stay inside the map ({}–{} m on X and Y)",
            cube[0], cube[3]
        )));
    }
    // Where the record stores its position.
    let at = match placement {
        CompiledPlacement::Brush(_) => [52, 68, 84],
        CompiledPlacement::Vegetation(_) => [40, 44, 48],
    };
    let mut record = r.bytes(span.at, span.size)?.to_vec();
    let old: [f32; 3] =
        std::array::from_fn(|i| f32::from_le_bytes(record[at[i]..at[i] + 4].try_into().unwrap()));
    let delta: [f32; 3] = std::array::from_fn(|i| position[i] - old[i]);
    for (i, offset) in at.iter().enumerate() {
        record[*offset..offset + 4].copy_from_slice(&position[i].to_le_bytes());
    }
    for i in 0..6 {
        let offset = 4 + i * 4;
        let moved =
            f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap()) + delta[i % 3];
        record[offset..offset + 4].copy_from_slice(&moved.to_le_bytes());
    }
    let block = r.u32(root + 28)? as usize;
    let insert = root + 32 + block;
    let mut output = Vec::with_capacity(source.len() + record.len());
    output.extend_from_slice(&source[..insert]);
    output.extend_from_slice(&record);
    output.extend_from_slice(&source[insert..]);
    output[root + 28..root + 32].copy_from_slice(&((block + record.len()) as u32).to_le_bytes());
    let total = u32::try_from(output.len()).map_err(|_| TerrainError::Limits)?;
    output[4..8].copy_from_slice(&total.to_le_bytes());
    let check = decode_compiled_terrain(&output)?;
    let copied = match placement {
        CompiledPlacement::Brush(index) => {
            let original = &terrain.brushes[index];
            check.brushes.len() == terrain.brushes.len() + 1
                && check.vegetation == terrain.vegetation
                && check.brushes.iter().any(|brush| {
                    let mut moved = original.matrix;
                    moved[3] = position[0];
                    moved[7] = position[1];
                    moved[11] = position[2];
                    brush.model == original.model
                        && brush.material == original.material
                        && brush.matrix == moved
                })
        }
        CompiledPlacement::Vegetation(index) => {
            let original = &terrain.vegetation[index];
            check.vegetation.len() == terrain.vegetation.len() + 1
                && check.brushes == terrain.brushes
                && check.vegetation.iter().any(|plant| {
                    plant.group == original.group
                        && plant.position == position
                        && plant.angles == original.angles
                })
        }
    };
    if !copied || check.heights != terrain.heights || check.skipped_blocks != 0 {
        return Err(format(
            "terrain.dat did not decode to exactly one more, moved placement",
        ));
    }
    Ok(output)
}

/// Rotation matrix (row-major) of a w,x,y,z quaternion.
fn rotation_matrix(q: [f32; 4]) -> [[f32; 3]; 3] {
    let norm = q
        .iter()
        .map(|v| v * v)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    let [w, x, y, z] = q.map(|v| v / norm);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// The row-major 3×4 world matrix of a transform: rotation × scale, then
/// translation, as [`CompiledBrush::matrix`] stores it.
fn transform_matrix(transform: &Transform) -> [f32; 12] {
    let r = rotation_matrix(transform.quaternion);
    let mut m = [0f32; 12];
    for row in 0..3 {
        for col in 0..3 {
            m[row * 4 + col] = r[row][col] * transform.scale[col];
        }
        m[row * 4 + 3] = transform.position[row];
    }
    m
}

/// Apply a row-major 3×4 affine matrix to a point.
fn apply_affine(m: &[f32; 12], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        m[row * 4] * p[0] + m[row * 4 + 1] * p[1] + m[row * 4 + 2] * p[2] + m[row * 4 + 3]
    })
}

/// Inverse of a row-major 3×4 affine matrix, when it is invertible.
fn invert_affine(m: &[f32; 12]) -> Option<[f32; 12]> {
    let a = |r: usize, c: usize| f64::from(m[r * 4 + c]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = |r: usize, c: usize| {
        // Cofactor of (c, r), divided by the determinant.
        let (r0, r1) = match c {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (c0, c1) = match r {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let minor = a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
        let sign = if (r + c).is_multiple_of(2) { 1.0 } else { -1.0 };
        sign * minor / det
    };
    let mut out = [0f32; 12];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 4 + c] = inv(r, c) as f32;
        }
        out[r * 4 + 3] = -((0..3).map(|c| inv(r, c) * a(c, 3)).sum::<f64>()) as f32;
    }
    Some(out)
}

/// `outer` after `inner`, both row-major 3×4 affine matrices.
fn compose_affine(outer: &[f32; 12], inner: &[f32; 12]) -> [f32; 12] {
    let mut out = [0f32; 12];
    for r in 0..3 {
        for c in 0..4 {
            let mut v = (0..3)
                .map(|k| outer[r * 4 + k] * inner[k * 4 + c])
                .sum::<f32>();
            if c == 3 {
                v += outer[r * 4 + 3];
            }
            out[r * 4 + c] = v;
        }
    }
    out
}

/// A vegetation record's Z, X and Y angles (1/256 turns) closest to a
/// rotation: the inverse of [`CompiledVegetation::transform`].
fn vegetation_angles(q: [f32; 4]) -> [u8; 3] {
    let norm = q
        .iter()
        .map(|v| v * v)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    let [w, x, y, z] = q.map(|v| v / norm);
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    let turn = |angle: f32| {
        (angle / std::f32::consts::TAU * 256.0)
            .round()
            .rem_euclid(256.0) as u8
    };
    [turn(yaw), turn(roll), turn(pitch)]
}

/// Move, rotate or scale brush and vegetation placements of a `terrain.dat`.
/// Each record keeps its octree cell while its new bounding box still fits
/// that cell; one that leaves it moves to the root block, which spans the
/// whole map. Positions must stay inside the map. A vegetation record stores
/// one uniform scale and angles in 1/256 turns, so its scale is the mean of
/// the requested axes and its rotation is the nearest it can hold. The
/// result must decode to the same placements, the edited ones changed.
pub fn transform_compiled_placements(
    source: &[u8],
    edits: &[(CompiledPlacement, Transform)],
) -> Result<Vec<u8>, TerrainError> {
    if edits.is_empty() || edits.len() > 4096 {
        return Err(TerrainError::Limits);
    }
    let terrain = decode_compiled_terrain(source)?;
    let r = Reader { data: source };
    let root = terrain.object_octree;
    let cube_of = |node: usize| -> [f32; 6] {
        std::array::from_fn(|i| r.f32(node + 4 + i * 4).unwrap_or(f32::NAN))
    };
    let map = cube_of(root);
    let mut expected_brushes = terrain.brushes.clone();
    let mut expected_vegetation = terrain.vegetation.clone();
    // (span, new record bytes, stays in its cell)
    let mut changes = Vec::<(RecordSpan, Vec<u8>, bool)>::new();
    for (placement, transform) in edits {
        if transform
            .position
            .iter()
            .chain(&transform.quaternion)
            .chain(&transform.scale)
            .any(|v| !v.is_finite())
            || transform
                .scale
                .iter()
                .any(|s| s.abs() < 1e-4 || s.abs() > 1e4)
            || (0..3).any(|i| transform.position[i] < map[i] || transform.position[i] > map[i + 3])
        {
            return Err(format(format!(
                "A terrain.dat placement must stay inside the map ({}–{} m on X and Y) with a non-zero scale",
                map[0], map[3]
            )));
        }
        let (span, old_matrix, new_matrix) = match *placement {
            CompiledPlacement::Brush(index) => {
                let brush = terrain
                    .brushes
                    .get(index)
                    .ok_or_else(|| format("That terrain.dat placement no longer exists"))?;
                let matrix = transform_matrix(transform);
                expected_brushes[index].matrix = matrix;
                (terrain.brush_records[index], brush.matrix, matrix)
            }
            CompiledPlacement::Vegetation(index) => {
                let plant = terrain
                    .vegetation
                    .get(index)
                    .ok_or_else(|| format("That terrain.dat placement no longer exists"))?;
                let scale = transform.scale.iter().map(|s| s.abs()).sum::<f32>() / 3.0;
                let edited = CompiledVegetation {
                    position: transform.position,
                    scale,
                    angles: vegetation_angles(transform.quaternion),
                    ..plant.clone()
                };
                let matrices = (
                    transform_matrix(&plant.transform()),
                    transform_matrix(&edited.transform()),
                );
                expected_vegetation[index] = edited;
                (terrain.vegetation_records[index], matrices.0, matrices.1)
            }
        };
        if changes.iter().any(|(other, _, _)| other.at == span.at) {
            return Err(format("The same terrain.dat placement was edited twice"));
        }
        let mut record = r.bytes(span.at, span.size)?.to_vec();
        let write = |record: &mut Vec<u8>, offset: usize, value: f32| {
            record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        match *placement {
            CompiledPlacement::Brush(_) => {
                for (i, value) in new_matrix.iter().enumerate() {
                    write(&mut record, 40 + i * 4, *value);
                }
            }
            CompiledPlacement::Vegetation(index) => {
                let plant = &expected_vegetation[index];
                for (i, value) in plant.position.iter().enumerate() {
                    write(&mut record, 40 + i * 4, *value);
                }
                write(&mut record, 52, plant.scale);
                record[57..60].copy_from_slice(&plant.angles);
            }
        }
        // The bounding box moves with the placement: its corners, carried
        // from the old placement to the new one.
        let delta = compose_affine(
            &new_matrix,
            &invert_affine(&old_matrix)
                .ok_or_else(|| format("That placement's transform cannot be inverted"))?,
        );
        let old_box: [f32; 6] = std::array::from_fn(|i| {
            f32::from_le_bytes(record[4 + i * 4..8 + i * 4].try_into().unwrap_or([0; 4]))
        });
        let mut new_box = [
            f32::INFINITY,
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for corner in 0..8 {
            let point = [
                old_box[if corner & 1 == 0 { 0 } else { 3 }],
                old_box[if corner & 2 == 0 { 1 } else { 4 }],
                old_box[if corner & 4 == 0 { 2 } else { 5 }],
            ];
            let moved = apply_affine(&delta, point);
            for axis in 0..3 {
                new_box[axis] = new_box[axis].min(moved[axis]);
                new_box[axis + 3] = new_box[axis + 3].max(moved[axis]);
            }
        }
        if new_box.iter().any(|v| !v.is_finite()) {
            return Err(TerrainError::Limits);
        }
        for (i, value) in new_box.iter().enumerate() {
            write(&mut record, 4 + i * 4, *value);
        }
        let cell = cube_of(span.node);
        let fits = span.node == root
            || (0..3)
                .all(|axis| new_box[axis] >= cell[axis] && new_box[axis + 3] <= cell[axis + 3]);
        changes.push((span, record, fits));
    }
    let mut output = source.to_vec();
    for (span, record, fits) in &changes {
        if *fits {
            output[span.at..span.at + span.size].copy_from_slice(record);
        }
    }
    // Records leaving their cell: remove them from the end backwards, so the
    // earlier offsets stay valid, then append them all to the root block.
    let mut leaving = changes
        .iter()
        .filter(|(_, _, fits)| !fits)
        .collect::<Vec<_>>();
    leaving.sort_by_key(|(span, _, _)| std::cmp::Reverse(span.at));
    let mut appended = Vec::new();
    for (span, record, _) in leaving {
        let block = Reader { data: &output }.u32(span.node + 28)? as usize;
        if block < span.size {
            return Err(TerrainError::Limits);
        }
        output.drain(span.at..span.at + span.size);
        output[span.node + 28..span.node + 32]
            .copy_from_slice(&((block - span.size) as u32).to_le_bytes());
        appended.extend_from_slice(record);
    }
    if !appended.is_empty() {
        let block = Reader { data: &output }.u32(root + 28)? as usize;
        let insert = root + 32 + block;
        output.splice(insert..insert, appended.iter().copied());
        output[root + 28..root + 32]
            .copy_from_slice(&((block + appended.len()) as u32).to_le_bytes());
    }
    let total = u32::try_from(output.len()).map_err(|_| TerrainError::Limits)?;
    output[4..8].copy_from_slice(&total.to_le_bytes());
    let check = decode_compiled_terrain(&output)?;
    // Order changes for records that moved to the root; compare as sets.
    let sorted = |mut items: Vec<String>| {
        items.sort();
        items
    };
    let brushes = |list: &[CompiledBrush]| {
        sorted(
            list.iter()
                .map(|b| {
                    format!(
                        "{} {:?} {} {:?}",
                        b.model,
                        b.material,
                        b.layer,
                        b.matrix.map(f32::to_bits)
                    )
                })
                .collect(),
        )
    };
    let plants = |list: &[CompiledVegetation]| {
        sorted(
            list.iter()
                .map(|v| {
                    format!(
                        "{} {} {:?} {} {:?}",
                        v.group,
                        v.layer,
                        v.position.map(f32::to_bits),
                        v.scale.to_bits(),
                        v.angles
                    )
                })
                .collect(),
        )
    };
    if brushes(&check.brushes) != brushes(&expected_brushes)
        || plants(&check.vegetation) != plants(&expected_vegetation)
        || check.heights != terrain.heights
        || check.skipped_blocks != 0
    {
        return Err(format(
            "terrain.dat did not decode to the same placements with the edited ones moved",
        ));
    }
    Ok(output)
}

/// Decode one `merged_meshes_sectors/sector_X_Y_Z.dat` file.
pub fn decode_merged_mesh_sector(data: &[u8]) -> Result<Vec<MergedMeshInstance>, TerrainError> {
    let r = Reader { data };
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if r.u32(at)? != MERGED_MAGIC {
            return Err(format(format!("merged mesh block at {at} has no magic")));
        }
        let cell = [r.u32(at + 4)?, r.u32(at + 8)?, r.u32(at + 12)?];
        let group = r.u32(at + 16)? as usize;
        let count = r.u32(at + 20)? as usize;
        if cell.iter().any(|c| *c > 4096) || out.len() + count > MAX_MERGED_INSTANCES {
            return Err(TerrainError::Limits);
        }
        let origin = cell.map(|c| c as f32 * MERGED_CELL);
        for instance in r.bytes(at + 24, count * 12)?.as_chunks::<12>().0 {
            let fraction = |i: usize| {
                f32::from(u16::from_le_bytes([instance[i], instance[i + 1]])) / 65536.0
                    * MERGED_CELL
            };
            let q = [instance[10], instance[7], instance[8], instance[9]]
                .map(|b| f32::from(b as i8) / 127.0);
            let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
            out.push(MergedMeshInstance {
                group,
                position: [
                    origin[0] + fraction(0),
                    origin[1] + fraction(2),
                    origin[2] + fraction(4),
                ],
                scale: f32::from(instance[6]) / 64.0,
                rotation: if length > 0.5 {
                    q.map(|v| v / length)
                } else {
                    [1.0, 0.0, 0.0, 0.0]
                },
            });
        }
        at += 24 + count * 12;
    }
    Ok(out)
}

/// Every instance in a map's `terrain/merged_meshes_sectors` files.
pub fn read_merged_mesh_sectors(
    files: &[std::path::PathBuf],
) -> Result<Vec<MergedMeshInstance>, TerrainError> {
    let mut out = Vec::new();
    for path in files {
        let meta = fs::symlink_metadata(path)?;
        if !meta.file_type().is_file() || meta.len() > MAX_FILE as u64 {
            return Err(TerrainError::Limits);
        }
        out.extend(decode_merged_mesh_sector(&fs::read(path)?)?);
        if out.len() > MAX_MERGED_INSTANCES {
            return Err(TerrainError::Limits);
        }
    }
    Ok(out)
}

impl MergedMeshInstance {
    pub fn transform(&self) -> Transform {
        Transform {
            position: self.position,
            quaternion: self.rotation,
            scale: [self.scale; 3],
        }
    }
}

impl CompiledDecal {
    /// The decal's X, Y and Z axes: the rotation's columns.
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let m = &self.rotation;
        [0, 1, 2].map(|c| [m[c], m[3 + c], m[6 + c]])
    }
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

/// Largest height a sector can hold above its own offset: 12 bits of 0.05 m.
const MAX_SAMPLE_SPAN: f32 = 4095.0 * HEIGHT_STEP;
/// Local surface slots a sector can address; 14 and 15 are reserved.
const MAX_PALETTE: usize = 14;

/// Bounds, level-of-detail errors and surface types a rewritten node reports
/// to its parent.
struct NodeSummary {
    min_z: f32,
    max_z: f32,
    errors: [f32; 5],
    palette: Vec<u8>,
}

struct Brush<'a> {
    heights: &'a [f32],
    side: usize,
    /// Inclusive sample range the edit touches: x0, y0, x1, y1.
    changed: [usize; 4],
    /// Painting: the SurfaceTypeID the edit gives a sample, if any.
    paint: Option<&'a dyn Fn(usize, usize) -> Option<u8>>,
    /// Painting a hole turns it into ground; otherwise holes stay holes.
    fill_holes: bool,
}

impl Brush<'_> {
    fn painted(&self, x: usize, y: usize) -> Option<u8> {
        self.paint.and_then(|paint| paint(x, y))
    }
}

/// Re-encode the sectors in `changed` to `heights` and the edit's paint, then
/// prove the result decodes to exactly the intended heights and `surfaces`.
fn encode_terrain_edit(
    source: &[u8],
    terrain: &CompiledTerrain,
    heights: &[f32],
    surfaces: &[u8],
    changed: [usize; 4],
    paint: Option<&dyn Fn(usize, usize) -> Option<u8>>,
    fill_holes: bool,
) -> Result<Vec<u8>, TerrainError> {
    let r = Reader { data: source };
    let (_, at) = r.table(32, 352)?;
    let (_, at) = r.table(at, 256)?;
    let (_, start) = r.table(at, 256)?;
    let brush = Brush {
        heights,
        side: terrain.units as usize + 1,
        changed,
        paint,
        fill_holes,
    };
    let mut output = source[..start].to_vec();
    let (end, _) = rewrite_terrain_node(
        &r,
        start,
        terrain.units * terrain.unit_size as u32,
        terrain,
        &brush,
        &mut output,
    )?;
    output.extend_from_slice(&source[end..]);
    let size = u32::try_from(output.len()).map_err(|_| TerrainError::Limits)?;
    output[4..8].copy_from_slice(&size.to_le_bytes());
    // The result must decode, every height must land on the brush's target
    // within one quantization step, and every surface must be as intended.
    let check = decode_compiled_terrain(&output)?;
    if check
        .heights
        .iter()
        .zip(heights)
        .any(|(got, want)| (got - want).abs() > HEIGHT_STEP * 0.51)
    {
        return Err(format(
            "Re-encoded terrain.dat heights do not match the edit",
        ));
    }
    if check.surfaces != surfaces {
        return Err(format(
            "Re-encoded terrain.dat surfaces do not match the edit",
        ));
    }
    Ok(output)
}

/// Write a [`crate::TerrainPatch`] into `terrain.dat`: the patch's heights,
/// and its SurfaceTypeIDs where it paints. Painting a hole makes it ground,
/// which is how the brush creates new terrain. The touched
/// sectors are re-encoded as [`edit_compiled_terrain_bytes`] describes.
pub fn patch_compiled_terrain_bytes(
    source: &[u8],
    patch: &crate::terrain::TerrainPatch,
) -> Result<Vec<u8>, TerrainError> {
    let terrain = decode_compiled_terrain(source)?;
    let side = terrain.units as usize + 1;
    patch.check(side as u32, side as u32)?;
    if patch.surfaces.iter().flatten().any(|id| *id >= 127) {
        return Err(format("SurfaceTypeID must be below 127"));
    }
    let (x0, y0) = (patch.x as usize, patch.y as usize);
    let (w, h) = (patch.width as usize, patch.height as usize);
    let target = |x: usize, y: usize| {
        (x >= x0 && y >= y0 && x < x0 + w && y < y0 + h).then(|| (x - x0) * h + (y - y0))
    };
    let mut heights = terrain.heights.clone();
    let mut surfaces = terrain.surfaces.clone();
    let mut differs = false;
    for i in 0..w {
        for j in 0..h {
            let index = (x0 + i) * side + y0 + j;
            let at = i * h + j;
            differs |= (heights[index] - patch.heights[at]).abs() > HEIGHT_STEP * 0.5;
            heights[index] = patch.heights[at];
            // A patch may paint a hole, which makes it ground again.
            if let Some(Some(id)) = patch.surfaces.get(at) {
                differs |= surfaces[index] != *id;
                surfaces[index] = *id;
            }
        }
    }
    if !differs {
        return Err(format("The terrain patch changed no samples"));
    }
    let paint = |x: usize, y: usize| patch.surfaces.get(target(x, y)?).copied().flatten();
    encode_terrain_edit(
        source,
        &terrain,
        &heights,
        &surfaces,
        [x0, y0, x0 + w - 1, y0 + h - 1],
        (!patch.surfaces.is_empty()).then_some(&paint as &dyn Fn(usize, usize) -> Option<u8>),
        true,
    )
}

/// Edit `terrain.dat` with the same disk brush as [`crate::edit_terrain_bytes`]
/// uses on `Heightmap.dat`: every sample within `radius` of sample (`x`, `y`)
/// is raised by `delta_height` (Sculpt) or given surface type `layer_id`
/// (Paint). Holes stay holes.
///
/// For painting, `layer_id` is a `SurfaceTypeID` from the map's compiled
/// `leveldata.xml`. Each sample's low four bits pick an entry of its sector's
/// surface palette; bits past the palette's end mean its last entry (this
/// reproduces 96% of the Tutorial's editor paint exactly, the rest within one
/// paint pixel). A type a sector lacks is appended to its palette, and to its
/// parents', which list every type below them.
///
/// Only the sectors the brush touches are re-encoded, at full resolution
/// (flat sectors are stored coarser). Their bounding boxes and level-of-detail
/// errors, and their parents', only ever grow, so the engine never culls or
/// simplifies the new shape away. Everything else, including the object
/// octree, is copied byte for byte.
pub fn edit_compiled_terrain_bytes(
    source: &[u8],
    edit: crate::terrain::TerrainBrushEdit,
) -> Result<Vec<u8>, TerrainError> {
    use crate::terrain::TerrainBrushEdit;
    let (x, y, radius) = match edit {
        TerrainBrushEdit::Sculpt { x, y, radius, .. }
        | TerrainBrushEdit::Paint { x, y, radius, .. } => (x, y, radius),
    };
    let terrain = decode_compiled_terrain(source)?;
    let side = terrain.units as usize + 1;
    let (cx, cy, radius) = (x as usize, y as usize, radius as usize);
    if cx >= side || cy >= side || radius > 128 {
        return Err(TerrainError::Limits);
    }
    let (delta, paint) = match edit {
        TerrainBrushEdit::Sculpt { delta_height, .. } => {
            if !delta_height.is_finite() || delta_height.abs() > MAX_SAMPLE_SPAN {
                return Err(TerrainError::Limits);
            }
            (delta_height, None)
        }
        TerrainBrushEdit::Paint { layer_id, .. } => {
            if layer_id >= 127 {
                return Err(format("SurfaceTypeID must be below 127"));
            }
            (0.0, Some((layer_id, [cx, cy], radius)))
        }
    };
    let mut heights = terrain.heights.clone();
    let mut surfaces = terrain.surfaces.clone();
    let mut changed = [usize::MAX, usize::MAX, 0, 0];
    let mut differs = false;
    for sx in cx.saturating_sub(radius)..=(cx + radius).min(side - 1) {
        for sy in cy.saturating_sub(radius)..=(cy + radius).min(side - 1) {
            let (dx, dy) = (sx.abs_diff(cx), sy.abs_diff(cy));
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let index = sx * side + sy;
            heights[index] += delta;
            if let Some((id, ..)) = paint
                && surfaces[index] != HOLE_ID
            {
                differs |= surfaces[index] != id;
                surfaces[index] = id;
            }
            changed = [
                changed[0].min(sx),
                changed[1].min(sy),
                changed[2].max(sx),
                changed[3].max(sy),
            ];
        }
    }
    if changed[0] == usize::MAX
        || (paint.is_none() && delta == 0.0)
        || (paint.is_some() && !differs)
    {
        return Err(format("The terrain brush changed no samples"));
    }
    let disk = |x: usize, y: usize| {
        let (id, [cx, cy], radius) = paint?;
        let (dx, dy) = (x.abs_diff(cx), y.abs_diff(cy));
        (dx * dx + dy * dy <= radius * radius).then_some(id)
    };
    encode_terrain_edit(
        source,
        &terrain,
        &heights,
        &surfaces,
        changed,
        paint
            .is_some()
            .then_some(&disk as &dyn Fn(usize, usize) -> Option<u8>),
        false,
    )
}

fn rewrite_terrain_node(
    r: &Reader<'_>,
    at: usize,
    size: u32,
    terrain: &CompiledTerrain,
    brush: &Brush<'_>,
    out: &mut Vec<u8>,
) -> Result<(usize, NodeSummary), TerrainError> {
    let n = r.u32(at + 36)? as usize;
    let palette_len = r.u32(at + 40)? as usize;
    let samples_at = at + 44;
    let errors_at = samples_at + ((n * n * 2 + 3) & !3);
    let palette_at = errors_at + 20;
    let end = palette_at + ((palette_len + 3) & !3);
    let mut errors = [0.0f32; 5];
    for (index, error) in errors.iter_mut().enumerate() {
        *error = r.f32(errors_at + index * 4)?;
    }
    let mut summary = NodeSummary {
        min_z: r.f32(at + 12)?,
        max_z: r.f32(at + 24)?,
        errors,
        palette: r.bytes(palette_at, palette_len)?.to_vec(),
    };
    if size > terrain.sector_size {
        let mut children = Vec::new();
        let mut next = end;
        for _ in 0..4 {
            let (after, child) =
                rewrite_terrain_node(r, next, size / 2, terrain, brush, &mut children)?;
            next = after;
            summary.min_z = summary.min_z.min(child.min_z);
            summary.max_z = summary.max_z.max(child.max_z);
            for (mine, theirs) in summary.errors.iter_mut().zip(child.errors) {
                *mine = mine.max(theirs);
            }
            // A parent lists every surface type used below it.
            for id in child.palette {
                if !summary.palette.contains(&id) {
                    summary.palette.push(id);
                }
            }
        }
        write_node(
            out,
            r.bytes(at, 44)?,
            n,
            r.bytes(samples_at, n * n * 2)?,
            &summary,
        );
        out.extend_from_slice(&children);
        return Ok((next, summary));
    }
    let unit = terrain.unit_size;
    let cells = (terrain.sector_size as f32 / unit) as usize;
    let x0 = (r.f32(at + 4)? / unit) as usize;
    let y0 = (r.f32(at + 8)? / unit) as usize;
    let [cx0, cy0, cx1, cy1] = brush.changed;
    if x0 > cx1 || x0 + cells < cx0 || y0 > cy1 || y0 + cells < cy0 {
        out.extend_from_slice(r.bytes(at, end - at)?);
        return Ok((end, summary));
    }
    // Re-encode this sector at full resolution, keeping each sample's
    // surface from the nearest stored sample, as the decoder does. Bits past
    // the palette's end are pinned to its last entry before it can grow.
    let step = cells / (n - 1);
    let full = cells + 1;
    let height = |lx: usize, ly: usize| brush.heights[(x0 + lx) * brush.side + (y0 + ly)];
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    for lx in 0..full {
        for ly in 0..full {
            low = low.min(height(lx, ly));
            high = high.max(height(lx, ly));
        }
    }
    let offset = r.f32(at + 28)?.min(low);
    if high - offset > MAX_SAMPLE_SPAN {
        return Err(format(format!(
            "The edit makes a {}×{} m sector span more than {MAX_SAMPLE_SPAN} m of height",
            terrain.sector_size, terrain.sector_size
        )));
    }
    let last = summary.palette.len().saturating_sub(1) as u16;
    let mut samples = Vec::with_capacity(full * full);
    for lx in 0..full {
        for ly in 0..full {
            let (fx, fy) = (lx as f32 / step as f32, ly as f32 / step as f32);
            let mut bits = r.u16(
                samples_at
                    + (((fx + 0.5) as usize).min(n - 1) * n + ((fy + 0.5) as usize).min(n - 1)) * 2,
            )? & 15;
            if bits != HOLE_SURFACE && bits > last {
                bits = last;
            }
            if (bits != HOLE_SURFACE || brush.fill_holes)
                && let Some(id) = brush.painted(x0 + lx, y0 + ly)
            {
                bits = match summary.palette.iter().position(|known| *known == id) {
                    Some(slot) => slot as u16,
                    None if summary.palette.len() < MAX_PALETTE => {
                        summary.palette.push(id);
                        (summary.palette.len() - 1) as u16
                    }
                    None => {
                        return Err(format(format!(
                            "The sector at {x0},{y0} already uses {MAX_PALETTE} surface types"
                        )));
                    }
                };
            }
            let steps = ((height(lx, ly) - offset) / HEIGHT_STEP)
                .round()
                .clamp(0.0, 4095.0);
            samples.push(((steps as u16) << 4) | bits);
        }
    }
    let quantized =
        |lx: usize, ly: usize| offset + f32::from(samples[lx * full + ly] >> 4) * HEIGHT_STEP;
    // How far each coarser level of detail strays from the full grid.
    for (level, error) in summary.errors.iter_mut().enumerate() {
        let coarse = 1usize << level;
        if coarse > cells {
            continue;
        }
        let mut worst = 0.0f32;
        for lx in 0..full {
            for ly in 0..full {
                let (ix, iy) = ((lx / coarse) * coarse, (ly / coarse) * coarse);
                let (jx, jy) = ((ix + coarse).min(cells), (iy + coarse).min(cells));
                let tx = (lx - ix) as f32 / coarse as f32;
                let ty = (ly - iy) as f32 / coarse as f32;
                let estimate = (quantized(ix, iy) * (1.0 - tx) + quantized(jx, iy) * tx)
                    * (1.0 - ty)
                    + (quantized(ix, jy) * (1.0 - tx) + quantized(jx, jy) * tx) * ty;
                worst = worst.max((estimate - quantized(lx, ly)).abs());
            }
        }
        *error = error.max(worst);
    }
    summary.min_z = summary.min_z.min(low);
    summary.max_z = summary.max_z.max(high);
    let mut header = r.bytes(at, 44)?.to_vec();
    header[28..32].copy_from_slice(&offset.to_le_bytes());
    let bytes = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect::<Vec<_>>();
    write_node(out, &header, full, &bytes, &summary);
    Ok((end, summary))
}

/// Append one quadtree node: its header with the summary's bounds, sample
/// count and palette size, the samples, the errors and the padded palette.
/// Node starts are 4-byte aligned, and every node's length is a multiple of 4.
fn write_node(out: &mut Vec<u8>, header: &[u8], n: usize, samples: &[u8], summary: &NodeSummary) {
    let start = out.len();
    out.extend_from_slice(header);
    out[start + 12..start + 16].copy_from_slice(&summary.min_z.to_le_bytes());
    out[start + 24..start + 28].copy_from_slice(&summary.max_z.to_le_bytes());
    out[start + 36..start + 40].copy_from_slice(&(n as u32).to_le_bytes());
    out[start + 40..start + 44].copy_from_slice(&(summary.palette.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    out.resize(start + 44 + ((samples.len() + 3) & !3), 0);
    for error in summary.errors {
        out.extend_from_slice(&error.to_le_bytes());
    }
    out.extend_from_slice(&summary.palette);
    out.resize(out.len() + ((4 - summary.palette.len() % 4) % 4), 0);
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
    fn sculpts_a_coarse_sector_at_full_resolution() {
        use crate::terrain::TerrainBrushEdit;
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        let side = 17usize;
        let edited = edit_compiled_terrain_bytes(
            &source,
            TerrainBrushEdit::Sculpt {
                x: 4,
                y: 4,
                radius: 2,
                delta_height: 3.0,
            },
        )
        .unwrap();
        let after = decode_compiled_terrain(&edited).unwrap();
        for sx in 0..side {
            for sy in 0..side {
                let index = sx * side + sy;
                let (dx, dy) = (sx.abs_diff(4), sy.abs_diff(4));
                let want = before.heights[index] + if dx * dx + dy * dy <= 4 { 3.0 } else { 0.0 };
                assert!((after.heights[index] - want).abs() <= 0.026, "({sx},{sy})");
            }
        }
        assert_eq!(after.holes, before.holes);
        assert_eq!(after.brushes, before.brushes);
        assert_eq!(after.vegetation, before.vegetation);
        assert_eq!(
            edited.len(),
            u32::from_le_bytes(edited[4..8].try_into().unwrap()) as usize
        );
        // A sector cannot span more than 12 bits of 0.05 m steps.
        assert!(
            edit_compiled_terrain_bytes(
                &source,
                TerrainBrushEdit::Sculpt {
                    x: 4,
                    y: 4,
                    radius: 1,
                    delta_height: 250.0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn terrain_patch_sets_heights_and_paint_in_a_rectangle() {
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        let side = 17usize;
        let mut heights = Vec::new();
        for i in 0..4 {
            for j in 0..3 {
                heights.push(12.0 + i as f32 * 0.5 + j as f32 * 0.25);
            }
        }
        let patch = crate::terrain::TerrainPatch {
            x: 2,
            y: 3,
            width: 4,
            height: 3,
            heights: heights.clone(),
            surfaces: (0..12).map(|i| (i % 2 == 0).then_some(9)).collect(),
        };
        let edited = patch_compiled_terrain_bytes(&source, &patch).unwrap();
        let after = decode_compiled_terrain(&edited).unwrap();
        for i in 0..4 {
            for j in 0..3 {
                let index = (2 + i) * side + 3 + j;
                let at = i * 3 + j;
                assert!((after.heights[index] - heights[at]).abs() <= HEIGHT_STEP * 0.51);
                let want = if at % 2 == 0 {
                    9
                } else {
                    before.surfaces[index]
                };
                assert_eq!(after.surfaces[index], want);
            }
        }
        assert_eq!(after.heights[0], before.heights[0]);
        assert_eq!(after.brushes, before.brushes);
        let unchanged = crate::terrain::TerrainPatch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            heights: vec![before.heights[0]],
            surfaces: Vec::new(),
        };
        assert!(patch_compiled_terrain_bytes(&source, &unchanged).is_err());
    }

    #[test]
    fn paints_surface_types_through_the_sector_palette() {
        use crate::terrain::TerrainBrushEdit;
        let mut source = fixture();
        // Sample (0, 1) carries bits 3 past the one-entry palette [4]: it
        // means the palette's last entry, and must keep meaning type 4.
        let samples = source
            .windows(6)
            .position(|w| w == [0, 0, 160, 0, 64, 1])
            .unwrap();
        source[samples + 2] |= 3;
        let before = decode_compiled_terrain(&source).unwrap();
        let side = 17usize;
        assert_eq!(before.surfaces[1], 4);
        assert_eq!(before.surfaces[16 * side + 16], HOLE_ID);
        let edited = edit_compiled_terrain_bytes(
            &source,
            TerrainBrushEdit::Paint {
                x: 12,
                y: 4,
                radius: 2,
                layer_id: 7,
            },
        )
        .unwrap();
        let after = decode_compiled_terrain(&edited).unwrap();
        for sx in 0..side {
            for sy in 0..side {
                let index = sx * side + sy;
                let (dx, dy) = (sx.abs_diff(12), sy.abs_diff(4));
                let inside = dx * dx + dy * dy <= 4 && before.surfaces[index] != HOLE_ID;
                let want = if inside { 7 } else { before.surfaces[index] };
                assert_eq!(after.surfaces[index], want, "({sx},{sy})");
                assert!((after.heights[index] - before.heights[index]).abs() <= 0.026);
            }
        }
        assert_eq!(after.holes, before.holes);
        // Painting what is already there changes nothing and is refused.
        assert!(
            edit_compiled_terrain_bytes(
                &source,
                TerrainBrushEdit::Paint {
                    x: 1,
                    y: 1,
                    radius: 0,
                    layer_id: 4
                }
            )
            .is_err()
        );
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

    fn record_header(out: &mut Vec<u8>, kind: u32, layer: u16) {
        u(out, &[kind]);
        f(out, &[0.0; 6]);
        out.extend_from_slice(&layer.to_le_bytes());
        out.extend_from_slice(&[0; 6]);
    }

    #[test]
    fn frames_two_size_records_and_decodes_decals_roads_and_water() {
        let mut block = Vec::new();
        // A short 0x16 record: only 92 bytes frames the block exactly.
        record_header(&mut block, 0x16, 1);
        block.extend_from_slice(&[0; 92 - 36]);
        // Decal: terrain projection, depth byte 204 (0.2), material 0, priority 30.
        record_header(&mut block, DECAL, 3);
        block.extend_from_slice(&[0, 0, 0, 0]);
        block.extend_from_slice(&[2, 0, 1, 204]);
        f(&mut block, &[10.0, 20.0, 30.0, 0.0, 0.0, 2.0]);
        f(
            &mut block,
            &[0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0],
        );
        u(&mut block, &[0, 30, 0]);
        // Road: 4 vertices, priority 2, material 0, texture end 3.5.
        record_header(&mut block, ROAD, 4);
        u(&mut block, &[0, 4, 2, 0, 0]);
        f(&mut block, &[3.5, 0.0, 3.5, 100.0]);
        f(
            &mut block,
            &[0., 0., 1., 0., 4., 1., 8., 0., 1., 8., 4., 1.],
        );
        // Water: 3 points (and none in the second list), material 0.
        record_header(&mut block, WATER, 5);
        let mut water = vec![0u8; 244 - 36];
        water[52 - 36..56 - 36].copy_from_slice(&0u32.to_le_bytes());
        water[56 - 36..60 - 36].copy_from_slice(&2.0f32.to_le_bytes());
        water[124 - 36..128 - 36].copy_from_slice(&3u32.to_le_bytes());
        block.extend_from_slice(&water);
        f(&mut block, &[0., 0., 5., 10., 0., 5., 0., 10., 5.]);
        let mut d = fixture();
        // Replace the fixture's octree block (brush 104 + vegetation 64 bytes).
        let octree = d.len() - 32 - 168;
        d.truncate(octree + 28);
        u(&mut d, &[block.len() as u32]);
        d.extend_from_slice(&block);
        let size = d.len() as u32;
        d[4..8].copy_from_slice(&size.to_le_bytes());
        let t = decode_compiled_terrain(&d).unwrap();
        assert_eq!((t.skipped_blocks, t.other_records), (0, 1));
        let decal = &t.decals[0];
        assert_eq!((decal.material, decal.layer), (Some(0), 3));
        assert_eq!(decal.position, [10.0, 20.0, 30.0]);
        assert!((decal.depth - 0.2).abs() < 1e-6 && decal.terrain_only);
        assert_eq!((decal.scale, decal.sort_priority), (2.0, 30));
        assert_eq!(decal.axes()[0], [0.0, 1.0, 0.0]);
        let road = &t.roads[0];
        assert_eq!((road.sort_priority, road.texture_end), (2, 3.5));
        assert_eq!(road.vertices[3], [8.0, 4.0, 1.0]);
        let water = &t.water[0];
        assert_eq!((water.material, water.fog_density), (Some(0), 2.0));
        assert_eq!(water.points[1], [10.0, 0.0, 5.0]);
    }

    #[test]
    fn terrain_patch_painting_a_hole_creates_ground() {
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        let side = 17usize;
        assert!(before.holes[15 * side + 15]);
        let (x0, y0, w, h) = (12usize, 12usize, 5usize, 5usize);
        let mut heights = Vec::new();
        for i in 0..w {
            for j in 0..h {
                heights.push(before.heights[(x0 + i) * side + y0 + j]);
            }
        }
        let patch = crate::terrain::TerrainPatch {
            x: x0 as u32,
            y: y0 as u32,
            width: w as u32,
            height: h as u32,
            heights,
            surfaces: vec![Some(4); w * h],
        };
        let filled = patch_compiled_terrain_bytes(&source, &patch).unwrap();
        let after = decode_compiled_terrain(&filled).unwrap();
        assert!(after.holes.iter().all(|hole| !hole), "every hole filled");
        assert_eq!(after.surfaces[16 * side + 16], 4);
        assert_eq!(after.brushes, before.brushes);
    }

    #[test]
    fn copies_a_brush_or_vegetation_record_to_a_new_position() {
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        let copied =
            copy_compiled_placement(&source, CompiledPlacement::Brush(0), [9.0, 10.0, 11.0])
                .unwrap();
        assert_eq!(copied.len(), source.len() + 104);
        let after = decode_compiled_terrain(&copied).unwrap();
        assert_eq!(after.brushes.len(), 2);
        let copy = &after.brushes[1];
        assert_eq!(
            (copy.model, copy.material),
            (before.brushes[0].model, before.brushes[0].material)
        );
        assert_eq!(
            [copy.matrix[3], copy.matrix[7], copy.matrix[11]],
            [9.0, 10.0, 11.0]
        );
        // Rotation and scale are untouched.
        assert_eq!(copy.matrix[0], before.brushes[0].matrix[0]);
        assert_eq!(after.vegetation, before.vegetation);
        let plant =
            copy_compiled_placement(&source, CompiledPlacement::Vegetation(0), [2.0, 3.0, 4.0])
                .unwrap();
        let after = decode_compiled_terrain(&plant).unwrap();
        assert_eq!(after.vegetation.len(), 2);
        assert_eq!(after.vegetation[0], before.vegetation[0]);
        assert_eq!(after.vegetation[1].position, [2.0, 3.0, 4.0]);
        // Outside the map cube (0–16 m here) is refused.
        assert!(
            copy_compiled_placement(&source, CompiledPlacement::Brush(0), [20.0, 1.0, 1.0])
                .is_err()
        );
    }

    #[test]
    fn deletes_one_brush_or_vegetation_record() {
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        assert_eq!(before.brush_records.len(), 1);
        assert_eq!(before.brush_records[0].size, 104);
        let without_brush =
            delete_compiled_placement(&source, CompiledPlacement::Brush(0)).unwrap();
        assert_eq!(without_brush.len(), source.len() - 104);
        let after = decode_compiled_terrain(&without_brush).unwrap();
        assert!(after.brushes.is_empty());
        assert_eq!(after.vegetation, before.vegetation);
        assert_eq!(after.heights, before.heights);
        let without_plant =
            delete_compiled_placement(&source, CompiledPlacement::Vegetation(0)).unwrap();
        let after = decode_compiled_terrain(&without_plant).unwrap();
        assert!(after.vegetation.is_empty());
        assert_eq!(after.brushes, before.brushes);
        assert!(delete_compiled_placement(&source, CompiledPlacement::Brush(1)).is_err());
    }

    #[test]
    fn decodes_merged_mesh_sector_instances() {
        let mut d = Vec::new();
        u(&mut d, &[MERGED_MAGIC, 2, 3, 4, 7, 2]);
        // Half way across the cell, scale 1, identity rotation.
        d.extend_from_slice(&[0, 0x80, 0, 0x40, 0, 0, 64, 0, 0, 0, 127, 0]);
        // 90 degrees about Z, scale 0.5.
        d.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 32, 0, 0, 90, 90, 0]);
        let instances = decode_merged_mesh_sector(&d).unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].group, 7);
        assert_eq!(instances[0].position, [40.0, 52.0, 64.0]);
        assert_eq!(instances[0].rotation, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(instances[1].scale, 0.5);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((instances[1].rotation[0] - half).abs() < 1e-6);
        assert!((instances[1].rotation[3] - half).abs() < 1e-6);
        assert!(decode_merged_mesh_sector(&d[..d.len() - 1]).is_err());
        d[0] ^= 1;
        assert!(decode_merged_mesh_sector(&d).is_err());
    }

    /// Every installed level.pak's terrain.dat and merged sectors decode fully.
    #[test]
    #[ignore = "reads the local asset library"]
    fn real_compiled_terrains_frame_every_block() {
        let root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let mut checked = 0;
        for entry in fs::read_dir(root).unwrap().flatten() {
            let terrain = entry.path().join("files/terrain/terrain.dat");
            let Ok(t) = read_compiled_terrain(&terrain) else {
                continue;
            };
            let folder = entry.path().join("files/terrain/merged_meshes_sectors");
            let files = fs::read_dir(&folder)
                .map(|dir| dir.flatten().map(|e| e.path()).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "dat"))
                .collect::<Vec<_>>();
            let merged = read_merged_mesh_sectors(&files).unwrap();
            println!(
                "{}: {} brushes, {} vegetation, {} decals, {} roads, {} water, {} merged, {} other",
                entry.file_name().to_string_lossy(),
                t.brushes.len(),
                t.vegetation.len(),
                t.decals.len(),
                t.roads.len(),
                t.water.len(),
                merged.len(),
                t.other_records
            );
            assert_eq!(t.skipped_blocks, 0);
            assert!(merged.iter().all(|m| m.group < t.vegetation_groups.len()));
            // Copying any one placement inside the map adds exactly it.
            let bytes = fs::read(&terrain).unwrap();
            let middle = t.units as f32 * t.unit_size * 0.5;
            for placement in [
                CompiledPlacement::Brush(t.brushes.len() / 3),
                CompiledPlacement::Vegetation(t.vegetation.len() / 3),
            ] {
                let edited =
                    copy_compiled_placement(&bytes, placement, [middle, middle, 100.0]).unwrap();
                let after = decode_compiled_terrain(&edited).unwrap();
                assert_eq!(
                    after.brushes.len() + after.vegetation.len(),
                    t.brushes.len() + t.vegetation.len() + 1
                );
            }
            // Deleting any one placement leaves exactly the rest.
            for placement in [
                CompiledPlacement::Brush(t.brushes.len() / 2),
                CompiledPlacement::Vegetation(t.vegetation.len() / 2),
            ] {
                let edited = delete_compiled_placement(&bytes, placement).unwrap();
                let after = decode_compiled_terrain(&edited).unwrap();
                assert_eq!(
                    after.brushes.len() + after.vegetation.len() + 1,
                    t.brushes.len() + t.vegetation.len()
                );
            }
            // Moving, turning and scaling a nearby placement in place, and one
            // far across the map out of its octree cell, keeps every record.
            let brush = t.brushes.len() / 4;
            let plant = t.vegetation.len() / 4;
            let mut nudged = t.brushes[brush].transform();
            nudged.position[2] += 0.25;
            let mut far = t.vegetation[plant].transform();
            far.position = [middle * 0.3, middle * 1.7, 120.0];
            far.quaternion = [
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
            ];
            far.scale = [1.5; 3];
            let edited = transform_compiled_placements(
                &bytes,
                &[
                    (CompiledPlacement::Brush(brush), nudged),
                    (CompiledPlacement::Vegetation(plant), far),
                ],
            )
            .unwrap();
            let after = decode_compiled_terrain(&edited).unwrap();
            assert_eq!(after.brushes.len(), t.brushes.len());
            assert_eq!(after.vegetation.len(), t.vegetation.len());
            assert!(
                after.vegetation.iter().any(|v| v.position == far.position
                    && v.scale == 1.5
                    && v.angles == [64, 0, 0])
            );
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn moves_rotates_and_scales_brush_and_vegetation_records() {
        let source = fixture();
        let before = decode_compiled_terrain(&source).unwrap();
        // A quarter turn about Z, twice as tall, moved to (6, 7, 8).
        let quarter = [
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        let brush = Transform {
            position: [6.0, 7.0, 8.0],
            quaternion: quarter,
            scale: [1.0, 1.0, 2.0],
        };
        let plant = Transform {
            position: [3.0, 4.0, 5.0],
            quaternion: quarter,
            scale: [2.0; 3],
        };
        let edited = transform_compiled_placements(
            &source,
            &[
                (CompiledPlacement::Brush(0), brush),
                (CompiledPlacement::Vegetation(0), plant),
            ],
        )
        .unwrap();
        // In place: the same size, the same order.
        assert_eq!(edited.len(), source.len());
        let after = decode_compiled_terrain(&edited).unwrap();
        let moved = after.brushes[0].transform();
        assert_eq!(moved.position, [6.0, 7.0, 8.0]);
        for (a, b) in moved.scale.iter().zip([1.0, 1.0, 2.0]) {
            assert!((a - b).abs() < 1e-5, "{:?}", moved.scale);
        }
        let dot = moved
            .quaternion
            .iter()
            .zip(quarter)
            .map(|(a, b)| a * b)
            .sum::<f32>();
        assert!(dot.abs() > 0.9999, "{:?}", moved.quaternion);
        assert_eq!(after.brushes[0].model, before.brushes[0].model);
        let plant_after = &after.vegetation[0];
        assert_eq!(plant_after.position, [3.0, 4.0, 5.0]);
        assert_eq!(plant_after.scale, 2.0);
        assert_eq!(plant_after.angles, [64, 0, 0]);
        // The vegetation angles survive a round trip through a quaternion.
        for angles in [[0, 0, 0], [64, 0, 0], [200, 10, 0], [5, 0, 30]] {
            let plant = CompiledVegetation {
                angles,
                ..before.vegetation[0].clone()
            };
            assert_eq!(vegetation_angles(plant.transform().quaternion), angles);
        }
        // Outside the map, or a zero scale, is refused.
        let mut outside = brush;
        outside.position[0] = 40.0;
        assert!(
            transform_compiled_placements(&source, &[(CompiledPlacement::Brush(0), outside)])
                .is_err()
        );
        let mut flat = brush;
        flat.scale[2] = 0.0;
        assert!(
            transform_compiled_placements(&source, &[(CompiledPlacement::Brush(0), flat)]).is_err()
        );
    }
}
