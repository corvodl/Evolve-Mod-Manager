//! CryEngine .lyr / mission XML placed-object scene graph and reversible,
//! non-overwriting transform editing. Source XML is never edited in place.
//! Terrain geometry is handled separately in terrain.rs.
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const MAX_LAYER_BYTES: usize = 48 * 1024 * 1024;
const MAX_OBJECTS: usize = 200_000;
const MAX_DEPTH: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("Malformed or unsupported level placement XML: {0}")]
    Xml(String),
    #[error("Level scene contains unsafe or excessive placements")]
    Limits,
    #[error("No exactly matching GUID exists for the requested edit")]
    NoMatchingGuid,
    #[error("Duplicate level object GUID: {0}")]
    DuplicateGuid(String),
    #[error("Cyclic level object parent hierarchy: {0}")]
    Cycle(String),
    #[error("Output already exists")]
    Exists,
    #[error("File access error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: [f32; 3],
    /// CryEngine source order: w,x,y,z.
    pub quaternion: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            quaternion: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0; 3],
        }
    }
}
impl Transform {
    fn normalize(mut self) -> Result<Self, SceneError> {
        if self
            .position
            .iter()
            .chain(self.quaternion.iter())
            .chain(self.scale.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e8)
        {
            return Err(SceneError::Limits);
        }
        let norm = self.quaternion.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm < 1e-8 {
            self.quaternion = [1.0, 0.0, 0.0, 0.0];
        } else {
            for c in &mut self.quaternion {
                *c /= norm;
            }
        }
        Ok(self)
    }
    fn combine(self, other: Self) -> Result<Self, SceneError> {
        let scaled = [
            other.position[0] * self.scale[0],
            other.position[1] * self.scale[1],
            other.position[2] * self.scale[2],
        ];
        let rotated = rotate(self.quaternion, scaled);
        let world = Self {
            position: [
                self.position[0] + rotated[0],
                self.position[1] + rotated[1],
                self.position[2] + rotated[2],
            ],
            quaternion: mult(self.quaternion, other.quaternion),
            scale: [
                self.scale[0] * other.scale[0],
                self.scale[1] * other.scale[1],
                self.scale[2] * other.scale[2],
            ],
        };
        world.normalize()
    }
}
fn mult(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [w, x, y, z] = a;
    let [p, q, r, s] = b;
    [
        w * p - x * q - y * r - z * s,
        w * q + x * p + y * s - z * r,
        w * r - x * s + y * p + z * q,
        w * s + x * r - y * q + z * p,
    ]
}
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let pure = [0., v[0], v[1], v[2]];
    let applied = mult(mult(q, pure), [q[0], -q[1], -q[2], -q[3]]);
    [applied[1], applied[2], applied[3]]
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlacedObject {
    pub id: String,
    pub parent: String,
    pub layer: String,
    pub name: String,
    pub kind: String,
    pub model: String,
    pub material: String,
    pub hidden: bool,
    pub local: Transform,
    pub world: Transform,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelScene {
    pub schema: u32,
    pub objects: Vec<PlacedObject>,
    pub diagnostics: Vec<String>,
}
fn is_placement(tag: &[u8]) -> bool {
    let short = tag
        .rsplit(|b| *b == b':' || *b == b'}')
        .next()
        .unwrap_or(tag);
    [b"Object".as_slice(), b"Entity", b"Brush", b"Prefab"]
        .iter()
        .any(|name| short.eq_ignore_ascii_case(name))
}
fn attr(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>, SceneError> {
    for a in e.attributes() {
        let a = a.map_err(|e| SceneError::Xml(e.to_string()))?;
        if a.key.as_ref().eq_ignore_ascii_case(key) {
            return a
                .unescape_value()
                .map(|v| Some(v.into_owned()))
                .map_err(|e| SceneError::Xml(e.to_string()));
        }
    }
    Ok(None)
}
// Sound/reverb volumes contain <Entities><Entity Id="..."/></Entities>
// links. They refer to placements elsewhere; they are not new instances.
// Require both the reference context and an ID-only record so genuine nested
// entity definitions with authored properties continue to be inspected.
fn is_entity_reference(e: &BytesStart<'_>, parents: &[Vec<u8>]) -> Result<bool, SceneError> {
    if !e.name().as_ref().eq_ignore_ascii_case(b"Entity")
        || !parents
            .last()
            .is_some_and(|p| p.eq_ignore_ascii_case(b"Entities"))
        || !parents.iter().any(|p| is_placement(p))
    {
        return Ok(false);
    }
    let mut has_id = false;
    for attribute in e.attributes() {
        let attribute = attribute.map_err(|e| SceneError::Xml(e.to_string()))?;
        if attribute.key.as_ref().eq_ignore_ascii_case(b"Id")
            || attribute.key.as_ref().eq_ignore_ascii_case(b"EntityId")
        {
            has_id = true;
        } else {
            return Ok(false);
        }
    }
    Ok(has_id)
}
fn vector<const N: usize>(raw: Option<String>, default: [f32; N]) -> [f32; N] {
    let Some(raw) = raw else {
        return default;
    };
    let parts = raw
        .split(',')
        .map(|v| v.trim().parse::<f32>())
        .collect::<Vec<_>>();
    if parts.len() != N || parts.iter().any(|v| v.is_err()) {
        return default;
    }
    let mut out = default;
    for (dst, value) in out.iter_mut().zip(parts) {
        let Ok(n) = value else { return default };
        *dst = n;
    }
    if out.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
        default
    } else {
        out
    }
}
fn renderable(value: &str) -> bool {
    let v = value.trim().replace('\\', "/").to_ascii_lowercase();
    [
        ".cgf", ".cga", ".cdf", ".chr", ".skin", ".skinm", ".chrm", ".cgam", ".cgfm",
    ]
    .iter()
    .any(|ext| v.ends_with(ext))
}
fn parse_element(
    e: &BytesStart<'_>,
    layer: &str,
    idx: usize,
) -> Result<Option<PlacedObject>, SceneError> {
    if !is_placement(e.name().as_ref()) {
        return Ok(None);
    }
    let id = attr(e, b"Id")?
        .or(attr(e, b"EntityId")?)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{layer}#{idx}"))
        .to_ascii_lowercase();
    let parent = attr(e, b"Parent")?
        .or(attr(e, b"Group")?)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut model = String::new();
    for key in [b"Prefab".as_slice(), b"Geometry", b"Model", b"FileName"] {
        if let Some(value) = attr(e, key)?
            && renderable(&value)
        {
            model = value.replace('\\', "/");
            break;
        }
    }
    let local = Transform {
        position: vector(attr(e, b"Pos")?, [0., 0., 0.]),
        quaternion: vector(attr(e, b"Rotate")?, [1., 0., 0., 0.]),
        scale: vector(attr(e, b"Scale")?, [1., 1., 1.]),
    }
    .normalize()?;
    let kind = std::str::from_utf8(e.name().as_ref())
        .map_err(|_| SceneError::Xml("invalid tag".into()))?
        .to_owned();
    Ok(Some(PlacedObject {
        id: id.clone(),
        parent,
        layer: layer.into(),
        name: attr(e, b"Name")?.unwrap_or(id),
        kind,
        model,
        material: attr(e, b"Material")?.unwrap_or_default(),
        hidden: attr(e, b"Hidden")?.as_deref() == Some("1"),
        local,
        world: local,
        status: String::new(),
    }))
}
fn scan_placements(xml: &[u8], layer: &str) -> Result<Vec<PlacedObject>, SceneError> {
    if xml.is_empty()
        || xml.len() > MAX_LAYER_BYTES
        || xml.windows(9).any(|v| v.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || xml.windows(8).any(|v| v.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(SceneError::Limits);
    }
    let mut reader = Reader::from_reader(xml);
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut parents = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| SceneError::Xml(e.to_string()))?
        {
            Event::Start(e) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(SceneError::Limits);
                }
                if !is_entity_reference(&e, &parents)?
                    && let Some(obj) = parse_element(&e, layer, objects.len())?
                {
                    objects.push(obj);
                }
                parents.push(e.name().as_ref().to_vec());
            }
            Event::Empty(e) => {
                if !is_entity_reference(&e, &parents)?
                    && let Some(obj) = parse_element(&e, layer, objects.len())?
                {
                    objects.push(obj);
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return Err(SceneError::Xml("extra closing tag".into()));
                }
                depth -= 1;
                parents.pop();
            }
            Event::DocType(_) | Event::GeneralRef(_) => return Err(SceneError::Limits),
            Event::Eof => break,
            _ => (),
        }
        if objects.len() > MAX_OBJECTS {
            return Err(SceneError::Limits);
        }
    }
    if depth != 0 {
        return Err(SceneError::Xml("truncated object layer".into()));
    }
    Ok(objects)
}
/// Parse multiple original sources with stable GUIDs; duplicate GUIDs or cycles
/// are errors rather than silent transforms on the wrong character/map object.
pub fn parse_level_scene(layers: &[(&str, &[u8])]) -> Result<LevelScene, SceneError> {
    if layers.is_empty() || layers.len() > 1024 {
        return Err(SceneError::Limits);
    }
    let mut objects = Vec::new();
    let mut seen = HashSet::new();
    for (layer, data) in layers {
        let decoded = crate::cryxml::xml_bytes(data, MAX_LAYER_BYTES)
            .map_err(|e| SceneError::Xml(e.to_string()))?;
        for obj in scan_placements(&decoded, layer)? {
            if !seen.insert(obj.id.clone()) {
                return Err(SceneError::DuplicateGuid(obj.id));
            }
            objects.push(obj);
            if objects.len() > MAX_OBJECTS {
                return Err(SceneError::Limits);
            }
        }
    }
    let index = objects
        .iter()
        .enumerate()
        .map(|(i, obj)| (obj.id.clone(), i))
        .collect::<HashMap<_, _>>();
    let mut world: Vec<Option<Transform>> = vec![None; objects.len()];
    let mut diagnostic = Vec::new();
    for start in 0..objects.len() {
        if world[start].is_some() {
            continue;
        }
        let mut order = Vec::new();
        let mut visiting = HashSet::new();
        let mut current = Some(start);
        while let Some(idx) = current {
            if world[idx].is_some() {
                break;
            }
            if !visiting.insert(idx) {
                return Err(SceneError::Cycle(objects[idx].id.clone()));
            }
            order.push(idx);
            if order.len() > MAX_OBJECTS {
                return Err(SceneError::Limits);
            }
            current = index.get(&objects[idx].parent).copied();
        }
        while let Some(idx) = order.pop() {
            let parent = index.get(&objects[idx].parent).copied();
            let base = parent.and_then(|p| world[p]).unwrap_or_default();
            world[idx] = Some(base.combine(objects[idx].local)?);
        }
    }
    for (i, obj) in objects.iter_mut().enumerate() {
        obj.world = world[i].unwrap_or_default();
        obj.status = if obj.hidden {
            "hidden in editor".into()
        } else if !obj.parent.is_empty() && !index.contains_key(&obj.parent) {
            diagnostic.push(format!("{} refers to absent parent {}", obj.id, obj.parent));
            "unresolved parent GUID".into()
        } else if obj.model.is_empty() {
            "no explicit renderable mesh reference".into()
        } else {
            "model reference available".into()
        };
    }
    Ok(LevelScene {
        schema: 1,
        objects,
        diagnostics: diagnostic,
    })
}

pub fn load_level_scene(path: &Path) -> Result<LevelScene, SceneError> {
    let stat = fs::symlink_metadata(path)?;
    if !stat.file_type().is_file() || stat.len() > MAX_LAYER_BYTES as u64 {
        return Err(SceneError::Limits);
    }
    let data = fs::read(path)?;
    parse_level_scene(&[(
        path.file_name().and_then(|n| n.to_str()).unwrap_or("layer"),
        &data,
    )])
}

/// Creates a modified COPY of one XML layer with a single exact GUID's
/// transforms changed. No mission/LYR file is altered in place.
pub fn export_edited_layer(
    source: &Path,
    target: &Path,
    guid: &str,
    transform: Transform,
) -> Result<(), SceneError> {
    let valid = transform.normalize()?;
    if guid.is_empty() {
        return Err(SceneError::NoMatchingGuid);
    }
    let bytes = fs::read(source)?;
    let bytes = crate::cryxml::xml_bytes(&bytes, MAX_LAYER_BYTES)
        .map_err(|e| SceneError::Xml(e.to_string()))?;
    if bytes.len() > MAX_LAYER_BYTES {
        return Err(SceneError::Limits);
    }
    let parsed = parse_level_scene(&[(
        source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("layer"),
        &bytes,
    )])?;
    if parsed
        .objects
        .iter()
        .filter(|o| o.id.eq_ignore_ascii_case(guid))
        .count()
        != 1
    {
        return Err(SceneError::NoMatchingGuid);
    }
    let mut reader = Reader::from_reader(bytes.as_ref());
    let mut writer = Writer::new(Vec::with_capacity(bytes.len() + 128));
    let mut edited = 0usize;
    let mut parents = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| SceneError::Xml(e.to_string()))?;
        if matches!(event, Event::Eof) {
            break;
        }
        let reference = match &event {
            Event::Start(e) | Event::Empty(e) => is_entity_reference(e, &parents)?,
            _ => false,
        };
        match &event {
            Event::Start(e) => parents.push(e.name().as_ref().to_vec()),
            Event::End(_) => {
                parents.pop();
            }
            _ => (),
        }
        let adjusted = match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if !reference && is_placement(e.name().as_ref()) =>
            {
                let found = attr(e, b"Id")?
                    .or(attr(e, b"EntityId")?)
                    .unwrap_or_default();
                if found.eq_ignore_ascii_case(guid) {
                    edited += 1;
                    let mut attrs = Vec::new();
                    for a in e.attributes() {
                        let a = a.map_err(|e| SceneError::Xml(e.to_string()))?;
                        let key = std::str::from_utf8(a.key.as_ref())
                            .map_err(|_| SceneError::Limits)?
                            .to_owned();
                        let value = a
                            .unescape_value()
                            .map_err(|e| SceneError::Xml(e.to_string()))?
                            .into_owned();
                        if !matches!(key.as_str(), "Pos" | "Rotate" | "Scale") {
                            attrs.push((key, value));
                        }
                    }
                    attrs.extend([
                        (
                            "Pos".into(),
                            valid
                                .position
                                .iter()
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(","),
                        ),
                        (
                            "Rotate".into(),
                            valid
                                .quaternion
                                .iter()
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(","),
                        ),
                        (
                            "Scale".into(),
                            valid
                                .scale
                                .iter()
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(","),
                        ),
                    ]);
                    let tag = std::str::from_utf8(e.name().as_ref())
                        .map_err(|_| SceneError::Limits)?
                        .to_owned();
                    let mut replacement = BytesStart::new(tag);
                    for (name, value) in &attrs {
                        replacement.push_attribute((name.as_str(), value.as_str()));
                    }
                    if matches!(event, Event::Start(_)) {
                        Event::Start(replacement.into_owned())
                    } else {
                        Event::Empty(replacement.into_owned())
                    }
                } else {
                    event.into_owned()
                }
            }
            other => other.into_owned(),
        };
        writer
            .write_event(adjusted)
            .map_err(|e| SceneError::Xml(e.to_string()))?;
    }
    if edited != 1 {
        return Err(SceneError::NoMatchingGuid);
    }
    let new_bytes = writer.into_inner();
    let _ = parse_level_scene(&[("modified", &new_bytes)])?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                SceneError::Exists
            } else {
                SceneError::Io(e)
            }
        })?;
    if let Err(e) = output.write_all(&new_bytes).and_then(|_| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(target);
        return Err(SceneError::Io(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_links_are_not_placements_and_are_not_edited() {
        let xml = br#"<Objects><Object Id="a" Name="reverb" Pos="1,2,3"/>
            <Object Id="b"><Entities><Entity Id="a"/><Entity Id="a"></Entity></Entities></Object>
            <Entity Id="c"/></Objects>"#;
        let scene = parse_level_scene(&[("sound.lyr", xml)]).unwrap();
        assert_eq!(scene.objects.len(), 3);
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("sound.lyr");
        let output = temp.path().join("edited.lyr");
        fs::write(&source, xml).unwrap();
        export_edited_layer(
            &source,
            &output,
            "a",
            Transform {
                position: [7., 8., 9.],
                ..Default::default()
            },
        )
        .unwrap();
        let edited = fs::read_to_string(&output).unwrap();
        assert!(edited.contains("<Entity Id=\"a\"/>"));
        assert!(edited.contains("<Entity Id=\"a\"></Entity>"));
        assert_eq!(
            load_level_scene(&output).unwrap().objects[0].local.position,
            [7., 8., 9.]
        );
        assert_eq!(fs::read(&source).unwrap(), xml);
    }
    #[test]
    fn calculates_nested_world_transform_and_refuses_cycles() {
        let xml=br#"<Objects>
          <Object Id="A" Name="Parent" Pos="5,6,7" Rotate="1,0,0,0" Scale="2,2,2" Geometry="Objects/tree.cgf"/>
          <Entity Id="B" Parent="A" Pos="1,2,3" FileName="Objects/prop.cgf"/>
        </Objects>"#;
        let scene = parse_level_scene(&[("level.lyr", xml)]).unwrap();
        assert_eq!(scene.objects.len(), 2);
        assert_eq!(scene.objects[1].world.position, [7., 10., 13.]);
        let cycle = br#"<Objects><Object Id="a" Parent="b"/><Object Id="b" Parent="a"/></Objects>"#;
        assert!(matches!(
            parse_level_scene(&[("bad.lyr", cycle)]),
            Err(SceneError::Cycle(_))
        ));
    }
    #[test]
    fn refuses_duplicate_ids_and_untrusted_xml() {
        let xml = br#"<Objects><Object Id="A"/><Object Id="a"/></Objects>"#;
        assert!(matches!(
            parse_level_scene(&[("a.lyr", xml)]),
            Err(SceneError::DuplicateGuid(_))
        ));
        assert!(parse_level_scene(&[("x", b"<!DOCTYPE x><Objects/>")]).is_err());
    }
    #[test]
    fn exports_transform_only_to_new_layer() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("mission_mission0.xml");
        let target = temp.path().join("edited_mission.xml");
        fs::write(
            &source,
            br#"<Objects><Object Id="E4" Pos="0,0,0" Geometry="Objects/rock.cgf"/></Objects>"#,
        )
        .unwrap();
        let transform = Transform {
            position: [2., 3., 4.],
            ..Transform::default()
        };
        export_edited_layer(&source, &target, "e4", transform).unwrap();
        let scene = load_level_scene(&target).unwrap();
        assert_eq!(scene.objects[0].world.position, [2., 3., 4.]);
        assert_eq!(
            load_level_scene(&source).unwrap().objects[0].world.position,
            [0., 0., 0.]
        );
        assert!(matches!(
            export_edited_layer(&source, &target, "e4", transform),
            Err(SceneError::Exists)
        ));
    }
}
