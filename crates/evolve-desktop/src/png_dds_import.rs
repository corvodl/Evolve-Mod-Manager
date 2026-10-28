//! Bounded PNG -> DDS replacement encoder for editable Modding workspaces.
//! Keep the original DDS header, dimensions, mip count and GPU format.
//! Unsupported/split textures fail without changing the managed asset.
use evolve_core::{DdsFormat, inspect_dds};
use image::{RgbaImage, imageops::FilterType};
use std::fs;
use std::path::Path;

fn read_u32(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(at..at + 4)
        .ok_or("Truncated original DDS header")?
        .try_into()
        .map_err(|_| "Invalid DDS header")?;
    Ok(u32::from_le_bytes(bytes))
}
fn rgb565(c: [u8; 3]) -> u16 {
    ((u16::from(c[0]) * 31 + 127) / 255) << 11
        | ((u16::from(c[1]) * 63 + 127) / 255) << 5
        | ((u16::from(c[2]) * 31 + 127) / 255)
}
fn rgb_from_565(v: u16) -> [u8; 3] {
    let r = ((v >> 11) & 31) as u32;
    let g = ((v >> 5) & 63) as u32;
    let b = (v & 31) as u32;
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}
fn distance(a: &[u8; 3], b: &[u8; 3]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs().pow(2))
        .sum()
}
fn color_block(block: &[[u8; 4]; 16], transparent: bool) -> [u8; 8] {
    // Pick farthest RGB pair as endpoint seeds. More stable than luminosity
    // minima on chromatic art, while keeping encoding independent of DLLs.
    let mut best = (0, 0, 0u32);
    for i in 0..16 {
        for j in i + 1..16 {
            let a = [block[i][0], block[i][1], block[i][2]];
            let b = [block[j][0], block[j][1], block[j][2]];
            let d = distance(&a, &b);
            if d > best.2 {
                best = (i, j, d);
            }
        }
    }
    let mut c0 = rgb565([block[best.0][0], block[best.0][1], block[best.0][2]]);
    let mut c1 = rgb565([block[best.1][0], block[best.1][1], block[best.1][2]]);
    if transparent {
        if c0 > c1 {
            std::mem::swap(&mut c0, &mut c1);
        }
    } else {
        if c0 < c1 {
            std::mem::swap(&mut c0, &mut c1);
        }
        if c0 == c1 {
            if c0 < u16::MAX {
                c0 += 1;
            } else {
                c1 -= 1;
            }
        }
    }
    let a = rgb_from_565(c0);
    let b = rgb_from_565(c1);
    let mut palette = [a, b, [0; 3], [0; 3]];
    if transparent {
        for c in 0..3 {
            palette[2][c] = ((u16::from(a[c]) + u16::from(b[c])) / 2) as u8;
        }
    } else {
        for c in 0..3 {
            palette[2][c] = ((2 * u16::from(a[c]) + u16::from(b[c])) / 3) as u8;
            palette[3][c] = ((u16::from(a[c]) + 2 * u16::from(b[c])) / 3) as u8;
        }
    }
    let mut bits = 0u32;
    for (i, pixel) in block.iter().enumerate() {
        let index = if transparent && pixel[3] < 128 {
            3u32
        } else {
            let rgb = [pixel[0], pixel[1], pixel[2]];
            (0..if transparent { 3 } else { 4 })
                .min_by_key(|&j| distance(&rgb, &palette[j]))
                .unwrap_or(0) as u32
        };
        bits |= index << (i * 2);
    }
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&c0.to_le_bytes());
    bytes[2..4].copy_from_slice(&c1.to_le_bytes());
    bytes[4..8].copy_from_slice(&bits.to_le_bytes());
    bytes
}
fn alpha_block(pixels: &[u8; 16]) -> [u8; 8] {
    let hi = *pixels.iter().max().unwrap_or(&255);
    let lo = *pixels.iter().min().unwrap_or(&0);
    let mut palette = [hi, lo, 0, 0, 0, 0, 0, 0];
    if hi > lo {
        for i in 1..=6 {
            palette[i + 1] = ((u16::from(7 - i as u8) * u16::from(hi)
                + u16::from(i as u8) * u16::from(lo))
                / 7) as u8;
        }
    } else {
        for i in 1..=4 {
            palette[i + 1] = ((u16::from(5 - i as u8) * u16::from(hi)
                + u16::from(i as u8) * u16::from(lo))
                / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    let mut bits = 0u64;
    for (i, value) in pixels.iter().enumerate() {
        let index = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| {
                (i16::from(**candidate) - i16::from(*value)).unsigned_abs()
            })
            .map_or(0, |(index, _)| index) as u64;
        bits |= index << (i * 3);
    }
    let mut bytes = [0u8; 8];
    bytes[0] = hi;
    bytes[1] = lo;
    bytes[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
    bytes
}
fn block_pixels(img: &RgbaImage, x: u32, y: u32) -> [[u8; 4]; 16] {
    let mut pixels = [[0; 4]; 16];
    for row in 0..4u32 {
        for col in 0..4u32 {
            pixels[(row * 4 + col) as usize] = img
                .get_pixel(
                    (x + col).min(img.width() - 1),
                    (y + row).min(img.height() - 1),
                )
                .0;
        }
    }
    pixels
}
fn encode_mip(image: &RgbaImage, format: DdsFormat, header: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match format {
        DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3 | DdsFormat::Bc4 | DdsFormat::Bc5 => {
            for by in 0..image.height().div_ceil(4) {
                for bx in 0..image.width().div_ceil(4) {
                    let block = block_pixels(image, bx * 4, by * 4);
                    if format == DdsFormat::Bc2 {
                        let mut alpha = 0u64;
                        for (i, p) in block.iter().enumerate() {
                            alpha |=
                                u64::from(((u16::from(p[3]) * 15 + 127) / 255) as u8) << (4 * i);
                        }
                        out.extend_from_slice(&alpha.to_le_bytes());
                    }
                    if matches!(format, DdsFormat::Bc3 | DdsFormat::Bc4 | DdsFormat::Bc5) {
                        let alpha = std::array::from_fn(|i| {
                            block[i][if format == DdsFormat::Bc3 { 3 } else { 0 }]
                        });
                        out.extend_from_slice(&alpha_block(&alpha));
                    }
                    if format == DdsFormat::Bc5 {
                        let green = std::array::from_fn(|i| block[i][1]);
                        out.extend_from_slice(&alpha_block(&green));
                    }
                    if matches!(format, DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3) {
                        out.extend_from_slice(&color_block(
                            &block,
                            format == DdsFormat::Bc1 && block.iter().any(|p| p[3] < 128),
                        ));
                    }
                }
            }
        }
        DdsFormat::Rgba32 | DdsFormat::Rgb24 => {
            let depth = if format == DdsFormat::Rgba32 { 4 } else { 3 };
            let dxgi = header.get(84..88) == Some(&b"DX10"[..]);
            let masks = if dxgi {
                match read_u32(header, 128)? {
                    28 | 29 => [0xff, 0xff00, 0xff0000, 0xff000000],
                    87 | 91 => [0xff0000, 0xff00, 0xff, 0xff000000],
                    _ => return Err("DDS DX10 pixel layout not supported for PNG import".into()),
                }
            } else {
                [
                    read_u32(header, 92)?,
                    read_u32(header, 96)?,
                    read_u32(header, 100)?,
                    read_u32(header, 104)?,
                ]
            };
            for &mask in &masks[..depth] {
                if mask.count_ones() != 8 {
                    return Err("DDS channel masks are not 8-bit; cannot safely encode".into());
                }
            }
            for pixel in image.pixels() {
                let mut word = 0u32;
                for (channel, mask) in masks.iter().enumerate() {
                    if *mask != 0 {
                        word |= u32::from(pixel.0[channel]) << mask.trailing_zeros();
                    }
                }
                out.extend_from_slice(&word.to_le_bytes()[..depth]);
            }
        }
        DdsFormat::Alpha8 => out.extend(image.pixels().map(|p| p[3])),
        DdsFormat::Luminance8 => out.extend(image.pixels().map(|p| {
            ((u32::from(p[0]) * 54 + u32::from(p[1]) * 183 + u32::from(p[2]) * 19 + 128) / 256)
                as u8
        })),
        DdsFormat::Ctx1 => {
            return Err("CTX1 textures require a dedicated original-layout encoder".into());
        }
    }
    Ok(out)
}

/// Convert an external PNG into exactly the selected original DDS's format,
/// dimensions, mip count and header, only for complete linear 2D DDS files.
/// The temp output is consumed by the existing manifest/backup transaction.
pub(super) fn convert(png: &Path, target_dds: &Path, output: &Path) -> Result<(), String> {
    if !png
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Err("Choose a PNG image for DDS conversion".into());
    }
    let info = inspect_dds(target_dds).map_err(|e| format!("Original DDS is not complete: {e}"))?;
    if info.streaming_parts != 1 {
        return Err("This DDS uses multiple .dds.N streaming fragments. Import a separately encoded complete texture set; a single PNG cannot replace one fragment safely.".into());
    }
    let source = fs::read(target_dds).map_err(|e| e.to_string())?;
    if source.len() < 128 || &source[..4] != b"DDS " {
        return Err("Original DDS header is invalid".into());
    }
    // CryEngine 2012 console-tiled data cannot use the PC block encoder.
    if &source[124..128] == b"FYRC" || &source[124..128] == b"CRYF" {
        return Err("Console-tiled DDS texture requires a native-layout encoder".into());
    }
    let offset = if &source[84..88] == b"DX10" { 148 } else { 128 };
    if source.len() < offset {
        return Err("Original DDS header is incomplete".into());
    }
    let metadata = fs::symlink_metadata(png).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > 256 * 1024 * 1024 {
        return Err("PNG must be a regular image no larger than 256 MiB".into());
    }
    let reader = image::ImageReader::open(png)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if reader.format() != Some(image::ImageFormat::Png) {
        return Err("The selected image must actually contain PNG data".into());
    }
    let dimensions = reader.into_dimensions().map_err(|e| e.to_string())?;
    if dimensions != (info.width, info.height) {
        return Err(format!(
            "PNG dimensions {}x{} do not match original DDS {}x{}. Resize the image first.",
            dimensions.0, dimensions.1, info.width, info.height
        ));
    }
    let rgba = image::open(png)
        .map_err(|e| format!("Cannot decode PNG: {e}"))?
        .to_rgba8();
    let mut bytes = source[..offset].to_vec();
    for mip in 0..info.mipmaps {
        let width = (info.width >> mip).max(1);
        let height = (info.height >> mip).max(1);
        let level = if mip == 0 {
            rgba.clone()
        } else {
            image::imageops::resize(&rgba, width, height, FilterType::Triangle)
        };
        bytes.extend_from_slice(&encode_mip(&level, info.format, &source[..offset])?);
    }
    if bytes.len() != source.len() {
        return Err(format!(
            "Encoded DDS size mismatch ({} vs {}); refusing unsafe replacement",
            bytes.len(),
            source.len()
        ));
    }
    fs::write(output, bytes).map_err(|e| e.to_string())?;
    let result = inspect_dds(output).map_err(|e| format!("Encoded DDS failed validation: {e}"))?;
    if result.width != info.width
        || result.height != info.height
        || result.mipmaps != info.mipmaps
        || result.format != info.format
    {
        return Err("Encoded DDS differs from original GPU layout".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bc1_and_bc3_encode_valid_blocks() {
        let opaque = [[255, 0, 0, 255]; 16];
        let bc1 = color_block(&opaque, false);
        assert!(u16::from_le_bytes([bc1[0], bc1[1]]) > u16::from_le_bytes([bc1[2], bc1[3]]));
        let mut varied = opaque;
        varied[0][3] = 0;
        let transparent = color_block(&varied, true);
        assert_eq!(
            u32::from_le_bytes(transparent[4..].try_into().unwrap()) & 3,
            3
        );
        let alpha: [u8; 16] = std::array::from_fn(|i| (i * 17) as u8);
        assert_eq!(alpha_block(&alpha).len(), 8);
    }
    #[test]
    fn rejects_png_dimension_mismatch_and_preserves_original() {
        let dir = tempfile::tempdir().unwrap();
        let dds = dir.path().join("source.dds");
        let png = dir.path().join("replacement.png");
        let out = dir.path().join("output.dds");
        let mut bytes = vec![0u8; 128 + 8];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DXT1");
        fs::write(&dds, &bytes).unwrap();
        image::RgbaImage::new(2, 2).save(&png).unwrap();
        assert!(
            convert(&png, &dds, &out)
                .unwrap_err()
                .contains("dimensions")
        );
        assert_eq!(fs::read(dds).unwrap(), bytes);
        assert!(!out.exists());
    }
}
