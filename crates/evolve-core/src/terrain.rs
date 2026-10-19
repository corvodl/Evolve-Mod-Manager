//! Read-only CryEngine Stage 2 source Heightmap.dat decoding.
//! Geometry is emitted in source world XY / Z-up coordinates; no geometry
//! is invented when markers, dimensions or sample boundaries are missing.
use quick_xml::events::Event;
use quick_xml::Reader;
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
    let chunk = bytes.get(position..position + 4)
        .ok_or_else(|| TerrainError::Format("missing u32".into()))?;
    Ok(u32::from_le_bytes(chunk.try_into().map_err(|_| TerrainError::Limits)?))
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
        match reader.read_event().map_err(|e| TerrainError::Format(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"Heightmap" => {
                let mut params = [None, None, None, None];
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| TerrainError::Format(e.to_string()))?;
                    let key = attr.key.as_ref();
                    let index = [b"Width".as_slice(), b"Height", b"UnitSize", b"MaxHeight"]
                        .iter().position(|name| key == *name);
                    if let Some(i) = index {
                        let value = attr.unescape_value().map_err(|e| TerrainError::Format(e.to_string()))?;
                        params[i] = Some(value.to_string());
                    }
                }
                let get = |i: usize| params[i].as_deref()
                    .ok_or_else(|| TerrainError::Format("missing terrain attribute".into()));
                let w = get(0)?.parse::<u32>().map_err(|_| TerrainError::Limits)?;
                let h = get(1)?.parse::<u32>().map_err(|_| TerrainError::Limits)?;
                let unit = get(2)?.parse::<f32>().map_err(|_| TerrainError::Limits)?;
                let height = get(3)?.parse::<f32>().map_err(|_| TerrainError::Limits)?;
                if w < 16 || h < 16 || w > MAX_TERRAIN_DIM || h > MAX_TERRAIN_DIM
                    || !w.is_power_of_two() || !h.is_power_of_two()
                    || !unit.is_finite() || !(0.0..=1000.0).contains(&unit) || unit == 0.0
                    || !height.is_finite() || !(0.0..=65536.0).contains(&height) || height == 0.0
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
        return Err(TerrainError::Format("HeightmapData section marker missing".into()));
    }
    let header_at = section_at + 4 + SECTION.len();
    if get_u32(source, header_at)? as usize != bytes_len
        || get_u32(source, header_at + 4)? as usize != bytes_len
        || get_u32(source, header_at + 8)? != 0
    {
        return Err(TerrainError::Format("height payload length mismatch".into()));
    }
    let data_at = header_at + 12;
    let end = data_at.checked_add(bytes_len).ok_or(TerrainError::Limits)?;
    if source.get(end..end + NEXT.len()) != Some(NEXT) {
        return Err(TerrainError::Format("missing successor section".into()));
    }
    let values = source[data_at..end].as_chunks::<2>().0.iter()
        .map(|part| u16::from_le_bytes([part[0], part[1]])).collect::<Vec<_>>();
    Ok((TerrainInfo {
        width, height, unit, max_height,
        decoded_samples: values.len(), stride: 1,
    }, values))
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
    let stride = (info.width.max(info.height) - 1).div_ceil(render_grid - 1).max(1);
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
            uv.push([x as f32 / (info.width - 1) as f32, y as f32 / (info.height - 1) as f32]);
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
    Ok(TerrainMesh { info, vertices, normals, uv, indices })
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
    let mut output = OpenOptions::new().create_new(true).write(true).open(destination)
        .map_err(|e| if e.kind() == io::ErrorKind::AlreadyExists { TerrainError::Exists } else { TerrainError::Io(e) })?;
    let written = (|| -> Result<(), TerrainError> {
        writeln!(output, "# Evolve source heightmap: world XY, Z-up")?;
        for xyz in &mesh.vertices {
            writeln!(output, "v {:.9} {:.9} {:.9}", xyz[0], xyz[1], xyz[2])?;
        }
        for uv in &mesh.uv {
            writeln!(output, "vt {:.9} {:.9}", uv[0], uv[1])?;
        }
        for normal in &mesh.normals {
            writeln!(output, "vn {:.9} {:.9} {:.9}", normal[0], normal[1], normal[2])?;
        }
        for triangle in mesh.indices.as_chunks::<3>().0.iter() {
            let parts = triangle.iter().map(|idx| {
                let one = *idx + 1;
                format!("{one}/{one}/{one}")
            }).collect::<Vec<_>>();
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
        let idx = data.windows(10).position(|part| part == b"Width=\"16\"").unwrap();
        data[idx + 7] = b'3';
        assert!(decode_terrain_bytes(&data, 16).is_err());
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
        assert!(matches!(export_terrain_obj(&mesh, &output), Err(TerrainError::Exists)));
    }
}
