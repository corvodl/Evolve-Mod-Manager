//! Read-only CryEngine Stage 2 source Heightmap.dat decoding.
//! Geometry is emitted in source world XY / Z-up coordinates; no geometry
//! is invented when markers, dimensions or sample boundaries are missing.
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub const MAX_TERRAIN_FILE: usize = 80 * 1024 * 1024;
pub const MAX_TERRAIN_DIM: u32 = 4096;
pub const MAX_RENDER_GRID: u32 = 257;
const SECTION: &[u8] = b"\x0eHeightmapDataW";
const NEXT: &[u8] = b"\x1bHeightmapLayerIdBitmap_ver2";

#[derive(Debug, thiserror::Error)]
pub enum TerrainError {
    #[error("Unsupported or truncated CryEngine Heightmap.dat layout: {0}")]
    Format(String),
    #[error("Terrain dimensions or output size exceed limits")]
    Limits,
    #[error("Output file already exists")]
    Exists,
    #[error("File access error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerrainInfo {
    pub width: u32,
    pub height: u32,
    pub unit: f32,
    pub max_height: f32,
    pub decoded_samples: usize,
    pub stride: u32,
}

#[derive(Clone, Debug)]
pub struct TerrainMesh {
    pub info: TerrainInfo,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

fn get_u32(bytes: &[u8], position: usize) -> Result<u32, TerrainError> {
    let chunk = bytes
        .get(position..position + 4)
        .ok_or_else(|| TerrainError::Format("missing u32".into()))?;
    Ok(u32::from_le_bytes(
        chunk.try_into().map_err(|_| TerrainError::Limits)?,
    ))
}

fn parse_metadata(xml: &[u8]) -> Result<(u32, u32, f32, f32), TerrainError> {
    if xml.windows(9).any(|p| p.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || xml.windows(8).any(|p| p.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(TerrainError::Format("unsafe XML entities".into()));
    }
    let mut reader = Reader::from_reader(xml);
    let mut result = None;
    loop {
        match reader
            .read_event()
            .map_err(|e| TerrainError::Format(e.to_string()))?
        {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"Heightmap" => {
                let mut params = [None, None, None, None];
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| TerrainError::Format(e.to_string()))?;
                    let key = attr.key.as_ref();
                    let index = [b"Width".as_slice(), b"Height", b"UnitSize", b"MaxHeight"]
                        .iter()
                        .position(|name| key == *name);
                    if let Some(i) = index {
                        let value = attr
                            .unescape_value()
                            .map_err(|e| TerrainError::Format(e.to_string()))?;
                        params[i] = Some(value.to_string());
                    }
                }
                let get = |i: usize| {
                    params[i]
                        .as_deref()
                        .ok_or_else(|| TerrainError::Format("missing terrain attribute".into()))
                };
                let w = get(0)?.parse::<u32>().map_err(|_| TerrainError::Limits)?;
                let h = get(1)?.parse::<u32>().map_err(|_| TerrainError::Limits)?;
                let unit = get(2)?.parse::<f32>().map_err(|_| TerrainError::Limits)?;
                let height = get(3)?.parse::<f32>().map_err(|_| TerrainError::Limits)?;
                if w < 16
                    || h < 16
                    || w > MAX_TERRAIN_DIM
                    || h > MAX_TERRAIN_DIM
                    || !w.is_power_of_two()
                    || !h.is_power_of_two()
                    || !unit.is_finite()
                    || !(0.0..=1000.0).contains(&unit)
                    || unit == 0.0
                    || !height.is_finite()
                    || !(0.0..=65536.0).contains(&height)
                    || height == 0.0
                {
                    return Err(TerrainError::Limits);
                }
                result = Some((w, h, unit, height));
                break;
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(TerrainError::Format("unsafe XML document".into()));
            }
            Event::Eof => break,
            _ => (),
        }
    }
    result.ok_or_else(|| TerrainError::Format("Heightmap root missing".into()))
}

fn parse_raw(source: &[u8]) -> Result<(TerrainInfo, Vec<u16>), TerrainError> {
    if !(24..=MAX_TERRAIN_FILE).contains(&source.len()) || source[0] != 0xff {
        return Err(TerrainError::Format("unsupported source header".into()));
    }
    let xml_length = usize::from(u16::from_le_bytes([source[1], source[2]]));
    if xml_length < 15 || xml_length + 3 >= source.len() {
        return Err(TerrainError::Format("truncated metadata header".into()));
    }
    let (width, height, unit, max_height) = parse_metadata(&source[3..3 + xml_length])?;
    let count = usize::try_from(width * height).map_err(|_| TerrainError::Limits)?;
    let bytes_len = count.checked_mul(2).ok_or(TerrainError::Limits)?;
    let section_at = 3 + xml_length;
    if source.get(section_at..section_at + 4 + SECTION.len())
        != Some([&4u32.to_le_bytes()[..], SECTION].concat().as_slice())
    {
        return Err(TerrainError::Format(
            "HeightmapData section marker missing".into(),
        ));
    }
    let header_at = section_at + 4 + SECTION.len();
    if get_u32(source, header_at)? as usize != bytes_len
        || get_u32(source, header_at + 4)? as usize != bytes_len
        || get_u32(source, header_at + 8)? != 0
    {
        return Err(TerrainError::Format(
            "height payload length mismatch".into(),
        ));
    }
    let data_at = header_at + 12;
    let end = data_at.checked_add(bytes_len).ok_or(TerrainError::Limits)?;
    if source.get(end..end + NEXT.len()) != Some(NEXT) {
        return Err(TerrainError::Format("missing successor section".into()));
    }
    let values = source[data_at..end]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|part| u16::from_le_bytes([part[0], part[1]]))
        .collect::<Vec<_>>();
    Ok((
        TerrainInfo {
            width,
            height,
            unit,
            max_height,
            decoded_samples: values.len(),
            stride: 1,
        },
        values,
    ))
}

fn positions(total: u32, stride: u32) -> Vec<u32> {
    let mut positions = (0..total).step_by(stride as usize).collect::<Vec<_>>();
    if positions.last().copied() != Some(total - 1) {
        positions.push(total - 1);
    }
    positions
}

pub fn decode_terrain_bytes(source: &[u8], render_grid: u32) -> Result<TerrainMesh, TerrainError> {
    let (mut info, samples) = parse_raw(source)?;
    if !(16..=MAX_RENDER_GRID).contains(&render_grid) {
        return Err(TerrainError::Limits);
    }
    let stride = (info.width.max(info.height) - 1)
        .div_ceil(render_grid - 1)
        .max(1);
    info.stride = stride;
    let xs = positions(info.width, stride);
    let ys = positions(info.height, stride);
    let count = xs.len() * ys.len();
    if count > (MAX_RENDER_GRID as usize + 2).pow(2) {
        return Err(TerrainError::Limits);
    }
    let at = |x: u32, y: u32| {
        // CryEngine stores world X as the first array dimension.
        let index = x as usize * info.height as usize + y as usize;
        f32::from(samples[index]) * info.max_height / 65535.0
    };
    let mut vertices = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut uv = Vec::with_capacity(count);
    for &x in &xs {
        for &y in &ys {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(info.width - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(info.height - 1);
            let dx = (at(xp, y) - at(xm, y)) / ((xp - xm) as f32 * info.unit);
            let dy = (at(x, yp) - at(x, ym)) / ((yp - ym) as f32 * info.unit);
            let mut n = [-dx, -dy, 1.0f32];
            let size = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
            for part in &mut n {
                *part /= size;
            }
            vertices.push([x as f32 * info.unit, y as f32 * info.unit, at(x, y)]);
            normals.push(n);
            uv.push([
                x as f32 / (info.width - 1) as f32,
                y as f32 / (info.height - 1) as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity((xs.len() - 1) * (ys.len() - 1) * 6);
    for x in 0..xs.len() - 1 {
        for y in 0..ys.len() - 1 {
            let a = (x * ys.len() + y) as u32;
            let b = a + ys.len() as u32;
            let c = a + 1;
            indices.extend_from_slice(&[a, b, c, b, b + 1, c]);
        }
    }
    Ok(TerrainMesh {
        info,
        vertices,
        normals,
        uv,
        indices,
    })
}

/// One authentic terrain paint layer entry from Heightmap.dat's XML header.
/// This is not the baked terrain diffuse: CryEngine stores its surface tiles
/// separately in TerrainTexture.xml and terraintexture.pak.
#[derive(Clone, Debug)]
pub struct TerrainPaintMap {
    pub width: u32,
    pub height: u32,
    pub layer_names: std::collections::BTreeMap<u8, String>,
    /// Exactly width * height bytes in the original layer bitmap order.
    pub ids: Vec<u8>,
}

impl TerrainPaintMap {
    pub fn id_at(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.ids
            .get(x as usize * self.height as usize + y as usize)
            .copied()
    }
}

/// Decode the optional native v2 layer-ID bitmap without altering the source.
/// Unknown trailing compiled terrain sections remain opaque. The bitmap must
/// be complete and uncompressed with the original verified section markers.
pub fn decode_terrain_paint_bytes(bytes: &[u8]) -> Result<Option<TerrainPaintMap>, TerrainError> {
    if !(24..=MAX_TERRAIN_FILE).contains(&bytes.len()) || bytes[0] != 0xff {
        return Err(TerrainError::Format("unsupported terrain header".into()));
    }
    let xml_len = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
    let xml_end = 3usize.checked_add(xml_len).ok_or(TerrainError::Limits)?;
    let xml = bytes
        .get(3..xml_end)
        .ok_or_else(|| TerrainError::Format("truncated terrain layer metadata".into()))?;
    let (width, height, _, _) = parse_metadata(xml)?;
    let height_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(2))
        .ok_or(TerrainError::Limits)?;
    let header = xml_end
        .checked_add(4 + SECTION.len())
        .ok_or(TerrainError::Limits)?;
    if bytes.get(xml_end..header) != Some([&4u32.to_le_bytes()[..], SECTION].concat().as_slice())
        || get_u32(bytes, header)? as usize != height_bytes
        || get_u32(bytes, header + 4)? as usize != height_bytes
        || get_u32(bytes, header + 8)? != 0
    {
        return Err(TerrainError::Format(
            "invalid original heightmap section".into(),
        ));
    }
    let next = header
        .checked_add(12)
        .and_then(|p| p.checked_add(height_bytes))
        .ok_or(TerrainError::Limits)?;
    if bytes.get(next..next + NEXT.len()) != Some(NEXT) {
        return Err(TerrainError::Format("terrain paint marker missing".into()));
    }
    let meta = next + NEXT.len();
    if meta == bytes.len() {
        return Ok(None);
    } // Old height-only exports
    let size = get_u32(bytes, meta)? as usize;
    if size != (width as usize * height as usize)
        || get_u32(bytes, meta + 4)? as usize != size
        || get_u32(bytes, meta + 8)? != 0
    {
        return Err(TerrainError::Format(
            "unexpected terrain layer bitmap layout".into(),
        ));
    }
    let start = meta + 12;
    let ids = bytes
        .get(start..start + size)
        .ok_or_else(|| TerrainError::Format("truncated terrain layer bitmap".into()))?
        .to_vec();
    let mut names = std::collections::BTreeMap::new();
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader
            .read_event()
            .map_err(|e| TerrainError::Format(e.to_string()))?
        {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"Layer" => {
                let mut id = None;
                let mut name = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| TerrainError::Format(e.to_string()))?;
                    if attr.key.as_ref() == b"LayerId" {
                        id = attr
                            .unescape_value()
                            .ok()
                            .and_then(|v| v.parse::<u8>().ok());
                    } else if attr.key.as_ref() == b"Name" {
                        name = attr.unescape_value().ok().map(|v| v.to_string());
                    }
                }
                if let (Some(id), Some(name)) = (id, name)
                    && name.len() <= 128
                {
                    names.insert(id, name);
                }
                if names.len() > 256 {
                    return Err(TerrainError::Limits);
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(TerrainError::Format("unsafe layer XML".into()));
            }
            Event::Eof => break,
            _ => (),
        }
    }
    Ok(Some(TerrainPaintMap {
        width,
        height,
        layer_names: names,
        ids,
    }))
}

/// Validated original CryEngine baked terrain texture package header.
/// The tile payload/quadtree still needs exact TerrainTexture.xml mapping:
/// this inspection intentionally cannot be mistaken for rendered terrain.
#[derive(Clone, Debug)]
pub struct TerrainTextureHeader {
    pub version: u16,
    pub flags: u16,
    pub color_multiplier: f32,
    pub layers: Vec<TerrainTextureLayer>,
}
#[derive(Clone, Debug)]
pub struct TerrainTextureLayer {
    pub sector_pixels: u16,
    pub texture_format: u32,
    pub sector_bytes: u32,
}
pub fn read_terrain_texture_header(path: &Path) -> Result<TerrainTextureHeader, TerrainError> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() < 24 || meta.len() > 2 * 1024 * 1024 * 1024 {
        return Err(TerrainError::Limits);
    }
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header)?;
    if header[..4] != *b"CRY\0" || header[4] != 100 {
        return Err(TerrainError::Format(
            "Not a CryEngine baked terrain surface texture header".into(),
        ));
    }
    let version = u16::from_le_bytes([header[6], header[7]]);
    if !matches!(version, 9 | 10) {
        return Err(TerrainError::Format(format!(
            "Unsupported terrain texture container version {version}"
        )));
    }
    let count = u16::from_le_bytes([header[8], header[9]]) as usize;
    let flags = u16::from_le_bytes([header[10], header[11]]);
    if !(1..=8).contains(&count) || flags & 2 != 0 {
        return Err(TerrainError::Format(
            "Unsupported layer count or big-endian terrain surface data".into(),
        ));
    }
    let color_multiplier = f32::from_le_bytes(header[12..16].try_into().unwrap());
    if !color_multiplier.is_finite() || !(0.0..=256.0).contains(&color_multiplier) {
        return Err(TerrainError::Limits);
    }
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let mut record = [0u8; 12];
        file.read_exact(&mut record)?;
        let sector_pixels = u16::from_le_bytes([record[0], record[1]]);
        let reserved = u16::from_le_bytes([record[2], record[3]]);
        let texture_format = u32::from_le_bytes(record[4..8].try_into().unwrap());
        let sector_bytes = u32::from_le_bytes(record[8..12].try_into().unwrap());
        if reserved != 0
            || sector_pixels < 4
            || sector_pixels > 8192
            || !sector_pixels.is_power_of_two()
            || sector_bytes == 0
            || sector_bytes > 64 * 1024 * 1024
        {
            return Err(TerrainError::Format(
                "Invalid baked terrain layer descriptor".into(),
            ));
        }
        layers.push(TerrainTextureLayer {
            sector_pixels,
            texture_format,
            sector_bytes,
        });
    }
    Ok(TerrainTextureHeader {
        version,
        flags,
        color_multiplier,
        layers,
    })
}

pub fn read_terrain_paint(path: &Path) -> Result<Option<TerrainPaintMap>, TerrainError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_TERRAIN_FILE as u64 {
        return Err(TerrainError::Limits);
    }
    decode_terrain_paint_bytes(&fs::read(path)?)
}

pub fn read_terrain(path: &Path, render_grid: u32) -> Result<TerrainMesh, TerrainError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_TERRAIN_FILE as u64 {
        return Err(TerrainError::Limits);
    }
    decode_terrain_bytes(&fs::read(path)?, render_grid)
}

/// Blender-compatible Wavefront OBJ with original XY, Z-up, UVs and normals.
/// A full map scene still requires CryEngine object geometry and materials.
pub fn export_terrain_obj(mesh: &TerrainMesh, destination: &Path) -> Result<(), TerrainError> {
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                TerrainError::Exists
            } else {
                TerrainError::Io(e)
            }
        })?;
    let written = (|| -> Result<(), TerrainError> {
        writeln!(output, "# Evolve source heightmap: world XY, Z-up")?;
        for xyz in &mesh.vertices {
            writeln!(output, "v {:.9} {:.9} {:.9}", xyz[0], xyz[1], xyz[2])?;
        }
        for uv in &mesh.uv {
            writeln!(output, "vt {:.9} {:.9}", uv[0], uv[1])?;
        }
        for normal in &mesh.normals {
            writeln!(
                output,
                "vn {:.9} {:.9} {:.9}",
                normal[0], normal[1], normal[2]
            )?;
        }
        for triangle in mesh.indices.as_chunks::<3>().0.iter() {
            let parts = triangle
                .iter()
                .map(|idx| {
                    let one = *idx + 1;
                    format!("{one}/{one}/{one}")
                })
                .collect::<Vec<_>>();
            writeln!(output, "f {} {} {}", parts[0], parts[1], parts[2])?;
        }
        output.sync_all()?;
        Ok(())
    })();
    if written.is_err() {
        drop(output);
        let _ = fs::remove_file(destination);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> Vec<u8> {
        let xml = br#"<Heightmap Width="16" Height="16" UnitSize="2" MaxHeight="100"/>"#;
        let mut data = vec![0xff];
        data.extend_from_slice(&(xml.len() as u16).to_le_bytes());
        data.extend_from_slice(xml);
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(SECTION);
        let samples = 16 * 16 * 2;
        data.extend_from_slice(&(samples as u32).to_le_bytes());
        data.extend_from_slice(&(samples as u32).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        for x in 0..16u16 {
            for y in 0..16u16 {
                data.extend_from_slice(&(x * 1500 + y * 400).to_le_bytes());
            }
        }
        data.extend_from_slice(NEXT);
        data
    }

    #[test]
    fn decodes_height_grid_with_world_axes_and_surface_normals() {
        let mesh = decode_terrain_bytes(&sample_data(), 16).unwrap();
        assert_eq!(mesh.info.decoded_samples, 256);
        assert_eq!(mesh.vertices.len(), 256);
        assert_eq!(mesh.indices.len(), 15 * 15 * 6);
        assert_eq!(mesh.vertices[1][0], 0.0);
        assert_eq!(mesh.vertices[1][1], 2.0);
        assert_eq!(mesh.vertices[16][0], 2.0);
        assert!((mesh.vertices[16][2] - 1500.0 * 100.0 / 65535.0).abs() < 0.01);
        assert!(mesh.normals.iter().all(|n| n[2] > 0.0 && n[2] <= 1.0));
    }

    #[test]
    fn rejects_missing_successor_and_invalid_dimensions() {
        let mut data = sample_data();
        data.truncate(data.len() - NEXT.len());
        assert!(decode_terrain_bytes(&data, 16).is_err());
        let mut data = sample_data();
        let idx = data
            .windows(10)
            .position(|part| part == b"Width=\"16\"")
            .unwrap();
        data[idx + 7] = b'3';
        assert!(decode_terrain_bytes(&data, 16).is_err());
    }

    #[test]
    fn decodes_original_terrain_layer_id_bitmap_without_guessing_colors() {
        let mut data = sample_data();
        let width = 16 * 16;
        data.extend_from_slice(&(width as u32).to_le_bytes());
        data.extend_from_slice(&(width as u32).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend((0..width).map(|i| (i % 3) as u8));
        let paint = decode_terrain_paint_bytes(&data).unwrap().unwrap();
        assert_eq!(paint.id_at(0, 0), Some(0));
        assert_eq!(paint.id_at(1, 0), Some(1));
        assert_eq!(paint.id_at(16, 0), None);
        assert!(paint.layer_names.is_empty());
        data[3
            + br#"<Heightmap Width="16" Height="16" UnitSize="2" MaxHeight="100"/>"#.len()
            + 4
            + SECTION.len()
            + 12
            + 16 * 16 * 2
            + NEXT.len()] = 0xff;
        assert!(decode_terrain_paint_bytes(&data).is_err());
    }

    #[test]
    fn validates_original_baked_terrain_texture_layer_descriptors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("terraintexture.pak");
        let mut data = Vec::new();
        data.extend_from_slice(b"CRY\0");
        data.extend_from_slice(&[100, 0]);
        data.extend_from_slice(&9u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&256u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&11u32.to_le_bytes());
        data.extend_from_slice(&65536u32.to_le_bytes());
        std::fs::write(&file, &data).unwrap();
        let info = read_terrain_texture_header(&file).unwrap();
        assert_eq!(info.version, 9);
        assert_eq!(info.layers.len(), 1);
        assert_eq!(info.layers[0].sector_bytes, 65536);
        data[4] = 42;
        std::fs::write(&file, &data).unwrap();
        assert!(read_terrain_texture_header(&file).is_err());
    }

    #[test]
    fn exports_non_overwriting_blender_obj() {
        let mesh = decode_terrain_bytes(&sample_data(), 16).unwrap();
        let folder = tempfile::tempdir().unwrap();
        let output = folder.path().join("terrain.obj");
        export_terrain_obj(&mesh, &output).unwrap();
        let text = fs::read_to_string(&output).unwrap();
        assert!(text.contains("v 0.000000000 2.000000000"));
        assert!(text.contains("\nf 1/1/1 17/17/17 2/2/2\n"));
        assert!(matches!(
            export_terrain_obj(&mesh, &output),
            Err(TerrainError::Exists)
        ));
    }
}
