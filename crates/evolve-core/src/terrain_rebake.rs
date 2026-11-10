//! Native RGB24 terrain tile re-encoder for Evolve's map-owned baked grid.
//!
//! This is the **output stage** of native terrain rebaking. It writes the
//! original width/height + BGR byte layout and engine XY transpose exactly.
//! It does NOT synthesize the game shader's missing per-pixel paint alpha,
//! layer order, or compiled terrain. Only a separately authenticated,
//! full-resolution baked result may be passed to this encoder.
//!
//! The caller must use the managed Modding-project, backup-backed transaction
//! to persist returned changed members. Original game PAKs stay read-only.
use crate::terrain::TerrainError;
use image::RgbaImage;

const MAX_GRID_SIDE: usize = 8;
const MAX_TILE_DIM: usize = 2048;
const MAX_ATLAS_DIM: usize = 4096;

/// Native tile replacement. `None` denotes an original member whose bytes
/// must remain untouched. Order is tile0_0, tile1_0, ..., tile0_1, ...
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRgb24Rebake {
    pub tile_dimension: usize,
    pub grid_side: usize,
    pub changed_tiles: Vec<Option<Vec<u8>>>,
}

fn bad(message: &str) -> TerrainError {
    TerrainError::Format(message.to_owned())
}

fn verify_tile(source: &[u8]) -> Result<usize, TerrainError> {
    if source.len() < 8 {
        return Err(bad("truncated original terrain RGB24 tile header"));
    }
    let width = u32::from_le_bytes(source[0..4].try_into().map_err(|_| TerrainError::Limits)?);
    let height = u32::from_le_bytes(source[4..8].try_into().map_err(|_| TerrainError::Limits)?);
    let dimension = width as usize;
    if width != height
        || !(4..=MAX_TILE_DIM).contains(&dimension)
        || !width.is_power_of_two()
    {
        return Err(bad("unsupported original RGB24 terrain tile dimensions"));
    }
    let payload = dimension
        .checked_mul(dimension)
        .and_then(|n| n.checked_mul(3))
        .ok_or(TerrainError::Limits)?;
    if source.len() != 8 + payload {
        return Err(bad("truncated or trailing original RGB24 terrain payload"));
    }
    Ok(dimension)
}

/// Encode exact, already-shaded full-resolution RGB pixels into native tile
/// members. The input image uses the original *render* orientation, with U
/// indexed by world Y and V by world X. The saved format is BGR and transposes
/// both tile order and interior pixels in the same way as the verified reader.
///
/// No change is emitted for a tile whose new bytes match its original bytes.
/// The caller must not pass guessed categorical LayerId colors: this function
/// does not claim native pixel *generation* or validate shader provenance.
pub fn encode_original_rgb24_rebake(
    originals: &[&[u8]],
    side: usize,
    baked: &RgbaImage,
) -> Result<NativeRgb24Rebake, TerrainError> {
    if side == 0 || side > MAX_GRID_SIDE || side.checked_mul(side) != Some(originals.len()) {
        return Err(bad("missing, mismatched or unverified RGB24 terrain tile grid"));
    }
    let dimension = verify_tile(originals[0])?;
    let atlas_dimension = dimension.checked_mul(side).ok_or(TerrainError::Limits)?;
    if atlas_dimension > MAX_ATLAS_DIM
        || baked.width() as usize != atlas_dimension
        || baked.height() as usize != atlas_dimension
    {
        return Err(bad("baked RGB output does not match native tile resolution"));
    }
    // Do not silently strip nonopaque alpha or convert from a shader texture
    // which carries transparency/other channels in the alpha component.
    if baked.pixels().any(|pixel| pixel.0[3] != 255) {
        return Err(bad("native RGB24 terrain output requires opaque source pixels"));
    }
    let rgba = baked.as_raw();
    let mut changed_tiles = Vec::with_capacity(originals.len());
    for (index, source) in originals.iter().enumerate() {
        if verify_tile(source)? != dimension {
            return Err(bad("mixed-size original RGB24 terrain grid"));
        }
        let tile_x = index % side;
        let tile_y = index / side;
        let mut output = source.to_vec();
        for row in 0..dimension {
            for col in 0..dimension {
                let atlas_x = tile_y * dimension + row;
                let atlas_y = tile_x * dimension + col;
                let source_at = (atlas_y * atlas_dimension + atlas_x) * 4;
                let tile_at = 8 + (row * dimension + col) * 3;
                output[tile_at] = rgba[source_at + 2];
                output[tile_at + 1] = rgba[source_at + 1];
                output[tile_at + 2] = rgba[source_at];
            }
        }
        changed_tiles.push((output != *source).then_some(output));
    }
    Ok(NativeRgb24Rebake {
        tile_dimension: dimension,
        grid_side: side,
        changed_tiles,
    })
}

/// Reconstruct an original-format atlas from in-memory members for exact
/// round-trip verification. This shares the writer's bounds but reads the
/// data independently of the game-source file system.
pub fn decode_original_rgb24_grid(
    originals: &[&[u8]],
    side: usize,
) -> Result<RgbaImage, TerrainError> {
    if side == 0 || side > MAX_GRID_SIDE || side.checked_mul(side) != Some(originals.len()) {
        return Err(bad("missing original RGB24 terrain grid"));
    }
    let dimension = verify_tile(originals[0])?;
    let atlas_dim = side.checked_mul(dimension).ok_or(TerrainError::Limits)?;
    if atlas_dim > MAX_ATLAS_DIM {
        return Err(TerrainError::Limits);
    }
    let mut image = RgbaImage::new(atlas_dim as u32, atlas_dim as u32);
    let pixels = image.as_mut();
    for (index, tile) in originals.iter().enumerate() {
        if verify_tile(tile)? != dimension {
            return Err(bad("mixed-size original terrain RGB24 tiles"));
        }
        let tile_x = index % side;
        let tile_y = index / side;
        for row in 0..dimension {
            for col in 0..dimension {
                let from = 8 + (row * dimension + col) * 3;
                let atlas_x = tile_y * dimension + row;
                let atlas_y = tile_x * dimension + col;
                let to = (atlas_y * atlas_dim + atlas_x) * 4;
                pixels[to..to + 4].copy_from_slice(&[tile[from + 2], tile[from + 1], tile[from], 255]);
            }
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original_tile(dim: u32, index: u8) -> Vec<u8> {
        let mut tile = Vec::new();
        tile.extend_from_slice(&dim.to_le_bytes());
        tile.extend_from_slice(&dim.to_le_bytes());
        for y in 0..dim {
            for x in 0..dim {
                tile.extend_from_slice(&[index, x as u8 * 7, y as u8 * 11]);
            }
        }
        tile
    }

    #[test]
    fn native_four_tile_roundtrip_is_byte_identical() {
        let original: Vec<_> = (0..4).map(|i| original_tile(4, i)).collect();
        let refs: Vec<_> = original.iter().map(Vec::as_slice).collect();
        let atlas = decode_original_rgb24_grid(&refs, 2).unwrap();
        let result = encode_original_rgb24_rebake(&refs, 2, &atlas).unwrap();
        assert_eq!(result.grid_side, 2);
        assert_eq!(result.tile_dimension, 4);
        assert!(result.changed_tiles.iter().all(Option::is_none));
    }

    #[test]
    fn rebake_modifies_only_target_native_tile_without_mirroring() {
        let original: Vec<_> = (0..4).map(|i| original_tile(4, i)).collect();
        let refs: Vec<_> = original.iter().map(Vec::as_slice).collect();
        let mut atlas = decode_original_rgb24_grid(&refs, 2).unwrap();
        // Tile (1,0) is at atlas X=0..3, Y=4..7 after game transpose.
        atlas.get_pixel_mut(2, 7).0 = [201, 150, 77, 255];
        let rebake = encode_original_rgb24_rebake(&refs, 2, &atlas).unwrap();
        assert!(rebake.changed_tiles[0].is_none());
        assert!(rebake.changed_tiles[2].is_none());
        assert!(rebake.changed_tiles[3].is_none());
        let mut changed = original.clone();
        changed[1] = rebake.changed_tiles[1].clone().unwrap();
        assert_eq!(
            &changed[1][8 + ((2 * 4 + 3) * 3)..8 + ((2 * 4 + 3) * 3) + 3],
            &[77, 150, 201]
        );
        let encoded_refs: Vec<_> = changed.iter().map(Vec::as_slice).collect();
        assert_eq!(decode_original_rgb24_grid(&encoded_refs, 2).unwrap(), atlas);
    }

    #[test]
    fn reject_missing_tiles_mixed_dimensions_invalid_output_and_nonopaque_alpha() {
        let originals = [original_tile(4, 0), original_tile(4, 1), original_tile(4, 2), original_tile(4, 3)];
        let refs: Vec<_> = originals.iter().map(Vec::as_slice).collect();
        let atlas = decode_original_rgb24_grid(&refs, 2).unwrap();
        assert!(encode_original_rgb24_rebake(&refs[..3], 2, &atlas).is_err());
        assert!(encode_original_rgb24_rebake(&refs, 3, &atlas).is_err());
        assert!(encode_original_rgb24_rebake(&refs, 2, &RgbaImage::new(7, 8)).is_err());
        let mut alpha = atlas.clone();
        alpha.get_pixel_mut(1, 1).0[3] = 100;
        assert!(encode_original_rgb24_rebake(&refs, 2, &alpha).is_err());
        let extra = original_tile(8, 4);
        let mut wrong = refs.clone();
        wrong[2] = &extra;
        assert!(encode_original_rgb24_rebake(&wrong, 2, &atlas).is_err());
        let truncated = &refs[0][..refs[0].len() - 1];
        wrong[0] = truncated;
        assert!(encode_original_rgb24_rebake(&wrong, 2, &atlas).is_err());
    }

    #[test]
    fn original_rgb24_decoder_matches_public_file_backed_reader() {
        let dir = tempfile::tempdir().unwrap();
        let originals: Vec<_> = (0..4).map(|i| original_tile(4, i)).collect();
        let mut paths = Vec::new();
        for (i, bytes) in originals.iter().enumerate() {
            let path = dir.path().join(format!("tile{}_{}.raw", i % 2, i / 2));
            std::fs::write(&path, bytes).unwrap();
            paths.push(path);
        }
        let original_refs: Vec<_> = originals.iter().map(Vec::as_slice).collect();
        let native = decode_original_rgb24_grid(&original_refs, 2).unwrap();
        let original_reader = crate::terrain_tiles::read_terrain_rgb24_tiles(&paths, 2).unwrap();
        assert_eq!(native, original_reader.image);
    }
}
