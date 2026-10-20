//! DDS inspection, CryEngine streaming-mip reconstruction and selected PNG decoding.
//! Supports BC1/2/3/4U/5U and common 32-bit RGB(A) layouts. Unsupported types
//! fail closed; normal-map BC5 channels are exported raw (not shader reconstructed).
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_PIXELS: u64 = 40_000_000;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PARTS: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DdsFormat {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Ctx1,
    Rgba32,
}

/// GPU layout of the source DDS payload. A CryTek 2012 console texture is
/// not byte-compatible with a PC DDS even when its FourCC is DXT1/DXT5.
/// The caller selects this from verified model/build provenance, not from
/// the filename or a color-based guess that could damage Stage 2 maps.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DdsLayout {
    Linear,
    Xbox360Tiled,
}

impl DdsFormat {
    fn bytes_per_block(self) -> Option<usize> {
        match self {
            Self::Bc1 | Self::Bc4 | Self::Ctx1 => Some(8),
            Self::Bc2 | Self::Bc3 | Self::Bc5 => Some(16),
            Self::Rgba32 => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DdsInfo {
    pub schema: u32,
    pub width: u32,
    pub height: u32,
    pub mipmaps: u32,
    pub format: DdsFormat,
    pub streaming_parts: usize,
    pub top_mip_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DdsError {
    #[error("Select an existing DDS or complete DDS.0 file (not a symbolic link)")]
    InvalidFile,
    #[error("Invalid or unsupported DDS data: {0}")]
    Unsupported(String),
    #[error("DDS dimensions, texture length or fragment count exceed safety limits")]
    LimitsExceeded,
    #[error("Refusing to overwrite an existing output file")]
    OutputExists,
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("PNG encoding failure: {0}")]
    Image(#[from] image::ImageError),
}

fn u32_at(buffer: &[u8], offset: usize) -> Result<u32, DdsError> {
    let bytes = buffer
        .get(offset..offset + 4)
        .ok_or_else(|| DdsError::Unsupported("truncated header".into()))?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        DdsError::Unsupported("invalid header".into())
    })?))
}

fn checked_read(path: &Path) -> Result<Vec<u8>, DdsError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| DdsError::InvalidFile)?;
    if !metadata.file_type().is_file() {
        return Err(DdsError::InvalidFile);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(DdsError::LimitsExceeded);
    }
    Ok(fs::read(path)?)
}

fn parse_header(bytes: &[u8]) -> Result<(DdsInfo, usize, Option<[u32; 4]>), DdsError> {
    if bytes.len() < 128
        || &bytes[..4] != b"DDS "
        || u32_at(bytes, 4)? != 124
        || u32_at(bytes, 76)? != 32
    {
        return Err(DdsError::Unsupported("missing DDS magic/header".into()));
    }
    let flags = u32_at(bytes, 8)?;
    let height = u32_at(bytes, 12)?;
    let width = u32_at(bytes, 16)?;
    let mipmaps = if flags & 0x20000 != 0 {
        u32_at(bytes, 28)?
    } else {
        1
    };
    if flags & 0x1004 != 0x1004
        || width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || mipmaps == 0
        || mipmaps > 20
        || mipmaps > u32::BITS - width.max(height).leading_zeros()
    {
        return Err(DdsError::LimitsExceeded);
    }
    if (flags & 0x800000 != 0 && u32_at(bytes, 24)? != 1) || u32_at(bytes, 112)? != 0 {
        return Err(DdsError::Unsupported("volume/cubemap texture".into()));
    }
    let format_flags = u32_at(bytes, 80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or_else(|| DdsError::Unsupported("short FourCC".into()))?;
    let mut payload_offset = 128usize;
    let mut masks = None;
    let format = if format_flags & 4 != 0 {
        match fourcc {
            b"DXT1" => DdsFormat::Bc1,
            b"CTX1" => DdsFormat::Ctx1,
            b"DXT3" => DdsFormat::Bc2,
            b"DXT5" => DdsFormat::Bc3,
            b"ATI1" | b"BC4U" => DdsFormat::Bc4,
            b"ATI2" | b"BC5U" => DdsFormat::Bc5,
            b"DX10" => {
                if bytes.len() < 148 {
                    return Err(DdsError::Unsupported("missing DX10 header".into()));
                }
                payload_offset = 148;
                if u32_at(bytes, 132)? != 3 || u32_at(bytes, 136)? != 0 || u32_at(bytes, 140)? != 1
                {
                    return Err(DdsError::Unsupported(
                        "non-2D/array/cube DX10 layout".into(),
                    ));
                }
                match u32_at(bytes, 128)? {
                    71 | 72 => DdsFormat::Bc1,
                    74 | 75 => DdsFormat::Bc2,
                    77 | 78 => DdsFormat::Bc3,
                    80 | 81 => DdsFormat::Bc4,
                    83 | 84 => DdsFormat::Bc5,
                    28 | 29 | 87 | 91 => {
                        masks = Some(if matches!(u32_at(bytes, 128)?, 87 | 91) {
                            [0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000]
                        } else {
                            [0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000]
                        });
                        DdsFormat::Rgba32
                    }
                    n => return Err(DdsError::Unsupported(format!("DXGI format {n}"))),
                }
            }
            _ => return Err(DdsError::Unsupported(format!("FourCC {:?}", fourcc))),
        }
    } else {
        if format_flags & 0x40 == 0 || u32_at(bytes, 88)? != 32 {
            return Err(DdsError::Unsupported(
                "only 32-bit RGB DDS is supported".into(),
            ));
        }
        masks = Some([
            u32_at(bytes, 92)?,
            u32_at(bytes, 96)?,
            u32_at(bytes, 100)?,
            u32_at(bytes, 104)?,
        ]);
        DdsFormat::Rgba32
    };
    Ok((
        DdsInfo {
            schema: 1,
            width,
            height,
            mipmaps,
            format,
            streaming_parts: 1,
            top_mip_bytes: 0,
        },
        payload_offset,
        masks,
    ))
}

fn mip_sizes(info: &DdsInfo) -> Result<Vec<usize>, DdsError> {
    (0..info.mipmaps)
        .map(|level| {
            let w = (info.width >> level).max(1) as u64;
            let h = (info.height >> level).max(1) as u64;
            let size = if let Some(block_bytes) = info.format.bytes_per_block() {
                w.div_ceil(4)
                    .checked_mul(h.div_ceil(4))
                    .and_then(|n| n.checked_mul(block_bytes as u64))
            } else {
                w.checked_mul(h).and_then(|n| n.checked_mul(4))
            }
            .ok_or(DdsError::LimitsExceeded)?;
            usize::try_from(size).map_err(|_| DdsError::LimitsExceeded)
        })
        .collect()
}

fn parse_part_name(path: &Path) -> Result<(String, Option<usize>), DdsError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(DdsError::InvalidFile)?;
    let folded = name.to_ascii_lowercase();
    if folded.ends_with(".dds") {
        return Ok((folded, None));
    }
    let (base, num) = folded.rsplit_once(".dds.").ok_or(DdsError::InvalidFile)?;
    let index = num.parse::<usize>().map_err(|_| DdsError::InvalidFile)?;
    if index >= MAX_PARTS {
        return Err(DdsError::LimitsExceeded);
    }
    Ok((format!("{base}.dds"), Some(index)))
}

/// Reconstruct the highest mip from a complete texture or the verified
/// .dds.0 + .dds.1..N streaming set. Never guesses missing fragments.
type LoadedDds = (DdsInfo, Vec<u8>, usize, Option<[u32; 4]>);

fn load_dds(source: &Path) -> Result<LoadedDds, DdsError> {
    let (base, index) = parse_part_name(source)?;
    let header_path = if index.is_some_and(|i| i != 0) {
        source.with_file_name(format!("{base}.0"))
    } else {
        source.to_path_buf()
    };
    let initial = checked_read(&header_path)?;
    let (mut info, offset, masks) = parse_header(&initial)?;
    let sizes = mip_sizes(&info)?;
    info.top_mip_bytes = sizes[0];
    let expected = sizes
        .iter()
        .try_fold(0usize, |a, b| a.checked_add(*b))
        .ok_or(DdsError::LimitsExceeded)?;
    let payload_len = initial.len() - offset;
    if index.is_none() {
        if payload_len != expected {
            return Err(DdsError::Unsupported(
                "standalone DDS mip data incomplete or padded".into(),
            ));
        }
        return Ok((info, initial, offset, masks));
    }
    let parent = source.parent().ok_or(DdsError::InvalidFile)?;
    let mut parts: BTreeMap<usize, PathBuf> = BTreeMap::new();
    for candidate in fs::read_dir(parent)? {
        let path = candidate?.path();
        if let Ok((name, Some(n))) = parse_part_name(&path)
            && name == base
            && parts.insert(n, path).is_some()
        {
            return Err(DdsError::Unsupported(
                "duplicate numbered DDS fragment".into(),
            ));
        }
    }
    if !parts.contains_key(&0) || !parts.get(&0).is_some_and(|p| p == &header_path) {
        return Err(DdsError::Unsupported(
            "DDS .0 fragment spelling mismatch".into(),
        ));
    }
    if payload_len == expected && parts.len() == 1 {
        return Ok((info, initial, offset, masks));
    }
    let possible = (1..info.mipmaps as usize)
        .filter(|count| sizes[sizes.len() - count..].iter().sum::<usize>() == payload_len)
        .collect::<Vec<_>>();
    if possible.len() != 1 {
        return Err(DdsError::Unsupported(
            "DDS .0 is not a recognized mip tail".into(),
        ));
    }
    let tail = possible[0];
    let expected_parts = sizes.len() - tail + 1;
    if parts.len() != expected_parts
        || parts.keys().copied().collect::<Vec<_>>() != (0..expected_parts).collect::<Vec<_>>()
    {
        return Err(DdsError::Unsupported(
            "incomplete or noncontiguous split DDS set".into(),
        ));
    }
    let mut all = initial[..offset].to_vec();
    for i in (1..expected_parts).rev() {
        let bytes = checked_read(&parts[&i])?;
        if bytes.len() != sizes[sizes.len() - tail - i] {
            return Err(DdsError::Unsupported(format!(
                "DDS fragment .{i} length mismatch"
            )));
        }
        all.extend_from_slice(&bytes);
    }
    all.extend_from_slice(&initial[offset..]);
    if all.len() != offset + expected {
        return Err(DdsError::Unsupported(
            "reconstructed DDS length mismatch".into(),
        ));
    }
    info.streaming_parts = expected_parts;
    Ok((info, all, offset, masks))
}

pub fn inspect_dds(source: &Path) -> Result<DdsInfo, DdsError> {
    let (info, _, _, _) = load_dds(source)?;
    Ok(info)
}

fn color565(c: u16) -> [u8; 3] {
    let r = (c >> 11) as u8 & 31;
    let g = (c >> 5) as u8 & 63;
    let b = c as u8 & 31;
    [
        ((u16::from(r) * 255 + 15) / 31) as u8,
        ((u16::from(g) * 255 + 31) / 63) as u8,
        ((u16::from(b) * 255 + 15) / 31) as u8,
    ]
}

fn decode_bc1_colors(bytes: &[u8], opaque: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let c1 = u16::from_le_bytes([bytes[2], bytes[3]]);
    let a = color565(c0);
    let b = color565(c1);
    let mut palette = [[0u8; 4]; 4];
    palette[0] = [a[0], a[1], a[2], 255];
    palette[1] = [b[0], b[1], b[2], 255];
    if c0 > c1 || opaque {
        for channel in 0..3 {
            palette[2][channel] = ((2 * u16::from(a[channel]) + u16::from(b[channel])) / 3) as u8;
            palette[3][channel] = ((u16::from(a[channel]) + 2 * u16::from(b[channel])) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for channel in 0..3 {
            palette[2][channel] = ((u16::from(a[channel]) + u16::from(b[channel])) / 2) as u8;
        }
        palette[2][3] = 255;
    }
    let indices = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
    let mut result = [[0u8; 4]; 16];
    for (i, pixel) in result.iter_mut().enumerate() {
        *pixel = palette[((indices >> (2 * i)) & 3) as usize];
    }
    result
}

fn bc_alpha(bytes: &[u8]) -> [u8; 16] {
    let a = bytes[0];
    let b = bytes[1];
    let mut palette = [0u8; 8];
    palette[0] = a;
    palette[1] = b;
    if a > b {
        for i in 1..7 {
            palette[i + 1] = (((7 - i) as u16 * u16::from(a) + i as u16 * u16::from(b)) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            palette[i + 1] = (((5 - i) as u16 * u16::from(a) + i as u16 * u16::from(b)) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    let mut bits = 0u64;
    for (i, b) in bytes[2..8].iter().enumerate() {
        bits |= u64::from(*b) << (8 * i);
    }
    let mut result = [0u8; 16];
    for (i, value) in result.iter_mut().enumerate() {
        *value = palette[((bits >> (3 * i)) & 7) as usize];
    }
    result
}

fn decode_block(format: DdsFormat, bytes: &[u8]) -> [[u8; 4]; 16] {
    match format {
        DdsFormat::Bc1 => decode_bc1_colors(bytes, false),
        DdsFormat::Bc2 => {
            let mut colors = decode_bc1_colors(&bytes[8..16], true);
            for (i, pixel) in colors.iter_mut().enumerate() {
                let nibble = (bytes[i / 2] >> ((i % 2) * 4)) & 15;
                pixel[3] = nibble * 17;
            }
            colors
        }
        DdsFormat::Bc3 => {
            let alphas = bc_alpha(&bytes[..8]);
            let mut colors = decode_bc1_colors(&bytes[8..16], true);
            for (i, pixel) in colors.iter_mut().enumerate() {
                pixel[3] = alphas[i];
            }
            colors
        }
        DdsFormat::Bc4 => {
            let r = bc_alpha(bytes);
            let mut pixels = [[0u8; 4]; 16];
            for (i, pixel) in pixels.iter_mut().enumerate() {
                *pixel = [r[i], r[i], r[i], 255];
            }
            pixels
        }
        DdsFormat::Bc5 => {
            let r = bc_alpha(&bytes[..8]);
            let g = bc_alpha(&bytes[8..16]);
            let mut pixels = [[0u8; 4]; 16];
            for (i, pixel) in pixels.iter_mut().enumerate() {
                *pixel = [r[i], g[i], 0, 255];
            }
            pixels
        }
        DdsFormat::Ctx1 => {
            // Xbox 360 CTX1 encodes two 8-bit channels with two RG
            // endpoints and 2-bit indices, interpolated like opaque BC1.
            let c0 = [bytes[0], bytes[1]];
            let c1 = [bytes[2], bytes[3]];
            let palette = [
                c0,
                c1,
                [
                    ((u16::from(c0[0]) * 2 + u16::from(c1[0])) / 3) as u8,
                    ((u16::from(c0[1]) * 2 + u16::from(c1[1])) / 3) as u8,
                ],
                [
                    ((u16::from(c0[0]) + u16::from(c1[0]) * 2) / 3) as u8,
                    ((u16::from(c0[1]) + u16::from(c1[1]) * 2) / 3) as u8,
                ],
            ];
            let indices = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
            let mut pixels = [[0u8; 4]; 16];
            for (i, pixel) in pixels.iter_mut().enumerate() {
                let rg = palette[((indices >> (2 * i)) & 3) as usize];
                *pixel = [rg[0], rg[1], 0, 255];
            }
            pixels
        }
        DdsFormat::Rgba32 => [[0u8; 4]; 16],
    }
}

fn channel(value: u32, mask: u32, absent: u8) -> u8 {
    if mask == 0 {
        return absent;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let n = (value & mask) >> shift;
    let max = (1u64 << bits) - 1;
    ((u64::from(n) * 255 + max / 2) / max) as u8
}

fn validate_masks(m: [u32; 4]) -> Result<(), DdsError> {
    if m[0] == 0 || m[1] == 0 || m[2] == 0 {
        return Err(DdsError::Unsupported("RGB masks are required".into()));
    }
    let mut combined = 0u32;
    for mask in m {
        let shifted = mask >> mask.trailing_zeros().min(31);
        if mask != 0 && (shifted & shifted.wrapping_add(1)) != 0 {
            return Err(DdsError::Unsupported("noncontiguous channel mask".into()));
        }
        if combined & mask != 0 {
            return Err(DdsError::Unsupported("overlapping RGB channel mask".into()));
        }
        combined |= mask;
    }
    Ok(())
}

/// Address one GPU-compressed tile block in an Xbox 360 tiled surface.
/// All address arithmetic is bounded to the source's validated top mip.
fn xbox360_tiled_xy(offset: usize, width: usize, pitch: usize) -> (usize, usize) {
    let aligned_width = (width + 31) & !31;
    let log_bpp = (pitch >> 2) + ((pitch >> 1) >> (pitch >> 2));
    let offset_b = offset << log_bpp;
    let offset_t = ((offset_b & !4095) >> 3) + ((offset_b & 1792) >> 2) + (offset_b & 63);
    let offset_m = offset_t >> (7 + log_bpp);
    let macro_x = (offset_m % (aligned_width >> 5)) << 2;
    let tile_x = (((offset_t >> (5 + log_bpp)) & 2) + (offset_b >> 6)) & 3;
    let micro_x = ((((offset_t >> 1) & !15) + (offset_t & 15)) & ((pitch << 3) - 1)) >> log_bpp;
    let x = ((macro_x + tile_x) << 3) + micro_x;
    let macro_y = (offset_m / (aligned_width >> 5)) << 2;
    let tile_y = ((offset_t >> (6 + log_bpp)) & 1) + ((offset_b & 2048) >> 10);
    let micro_y =
        (((offset_t & (((pitch << 6) - 1) & !31)) + ((offset_t & 15) << 1)) >> (3 + log_bpp)) & !1;
    let y = ((macro_y + tile_y) << 3) + micro_y + ((offset_t & 16) >> 4);
    (x, y)
}

/// Convert a console top mip into the ordinary linear, little-endian block
/// order expected by our BC1/2/3/4/5/CTX1 CPU decoders.
fn untile_xbox360(
    tiled: &[u8],
    width: u32,
    height: u32,
    pitch: usize,
) -> Result<Vec<u8>, DdsError> {
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    if bw == 0
        || bh == 0
        || !bw.is_multiple_of(32)
        || !bh.is_multiple_of(32)
        || bw.checked_mul(bh).and_then(|v| v.checked_mul(pitch)) != Some(tiled.len())
    {
        return Err(DdsError::Unsupported(
            "Xbox 360 tiled surface has a non-macro-aligned or incomplete top mip".into(),
        ));
    }
    let mut linear = vec![0u8; tiled.len()];
    for offset in 0..bw * bh {
        let (x, y) = xbox360_tiled_xy(offset, bw, pitch);
        if x >= bw || y >= bh {
            return Err(DdsError::Unsupported(
                "Xbox 360 block address out of bounds".into(),
            ));
        }
        let from = offset * pitch;
        let to = (y * bw + x) * pitch;
        linear[to..to + pitch].copy_from_slice(&tiled[from..from + pitch]);
    }
    for word in linear.as_chunks_mut::<2>().0 {
        word.swap(0, 1);
    }
    Ok(linear)
}

/// Decode exactly the highest-quality full-resolution mipmap.
pub fn decode_dds_rgba(source: &Path) -> Result<(DdsInfo, RgbaImage), DdsError> {
    decode_dds_rgba_with_layout(source, DdsLayout::Linear)
}

/// Decode using the verified source platform, without modifying the game DDS.
pub fn decode_dds_rgba_with_layout(
    source: &Path,
    layout: DdsLayout,
) -> Result<(DdsInfo, RgbaImage), DdsError> {
    let (info, bytes, offset, masks) = load_dds(source)?;
    let pixels_len = u64::from(info.width) * u64::from(info.height) * 4;
    let capacity = usize::try_from(pixels_len).map_err(|_| DdsError::LimitsExceeded)?;
    let mut rgba = vec![0u8; capacity];
    let original = &bytes[offset..offset + info.top_mip_bytes];
    let converted = if layout == DdsLayout::Xbox360Tiled {
        let pitch = info.format.bytes_per_block().ok_or_else(|| {
            DdsError::Unsupported("Console uncompressed surface decoding is not verified".into())
        })?;
        Some(untile_xbox360(original, info.width, info.height, pitch)?)
    } else {
        None
    };
    let payload = converted.as_deref().unwrap_or(original);
    if info.format == DdsFormat::Rgba32 {
        let masks = masks.ok_or_else(|| DdsError::Unsupported("missing channel masks".into()))?;
        validate_masks(masks)?;
        let (destination_pixels, _) = rgba.as_chunks_mut::<4>();
        for (src, dest) in payload
            .as_chunks::<4>()
            .0
            .iter()
            .zip(destination_pixels.iter_mut())
        {
            let pixel = u32::from_le_bytes(*src);
            dest.copy_from_slice(&[
                channel(pixel, masks[0], 0),
                channel(pixel, masks[1], 0),
                channel(pixel, masks[2], 0),
                channel(pixel, masks[3], 255),
            ]);
        }
    } else {
        let block_size = info
            .format
            .bytes_per_block()
            .ok_or(DdsError::LimitsExceeded)?;
        let bw = info.width.div_ceil(4);
        let bh = info.height.div_ceil(4);
        for by in 0..bh {
            for bx in 0..bw {
                let start = ((by * bw + bx) as usize) * block_size;
                let colors = decode_block(info.format, &payload[start..start + block_size]);
                for y in 0..4 {
                    for x in 0..4 {
                        let px = bx * 4 + x;
                        let py = by * 4 + y;
                        if px < info.width && py < info.height {
                            let target = ((py * info.width + px) * 4) as usize;
                            rgba[target..target + 4].copy_from_slice(&colors[(y * 4 + x) as usize]);
                        }
                    }
                }
            }
        }
    }
    let image =
        RgbaImage::from_raw(info.width, info.height, rgba).ok_or(DdsError::LimitsExceeded)?;
    Ok((info, image))
}

/// Explicit opt-in PNG export. Never overwrites an existing PNG.
pub fn export_dds_png(source: &Path, destination: &Path) -> Result<DdsInfo, DdsError> {
    let (info, rgba) = decode_dds_rgba(source)?;
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(DdsError::OutputExists);
        }
        Err(error) => return Err(DdsError::Io(error)),
    };
    if let Err(error) = DynamicImage::ImageRgba8(rgba).write_to(&mut file, ImageFormat::Png) {
        drop(file);
        let _ = fs::remove_file(destination);
        return Err(DdsError::Image(error));
    }
    file.sync_all()?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::{DdsFormat, decode_block, untile_xbox360, xbox360_tiled_xy};

    #[test]
    fn xbox360_untile_reverses_block_order_and_16_bit_endian() {
        // A complete 128x128 DXT1 surface occupies one 32x32-block tile.
        let mut linear = vec![0u8; 32 * 32 * 8];
        for i in 0..32 * 32 {
            linear[i * 8..i * 8 + 2].copy_from_slice(&(i as u16).to_le_bytes());
        }
        let mut console = vec![0u8; linear.len()];
        for i in 0..32 * 32 {
            let (x, y) = xbox360_tiled_xy(i, 32, 8);
            let source = (y * 32 + x) * 8;
            let target = i * 8;
            console[target..target + 8].copy_from_slice(&linear[source..source + 8]);
        }
        for pair in console.as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
        assert_eq!(untile_xbox360(&console, 128, 128, 8).unwrap(), linear);
        assert!(untile_xbox360(&console, 124, 128, 8).is_err());
    }

    #[test]
    fn ctx1_preserves_interpolated_original_rg_channels() {
        let pixels = decode_block(DdsFormat::Ctx1, &[255, 0, 0, 255, 0xe4, 0, 0, 0]);
        assert_eq!(pixels[0], [255, 0, 0, 255]);
        assert_eq!(pixels[1], [0, 255, 0, 255]);
        assert_eq!(pixels[2], [170, 85, 0, 255]);
        assert_eq!(pixels[3], [85, 170, 0, 255]);
    }

    use super::*;

    fn header(w: u32, h: u32, mipmaps: u32, format: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&h.to_le_bytes());
        bytes[16..20].copy_from_slice(&w.to_le_bytes());
        bytes[28..32].copy_from_slice(&mipmaps.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(format);
        bytes
    }

    fn solid_bc1(red: bool) -> [u8; 8] {
        if red {
            [0, 248, 224, 7, 0, 0, 0, 0]
        } else {
            [224, 7, 0, 248, 0, 0, 0, 0]
        }
    }

    #[test]
    fn decodes_bc1_to_rgba_and_exports_png() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("diffuse.dds");
        let mut dds = header(4, 4, 1, b"DXT1");
        dds.extend_from_slice(&solid_bc1(true));
        fs::write(&src, dds).unwrap();
        let (info, result) = decode_dds_rgba(&src).unwrap();
        assert_eq!(info.format, DdsFormat::Bc1);
        assert_eq!(result.get_pixel(0, 0).0, [255, 0, 0, 255]);
        let out = temp.path().join("result.png");
        export_dds_png(&src, &out).unwrap();
        assert_eq!(image::open(&out).unwrap().width(), 4);
        assert!(matches!(
            export_dds_png(&src, &out),
            Err(DdsError::OutputExists)
        ));
    }

    #[test]
    fn rejects_missing_split_parts_and_reconstructs_top_mip() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("kala.dds.0");
        let mut tail = header(8, 8, 3, b"DXT1");
        tail.extend_from_slice(&solid_bc1(false));
        fs::write(&base, &tail).unwrap();
        assert!(inspect_dds(&base).is_err());
        fs::write(temp.path().join("kala.dds.1"), solid_bc1(false)).unwrap();
        fs::write(temp.path().join("kala.dds.2"), solid_bc1(true).repeat(4)).unwrap();
        let (info, rgba) = decode_dds_rgba(&base).unwrap();
        assert_eq!(info.streaming_parts, 3);
        assert_eq!(info.width, 8);
        assert_eq!(rgba.get_pixel(7, 7).0, [255, 0, 0, 255]);
    }

    #[test]
    fn rejects_truncated_payload_and_untrusted_dimensions() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("bad.dds");
        let mut bytes = header(4, 4, 1, b"DXT5");
        bytes.extend_from_slice(&[0u8; 8]);
        fs::write(&src, bytes).unwrap();
        assert!(inspect_dds(&src).is_err());
        let mut huge = header(65535, 65535, 1, b"DXT1");
        huge.extend_from_slice(&[0u8; 8]);
        fs::write(&src, huge).unwrap();
        assert!(matches!(inspect_dds(&src), Err(DdsError::LimitsExceeded)));
    }

    #[test]
    fn decodes_bgra_with_exact_original_channel_order() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("legacy.dds");
        let mut bytes = header(1, 1, 1, b"\0\0\0\0");
        bytes[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        for (i, mask) in [0x00ff0000u32, 0x0000ff00, 0x000000ff, 0xff000000]
            .iter()
            .enumerate()
        {
            bytes[92 + i * 4..96 + i * 4].copy_from_slice(&mask.to_le_bytes());
        }
        bytes.extend_from_slice(&[30, 20, 10, 255]);
        fs::write(&src, bytes).unwrap();
        let (_, rgba) = decode_dds_rgba(&src).unwrap();
        assert_eq!(rgba.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }
}
