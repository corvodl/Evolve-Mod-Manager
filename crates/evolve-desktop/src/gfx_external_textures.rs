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
// Higher bounded budget restores more original Stage 2 menu sprite tiles;
// final reconstructed movie is still capped by MAX_DECOMPRESSED (32 MiB).
const MAX_TOTAL_PIXELS: u64 = 48_000_000;

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

/// Give the original Scaleform img:// bitmap lookup an SWF linkage fallback.
///
/// GFx normally handles img:// through a C++ ImageCreator callback, not a
/// browser request. Ruffle's URL rewrite alone cannot implement that callback.
/// Exporting the *verified original* DDS as a temporary SWF bitmap under its
/// authored img:// linkage allows ordinary AVM1 BitmapData.loadBitmap lookups
/// to resolve without changing original game files or fabricating artwork.
/// Whether a particular movie uses this path still requires runtime testing.
pub(crate) fn embed_mode_bitmap_linkages(
    swf: &[u8],
    scratch: &Path,
    names: &[String],
) -> Result<(Vec<u8>, usize, usize), String> {
    if names.is_empty() {
        return Ok((swf.to_vec(), 0, 0));
    }
    if swf.len() > MAX_DECOMPRESSED || !swf.starts_with(b"FWS") {
        return Err("Mode bitmap injection requires a bounded temporary FWS".into());
    }
    let (head, movie_tags) = tags(swf)?;
    if !movie_tags.iter().any(|tag| tag.code == 1) {
        return Err("Mode bitmap injection requires an original ShowFrame".into());
    }
    // SWF character IDs are global, including original Scaleform 1008 tiles.
    // Never overwrite existing shapes, sprites, fonts, or bitmap resources.
    let mut reserved = HashSet::<u16>::new();
    for tag in &movie_tags {
        if matches!(
            tag.code,
            2 | 6
                | 7
                | 10
                | 11
                | 14
                | 20
                | 21
                | 22
                | 32
                | 34
                | 35
                | 36
                | 37
                | 39
                | 46
                | 48
                | 60
                | 75
                | 83
                | 84
                | 87
                | 90
                | 91
                | 1008
                | 1009
        ) && let Some(id) = u16_at(&tag.data, 0)
        {
            reserved.insert(id);
        }
    }
    let mut extra = Vec::new();
    let mut link_entries = Vec::new();
    let mut embedded = 0usize;
    let mut skipped = 0usize;
    let mut seen = HashSet::new();
    let source = scratch.join("runtime-images");
    let canonical_source = fs::canonicalize(&source).map_err(|e| e.to_string())?;
    for name in names.iter().take(64) {
        if !seen.insert(name.clone())
            || name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            skipped += 1;
            continue;
        }
        let local = source.join(format!("{name}.png"));
        let Ok(meta) = fs::symlink_metadata(&local) else {
            skipped += 1;
            continue;
        };
        if !meta.file_type().is_file() || meta.len() > 12 * 1024 * 1024 {
            skipped += 1;
            continue;
        }
        let Ok(real) = fs::canonicalize(&local) else {
            skipped += 1;
            continue;
        };
        if !real.starts_with(&canonical_source) {
            skipped += 1;
            continue;
        }
        let Ok(bytes) = fs::read(real) else {
            skipped += 1;
            continue;
        };
        let Ok(bitmap) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        else {
            skipped += 1;
            continue;
        };
        let image = bitmap.to_rgba8();
        let pixels = u64::from(image.width()) * u64::from(image.height());
        if pixels == 0 || pixels > MAX_CROP_PIXELS {
            skipped += 1;
            continue;
        }
        let Some(id) = (1..=u16::MAX).rev().find(|id| !reserved.contains(id)) else {
            skipped += 1;
            continue;
        };
        let original_linkage =
            format!("img://Libs/UI/FlashAssets/images/MenuModeSelect/{name}.dds");
        let encoded = encode_bitmap(id, &image)?;
        let mut entry = Vec::with_capacity(original_linkage.len() + 3);
        entry.extend(id.to_le_bytes());
        entry.extend(original_linkage.as_bytes());
        entry.push(0);
        // Preserve the existing fixed parser/movie cap. When an image would
        // exhaust it, skip that image instead of breaking the whole preview.
        let within_budget = swf
            .len()
            .checked_add(extra.len())
            .and_then(|n| n.checked_add(encoded.len()))
            .and_then(|n| n.checked_add(link_entries.len()))
            .and_then(|n| n.checked_add(entry.len()))
            .and_then(|n| n.checked_add(8))
            .is_some_and(|n| n <= MAX_DECOMPRESSED);
        if !within_budget {
            skipped += 1;
            continue;
        }
        reserved.insert(id);
        extra.extend(encoded);
        link_entries.extend(entry);
        embedded += 1;
    }
    skipped += names.len().saturating_sub(64);
    if embedded == 0 {
        return Ok((swf.to_vec(), 0, skipped));
    }
    let mut exports = (embedded as u16).to_le_bytes().to_vec();
    exports.extend(link_entries);
    encode_tag(56, &exports, &mut extra); // Standard SWF ExportAssets linkages.
    let mut output = swf[..head].to_vec();
    let mut inserted = false;
    for tag in movie_tags {
        if tag.code == 1 && !inserted {
            output.extend_from_slice(&extra);
            inserted = true;
        }
        encode_tag(tag.code, &tag.data, &mut output);
    }
    if output.len() > MAX_DECOMPRESSED {
        return Err("Original SWF + mode bitmap resources exceed safe preview cap".into());
    }
    let length = output.len() as u32;
    output[4..8].copy_from_slice(&length.to_le_bytes());
    Ok((output, embedded, skipped))
}

/// Redirect only the original main_menu.gfx ImageLoading.loadBitmap() URL
/// prefix in a *temporary* FWS to an isolated local preview route.
///
/// The actual AVM1 class calls MovieClipLoader.loadClip("img://" + virtual
/// DDS path, target). That loader does not use BitmapData.loadBitmap or
/// SWF ExportAssets linkages. Because "./img/" has exactly the same byte
/// length as "img://", we can patch its constant-pool entry in place without
/// altering any compiled ActionScript instructions or original game files.
/// The loopback server serves this *exact* MenuModeSelect virtual path from
/// the already verified, staged PNG derivative only.
pub(crate) fn route_mode_image_loadclip_urls(swf: &[u8]) -> Result<(Vec<u8>, usize), String> {
    if swf.len() > MAX_DECOMPRESSED || swf.len() < 12 || !swf.starts_with(b"FWS") {
        return Err("Mode image loadClip routing requires a bounded FWS".into());
    }
    if u32::try_from(swf.len()).ok() != Some(u32::from_le_bytes(swf[4..8].try_into().unwrap())) {
        return Err("Mode image loadClip routing requires an exact SWF length".into());
    }
    let (header, records) = tags(swf)?;
    let mut output = swf[..header].to_vec();
    let mut rewritten = 0usize;
    for tag in records {
        let mut data = tag.data;
        if tag.code == 59
            && data
                .windows(b"ImageLoading\0".len())
                .any(|s| s == b"ImageLoading\0")
            && data
                .windows(b"loadBitmap\0".len())
                .any(|s| s == b"loadBitmap\0")
        {
            for at in 0..data.len().saturating_sub(b"img://\0".len() - 1) {
                if &data[at..at + b"img://\0".len()] == b"img://\0" {
                    data[at..at + b"./img/\0".len()].copy_from_slice(b"./img/\0");
                    rewritten += 1;
                }
            }
        }
        encode_tag(tag.code, &data, &mut output);
    }
    if rewritten > 1 {
        return Err("Ambiguous original ImageLoading img:// rewrite".into());
    }
    if rewritten == 0 {
        return Ok((swf.to_vec(), 0));
    }
    if output.len() != swf.len() {
        return Err("Source image URL rewrite unexpectedly changed SWF length".into());
    }
    let final_length = output.len() as u32;
    output[4..8].copy_from_slice(&final_length.to_le_bytes());
    Ok((output, rewritten))
}

/// Rewrite only exact, source-qualified MenuModeSelect DDS string constants
/// to .png in the *temporary* AVM1 movie. Ruffle's MovieClipLoader decodes
/// browser images; leaving a .dds extension on a PNG HTTP response makes the
/// format ambiguous and can bypass its image loader. Both suffixes are four
/// bytes, so the original AVM1 bytecode offsets and function bodies are intact.
/// The source GFX and its verified DDS files are never edited.
pub(crate) fn route_mode_image_png_paths(
    swf: &[u8],
    verified_names: &[String],
) -> Result<(Vec<u8>, usize), String> {
    if swf.len() > MAX_DECOMPRESSED || swf.len() < 12 || !swf.starts_with(b"FWS") {
        return Err("Mode-card PNG path routing requires a bounded FWS".into());
    }
    if u32::try_from(swf.len()).ok() != Some(u32::from_le_bytes(swf[4..8].try_into().unwrap())) {
        return Err("Mode-card PNG path routing requires an exact SWF length".into());
    }
    let valid_names = verified_names
        .iter()
        .filter(|name| (1..=64).contains(&name.len())
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')))
        .collect::<Vec<_>>();
    let (header, records) = tags(swf)?;
    let mut output = swf[..header].to_vec();
    let mut rewritten = 0usize;
    for tag in records {
        let mut data = tag.data;
        if tag.code == 59 {
            for name in &valid_names {
                let exact = format!("Libs/UI/FlashAssets/images/MenuModeSelect/{name}.dds\0");
                for at in 0..data.len().saturating_sub(exact.len() - 1) {
                    if &data[at..at + exact.len()] == exact.as_bytes() {
                        let suffix = at + exact.len() - 5;
                        data[suffix..suffix + 4].copy_from_slice(b".png");
                        rewritten += 1;
                    }
                }
            }
        }
        encode_tag(tag.code, &data, &mut output);
    }
    if rewritten > valid_names.len().saturating_mul(4) {
        return Err("Too many source MenuModeSelect path substitutions".into());
    }
    if output.len() != swf.len() {
        return Err("Mode-card PNG routing unexpectedly changed SWF size".into());
    }
    let size = output.len() as u32;
    output[4..8].copy_from_slice(&size.to_le_bytes());
    Ok((output, rewritten))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changes_only_verified_source_menu_dds_paths_to_png_in_temporary_avm1() {
        let mut original = b"FWS\x08".to_vec();
        original.extend(0u32.to_le_bytes());
        original.extend([0x08, 0, 0, 12, 1, 0]);
        let original_card = b"Libs/UI/FlashAssets/images/MenuModeSelect/training.dds\0";
        let unrelated = b"Libs/UI/FlashAssets/images/Elsewhere/training.dds\0";
        let missing = b"Libs/UI/FlashAssets/images/MenuModeSelect/missing.dds\0";
        let mut action = vec![1, 0];
        action.extend_from_slice(original_card);
        action.extend_from_slice(unrelated);
        action.extend_from_slice(missing);
        encode_tag(59, &action, &mut original);
        encode_tag(0, &[], &mut original);
        let length = original.len() as u32;
        original[4..8].copy_from_slice(&length.to_le_bytes());
        let pristine = original.clone();
        let (patched, count) =
            route_mode_image_png_paths(&original, &["training".into()]).unwrap();
        assert_eq!(count, 1);
        assert_eq!(original, pristine);
        assert_eq!(patched.len(), original.len());
        assert!(patched.windows(b"MenuModeSelect/training.png\0".len())
            .any(|v| v == b"MenuModeSelect/training.png\0"));
        assert!(patched.windows(unrelated.len()).any(|v| v == unrelated));
        assert!(patched.windows(missing.len()).any(|v| v == missing));
        let (second, count) =
            route_mode_image_png_paths(&patched, &["training".into()]).unwrap();
        assert_eq!(count, 0);
        assert_eq!(second, patched);
    }

    #[test]
    fn routes_only_compiled_imageloading_movieclip_urls_without_touching_original() {
        // Synthetic SWF with a source-like ImageLoading constant pool.
        let mut original = b"FWS\x08".to_vec();
        original.extend(0u32.to_le_bytes());
        original.extend([0x08, 0, 0, 12, 1, 0]);
        let mut init_action = vec![1, 0]; // DoInitAction sprite ID
        init_action.extend(b"ImageLoading\0loadBitmap\0img://\0");
        encode_tag(59, &init_action, &mut original);
        encode_tag(1, &[], &mut original);
        encode_tag(0, &[], &mut original);
        let length = original.len() as u32;
        original[4..8].copy_from_slice(&length.to_le_bytes());
        let pristine = original.clone();
        let (patched, count) = route_mode_image_loadclip_urls(&original).unwrap();
        assert_eq!(count, 1);
        assert_eq!(original, pristine);
        assert_eq!(patched.len(), original.len());
        assert!(patched.windows(b"./img/\0".len()).any(|v| v == b"./img/\0"));
        assert!(!patched.windows(b"img://\0".len()).any(|v| v == b"img://\0"));
        assert_eq!(&patched[4..8], &(patched.len() as u32).to_le_bytes());

        let (patched_again, count) = route_mode_image_loadclip_urls(&patched).unwrap();
        assert_eq!(count, 0);
        assert_eq!(patched_again, patched);
    }

    #[test]
    fn img_protocol_outside_original_imageloading_is_untouched() {
        let mut source = b"FWS\x08".to_vec();
        source.extend(0u32.to_le_bytes());
        source.extend([0x08, 0, 0, 12, 1, 0]);
        encode_tag(
            59,
            b"\x01\x00DifferentClass\0loadBitmap\0img://\0",
            &mut source,
        );
        encode_tag(1, &[], &mut source);
        encode_tag(0, &[], &mut source);
        let length = source.len() as u32;
        source[4..8].copy_from_slice(&length.to_le_bytes());
        let (untouched, count) = route_mode_image_loadclip_urls(&source).unwrap();
        assert_eq!(count, 0);
        assert_eq!(untouched, source);
    }

    #[test]
    fn preview_bitmap_budgets_allow_large_original_menus_but_not_oversized_swf() {
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
    #[test]
    fn source_qualified_img_protocol_links_become_original_swf_assets() {
        // Isolated synthetic FWS. All real menu DDS content is supplied by
        // the user's Asset Library and is never bundled in the repository.
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join("runtime-images");
        fs::create_dir_all(&folder).unwrap();
        let png = folder.join("goliathTutorial.png");
        let sample = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        image::DynamicImage::ImageRgba8(sample)
            .save_with_format(&png, image::ImageFormat::Png)
            .unwrap();
        let mut source = b"FWS\x08".to_vec();
        source.extend(0u32.to_le_bytes());
        source.extend([0x08, 0]);
        source.extend([0, 12, 1, 0]);
        encode_tag(69, &[0, 0, 0, 0], &mut source);
        encode_tag(1, &[], &mut source);
        encode_tag(0, &[], &mut source);
        let length = source.len() as u32;
        source[4..8].copy_from_slice(&length.to_le_bytes());
        let (linked, added, missing) = embed_mode_bitmap_linkages(
            &source,
            scratch.path(),
            &["goliathTutorial".to_string(), "notStaged".to_string()],
        )
        .unwrap();
        assert_eq!((added, missing), (1, 1));
        let (_header, tags) = tags(&linked).unwrap();
        assert_eq!(tags.iter().filter(|t| t.code == 36).count(), 1);
        let position = tags.iter().position(|t| t.code == 56).unwrap();
        assert!(position < tags.iter().position(|t| t.code == 1).unwrap());
        let exports = &tags[position].data;
        assert_eq!(&exports[..2], &1u16.to_le_bytes());
        assert!(
            exports
                .windows(
                    b"img://Libs/UI/FlashAssets/images/MenuModeSelect/goliathTutorial.dds".len()
                )
                .any(
                    |v| v == b"img://Libs/UI/FlashAssets/images/MenuModeSelect/goliathTutorial.dds"
                )
        );
        assert!(source.windows(4).any(|v| v == b"FWS\x08"));
        assert!(!source.windows(6).any(|v| v == b"img://"));
        assert_eq!(&linked[4..8], &(linked.len() as u32).to_le_bytes());
    }

    #[test]
    fn unsafe_linkage_names_are_rejected_without_source_file_reads() {
        let scratch = tempfile::tempdir().unwrap();
        fs::create_dir_all(scratch.path().join("runtime-images")).unwrap();
        let mut source = b"FWS\x08".to_vec();
        source.extend(0u32.to_le_bytes());
        source.extend([0x08, 0, 0, 12, 1, 0]);
        encode_tag(1, &[], &mut source);
        encode_tag(0, &[], &mut source);
        let length = source.len() as u32;
        source[4..8].copy_from_slice(&length.to_le_bytes());
        let (movie, accepted, rejected) = embed_mode_bitmap_linkages(
            &source,
            scratch.path(),
            &["../secret".to_string(), "C:".to_string()],
        )
        .unwrap();
        assert_eq!((accepted, rejected), (0, 2));
        assert_eq!(movie, source);
    }
}
