//! Read-only reconstruction of CryEngine's baked terrain diffuse quadtree.
//! The compressed surface packages are not ZIP archives. They carry a CRY
//! header, layer descriptors, a signed-int16 pre-order quadtree index and
//! consecutively interleaved compressed sector blocks (Cry3DEngine loader).
//! Paint layers were baked into RGB already; detail-material overlays are a
//! distinct stage, never fabricated here.
use crate::dds::{DdsFormat, decode_linear_bc_surface};
use crate::terrain::{TerrainError, read_terrain_texture_header};
use image::{Rgba, RgbaImage};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_ATLAS: u32 = 2048;
const MAX_NODES: usize = 32_767;
const MAX_TILES: usize = 8192;
const MAX_SECTOR_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TerrainTextureAtlas {
    pub image: RgbaImage,
    pub nodes: usize,
    pub decoded_tiles: usize,
    pub color_format: u32,
    /// True when some quadtree tiles had no original color source.
    pub incomplete: bool,
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    index: usize,
    x: u32,
    y: u32,
    size: u32,
}

fn invalid(message: &str) -> TerrainError {
    TerrainError::Format(message.into())
}

fn parse_tree(indices: &[i16]) -> Result<Vec<Tile>, TerrainError> {
    fn node(
        indices: &[i16],
        cursor: &mut usize,
        x: u32,
        y: u32,
        size: u32,
        depth: u32,
        out: &mut Vec<Tile>,
    ) -> Result<(), TerrainError> {
        if depth > 16 || out.len() > MAX_NODES || *cursor >= indices.len() {
            return Err(TerrainError::Limits);
        }
        let raw = indices[*cursor];
        *cursor += 1;
        if raw < 0 {
            return Ok(());
        }
        out.push(Tile {
            index: raw as usize,
            x,
            y,
            size,
        });
        // The engine writes FOUR child slots for every node that has its
        // own tile. Each child is either a negative sentinel or a subtree.
        if *cursor == indices.len() {
            // Only accepted at root for older flat terrain index tables.
            return Ok(());
        }
        if size < 2 {
            return Err(invalid("terrain texture quadtree too deep"));
        }
        let half = size / 2;
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            node(
                indices,
                cursor,
                x + dx * half,
                y + dy * half,
                half,
                depth + 1,
                out,
            )?;
        }
        Ok(())
    }
    if indices.is_empty() || indices[0] < 0 {
        return Err(invalid("terrain texture root has no tile"));
    }
    // Logical map coordinates are independent of pixel resolution.
    let mut out = Vec::new();
    let mut cursor = 0;
    node(indices, &mut cursor, 0, 0, MAX_ATLAS, 0, &mut out)?;
    if cursor != indices.len() {
        return Err(invalid("terrain quadtree index contains unused entries"));
    }
    Ok(out)
}

fn map_format(value: u32) -> Option<DdsFormat> {
    match value {
        22 => Some(DdsFormat::Bc1),
        23 => Some(DdsFormat::Bc2),
        24 => Some(DdsFormat::Bc3),
        25 => Some(DdsFormat::Bc4),
        27 => Some(DdsFormat::Bc5),
        _ => None,
    }
}

fn color_sector(format: u32, dimension: u32, bytes: &[u8]) -> Result<RgbaImage, TerrainError> {
    if format == 2 {
        let count = (dimension as usize)
            .checked_mul(dimension as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(TerrainError::Limits)?;
        let pixels = bytes
            .get(..count)
            .ok_or_else(|| invalid("truncated RGBA terrain sector"))?;
        return RgbaImage::from_raw(dimension, dimension, pixels.to_vec())
            .ok_or(TerrainError::Limits);
    }
    let format =
        map_format(format).ok_or_else(|| invalid("unsupported baked terrain color format"))?;
    decode_linear_bc_surface(dimension, dimension, format, bytes)
        .map_err(|e| TerrainError::Format(format!("baked terrain BC decode: {e}")))
}

fn stamp(atlas: &mut RgbaImage, decoded: &RgbaImage, tile: Tile, multiplier: f32) {
    let width = atlas.width();
    let dim = decoded.width();
    let right = tile.x.saturating_add(tile.size).min(width);
    let bottom = tile.y.saturating_add(tile.size).min(width);
    // Cry3DEngine calculates the terrain texture U axis from world Y
    // and V from world X. Transpose here so terrain UV0=[worldX,worldY]
    // can sample the assembled image without changes to original geometry.
    for x in tile.x..right {
        let local_v = ((x - tile.x) as u64 * dim as u64 / tile.size as u64) as u32;
        for y in tile.y..bottom {
            let local_u = ((y - tile.y) as u64 * dim as u64 / tile.size as u64) as u32;
            let source = decoded.get_pixel(local_u.min(dim - 1), local_v.min(dim - 1));
            let mut color = *source;
            for channel in &mut color.0[..3] {
                *channel = (f32::from(*channel) * multiplier).round().clamp(0.0, 255.0) as u8;
            }
            // Terrain surfaces are opaque; BC3's alpha is a separate baked
            // channel, not an instruction to cut holes in the terrain mesh.
            color.0[3] = 255;
            atlas.put_pixel(x, y, color);
        }
    }
}

/// Builds one bounded, baked color texture for the original level heightmap.
/// Each valid higher-resolution child replaces the corresponding region of
/// its parent. Unpainted branches retain their parent tile, like CryEngine.
/// No original map files or PAK bytes are changed.
pub fn read_terrain_texture_atlas(path: &Path) -> Result<TerrainTextureAtlas, TerrainError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() < 32 || meta.len() > MAX_FILE {
        return Err(TerrainError::Limits);
    }
    let header = read_terrain_texture_header(path)?;
    if header.flags & 2 != 0 {
        return Err(invalid("big endian terrain tiles unsupported"));
    }
    let first = header
        .layers
        .first()
        .ok_or_else(|| invalid("no baked color layer"))?;
    let dim = u32::from(first.sector_pixels);
    let sector_bytes = first.sector_bytes as usize;
    if sector_bytes == 0 || sector_bytes > MAX_SECTOR_BYTES {
        return Err(TerrainError::Limits);
    }
    let stride = header
        .layers
        .iter()
        .try_fold(0usize, |sum, layer| {
            sum.checked_add(layer.sector_bytes as usize)
        })
        .ok_or(TerrainError::Limits)?;
    if stride == 0 || stride > 128 * 1024 * 1024 {
        return Err(TerrainError::Limits);
    }
    let index_at = 16u64 + header.layers.len() as u64 * 12;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(index_at))?;
    let mut count_bytes = [0u8; 2];
    file.read_exact(&mut count_bytes)?;
    let count = u16::from_le_bytes(count_bytes) as usize;
    if count == 0 || count > MAX_NODES {
        return Err(TerrainError::Limits);
    }
    let mut index_bytes = vec![0u8; count * 2];
    file.read_exact(&mut index_bytes)?;
    let indices = index_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let nodes = parse_tree(&indices)?;
    if nodes.is_empty() || nodes.len() > MAX_TILES {
        return Err(TerrainError::Limits);
    }
    let data_at = index_at + 2 + index_bytes.len() as u64;
    let mut image = RgbaImage::from_pixel(MAX_ATLAS, MAX_ATLAS, Rgba([255, 255, 255, 255]));
    let mut sector = vec![0u8; sector_bytes];
    let mut decoded_tiles = 0;
    let color_multiplier = header.color_multiplier;
    if !color_multiplier.is_finite() || color_multiplier <= 0.0 {
        return Err(invalid("invalid baked color correction multiplier"));
    }
    for tile in nodes.iter().copied() {
        let tile_at = (tile.index as u64)
            .checked_mul(stride as u64)
            .and_then(|p| data_at.checked_add(p))
            .ok_or(TerrainError::Limits)?;
        if tile_at
            .checked_add(sector_bytes as u64)
            .is_none_or(|end| end > meta.len())
        {
            return Err(invalid("terrain tile index points beyond source package"));
        }
        file.seek(SeekFrom::Start(tile_at))?;
        file.read_exact(&mut sector)?;
        let decoded = color_sector(first.texture_format, dim, &sector)?;
        stamp(&mut image, &decoded, tile, color_multiplier);
        decoded_tiles += 1;
    }
    Ok(TerrainTextureAtlas {
        image,
        nodes: nodes.len(),
        decoded_tiles,
        color_format: first.texture_format,
        incomplete: false,
    })
}

/// Read a complete, manifest-verified square grid of raw RGB24 terrain tiles.
/// Each tile stores LE u32 width/height followed by width*height*3 RGB bytes.
/// The caller supplies tiles in row-major order: tile0_0, tile1_0, ...
/// then tile0_1, tile1_1, ...; all files must belong to ONE imported PAK.
/// Bounded to a 4096x4096 RGBA atlas so previews do not allocate without limit.
/// The original game PAK and imported tile files are strictly read-only.
pub fn read_terrain_rgb24_tiles(
    paths: &[std::path::PathBuf],
    side: usize,
) -> Result<TerrainTextureAtlas, TerrainError> {
    if side == 0 || side > 8 || side.checked_mul(side) != Some(paths.len()) {
        return Err(invalid("RGB24 terrain grid has invalid dimensions or missing members"));
    }
    let mut image: Option<RgbaImage> = None;
    let mut tile_dim = 0usize;
    for (tile_index, path) in paths.iter().enumerate() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.len() < 8 || metadata.len() > 48 * 1024 * 1024 {
            return Err(TerrainError::Limits);
        }
        let mut file = File::open(path)?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let width = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let height = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if width != height || !(4..=2048).contains(&width) || !width.is_power_of_two() {
            return Err(invalid("invalid RGB24 terrain tile dimensions"));
        }
        let atlas_dim = side.checked_mul(width).ok_or(TerrainError::Limits)?;
        if atlas_dim > 4096 {
            return Err(invalid("RGB24 terrain atlas exceeds 4096-pixel preview limit"));
        }
        if tile_index == 0 {
            tile_dim = width;
            image = Some(RgbaImage::from_pixel(atlas_dim as u32, atlas_dim as u32, Rgba([0, 0, 0, 255])));
        } else if width != tile_dim {
            return Err(invalid("RGB24 terrain tile dimensions differ"));
        }
        let pixels_len = width.checked_mul(height).and_then(|pixels| pixels.checked_mul(3)).ok_or(TerrainError::Limits)?;
        if metadata.len() != (8 + pixels_len) as u64 {
            return Err(invalid("RGB24 terrain tile has a truncated or trailing payload"));
        }
        let mut pixels = vec![0u8; pixels_len];
        file.read_exact(&mut pixels)?;
        let atlas = image.as_mut().ok_or(TerrainError::Limits)?;
        let tile_x = tile_index % side;
        let tile_y = tile_index / side;
        for row in 0..height {
            for col in 0..width {
                let from = (row * width + col) * 3;
                let to = ((tile_y * height + row) * atlas_dim + tile_x * width + col) * 4;
                let destination = &mut atlas.as_mut()[to..to + 4];
                destination[..3].copy_from_slice(&pixels[from..from + 3]);
                destination[3] = 255;
            }
        }
    }
    Ok(TerrainTextureAtlas {
        image: image.ok_or(TerrainError::Limits)?,
        nodes: paths.len(),
        decoded_tiles: paths.len(),
        color_format: 3, // RGB24 (not a CRY/BC compression format)
        incomplete: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tree: &[i16], tiles: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"CRY\0");
        out.extend_from_slice(&[100, 0]);
        out.extend_from_slice(&9u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1.0f32.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&64u32.to_le_bytes());
        out.extend_from_slice(&(tree.len() as u16).to_le_bytes());
        for i in tree {
            out.extend_from_slice(&i.to_le_bytes());
        }
        for tile in tiles {
            for _ in 0..16 {
                out.extend_from_slice(tile);
            }
        }
        out
    }
    #[test]
    fn reconstructs_four_raw_rgb24_terrain_tiles_without_a_cry_header() {
        let dir = tempfile::tempdir().unwrap();
        let dim = 4u32;
        let colors = [[10u8, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let paths: [std::path::PathBuf; 4] = std::array::from_fn(|i| {
            let tile = dir.path().join(format!("tile{}_{}.raw", i % 2, i / 2));
            let mut data = Vec::new();
            data.extend_from_slice(&dim.to_le_bytes());
            data.extend_from_slice(&dim.to_le_bytes());
            for _ in 0..dim * dim { data.extend_from_slice(&colors[i]); }
            std::fs::write(&tile, data).unwrap();
            tile
        });
        let atlas = read_terrain_rgb24_tiles(&paths, 2).unwrap();
        assert_eq!(atlas.image.dimensions(), (8, 8));
        assert_eq!(atlas.decoded_tiles, 4);
        assert_eq!(atlas.color_format, 3);
        for (x, y, index) in [(0, 0, 0), (4, 0, 1), (0, 4, 2), (4, 4, 3)] {
            let sample = atlas.image.get_pixel(x, y);
            assert_eq!(&sample.0[..3], &colors[index]);
            assert_eq!(sample.0[3], 255);
        }
        std::fs::write(&paths[3], b"short").unwrap();
        assert!(read_terrain_rgb24_tiles(&paths).is_err());
    }

    #[test]
    fn reconstructs_ice_crashsite_four_by_four_raw_rgb24_grid() {
        let dir = tempfile::tempdir().unwrap();
        let side = 4usize;
        let dim = 4u32;
        let mut paths = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let path = dir.path().join(format!("tile{x}_{y}.raw"));
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&dim.to_le_bytes());
                bytes.extend_from_slice(&dim.to_le_bytes());
                for _ in 0..dim * dim {
                    bytes.extend_from_slice(&[x as u8 * 50, y as u8 * 50, 120]);
                }
                std::fs::write(&path, bytes).unwrap();
                paths.push(path);
            }
        }
        let atlas = read_terrain_rgb24_tiles(&paths, side).unwrap();
        assert_eq!(atlas.image.dimensions(), (16, 16));
        assert_eq!(atlas.decoded_tiles, 16);
        assert_eq!(atlas.image.get_pixel(0, 0).0, [0, 0, 120, 255]);
        assert_eq!(atlas.image.get_pixel(15, 0).0, [150, 0, 120, 255]);
        assert_eq!(atlas.image.get_pixel(0, 15).0, [0, 150, 120, 255]);
        assert_eq!(atlas.image.get_pixel(15, 15).0, [150, 150, 120, 255]);
        assert!(read_terrain_rgb24_tiles(&paths[..15], side).is_err());
        std::fs::write(&paths[15], b"invalid truncated tile").unwrap();
        assert!(read_terrain_rgb24_tiles(&paths, side).is_err());
    }

    #[test]
    fn reconstructs_baked_parent_and_higher_detail_child() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("terraintexture.pak");
        // Root 0, child 1 covers X-low/Y-low quadrant.
        let bytes = fixture(
            &[0, 1, -1, -1, -1, -1, -1, -1, -1],
            &[[40, 70, 100, 255], [180, 50, 30, 255]],
        );
        std::fs::write(&file, bytes).unwrap();
        let atlas = read_terrain_texture_atlas(&file).unwrap();
        assert_eq!(atlas.decoded_tiles, 2);
        assert_eq!(atlas.image.get_pixel(20, 20).0, [180, 50, 30, 255]);
        assert_eq!(atlas.image.get_pixel(1700, 1700).0, [40, 70, 100, 255]);
    }
    #[test]
    fn rejects_truncated_or_inconsistent_quadtree() {
        assert!(parse_tree(&[0, 1]).is_err());
        assert!(parse_tree(&[0, -1, -1, -1, -1, 9]).is_err());
    }
}
