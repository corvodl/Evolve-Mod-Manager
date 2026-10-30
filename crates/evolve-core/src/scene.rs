//! CryEngine .lyr / mission XML placed-object scene graph and reversible,
//! non-overwriting transform editing. Source XML is never edited in place.
//! Terrain geometry is handled separately in terrain.rs.
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;

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
    #[error("Export destination is inside a protected original asset or library directory")]
    UnsafeDestination,
    #[error("Original level source changed; reload it before exporting")]
    SourceChanged,
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
    resolve_placements(objects, &HashSet::new())
}

/// Read-only recovery is deliberately separate from the strict editing
/// parser. Identical cross-layer GUID duplicates are harmless replicated
/// declarations; conflicting duplicates retain only the first source-order
/// placement, and no parent is ever resolved *through* an ambiguous GUID.
/// All unique placements are then transformed against the combined cross-layer
/// hierarchy, unlike per-layer parsing which loses valid parent transforms.
#[derive(Debug)]
pub struct ReadOnlySceneRecovery {
    pub scene: LevelScene,
    pub duplicate_declarations: usize,
    pub conflicting_guids: usize,
}

pub fn parse_level_scene_read_only(layers: &[(&str, &[u8])]) -> Result<ReadOnlySceneRecovery, SceneError> {
    if layers.is_empty() || layers.len() > 1024 {
        return Err(SceneError::Limits);
    }
    let mut objects = Vec::<PlacedObject>::new();
    let mut seen = HashMap::<String, usize>::new();
    let mut ambiguous = HashSet::<String>::new();
    let mut duplicates = 0usize;
    for (layer, data) in layers {
        let decoded = crate::cryxml::xml_bytes(data, MAX_LAYER_BYTES)
            .map_err(|e| SceneError::Xml(e.to_string()))?;
        for obj in scan_placements(&decoded, layer)? {
            if let Some(previous) = seen.get(&obj.id).copied() {
                duplicates += 1;
                let prior = &objects[previous];
                if prior.parent != obj.parent
                    || prior.name != obj.name
                    || prior.kind != obj.kind
                    || prior.model != obj.model
                    || prior.material != obj.material
                    || prior.hidden != obj.hidden
                    || prior.local != obj.local
                {
                    ambiguous.insert(obj.id);
                }
            } else {
                seen.insert(obj.id.clone(), objects.len());
                objects.push(obj);
                if objects.len() > MAX_OBJECTS {
                    return Err(SceneError::Limits);
                }
            }
        }
    }
    let mut scene = resolve_placements(objects, &ambiguous)?;
    if duplicates > 0 {
        scene.diagnostics.push(format!(
            "Read-only recovery skipped {duplicates} duplicate GUID declarations ({} conflicting IDs).              Cross-layer parent transforms were recovered for uniquely identified parents;              ambiguous parents remain unresolved. Original layer XML was not modified.",
            ambiguous.len()
        ));
    }
    Ok(ReadOnlySceneRecovery {
        scene,
        duplicate_declarations: duplicates,
        conflicting_guids: ambiguous.len(),
    })
}

fn resolve_placements(
    mut objects: Vec<PlacedObject>,
    ambiguous: &HashSet<String>,
) -> Result<LevelScene, SceneError> {
    let index = objects
        .iter()
        .enumerate()
        .filter(|(_, obj)| !ambiguous.contains(&obj.id))
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

/// Apply one authored local TRS edit. Treat the selected node and its
/// descendants as dirty; cached world transforms outside that subtree are
/// left untouched. The UI applies on explicit confirmation, not each keypress.
///
/// Constructing the child adjacency list and the returned scene clone is
/// O(scene size), but world-matrix composition is O(affected subtree size),
/// so a small edit does not repeatedly recompute a huge sibling hierarchy.
pub fn edit_level_transform(
    scene: &LevelScene,
    guid: &str,
    transform: Transform,
) -> Result<LevelScene, SceneError> {
    use std::collections::VecDeque;

    let normalized = transform.normalize()?;
    if guid.trim().is_empty() || scene.objects.len() > MAX_OBJECTS {
        return Err(SceneError::NoMatchingGuid);
    }
    let mut edited = scene.clone();
    let mut index = HashMap::with_capacity(edited.objects.len());
    for (i, object) in edited.objects.iter().enumerate() {
        if index.insert(object.id.to_ascii_lowercase(), i).is_some() {
            return Err(SceneError::DuplicateGuid(object.id.clone()));
        }
    }
    let selected = index
        .get(&guid.to_ascii_lowercase())
        .copied()
        .ok_or(SceneError::NoMatchingGuid)?;
    let mut children = vec![Vec::new(); edited.objects.len()];
    for (i, object) in edited.objects.iter().enumerate() {
        if let Some(&parent) = index.get(&object.parent.to_ascii_lowercase()) {
            children[parent].push(i);
        }
    }
    edited.objects[selected].local = normalized;
    // A validated scene already has cached, correct ancestor world matrices.
    let mut dirty = VecDeque::from([selected]);
    let mut visited = HashSet::new();
    while let Some(i) = dirty.pop_front() {
        if !visited.insert(i) {
            return Err(SceneError::Cycle(edited.objects[i].id.clone()));
        }
        let parent_world = index
            .get(&edited.objects[i].parent.to_ascii_lowercase())
            .map(|&parent| edited.objects[parent].world)
            .unwrap_or_default();
        edited.objects[i].world = parent_world.combine(edited.objects[i].local)?;
        dirty.extend(children[i].iter().copied());
        if dirty.len() > MAX_OBJECTS {
            return Err(SceneError::Limits);
        }
    }
    Ok(edited)
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
pub fn export_edited_layers(
    source: &Path,
    target: &Path,
    edits: &[(String, Transform)],
) -> Result<(), SceneError> {
    if edits.is_empty() || edits.len() > 128 {
        return Err(SceneError::Limits);
    }
    let mut requested = HashMap::new();
    for (guid, transform) in edits {
        if guid.is_empty()
            || requested
                .insert(guid.to_ascii_lowercase(), transform.normalize()?)
                .is_some()
        {
            return Err(SceneError::NoMatchingGuid);
        }
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
    if requested.keys().any(|guid| {
        parsed
            .objects
            .iter()
            .filter(|object| &object.id == guid)
            .count()
            != 1
    }) {
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
                if let Some(valid) = requested.get(&found.to_ascii_lowercase()) {
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
    if edited != requested.len() {
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

/// Reject aliases and symlinked paths into original assets, including
/// symlinked parent directories and a destination that is already occupied.
/// Re-resolve on both sides of serialization; never use an output-relative
/// path or infer safety from the spelling of a directory name.
fn verified_output_path(
    source: &Path,
    target: &Path,
    protected_roots: &[PathBuf],
) -> Result<PathBuf, SceneError> {
    if !target.is_absolute()
        || !target
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lyr") || ext.eq_ignore_ascii_case("xml"))
    {
        return Err(SceneError::UnsafeDestination);
    }
    let file = target.file_name().ok_or(SceneError::UnsafeDestination)?;
    let canonical_parent = fs::canonicalize(target.parent().ok_or(SceneError::UnsafeDestination)?)?;
    if !fs::metadata(&canonical_parent)?.is_dir() {
        return Err(SceneError::UnsafeDestination);
    }
    let full = canonical_parent.join(file);
    // symlink_metadata catches dangling symbolic links that Path::exists skips.
    match fs::symlink_metadata(&full) {
        Ok(_) => return Err(SceneError::Exists),
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => return Err(SceneError::Io(err)),
    }
    let source = fs::canonicalize(source)?;
    let source_parent = source.parent().ok_or(SceneError::UnsafeDestination)?;
    if canonical_parent.starts_with(source_parent) {
        return Err(SceneError::UnsafeDestination);
    }
    for root in protected_roots {
        let canonical = fs::canonicalize(root)?;
        if !fs::metadata(&canonical)?.is_dir() || canonical_parent.starts_with(&canonical) {
            return Err(SceneError::UnsafeDestination);
        }
    }
    Ok(full)
}

fn original_layer_digest(path: &Path) -> Result<String, SceneError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_LAYER_BYTES as u64 {
        return Err(SceneError::Limits);
    }
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Stage an edited layer outside every protected original directory and only
/// publish a complete, validated new file after source SHA-256 verification.
/// A duplicate GUID, source modification, or target collision never causes
/// an existing file to be deleted or replaced.
pub fn export_edited_layers_guarded(
    source: &Path,
    target: &Path,
    edits: &[(String, Transform)],
    protected_roots: &[PathBuf],
    expected_source_sha256: &str,
) -> Result<(), SceneError> {
    if expected_source_sha256.len() != 64
        || !expected_source_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(SceneError::SourceChanged);
    }
    let output = verified_output_path(source, target, protected_roots)?;
    if original_layer_digest(source)? != expected_source_sha256 {
        return Err(SceneError::SourceChanged);
    }
    let parent = output.parent().ok_or(SceneError::UnsafeDestination)?;
    // Create staging only in a directory that has already passed the guarded
    // canonical destination check. The temporary workspace is removed on error.
    let workspace = tempfile::Builder::new()
        .prefix(".evolve-level-edit-")
        .tempdir_in(parent)?;
    let staged = workspace.path().join("edited.lyr");
    export_edited_layers(source, &staged, edits)?;
    if original_layer_digest(source)? != expected_source_sha256 {
        return Err(SceneError::SourceChanged);
    }
    // Validate paths again before the no-clobber publish, in case any user
    // preferences or symlink targets changed while the XML was serialized.
    let output = verified_output_path(source, target, protected_roots)?;
    let mut staged_file = fs::File::open(&staged)?;
    let mut publishing = tempfile::Builder::new()
        .prefix(".evolve-level-final-")
        .tempfile_in(output.parent().ok_or(SceneError::UnsafeDestination)?)?;
    io::copy(&mut staged_file, publishing.as_file_mut())?;
    publishing.as_file_mut().sync_all()?;
    if original_layer_digest(source)? != expected_source_sha256 {
        return Err(SceneError::SourceChanged);
    }
    let output = verified_output_path(source, target, protected_roots)?;
    publishing.persist_noclobber(&output).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            SceneError::Exists
        } else {
            SceneError::Io(error.error)
        }
    })?;
    Ok(())
}

/// Compatibility convenience API for a single verified GUID edit.
pub fn export_edited_layer(
    source: &Path,
    target: &Path,
    guid: &str,
    transform: Transform,
) -> Result<(), SceneError> {
    export_edited_layers(source, target, &[(guid.to_owned(), transform)])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undoable_local_edit_propagates_to_child_world_transform() {
        let src = br#"<Objects><Object Id="parent" Pos="1,2,3"/><Object Id="child" Parent="parent" Pos="4,5,6"/></Objects>"#;
        let original = parse_level_scene(&[("example.lyr", src)]).unwrap();
        let changed = edit_level_transform(
            &original,
            "parent",
            Transform {
                position: [11.0, 12.0, 13.0],
                ..Transform::default()
            },
        )
        .unwrap();
        assert_eq!(changed.objects[0].world.position, [11.0, 12.0, 13.0]);
        assert_eq!(changed.objects[1].world.position, [15.0, 17.0, 19.0]);
        assert_eq!(original.objects[1].world.position, [5.0, 7.0, 9.0]);
        let restored = edit_level_transform(&changed, "parent", original.objects[0].local).unwrap();
        assert_eq!(
            restored.objects[1].world.position,
            original.objects[1].world.position
        );
    }

    #[test]
    fn editing_a_leaf_preserves_other_world_transforms_and_authoring_order() {
        let data = br#"<Objects>
            <Object Id="left" Parent="root" Pos="1,0,0"/>
            <Object Id="leaf" Parent="left" Pos="2,0,0"/>
            <Object Id="right" Parent="root" Pos="0,10,0"/>
            <Object Id="root" Pos="4,5,6"/>
            <Object Id="unrelated" Pos="9,9,9"/>
        </Objects>"#;
        let original = parse_level_scene(&[("test.lyr", data)]).unwrap();
        let changed = edit_level_transform(
            &original,
            "left",
            Transform {
                position: [7., 0., 0.],
                ..Transform::default()
            },
        )
        .unwrap();
        assert_eq!(changed.objects[0].world.position, [11., 5., 6.]);
        assert_eq!(changed.objects[1].world.position, [13., 5., 6.]);
        for i in [2, 3, 4] {
            assert_eq!(changed.objects[i].world, original.objects[i].world);
            assert_eq!(changed.objects[i].local, original.objects[i].local);
        }
        assert_eq!(original.objects[0].local.position, [1., 0., 0.]);
        let restored = edit_level_transform(&changed, "left", original.objects[0].local).unwrap();
        assert_eq!(restored.objects[1].world, original.objects[1].world);
    }

    #[test]
    fn large_sibling_hierarchy_recomputes_only_the_affected_branch() {
        let mut xml = String::from("<Objects><Object Id=\"root\"/>");
        for i in 0..5000 {
            xml.push_str(&format!(
                "<Object Id=\"sibling{i}\" Parent=\"root\" Pos=\"1,0,0\"/>"
            ));
        }
        xml.push_str("<Object Id=\"leaf\" Parent=\"sibling1\" Pos=\"2,0,0\"/></Objects>");
        let initial = parse_level_scene(&[("large.lyr", xml.as_bytes())]).unwrap();
        let leaf = initial.objects.iter().position(|o| o.id == "leaf").unwrap();
        let changed = edit_level_transform(
            &initial,
            "sibling1",
            Transform {
                position: [5., 0., 0.],
                ..Transform::default()
            },
        )
        .unwrap();
        assert_eq!(changed.objects[leaf].world.position, [7., 0., 0.]);
        assert_eq!(changed.objects[2500].world, initial.objects[2500].world);
    }

    #[test]
    fn batch_export_edits_only_exact_guids_and_preserves_original() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.lyr");
        let dest = dir.path().join("modified.lyr");
        let bytes = br#"<Objects><Object Id="p" Name="Parent"/><Object Id="c" Parent="p" Pos="1,0,0"/></Objects>"#;
        fs::write(&source, bytes).unwrap();
        let changed = Transform {
            position: [3.0, 4.0, 5.0],
            ..Transform::default()
        };
        export_edited_layers(
            &source,
            &dest,
            &[("p".into(), changed), ("c".into(), changed)],
        )
        .unwrap();
        let result = load_level_scene(&dest).unwrap();
        assert_eq!(result.objects[0].local.position, changed.position);
        assert_eq!(result.objects[1].local.position, changed.position);
        assert_eq!(result.objects[1].world.position, [6.0, 8.0, 10.0]);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert!(export_edited_layers(&source, &dest, &[("p".into(), changed)]).is_err());
        assert!(
            export_edited_layers(
                &source,
                &dir.path().join("bad.lyr"),
                &[("missing".into(), changed)]
            )
            .is_err()
        );
    }

    #[test]
    fn guarded_export_refuses_source_folder_library_alias_and_occupied_destination() {
        let home = tempfile::tempdir().unwrap();
        let extracted = home.path().join("original-assets");
        let library = home.path().join("asset-library");
        let output = home.path().join("exports");
        fs::create_dir_all(&extracted).unwrap();
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&output).unwrap();
        let source = extracted.join("source.lyr");
        let bytes = br#"<Objects><Object Id="a" Pos="1,0,0"/></Objects>"#;
        fs::write(&source, bytes).unwrap();
        let expected = original_layer_digest(&source).unwrap();
        let edit = vec![(
            "a".into(),
            Transform {
                position: [4., 0., 0.],
                ..Transform::default()
            },
        )];
        let roots = vec![extracted.clone(), library.clone()];
        assert!(matches!(
            export_edited_layers_guarded(
                &source,
                &library.join("inside.lyr"),
                &edit,
                &roots,
                &expected,
            ),
            Err(SceneError::UnsafeDestination)
        ));
        assert!(matches!(
            export_edited_layers_guarded(
                &source,
                &extracted.join("inside.lyr"),
                &edit,
                &roots,
                &expected,
            ),
            Err(SceneError::UnsafeDestination)
        ));
        let target = output.join("modified.lyr");
        fs::write(&target, b"already here").unwrap();
        assert!(matches!(
            export_edited_layers_guarded(&source, &target, &edit, &roots, &expected),
            Err(SceneError::Exists)
        ));
        assert_eq!(fs::read(&target).unwrap(), b"already here");
        fs::remove_file(&target).unwrap();
        export_edited_layers_guarded(&source, &target, &edit, &roots, &expected).unwrap();
        assert_eq!(
            load_level_scene(&target).unwrap().objects[0].local.position,
            [4., 0., 0.]
        );
        assert_eq!(fs::read(source).unwrap(), bytes);
    }

    #[test]
    fn guarded_export_detects_changed_input_and_duplicate_guid_without_output() {
        let home = tempfile::tempdir().unwrap();
        let source_dir = home.path().join("original");
        let output_dir = home.path().join("output");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&output_dir).unwrap();
        let source = source_dir.join("layer.lyr");
        fs::write(&source, br#"<Objects><Object Id="a"/></Objects>"#).unwrap();
        let sha = original_layer_digest(&source).unwrap();
        let dest = output_dir.join("edited.lyr");
        let edit = vec![("a".into(), Transform::default())];
        fs::write(
            &source,
            br#"<Objects><Object Id="a" Pos="8,9,10"/></Objects>"#,
        )
        .unwrap();
        assert!(matches!(
            export_edited_layers_guarded(&source, &dest, &edit, &[], &sha),
            Err(SceneError::SourceChanged)
        ));
        assert!(!dest.exists());
        fs::write(
            &source,
            br#"<Objects><Object Id="a"/><Object Id="A"/></Objects>"#,
        )
        .unwrap();
        let duplicate_sha = original_layer_digest(&source).unwrap();
        assert!(matches!(
            export_edited_layers_guarded(&source, &dest, &edit, &[], &duplicate_sha),
            Err(SceneError::DuplicateGuid(_))
        ));
        assert!(!dest.exists());
    }

    #[cfg(unix)]
    #[test]
    fn guarded_export_rejects_symlink_into_asset_library() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let source_dir = home.path().join("src");
        let library = home.path().join("library");
        let alias = home.path().join("alias");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&library).unwrap();
        symlink(&library, &alias).unwrap();
        let src = source_dir.join("original.lyr");
        fs::write(&src, br#"<Objects><Object Id="a"/></Objects>"#).unwrap();
        let sha = original_layer_digest(&src).unwrap();
        let edited = vec![("a".into(), Transform::default())];
        assert!(matches!(
            export_edited_layers_guarded(
                &src,
                &alias.join("edited.lyr"),
                &edited,
                &[library],
                &sha
            ),
            Err(SceneError::UnsafeDestination)
        ));
    }

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
    fn read_only_duplicate_layer_recovery_keeps_cross_layer_parents() {
        let root = br#"<Objects><Object Id="root" Name="Root" Pos="5,0,0" Geometry="Objects/root.cgf"/></Objects>"#;
        let repeated = br#"<Objects><Object Id="root" Name="Root" Pos="5,0,0" Geometry="Objects/root.cgf"/><Object Id="child" Parent="root" Pos="1,0,0" Geometry="Objects/child.cgf"/></Objects>"#;
        assert!(matches!(parse_level_scene(&[("a.lyr", root), ("b.lyr", repeated)]),
            Err(SceneError::DuplicateGuid(_))));
        let result = parse_level_scene_read_only(&[("a.lyr", root), ("b.lyr", repeated)]).unwrap();
        assert_eq!(result.duplicate_declarations, 1);
        assert_eq!(result.conflicting_guids, 0);
        assert_eq!(result.scene.objects.len(), 2);
        let child = result.scene.objects.iter().find(|obj| obj.id == "child").unwrap();
        assert_eq!(child.world.position, [6.0, 0.0, 0.0]);
        assert_eq!(child.parent, "root");
    }

    #[test]
    fn conflicting_guid_never_becomes_a_trusted_parent_in_read_only_preview() {
        let root = br#"<Objects><Object Id="root" Pos="5,0,0"/></Objects>"#;
        let conflict = br#"<Objects><Object Id="root" Pos="100,0,0"/><Object Id="child" Parent="root" Pos="1,0,0"/></Objects>"#;
        let result = parse_level_scene_read_only(&[("a.lyr", root), ("b.lyr", conflict)]).unwrap();
        assert_eq!(result.conflicting_guids, 1);
        assert_eq!(result.duplicate_declarations, 1);
        let child = result.scene.objects.iter().find(|obj| obj.id == "child").unwrap();
        assert_eq!(child.world.position, [1.0, 0.0, 0.0]);
        assert_eq!(child.status, "unresolved parent GUID");
        assert_eq!(result.scene.objects.len(), 2);
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
