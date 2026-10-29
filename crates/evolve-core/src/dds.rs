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
    Alpha8,
    Luminance8,
    Rgb24,
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
            Self::Rgba32 | Self::Rgb24 | Self::Alpha8 | Self::Luminance8 => None,
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
                    65 => DdsFormat::Alpha8,
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
        if format_flags & 1 != 0 && u32_at(bytes, 88)? == 8 && u32_at(bytes, 104)? == 0xff {
            return Ok((
                DdsInfo {
                    schema: 1,
                    width,
                    height,
                    mipmaps,
                    format: DdsFormat::Alpha8,
                    streaming_parts: 1,
                    top_mip_bytes: 0,
                },
                payload_offset,
                None,
            ));
        }
        if format_flags & 0x20000 != 0 && u32_at(bytes, 88)? == 8 {
            DdsFormat::Luminance8
        } else if format_flags & 0x40 != 0 && matches!(u32_at(bytes, 88)?, 24 | 32) {
            masks = Some([
                u32_at(bytes, 92)?,
                u32_at(bytes, 96)?,
                u32_at(bytes, 100)?,
                u32_at(bytes, 104)?,
            ]);
            if u32_at(bytes, 88)? == 24 {
                DdsFormat::Rgb24
            } else {
                DdsFormat::Rgba32
            }
        } else {
            return Err(DdsError::Unsupported(
                "Unsupported uncompressed DDS depth or channel flags".into(),
            ));
        }
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
                w.checked_mul(h).and_then(|n| {
                    n.checked_mul(match info.format {
                        DdsFormat::Alpha8 | DdsFormat::Luminance8 => 1,
                        DdsFormat::Rgb24 => 3,
                        _ => 4,
                    })
                })
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
    load_dds_with_parts(source, &[])
}

/// Combine verified externally-supplied streaming fragments with a local
/// .dds.0 mip tail. Each numbered part is still required exactly once;
/// callers must validate cross-archive source/build provenance separately.
fn load_dds_with_parts(source: &Path, additional: &[PathBuf]) -> Result<LoadedDds, DdsError> {
    if additional.len() >= MAX_PARTS {
        return Err(DdsError::LimitsExceeded);
    }
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
    for path in additional {
        let (name, index) = parse_part_name(path)?;
        let Some(number) = index else {
            return Err(DdsError::InvalidFile);
        };
        if name != base || number == 0 || parts.insert(number, path.clone()).is_some() {
            return Err(DdsError::Unsupported(
                "external streaming DDS fragments mismatch or duplicate local mip".into(),
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

/// Decode a verified raw CryEngine BC terrain sector (no DDS wrapper).
/// Source layout is PC-linear; unlike Xbox 360 DDS fragments, the baked
/// terrain surface package does not advertise any GPU swizzle.
pub fn decode_linear_bc_surface(
    width: u32,
    height: u32,
    format: DdsFormat,
    payload: &[u8],
) -> Result<RgbaImage, DdsError> {
    let block_bytes = format.bytes_per_block().ok_or_else(|| {
        DdsError::Unsupported("uncompressed terrain tile is not a BC surface".into())
    })?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(DdsError::LimitsExceeded);
    }
    let bw = width.div_ceil(4);
    let bh = height.div_ceil(4);
    let required = (bw as usize)
        .checked_mul(bh as usize)
        .and_then(|v| v.checked_mul(block_bytes))
        .ok_or(DdsError::LimitsExceeded)?;
    if payload.len() < required {
        return Err(DdsError::Unsupported(
            "incomplete compressed terrain tile".into(),
        ));
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let block_at = ((by * bw + bx) as usize) * block_bytes;
            let decoded = decode_block(format, &payload[block_at..block_at + block_bytes]);
            for y in 0..4 {
                for x in 0..4 {
                    let px = bx * 4 + x;
                    let py = by * 4 + y;
                    if px < width && py < height {
                        let dst = ((py * width + px) * 4) as usize;
                        pixels[dst..dst + 4].copy_from_slice(&decoded[(y * 4 + x) as usize]);
                    }
                }
            }
        }
    }
    RgbaImage::from_raw(width, height, pixels).ok_or(DdsError::LimitsExceeded)
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
                    ((u16::from(c0[0]) * 2 + u16::from(c1[0]) + 1) / 3) as u8,
                    ((u16::from(c0[1]) * 2 + u16::from(c1[1]) + 1) / 3) as u8,
                ],
                [
                    ((u16::from(c0[0]) + u16::from(c1[0]) * 2 + 1) / 3) as u8,
                    ((u16::from(c0[1]) + u16::from(c1[1]) * 2 + 1) / 3) as u8,
                ],
            ];
            let indices = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
            let mut pixels = [[0u8; 4]; 16];
            for (i, pixel) in pixels.iter_mut().enumerate() {
                let rg = palette[((indices >> (2 * i)) & 3) as usize];
                // CTX1 is a two-channel Xenos normal-map format. Mirror
                // the private manager's RG-to-XYZ reconstruction so direct
                // image preview and PNG/FBX texture exports don't have a
                // black/blue-zero B channel. The GPU still samples exact RG
                // and independently reconstructs tangent-space Z.
                let x = f32::from(rg[0]) / 127.5 - 1.0;
                let y = f32::from(rg[1]) / 127.5 - 1.0;
                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                let length = (x * x + y * y + z * z).sqrt().max(f32::EPSILON);
                let blue = (((z / length + 1.0) * 127.5).round() as i32).clamp(0, 255) as u8;
                *pixel = [rg[0], rg[1], blue, 255];
            }
            pixels
        }
        DdsFormat::Rgba32 | DdsFormat::Rgb24 | DdsFormat::Alpha8 | DdsFormat::Luminance8 => {
            [[0u8; 4]; 16]
        }
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

/// Decode the top mip using the verified source header platform layout.
/// This shared path also feeds embedded FBX materials, not just PNG export.
pub fn decode_dds_rgba(source: &Path) -> Result<(DdsInfo, RgbaImage), DdsError> {
    decode_dds_rgba_with_layout(source, source_layout_for_png(source)?)
}

/// Decode using the verified source platform, without modifying the game DDS.
pub fn decode_dds_rgba_with_layout(
    source: &Path,
    layout: DdsLayout,
) -> Result<(DdsInfo, RgbaImage), DdsError> {
    decode_dds_rgba_with_layout_parts(source, layout, &[])
}

/// Decode a complete cross-archive mip chain without modifying or copying
/// any original DDS. Extra numbered fragments are validated by the same
/// dimensions, exact mip lengths and contiguous set checks as local parts.
pub fn decode_dds_rgba_with_parts(
    source: &Path,
    additional: &[PathBuf],
) -> Result<(DdsInfo, RgbaImage), DdsError> {
    decode_dds_rgba_with_layout_parts(source, source_layout_for_png(source)?, additional)
}

fn decode_dds_rgba_with_layout_parts(
    source: &Path,
    layout: DdsLayout,
    additional: &[PathBuf],
) -> Result<(DdsInfo, RgbaImage), DdsError> {
    let (info, bytes, offset, masks) = load_dds_with_parts(source, additional)?;
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
    if matches!(info.format, DdsFormat::Alpha8 | DdsFormat::Luminance8) {
        for (sample, pixel) in payload.iter().zip(rgba.as_chunks_mut::<4>().0.iter_mut()) {
            // The extracted .dds.0a alpha/normal auxiliaries are best
            // inspected as grayscale images rather than invisible RGBA alpha.
            pixel.copy_from_slice(&[*sample, *sample, *sample, 255]);
        }
    } else if matches!(info.format, DdsFormat::Rgba32 | DdsFormat::Rgb24) {
        let masks = masks.ok_or_else(|| DdsError::Unsupported("missing channel masks".into()))?;
        validate_masks(masks)?;
        let (destination_pixels, _) = rgba.as_chunks_mut::<4>();
        let pixel_stride = if info.format == DdsFormat::Rgb24 {
            3
        } else {
            4
        };
        for (src, dest) in payload
            .chunks_exact(pixel_stride)
            .zip(destination_pixels.iter_mut())
        {
            let pixel = if pixel_stride == 3 {
                u32::from_le_bytes([src[0], src[1], src[2], 0])
            } else {
                u32::from_le_bytes([src[0], src[1], src[2], src[3]])
            };
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

/// Platform selection shared by image decoding and PNG/FBX export, using the
/// historical Xbox DDS header marker and flags, never filename guesses.
fn source_layout_for_png(source: &Path) -> Result<DdsLayout, DdsError> {
    use std::io::Read;
    let mut bytes = [0u8; 128];
    let (base, index) = parse_part_name(source)?;
    let header = if index.is_some_and(|n| n != 0) {
        source.with_file_name(format!("{base}.0"))
    } else {
        source.to_path_buf()
    };
    let mut input = fs::File::open(header)?;
    if input.read_exact(&mut bytes).is_ok()
        && &bytes[..4] == b"DDS "
        && &bytes[124..128] == b"FYRC"
        && u32::from_le_bytes(bytes[80..84].try_into().unwrap()) & 4 != 0
        && matches!(
            u32::from_le_bytes(bytes[36..40].try_into().unwrap()),
            0x1800 | 0x1040 | 0x1840 | 0x1000 | 0x1440 | 0x1010
        )
        && matches!(
            u32::from_le_bytes(bytes[108..112].try_into().unwrap()),
            0x1000 | 0x401008
        )
    {
        Ok(DdsLayout::Xbox360Tiled)
    } else {
        Ok(DdsLayout::Linear)
    }
}

/// Explicit opt-in PNG export. Never overwrites an existing PNG.
pub fn export_dds_png(source: &Path, destination: &Path) -> Result<DdsInfo, DdsError> {
    if !destination
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err(DdsError::Unsupported("Output texture must be a PNG".into()));
    }
    let (info, rgba) = decode_dds_rgba_with_layout(source, source_layout_for_png(source)?)?;
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
    use super::{
        DdsFormat, DdsLayout, decode_block, source_layout_for_png, untile_xbox360, xbox360_tiled_xy,
    };

    #[test]
    fn png_export_layout_is_strictly_header_selected() {
        let dir = tempfile::tempdir().unwrap();
        let texture = dir.path().join("sample.dds.0");
        let mut header = [0u8; 128];
        header[..4].copy_from_slice(b"DDS ");
        header[124..128].copy_from_slice(b"FYRC");
        header[80..84].copy_from_slice(&4u32.to_le_bytes());
        header[36..40].copy_from_slice(&0x1800u32.to_le_bytes());
        header[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        std::fs::write(&texture, header).unwrap();
        assert_eq!(
            source_layout_for_png(&texture).unwrap(),
            DdsLayout::Xbox360Tiled
        );
        header[124..128].copy_from_slice(b"GAME");
        std::fs::write(&texture, header).unwrap();
        assert_eq!(source_layout_for_png(&texture).unwrap(), DdsLayout::Linear);
    }

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
        assert_eq!(&pixels[0][..2], &[255, 0]);
        assert_eq!(&pixels[1][..2], &[0, 255]);
        assert_eq!(&pixels[2][..2], &[170, 85]);
        assert_eq!(&pixels[3][..2], &[85, 170]);
        // Original RG is not modified by the private-format Z reconstruction.
        assert!(pixels[2][2] > 200);
        assert!(pixels.iter().all(|pixel| pixel[3] == 255));
        let neutral = decode_block(DdsFormat::Ctx1, &[128, 128, 128, 128, 0, 0, 0, 0]);
        assert!(neutral.iter().all(|pixel| pixel[2] >= 254));
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
    fn shared_fbx_decoder_uses_native_layout_for_local_and_cross_pak_mips() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("native.dds");
        let mut bytes = header(128, 128, 1, b"DXT1");
        bytes[124..128].copy_from_slice(b"FYRC");
        bytes[36..40].copy_from_slice(&0x1800u32.to_le_bytes());
        bytes[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        // Big-endian RGB565 red endpoint, repeated over one native tile.
        let native = [248, 0, 7, 224, 0, 0, 0, 0].repeat(32 * 32);
        bytes.extend(&native);
        std::fs::write(&source, &bytes).unwrap();
        let (_, decoded) = decode_dds_rgba(&source).unwrap();
        assert!(decoded.pixels().all(|p| p.0 == [255, 0, 0, 255]));
        let (_, forced_linear) = decode_dds_rgba_with_layout(&source, DdsLayout::Linear).unwrap();
        assert_ne!(decoded, forced_linear);
        bytes.truncate(128);
        bytes[28..32].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend([0u8; 2048]);
        let low = dir.path().join("native.dds.0");
        std::fs::write(&low, bytes).unwrap();
        let high_dir = dir.path().join("high");
        std::fs::create_dir(&high_dir).unwrap();
        let high = high_dir.join("native.dds.1");
        std::fs::write(&high, native).unwrap();
        let (_, cross) = decode_dds_rgba_with_parts(&low, &[high]).unwrap();
        assert_eq!(cross, decoded);
        // Ordinary PC headers retain their original linear interpretation.
        let mut pc = std::fs::read(&source).unwrap();
        pc[124..128].copy_from_slice(b"GAME");
        std::fs::write(&source, pc).unwrap();
        assert_eq!(decode_dds_rgba(&source).unwrap().1, forced_linear);
    }

    #[test]
    fn decodes_historical_luminance_and_24bit_rgb_without_fake_color_channels() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("uncompressed.dds");
        let mut lum = header(4, 4, 1, b"\0\0\0\0");
        lum[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        lum[80..84].copy_from_slice(&0x20000u32.to_le_bytes());
        lum[88..92].copy_from_slice(&8u32.to_le_bytes());
        lum.extend_from_slice(&[90u8; 16]);
        fs::write(&source, lum).unwrap();
        let (info, image) = decode_dds_rgba(&source).unwrap();
        assert_eq!(info.format, DdsFormat::Luminance8);
        assert_eq!(image.get_pixel(2, 2).0, [90, 90, 90, 255]);

        let mut rgb = header(4, 4, 1, b"\0\0\0\0");
        rgb[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        rgb[80..84].copy_from_slice(&0x40u32.to_le_bytes());
        rgb[88..92].copy_from_slice(&24u32.to_le_bytes());
        rgb[92..96].copy_from_slice(&0xff0000u32.to_le_bytes());
        rgb[96..100].copy_from_slice(&0x00ff00u32.to_le_bytes());
        rgb[100..104].copy_from_slice(&0x0000ffu32.to_le_bytes());
        rgb.extend_from_slice(&[17, 33, 240].repeat(16));
        fs::write(&source, rgb).unwrap();
        let (info, image) = decode_dds_rgba(&source).unwrap();
        assert_eq!(info.format, DdsFormat::Rgb24);
        assert_eq!(image.get_pixel(1, 3).0, [240, 33, 17, 255]);
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
    fn split_low_mips_and_external_high_mip_reconstruct_exact_original_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let low = dir.path().join("dds_low");
        let high = dir.path().join("dds_high");
        fs::create_dir(&low).unwrap();
        fs::create_dir(&high).unwrap();
        let source = low.join("scorpid_body.dds.0");
        let mut tail = header(8, 8, 3, b"DXT1");
        tail.extend_from_slice(&solid_bc1(false));
        fs::write(&source, tail).unwrap();
        fs::write(low.join("scorpid_body.dds.1"), solid_bc1(false)).unwrap();
        let high_mip = high.join("scorpid_body.dds.2");
        fs::write(&high_mip, solid_bc1(true).repeat(4)).unwrap();
        assert!(decode_dds_rgba(&source).is_err());
        let (info, rgba) =
            decode_dds_rgba_with_parts(&source, std::slice::from_ref(&high_mip)).unwrap();
        assert_eq!(info.streaming_parts, 3);
        assert_eq!(rgba.get_pixel(7, 7).0, [255, 0, 0, 255]);
        let wrong = high.join("another_monster.dds.2");
        fs::write(&wrong, solid_bc1(false).repeat(4)).unwrap();
        assert!(decode_dds_rgba_with_parts(&source, &[wrong]).is_err());
        assert!(decode_dds_rgba_with_parts(&source, &[high_mip.clone(), high_mip]).is_err());
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
