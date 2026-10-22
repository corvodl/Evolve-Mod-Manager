//! Read-only reconstruction of Scaleform external bitmap atlas tiles for FFDec.
//! UI_Assets contains the cooked DDS artwork; UI_Data supplies menu/element
//! relationships, never display-list coordinates. Shape and sprite transforms
//! remain in the compiled movie. The reconstructed movie is preview-only.
use evolve_core::dds::decode_dds_rgba;
use evolve_core::gfx::{MAX_DECOMPRESSED, inspect_gfx};
use evolve_core::{LibraryReferenceStatus, resolve_imported_reference};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use image::{RgbaImage, imageops};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_TAGS: usize = 80_000;
// Stage 2 main_menu.gfx can reference hundreds of packed image tiles.
// Keep meaningful ceilings, but do not cut off almost every later tile.
const MAX_BITMAPS: usize = 512;
const MAX_CROP_PIXELS: u64 = 4_000_000;
const MAX_TOTAL_PIXELS: u64 = 32_000_000;

/// A bitmap may be restored only if the encoded SWF remains within the
/// existing 32 MiB parser limit. Check before inserting so an oversized
/// atlas cannot turn the entire live preview into an error.
fn has_movie_budget(original: usize, encoded_so_far: usize, next: usize) -> bool {
    original
        .checked_add(encoded_so_far)
        .and_then(|n| n.checked_add(next))
        .is_some_and(|n| n <= MAX_DECOMPRESSED)
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SkipDetails {
    pub missing_source: usize,
    pub preview_limit: usize,
    pub bitmap_count_limit: usize,
    pub pixel_limit: usize,
    pub movie_size_limit: usize,
    pub invalid_tile: usize,
    pub duplicate_or_embedded: usize,
    pub decode_error: usize,
    pub missing_paths: Vec<String>,
}
impl SkipDetails {
    pub fn accumulate(&mut self, other: &Self) {
        self.missing_source += other.missing_source;
        self.preview_limit += other.preview_limit;
        self.bitmap_count_limit += other.bitmap_count_limit;
        self.pixel_limit += other.pixel_limit;
        self.movie_size_limit += other.movie_size_limit;
        self.invalid_tile += other.invalid_tile;
        self.duplicate_or_embedded += other.duplicate_or_embedded;
        self.decode_error += other.decode_error;
        for path in &other.missing_paths {
            if self.missing_paths.len() < 12 && !self.missing_paths.contains(path) {
                self.missing_paths.push(path.clone());
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderReport {
    pub movie: Vec<u8>,
    pub converted: usize,
    pub skipped: usize,
    pub skip_details: SkipDetails,
    pub ui_data: Option<String>,
}

#[derive(Clone)]
struct Tag {
    code: u16,
    data: Vec<u8>,
}
#[derive(Clone)]
struct Atlas {
    virtual_path: String,
    width: u16,
    height: u16,
}
#[derive(Clone, Copy)]
struct Tile {
    image: u16,
    rectangle: [u16; 4],
    character: u16,
}

fn u16_at(b: &[u8], n: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(n..n + 2)?.try_into().ok()?))
}
fn safe_virtual(path: &str) -> bool {
    !path.is_empty()
        && path.len() < 1024
        && !path.starts_with('/')
        && !path.contains(['\\', ':'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
fn image_path(movie: &str, name: &str) -> Option<String> {
    let movie_parent = movie.rsplit_once('/').map_or("", |(p, _)| p);
    let virtual_path =
        if name.to_ascii_lowercase().starts_with("libs/ui/") || movie_parent.is_empty() {
            name.to_owned()
        } else {
            format!("{movie_parent}/{name}")
        };
    if !safe_virtual(&virtual_path) || !virtual_path.to_ascii_lowercase().ends_with(".tif") {
        return None;
    }
    Some(format!("{}.dds", &virtual_path[..virtual_path.len() - 4]))
}
fn raw_movie(data: &[u8]) -> Result<Vec<u8>, String> {
    inspect_gfx(data).map_err(|e| e.to_string())?;
    if &data[..3] != b"CFX" {
        return Ok(data.to_vec());
    }
    let declared = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let decoder = ZlibDecoder::new(&data[8..]);
    let mut uncompressed = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED + 1) as u64)
        .read_to_end(&mut uncompressed)
        .map_err(|e| e.to_string())?;
    if uncompressed.len() + 8 != declared {
        return Err("Scaleform decompressed size mismatch".into());
    }
    let mut result = data[..8].to_vec();
    result[0] = b'G';
    result.extend(uncompressed);
    Ok(result)
}
fn tags(raw: &[u8]) -> Result<(usize, Vec<Tag>), String> {
    let n = usize::from(raw[8] >> 3);
    let mut offset = 8 + (5 + 4 * n).div_ceil(8) + 4;
    if offset > raw.len() {
        return Err("Truncated movie header".into());
    }
    let start = offset;
    let mut found = Vec::new();
    while offset + 2 <= raw.len() {
        let tag = u16_at(raw, offset).ok_or("Truncated Scaleform tag")?;
        offset += 2;
        let code = tag >> 6;
        let mut len = usize::from(tag & 63);
        if len == 63 {
            len = u32::from_le_bytes(
                raw.get(offset..offset + 4)
                    .ok_or("Truncated Scaleform long tag")?
                    .try_into()
                    .unwrap(),
            ) as usize;
            offset += 4;
        }
        if len > raw.len() - offset || found.len() >= MAX_TAGS {
            return Err("Invalid Scaleform tag length or count".into());
        }
        found.push(Tag {
            code,
            data: raw[offset..offset + len].to_vec(),
        });
        offset += len;
        if code == 0 {
            break;
        }
    }
    Ok((start, found))
}
fn encode_tag(code: u16, data: &[u8], output: &mut Vec<u8>) {
    if data.len() < 63 {
        output.extend(((code << 6) | data.len() as u16).to_le_bytes());
    } else {
        output.extend(((code << 6) | 63).to_le_bytes());
        output.extend((data.len() as u32).to_le_bytes());
    }
    output.extend(data);
}
fn encode_bitmap(id: u16, image: &RgbaImage) -> Result<Vec<u8>, String> {
    if image.width() == 0
        || image.height() == 0
        || image.width() > u16::MAX as u32
        || image.height() > u16::MAX as u32
    {
        return Err("Unsupported external bitmap dimensions".into());
    }
    let mut argb = Vec::with_capacity(image.len());
    for pixel in image.pixels() {
        argb.extend([pixel[3], pixel[0], pixel[1], pixel[2]]);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(&argb).map_err(|e| e.to_string())?;
    let compressed = encoder.finish().map_err(|e| e.to_string())?;
    let mut data = Vec::with_capacity(compressed.len() + 7);
    data.extend(id.to_le_bytes());
    data.push(5); // SWF DefineBitsLossless2 format 5, 32-bit ARGB.
    data.extend((image.width() as u16).to_le_bytes());
    data.extend((image.height() as u16).to_le_bytes());
    data.extend(compressed);
    let mut result = Vec::new();
    encode_tag(36, &data, &mut result);
    Ok(result)
}
fn image_resource(tag: &Tag) -> Option<u16> {
    if matches!(tag.code, 6 | 20 | 21 | 35 | 36 | 90) {
        u16_at(&tag.data, 0)
    } else {
        None
    }
}
fn atlas_def(tag: &Tag, movie: &str) -> Option<(u16, Atlas)> {
    if tag.code != 1009 || tag.data.len() < 13 {
        return None;
    }
    let id = u16_at(&tag.data, 0)?;
    let width = u16_at(&tag.data, 6)?;
    let height = u16_at(&tag.data, 8)?;
    let len = usize::from(*tag.data.get(11)?);
    if width == 0 || height == 0 || len == 0 || tag.data.len() != 13 + len || *tag.data.last()? != 0
    {
        return None;
    }
    let path = std::str::from_utf8(&tag.data[12..12 + len]).ok()?;
    Some((
        id,
        Atlas {
            virtual_path: image_path(movie, path)?,
            width,
            height,
        },
    ))
}
fn bitmap_tile(tag: &Tag) -> Option<Tile> {
    if tag.code != 1008 || tag.data.len() != 12 {
        return None;
    }
    Some(Tile {
        character: u16_at(&tag.data, 0)?,
        image: u16_at(&tag.data, 2)?,
        rectangle: [
            u16_at(&tag.data, 4)?,
            u16_at(&tag.data, 6)?,
            u16_at(&tag.data, 8)?,
            u16_at(&tag.data, 10)?,
        ],
    })
}

/// Resolve only exact same-folder or verified same-build library entries.
/// No unscoped basename search, cross-build fallback or archive writes.
fn resolve_image(
    virtual_image: &str,
    virtual_movie: &str,
    source: &Path,
    library_root: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let movie_parent = virtual_movie.rsplit_once('/').map_or("", |(p, _)| p);
    if let Some(suffix) = virtual_image
        .strip_prefix(movie_parent)
        .and_then(|v| v.strip_prefix('/'))
        .filter(|v| safe_virtual(v))
        && let Some(parent) = source.parent()
    {
        for filename in [format!("{suffix}.0"), suffix.to_owned()] {
            let candidate = parent.join(filename);
            if let Ok(meta) = fs::symlink_metadata(&candidate)
                && meta.file_type().is_file()
                && let (Ok(real), Ok(base)) =
                    (fs::canonicalize(&candidate), fs::canonicalize(parent))
                && real.starts_with(base)
            {
                return Ok(Some(real));
            }
        }
    }
    let (Some(root), Some(label)) = (library_root, build) else {
        return Ok(None);
    };
    if label.trim().is_empty() {
        return Ok(None);
    }
    match resolve_imported_reference(root, virtual_image, Some(label)).map_err(|e| e.to_string())? {
        LibraryReferenceStatus::Found(hit) => Ok(Some(PathBuf::from(hit.absolute_path))),
        LibraryReferenceStatus::Missing | LibraryReferenceStatus::Ambiguous(_) => Ok(None),
    }
}

/// UI_Data maps UIElement definitions to movies, and declares preloads.
/// Neither is permitted to invent a canvas position for a texture.
pub(crate) fn associated_ui_data(
    movie: &str,
    library_root: Option<&Path>,
    build: Option<&str>,
) -> Option<String> {
    let (Some(root), Some(build)) = (library_root, build) else {
        return None;
    };
    let basename = movie.rsplit('/').next()?.strip_suffix(".gfx")?;
    let element = basename
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<String>();
    let path = format!("UI/UIElements/{element}.xml");
    let LibraryReferenceStatus::Found(ref_info) =
        resolve_imported_reference(root, &path, Some(build)).ok()?
    else {
        return None;
    };
    let xml = fs::read_to_string(ref_info.absolute_path).ok()?;
    if xml.len() > 512 * 1024 {
        return None;
    }
    if xml
        .to_ascii_lowercase()
        .contains(&format!("{basename}.gfx").to_ascii_lowercase())
    {
        Some(path)
    } else {
        None
    }
}

/// Recover bitmap characters from Scaleform 1008 atlas mappings and 1009
/// DDS references. Inject them into a temporary FFDec movie as standard SWF
/// DefineBitsLossless2 records; authored fills, timeline and masks stay intact.
pub fn prepare_preview(
    data: &[u8],
    virtual_movie: &str,
    source: &Path,
    library_root: Option<&Path>,
    build: Option<&str>,
) -> Result<RenderReport, String> {
    if !safe_virtual(virtual_movie) {
        return Err("Unsafe virtual GFX path for texture resolution".into());
    }
    let raw = raw_movie(data)?;
    let (head, tags) = tags(&raw)?;
    let mut result = RenderReport {
        movie: data.to_vec(),
        ui_data: associated_ui_data(virtual_movie, library_root, build),
        ..Default::default()
    };
    let mut atlases = HashMap::new();
    let mut mappings = Vec::new();
    let mut counts = HashMap::<u16, usize>::new();
    let mut built_in = HashSet::new();
    for tag in &tags {
        if let Some((id, atlas)) = atlas_def(tag, virtual_movie) {
            atlases.entry(id).or_insert(atlas);
        }
        if let Some(tile) = bitmap_tile(tag) {
            *counts.entry(tile.character).or_default() += 1;
            mappings.push(tile);
        }
        if let Some(id) = image_resource(tag) {
            built_in.insert(id);
        }
    }
    if mappings.is_empty() {
        return Ok(result);
    }
    if atlases.is_empty() {
        result.skipped = mappings.len();
        result.skip_details.invalid_tile = mappings.len();
        return Ok(result);
    }
    let mut converted = HashMap::<u16, Vec<u8>>::new();
    let mut cached = HashMap::<String, RgbaImage>::new();
    let mut total_pixels = 0u64;
    let mut encoded_bytes = 0usize;
    for tile in mappings {
        if converted.len() >= MAX_BITMAPS {
            result.skipped += 1;
            result.skip_details.preview_limit += 1;
            result.skip_details.bitmap_count_limit += 1;
            continue;
        }
        if counts.get(&tile.character) != Some(&1) || built_in.contains(&tile.character) {
            result.skipped += 1;
            result.skip_details.duplicate_or_embedded += 1;
            continue;
        }
        let Some(atlas) = atlases.get(&tile.image) else {
            result.skipped += 1;
            result.skip_details.invalid_tile += 1;
            continue;
        };
        let [x0, y0, x1, y1] = tile.rectangle;
        let crop_pixels = u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));
        if x0 >= x1 || y0 >= y1 || x1 > atlas.width || y1 > atlas.height {
            result.skipped += 1;
            result.skip_details.invalid_tile += 1;
            continue;
        }
        if crop_pixels > MAX_CROP_PIXELS
            || total_pixels.saturating_add(crop_pixels) > MAX_TOTAL_PIXELS
        {
            result.skipped += 1;
            result.skip_details.preview_limit += 1;
            result.skip_details.pixel_limit += 1;
            continue;
        }
        if !cached.contains_key(&atlas.virtual_path) {
            let Some(path) = resolve_image(
                &atlas.virtual_path,
                virtual_movie,
                source,
                library_root,
                build,
            )?
            else {
                result.skipped += 1;
                result.skip_details.missing_source += 1;
                if result.skip_details.missing_paths.len() < 12
                    && !result
                        .skip_details
                        .missing_paths
                        .contains(&atlas.virtual_path)
                {
                    result
                        .skip_details
                        .missing_paths
                        .push(atlas.virtual_path.clone());
                }
                continue;
            };
            match decode_dds_rgba(&path) {
                Ok((_info, image)) => {
                    if cached.len() >= 3 {
                        cached.clear();
                    }
                    cached.insert(atlas.virtual_path.clone(), image);
                }
                Err(_) => {
                    result.skipped += 1;
                    result.skip_details.decode_error += 1;
                    continue;
                }
            }
        }
        let Some(image) = cached.get(&atlas.virtual_path) else {
            continue;
        };
        if x1 as u32 > image.width() || y1 as u32 > image.height() {
            result.skipped += 1;
            result.skip_details.invalid_tile += 1;
            continue;
        }
        let cut = imageops::crop_imm(
            image,
            x0 as u32,
            y0 as u32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        )
        .to_image();
        let encoded = encode_bitmap(tile.character, &cut)?;
        if !has_movie_budget(raw.len(), encoded_bytes, encoded.len()) {
            result.skipped += 1;
            result.skip_details.preview_limit += 1;
            result.skip_details.movie_size_limit += 1;
            continue;
        }
        encoded_bytes += encoded.len();
        converted.insert(tile.character, encoded);
        total_pixels += crop_pixels;
    }
    if converted.is_empty() {
        return Ok(result);
    }
    // Leave every original tag other than replaced 1008 bitmap definitions
    // in its original order. Only the FFDec scratch movie receives injected
    // standard bitmap characters. Never rewrite project or PAK from this.
    let mut payload = Vec::new();
    let mut inserted = false;
    let mut injected: Vec<_> = converted.into_iter().collect();
    injected.sort_by_key(|(id, _)| *id);
    let mut insert = Vec::new();
    for (_id, bytes) in &injected {
        insert.extend(bytes);
    }
    if !tags.iter().any(|t| t.code == 69) {
        payload.extend(&insert);
        inserted = true;
    }
    for tag in tags {
        if !inserted && tag.code == 69 {
            encode_tag(tag.code, &tag.data, &mut payload);
            payload.extend(&insert);
            inserted = true;
            continue;
        }
        if let Some(tile) = bitmap_tile(&tag)
            && injected.iter().any(|(id, _)| *id == tile.character)
        {
            continue;
        }
        encode_tag(tag.code, &tag.data, &mut payload);
    }
    if head + payload.len() > MAX_DECOMPRESSED {
        return Err("Reconstructed FFDec preview exceeds 32 MiB cap".into());
    }
    let mut output = raw[..head].to_vec();
    output.extend(payload);
    let size = output.len() as u32;
    output[4..8].copy_from_slice(&size.to_le_bytes());
    result.converted = injected.len();
    result.movie = output;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_bitmap_budgets_allow_large_original_menus_but_not_oversized_swf() {
        assert!(MAX_BITMAPS >= 411);
        assert!(MAX_TOTAL_PIXELS >= 32_000_000);
        assert!(has_movie_budget(500_000, 10_000_000, 3_000_000));
        assert!(!has_movie_budget(500_000, 31_000_000, 3_000_000));
        assert!(!has_movie_budget(usize::MAX, usize::MAX, 1));
    }
    #[test]
    fn preview_skip_diagnostics_preserve_independent_causes_and_safe_paths() {
        let mut first = SkipDetails {
            missing_source: 2,
            preview_limit: 3,
            pixel_limit: 3,
            missing_paths: vec!["Libs/UI/main_menu_I9.dds".into()],
            ..Default::default()
        };
        let second = SkipDetails {
            missing_source: 1,
            invalid_tile: 4,
            missing_paths: vec![
                "Libs/UI/main_menu_I9.dds".into(),
                "Libs/UI/main_menu_I10.dds".into(),
            ],
            ..Default::default()
        };
        first.accumulate(&second);
        assert_eq!(first.missing_source, 3);
        assert_eq!(first.preview_limit, 3);
        assert_eq!(first.pixel_limit, 3);
        assert_eq!(first.invalid_tile, 4);
        assert_eq!(first.missing_paths.len(), 2);
        assert_eq!(first.missing_paths[1], "Libs/UI/main_menu_I10.dds");
    }
    #[test]
    fn atlas_path_is_scoped_to_gfx_and_rejects_traversal() {
        assert_eq!(
            image_path("Libs/UI/main_menu.gfx", "main_menu_I9.tif"),
            Some("Libs/UI/main_menu_I9.dds".to_owned())
        );
        assert!(image_path("Libs/UI/main_menu.gfx", "../secret.tif").is_none());
        assert!(image_path("Libs/UI/main_menu.gfx", "C:/secret.tif").is_none());
    }
    #[test]
    fn parses_verified_compiled_external_image_records() {
        let mut tag = Tag {
            code: 1009,
            data: vec![0; 12],
        };
        tag.data[0..2].copy_from_slice(&9u16.to_le_bytes());
        tag.data[6..8].copy_from_slice(&128u16.to_le_bytes());
        tag.data[8..10].copy_from_slice(&64u16.to_le_bytes());
        tag.data[11] = 7;
        tag.data.extend(b"foo.tif");
        tag.data.push(0);
        assert_eq!(
            atlas_def(&tag, "Libs/UI/menu.gfx").unwrap().1.virtual_path,
            "Libs/UI/foo.dds"
        );
        let tile = Tag {
            code: 1008,
            data: [15u16, 9, 0, 0, 16, 16]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        assert_eq!(bitmap_tile(&tile).unwrap().character, 15);
    }
    #[test]
    fn swf_lossless_argb_bytes_are_zlib_compressed() {
        let img = RgbaImage::from_pixel(1, 1, image::Rgba([3, 5, 7, 11]));
        let block = encode_bitmap(7, &img).unwrap();
        assert_eq!(u16_at(&block, 0).unwrap() >> 6, 36);
        let mut z = ZlibDecoder::new(&block[9..]);
        let mut raw = vec![];
        z.read_to_end(&mut raw).unwrap();
        assert_eq!(raw, vec![11, 3, 5, 7]);
    }
}
