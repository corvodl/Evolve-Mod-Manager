//! Read-only legacy CryEngine Sandbox vegetation map decoder.
//!
//! Source format: CryEngine Sandbox CVegetationMap::SerializeInstances and
//! CNamedData::Serialize (Crytek/Lumberyard editor implementation). An editor
//! archive contains an MFC-style UTF-8 XML string followed by named blocks.
//! VegetationInstancesArray is packed 19/30/33-byte records for versions 1/2/3.
//! Reject unsupported or corrupted streams instead of guessing their layout.
use evolve_core::{LevelScene, PlacedObject, Transform};
use flate2::read::ZlibDecoder;
use quick_xml::{Reader, events::Event};
use std::{fs, io::Read, path::Path};

const MAX_FILE: usize = 192 * 1024 * 1024;
const MAX_XML: usize = 12 * 1024 * 1024;
const MAX_BINARY: usize = 128 * 1024 * 1024;
const MAX_NAMED: usize = 256;
const MAX_OBJECTS: usize = 4_096;
const MAX_INSTANCES: usize = 300_000;

#[derive(Clone, Debug)]
struct Prototype {
    path: String,
    category: String,
    hidden: bool,
}

#[derive(Clone, Debug)]
pub(super) struct VegetationInfo {
    pub version: u32,
    pub prototypes: usize,
    pub instances: usize,
    pub warnings: Vec<String>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(size)
            .ok_or("Vegetation size overflow")?;
        let out = self
            .bytes
            .get(self.pos..end)
            .ok_or("Truncated VegetationMap.dat")?;
        self.pos = end;
        Ok(out)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    // MFC conversion implemented by CryEngine's CArchive::readStringLength;
    // 0xff => u16, 0xffff => u32, 0xffffffff => u64.
    fn string(&mut self, limit: usize) -> Result<String, String> {
        let prefix = self.byte()?;
        let mut wide = false;
        let length = if prefix != 0xff {
            prefix as usize
        } else {
            let mut value = self.u16()?;
            if value == 0xfffe {
                wide = true;
                let next = self.byte()?;
                if next != 0xff {
                    next as usize
                } else {
                    value = self.u16()?;
                    self.extended(value)?
                }
            } else {
                self.extended(value)?
            }
        };
        let byte_len = length
            .checked_mul(if wide { 2 } else { 1 })
            .ok_or("Vegetation length overflow")?;
        if byte_len > limit {
            return Err("Vegetation archive string exceeded limit".into());
        }
        let bytes = self.take(byte_len)?;
        if wide {
            let vals = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            String::from_utf16(&vals).map_err(|e| e.to_string())
        } else {
            String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
        }
    }
    fn extended(&mut self, value: u16) -> Result<usize, String> {
        if value != 0xffff {
            return Ok(value as usize);
        }
        let n = self.u32()?;
        if n != u32::MAX {
            return Ok(n as usize);
        }
        let bytes = self.take(8)?;
        let n = u64::from_le_bytes(bytes.try_into().unwrap());
        usize::try_from(n).map_err(|_| "Vegetation string too long".into())
    }
}

fn safe_model(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    if normalized
        .split('/')
        .any(|v| v == "." || v == ".." || v.is_empty())
    {
        return None;
    }
    if ![".cgf", ".cga", ".cgfm", ".cgam"]
        .iter()
        .any(|ext| normalized.to_ascii_lowercase().ends_with(ext))
    {
        return None;
    }
    Some(normalized)
}

fn prototypes(xml: &[u8]) -> Result<(u32, Vec<Prototype>), String> {
    if xml.windows(9).any(|v| v.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || xml.windows(8).any(|v| v.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err("Vegetation XML DTD/entities are prohibited".into());
    }
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0usize;
    let mut in_objects: Option<usize> = None;
    let mut objects = Vec::new();
    let mut version = 1u32;
    let mut root = false;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let opening = matches!(&event, Event::Start(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if depth == 0 && name.eq_ignore_ascii_case(b"VegetationMap") {
                    root = true;
                    for attr in e.attributes() {
                        let a = attr.map_err(|e| e.to_string())?;
                        if a.key.as_ref().eq_ignore_ascii_case(b"Version") {
                            version = a
                                .unescape_value()
                                .map_err(|e| e.to_string())?
                                .parse()
                                .map_err(|_| "Invalid vegetation version")?;
                        }
                    }
                } else if depth == 1 && name.eq_ignore_ascii_case(b"Objects") {
                    in_objects = Some(depth + 1);
                } else if in_objects == Some(depth) && name.eq_ignore_ascii_case(b"Object") {
                    let mut model = String::new();
                    let mut category = String::new();
                    let mut hidden = false;
                    for attr in e.attributes() {
                        let a = attr.map_err(|e| e.to_string())?;
                        let value = a.unescape_value().map_err(|e| e.to_string())?.to_string();
                        if a.key.as_ref().eq_ignore_ascii_case(b"FileName") {
                            model = value
                        } else if a.key.as_ref().eq_ignore_ascii_case(b"Category") {
                            category = value
                        } else if a.key.as_ref().eq_ignore_ascii_case(b"Hidden") {
                            hidden = value == "1" || value.eq_ignore_ascii_case("true")
                        }
                    }
                    // Keep source indexing stable even when a prototype has an
                    // unsupported path; an instance must never attach to another
                    // prototype because an earlier one was filtered.
                    objects.push(Prototype {
                        path: safe_model(&model).unwrap_or_default(),
                        category,
                        hidden,
                    });
                    if objects.len() > MAX_OBJECTS {
                        return Err("Too many vegetation prototypes".into());
                    }
                }
                if opening {
                    depth += 1;
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if in_objects == Some(depth + 1)
                    && e.name().as_ref().eq_ignore_ascii_case(b"Objects")
                {
                    in_objects = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root || !(1..=3).contains(&version) {
        return Err("Unsupported VegetationMap XML version/root".into());
    }
    Ok((version, objects))
}

fn data_block(cursor: &mut Cursor<'_>) -> Result<Option<Vec<u8>>, String> {
    let count = cursor.u32()? as usize;
    if count > MAX_NAMED {
        return Err("Too many vegetation named blocks".into());
    }
    let mut found = None;
    for _ in 0..count {
        let key = cursor.string(512)?;
        let flags = cursor.u32()?;
        let compressed = (flags & 0x8000_0000) != 0;
        let encoded_len = (flags & 0x7fff_ffff) as usize;
        let original_len = cursor.u32()? as usize;
        let _extra = cursor.u32()?;
        if encoded_len > MAX_BINARY || original_len > MAX_BINARY {
            return Err("Vegetation named block exceeds size limit".into());
        }
        let raw = cursor.take(encoded_len)?;
        if key == "VegetationInstancesArray" {
            if found.is_some() {
                return Err("Duplicate vegetation instance stream".into());
            }
            let decoded = if compressed {
                let mut buf = Vec::new();
                ZlibDecoder::new(raw)
                    .take(original_len as u64 + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("Compressed vegetation stream: {e}"))?;
                if buf.len() != original_len {
                    return Err("Vegetation decompressed length mismatch".into());
                }
                buf
            } else {
                if encoded_len != original_len {
                    return Err("Vegetation length mismatch".into());
                }
                raw.to_vec()
            };
            found = Some(decoded);
        }
    }
    if cursor.pos != cursor.bytes.len() {
        return Err("Unexpected bytes after vegetation archive".into());
    }
    Ok(found)
}

fn quat_from_angles(x: f32, y: f32, z: f32) -> [f32; 4] {
    let (sx, cx) = (x * 0.5).sin_cos();
    let (sy, cy) = (y * 0.5).sin_cos();
    let (sz, cz) = (z * 0.5).sin_cos();
    [
        cx * cy * cz + sx * sy * sz,
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
    ]
}

pub(super) fn decode(bytes: &[u8], scene: &mut LevelScene) -> Result<VegetationInfo, String> {
    if bytes.len() > MAX_FILE {
        return Err("VegetationMap.dat exceeds file limit".into());
    }
    let mut cursor = Cursor::new(bytes);
    let xml = cursor.string(MAX_XML)?;
    let (version, models) = prototypes(xml.as_bytes())?;
    let Some(records) = data_block(&mut cursor)? else {
        return Ok(VegetationInfo {
            version,
            prototypes: models.len(),
            instances: 0,
            warnings: vec!["No vegetation instances block in this archive".into()],
        });
    };
    let stride = match version {
        1 => 19,
        2 => 30,
        3 => 33,
        _ => unreachable!(),
    };
    if records.len() % stride != 0 || records.len() / stride > MAX_INSTANCES {
        return Err("Vegetation instance stream has invalid stride/count".into());
    }
    let mut reader = Cursor::new(&records);
    let mut new_objects = Vec::new();
    let mut skipped = 0usize;
    for i in 0..(records.len() / stride) {
        let position = [reader.f32()?, reader.f32()?, reader.f32()?];
        let scale = reader.f32()?;
        let id = if version < 3 {
            reader.byte()? as usize
        } else {
            reader.u32()? as usize
        };
        let _brightness = reader.byte()?;
        let (rz, rx, ry) = if version == 1 {
            (
                reader.byte()? as f32 * std::f32::consts::TAU / 256.0,
                0.0,
                0.0,
            )
        } else {
            (reader.f32()?, reader.f32()?, reader.f32()?)
        };
        let Some(model) = models.get(id) else {
            skipped += 1;
            continue;
        };
        if model.hidden
            || model.path.is_empty()
            || !position.iter().all(|x| x.is_finite())
            || !scale.is_finite()
            || !(0.00001..=10000.0).contains(&scale)
            || ![rz, rx, ry].iter().all(|x| x.is_finite())
        {
            skipped += 1;
            continue;
        }
        let tf = Transform {
            position,
            scale: [scale; 3],
            quaternion: quat_from_angles(rx, ry, rz),
        };
        new_objects.push(PlacedObject {
            id: format!("__vegetation_{i}"),
            parent: String::new(),
            layer: "VegetationMap.dat".into(),
            name: format!("{} #{i}", model.category),
            kind: "Vegetation".into(),
            model: model.path.clone(),
            material: String::new(),
            hidden: false,
            local: tf,
            world: tf,
            status: "Original binary vegetation instance (read-only)".into(),
            populations: Vec::new(),
            home_area: String::new(),
            shape_points: Vec::new(),
            wildlife: evolve_core::scene::WildlifeMetadata::default(),
        });
    }
    let instances = new_objects.len();
    scene.objects.extend(new_objects);
    Ok(VegetationInfo {
        version,
        prototypes: models.len(),
        instances,
        warnings: if skipped > 0 {
            vec![format!(
                "{skipped} vegetation instances skipped (invalid/hidden/unsupported)"
            )]
        } else {
            vec![]
        },
    })
}

pub(super) fn inspect_file(path: &Path, scene: &mut LevelScene) -> Result<VegetationInfo, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE as u64 {
        return Err("Vegetation map must be a bounded regular file".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    decode(&bytes, scene)
}

/// Project-only mutation of the original packed instance array. The full
/// VegetationMap.dat envelope, prototype table, and unrelated named data
/// blocks are preserved. Import the resulting bytes through the verified
/// managed Modding project transaction, never directly into game/Asset Library.
#[derive(Clone, Debug)]
pub(super) enum VegetationEdit {
    Move { index: usize, position: [f32; 3] },
    Scale { index: usize, scale: f32 },
    Duplicate { index: usize, position: [f32; 3] },
    Delete { index: usize },
}

pub(super) fn edit_instances(bytes: &[u8], edit: VegetationEdit) -> Result<Vec<u8>, String> {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    if bytes.len() > MAX_FILE {
        return Err("VegetationMap.dat exceeds maximum source size".into());
    }
    let mut cursor = Cursor::new(bytes);
    let xml = cursor.string(MAX_XML)?;
    let (version, source_models) = prototypes(xml.as_bytes())?;
    let stride = match version {
        1 => 19,
        2 => 30,
        3 => 33,
        _ => return Err("Unsupported vegetation record version".into()),
    };
    let count = cursor.u32()? as usize;
    if count == 0 || count > MAX_NAMED {
        return Err("Unsupported number of vegetation data blocks".into());
    }
    let mut match_block = None;
    for _ in 0..count {
        let name = cursor.string(512)?;
        let flags_at = cursor.pos;
        let flags = cursor.u32()?;
        let packed_len = (flags & 0x7fff_ffff) as usize;
        let original_len = cursor.u32()? as usize;
        let _extra = cursor.u32()?;
        if packed_len > MAX_BINARY || original_len > MAX_BINARY {
            return Err("Vegetation named data size exceeds limits".into());
        }
        let data_at = cursor.pos;
        let original = cursor.take(packed_len)?;
        if name != "VegetationInstancesArray" {
            continue;
        }
        if match_block.is_some() {
            return Err("Duplicate vegetation instance data blocks".into());
        }
        let compressed = flags & 0x8000_0000 != 0;
        let mut records = if compressed {
            let mut decoded = Vec::new();
            ZlibDecoder::new(original)
                .take(original_len as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(|e| format!("Cannot decompress vegetation: {e}"))?;
            if decoded.len() != original_len {
                return Err("Incorrect decompressed vegetation size".into());
            }
            decoded
        } else {
            if packed_len != original_len {
                return Err("Original vegetation size mismatch".into());
            }
            original.to_vec()
        };
        if records.len() % stride != 0 || records.len() / stride > MAX_INSTANCES {
            return Err("Invalid vegetation record count or stride".into());
        }
        let count = records.len() / stride;
        let index = match &edit {
            VegetationEdit::Move { index, .. }
            | VegetationEdit::Scale { index, .. }
            | VegetationEdit::Duplicate { index, .. }
            | VegetationEdit::Delete { index } => *index,
        };
        if index >= count {
            return Err("Selected vegetation record not found".into());
        }
        let start = index * stride;
        let (prototype, scale) = if version < 3 {
            (
                records[start + 16] as usize,
                f32::from_le_bytes(records[start + 12..start + 16].try_into().unwrap()),
            )
        } else {
            (
                u32::from_le_bytes(records[start + 16..start + 20].try_into().unwrap()) as usize,
                f32::from_le_bytes(records[start + 12..start + 16].try_into().unwrap()),
            )
        };
        if prototype >= source_models.len()
            || !scale.is_finite()
            || !(0.00001..=10000.0).contains(&scale)
        {
            return Err("Vegetation prototype or scale is invalid".into());
        }
        match &edit {
            VegetationEdit::Move { position, .. } => {
                if !position.iter().all(|p| p.is_finite() && p.abs() <= 1e8) {
                    return Err("Invalid vegetation position".into());
                }
                for (axis, value) in position.iter().enumerate() {
                    records[start + axis * 4..start + axis * 4 + 4]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
            VegetationEdit::Scale { scale, .. } => {
                if !scale.is_finite() || !(0.00001..=10000.0).contains(scale) {
                    return Err("Invalid vegetation scale".into());
                }
                records[start + 12..start + 16].copy_from_slice(&scale.to_le_bytes());
            }
            VegetationEdit::Duplicate { position, .. } => {
                if count >= MAX_INSTANCES
                    || !position.iter().all(|p| p.is_finite() && p.abs() <= 1e8)
                {
                    return Err("Unsafe vegetation copy count or position".into());
                }
                let mut duplicate = records[start..start + stride].to_vec();
                for (axis, value) in position.iter().enumerate() {
                    duplicate[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
                }
                records.extend_from_slice(&duplicate);
            }
            VegetationEdit::Delete { .. } => {
                records.drain(start..start + stride);
            }
        }
        if records.len() > MAX_BINARY {
            return Err("Edited vegetation data exceeds size limit".into());
        }
        let packed = if compressed {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&records).map_err(|e| e.to_string())?;
            encoder.finish().map_err(|e| e.to_string())?
        } else {
            records.clone()
        };
        if packed.len() > MAX_BINARY || packed.len() > 0x7fff_ffff {
            return Err("Edited packed vegetation data exceeds size limit".into());
        }
        let new_flags = (if compressed { 0x8000_0000 } else { 0 }) | (packed.len() as u32);
        let mut output = Vec::with_capacity(bytes.len().saturating_add(stride));
        output.extend_from_slice(&bytes[..flags_at]);
        output.extend_from_slice(&new_flags.to_le_bytes());
        output.extend_from_slice(&(records.len() as u32).to_le_bytes());
        output.extend_from_slice(&bytes[flags_at + 8..data_at]); // Preserve source 'extra' word.
        output.extend_from_slice(&packed);
        output.extend_from_slice(&bytes[data_at + packed_len..]);
        if output.len() > MAX_FILE {
            return Err("Edited VegetationMap.dat exceeds file maximum".into());
        }
        match_block = Some(output);
    }
    if cursor.pos != bytes.len() {
        return Err("Extra bytes after VegetationMap.dat blocks".into());
    }
    let output = match_block.ok_or("Missing original VegetationInstancesArray")?;
    let mut parsed = LevelScene {
        schema: 1,
        objects: Vec::new(),
        diagnostics: Vec::new(),
    };
    let _ = decode(&output, &mut parsed)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive(xml: &str, instances: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(xml.len() as u8);
        out.extend_from_slice(xml.as_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        let key = b"VegetationInstancesArray";
        out.push(key.len() as u8);
        out.extend_from_slice(key);
        out.extend_from_slice(&(instances.len() as u32).to_le_bytes());
        out.extend_from_slice(&(instances.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(instances);
        out
    }
    fn blank() -> LevelScene {
        LevelScene {
            schema: 1,
            objects: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
    #[test]
    fn project_vegetation_record_mutation_round_trips_and_preserves_prototypes() {
        let xml = r#"<VegetationMap Version="3"><Objects><Object FileName="Objects/trees/tree.cgf" Category="Forest"/></Objects></VegetationMap>"#;
        let mut record = Vec::new();
        for value in [10.0f32, 20.0, 30.0, 1.25] {
            record.extend_from_slice(&value.to_le_bytes());
        }
        record.extend_from_slice(&0u32.to_le_bytes());
        record.push(255);
        for angle in [0.5f32, 0.0, 0.0] {
            record.extend_from_slice(&angle.to_le_bytes());
        }
        let source = archive(xml, &record);
        let moved = edit_instances(
            &source,
            VegetationEdit::Move {
                index: 0,
                position: [50.0, 20.0, 30.0],
            },
        )
        .unwrap();
        let mut scene = blank();
        assert_eq!(decode(&moved, &mut scene).unwrap().instances, 1);
        assert_eq!(scene.objects[0].world.position, [50.0, 20.0, 30.0]);
        let copied = edit_instances(
            &moved,
            VegetationEdit::Duplicate {
                index: 0,
                position: [60.0, 20.0, 30.0],
            },
        )
        .unwrap();
        assert_eq!(decode(&copied, &mut blank()).unwrap().instances, 2);
        let removed = edit_instances(&copied, VegetationEdit::Delete { index: 0 }).unwrap();
        let mut scene = blank();
        assert_eq!(decode(&removed, &mut scene).unwrap().instances, 1);
        assert_eq!(scene.objects[0].world.position, [60.0, 20.0, 30.0]);
        assert!(edit_instances(&source, VegetationEdit::Delete { index: 2 }).is_err());
    }

    #[test]
    fn version_three_decodes_exact_packed_fields() {
        let xml = r#"<VegetationMap Version="3"><Objects><Object FileName="Objects/trees/tree.cgf" Category="Forest"/></Objects></VegetationMap>"#;
        let mut record = Vec::new();
        for f in [10.0f32, 20.0, 30.0, 1.25] {
            record.extend_from_slice(&f.to_le_bytes());
        }
        record.extend_from_slice(&0u32.to_le_bytes());
        record.push(255);
        for f in [0.5f32, 0.0, 0.0] {
            record.extend_from_slice(&f.to_le_bytes());
        }
        assert_eq!(record.len(), 33);
        let mut scene = blank();
        let info = decode(&archive(xml, &record), &mut scene).unwrap();
        assert_eq!(info.instances, 1);
        assert_eq!(scene.objects[0].world.position, [10.0, 20.0, 30.0]);
        assert_eq!(scene.objects[0].world.scale, [1.25; 3]);
        assert_eq!(scene.objects[0].model, "Objects/trees/tree.cgf");
    }
    #[test]
    fn broken_or_ambiguous_streams_are_not_accepted() {
        let xml = r#"<VegetationMap Version="3"><Objects><Object FileName="../wrong.cgf"/></Objects></VegetationMap>"#;
        assert!(decode(&archive(xml, &[0u8; 34]), &mut blank()).is_err());
        let mut rec = [0u8; 33];
        rec[12..16].copy_from_slice(&1f32.to_le_bytes());
        let mut scene = blank();
        let summary = decode(&archive(xml, &rec), &mut scene).unwrap();
        assert_eq!(summary.instances, 0);
    }
    #[test]
    fn version_one_has_nineteen_byte_records() {
        let xml = r#"<VegetationMap Version="1"><Objects><Object FileName="Objects/grass.cgf"/></Objects></VegetationMap>"#;
        let mut row = Vec::new();
        for f in [0.0f32, 1.0, 2.0, 1.0] {
            row.extend_from_slice(&f.to_le_bytes());
        }
        row.extend_from_slice(&[0, 255, 64]);
        let mut scene = blank();
        assert_eq!(
            decode(&archive(xml, &row), &mut scene).unwrap().instances,
            1
        );
    }
}
