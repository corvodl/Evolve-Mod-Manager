//! Source-preserving, bounded edits for managed Modding project .lyr/XML files.
//! The caller must use the verified project import transaction. This writer
//! never opens game PAKs or imports from the read-only Asset Library.
use crate::scene::{PlacedObject, SceneError, Transform, edit_level_transform, parse_level_scene};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::collections::HashSet;

const MAX_BYTES: usize = 48 * 1024 * 1024;
const MAX_EVENTS: usize = 500_000;

#[derive(Clone, Debug)]
pub struct SpawnPropertiesEdit {
    pub populations: [String; 4],
    pub home_area: String,
    pub spawn_weight: f32,
    pub enabled: bool,
    pub spawn_on_round_start: bool,
    pub initial_health_percent: f32,
    pub group: String,
}

#[derive(Clone, Debug)]
pub enum LayerAuthoringEdit {
    /// Insert an authored geometry placement adjacent to a selected object.
    Insert {
        anchor_guid: String,
        new_guid: String,
        name: String,
        model: String,
        transform: Transform,
    },
    /// Persist one local transform to its actual owning project XML layer.
    Transform { guid: String, transform: Transform },
    /// Duplicate an existing XML placement, including its original authored
    /// nested properties, into the same layer. Never duplicate nested objects.
    Duplicate {
        guid: String,
        new_guid: String,
        name: String,
        /// The copy's local position; None places it 2 m along X.
        position: Option<[f32; 3]>,
    },
    /// Remove one object only when it has no scene descendants or home areas.
    Delete { guid: String },
    /// Modify only the known source-authored CreatureSpawn fields.
    Spawn {
        guid: String,
        settings: SpawnPropertiesEdit,
    },
    /// Modify source-authored ZooKeeperShape vertices in their original order.
    /// Count stays constant to avoid inventing engine-specific area topology.
    Shape { guid: String, points: Vec<[f32; 3]> },
}

impl LayerAuthoringEdit {
    fn target(&self) -> &str {
        match self {
            Self::Insert { anchor_guid, .. } => anchor_guid,
            Self::Transform { guid, .. }
            | Self::Duplicate { guid, .. }
            | Self::Delete { guid }
            | Self::Spawn { guid, .. }
            | Self::Shape { guid, .. } => guid,
        }
    }
}

fn authored_display_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" _-.()#".contains(&b))
}

fn bounded_guid(s: &str) -> bool {
    let raw = s
        .strip_prefix('{')
        .and_then(|id| id.strip_suffix('}'))
        .unwrap_or(s);
    bounded_name(raw)
}

fn bounded_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:/".contains(&b))
}

fn value(event: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>, SceneError> {
    for attr in event.attributes() {
        let attr = attr.map_err(|error| SceneError::Xml(error.to_string()))?;
        if attr.key.as_ref().eq_ignore_ascii_case(key) {
            return Ok(Some(
                attr.unescape_value()
                    .map_err(|error| SceneError::Xml(error.to_string()))?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

fn patched(
    event: &BytesStart<'_>,
    values: &[(&str, String)],
) -> Result<BytesStart<'static>, SceneError> {
    let name = std::str::from_utf8(event.name().as_ref())
        .map_err(|_| SceneError::Limits)?
        .to_owned();
    let mut out = BytesStart::new(name);
    let mut used = HashSet::new();
    for attr in event.attributes() {
        let attr = attr.map_err(|error| SceneError::Xml(error.to_string()))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|_| SceneError::Limits)?
            .to_owned();
        if let Some((_, replacement)) = values.iter().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            out.push_attribute((key.as_str(), replacement.as_str()));
            used.insert(key.to_ascii_lowercase());
        } else {
            // Preserve the raw, already escaped XML attribute value exactly.
            out.push_attribute((key.as_bytes(), attr.value.as_ref()));
        }
    }
    for (key, replacement) in values {
        if !used.contains(&key.to_ascii_lowercase()) {
            out.push_attribute((*key, replacement.as_str()));
        }
    }
    Ok(out.into_owned())
}

fn replace_start(
    event: &Event<'static>,
    fields: &[(&str, String)],
) -> Result<Event<'static>, SceneError> {
    Ok(match event {
        Event::Start(e) => Event::Start(patched(e, fields)?),
        Event::Empty(e) => Event::Empty(patched(e, fields)?),
        _ => return Err(SceneError::Limits),
    })
}

fn tag(event: &Event<'_>, expected: &[u8]) -> bool {
    match event {
        Event::Start(e) | Event::Empty(e) => e.name().as_ref().eq_ignore_ascii_case(expected),
        Event::End(e) => e.name().as_ref().eq_ignore_ascii_case(expected),
        _ => false,
    }
}

fn opening(event: &Event<'_>) -> bool {
    matches!(event, Event::Start(_) | Event::Empty(_))
}

fn placement_tag(event: &Event<'_>) -> bool {
    [b"Object".as_slice(), b"Entity", b"Brush", b"Prefab"]
        .iter()
        .any(|name| tag(event, name))
}

fn parse_events(xml: &[u8]) -> Result<Vec<Event<'static>>, SceneError> {
    if xml.len() > MAX_BYTES
        || xml.windows(9).any(|s| s.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || xml.windows(8).any(|s| s.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(SceneError::Limits);
    }
    let mut reader = Reader::from_reader(xml);
    let mut events = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| SceneError::Xml(e.to_string()))?;
        let end = matches!(event, Event::Eof);
        if !end {
            events.push(event.into_owned());
        }
        if events.len() > MAX_EVENTS {
            return Err(SceneError::Limits);
        }
        if end {
            break;
        }
    }
    Ok(events)
}

/// CryEngine's <Entities><Entity Id="..."/> children are links to
/// existing placed entities, not fresh placements. Exclude these exact
/// ID-only references when matching an authored entity for mutation.
fn entity_references(events: &[Event<'static>]) -> Result<HashSet<usize>, SceneError> {
    let mut parents: Vec<Vec<u8>> = Vec::new();
    let mut links = HashSet::new();
    for (index, event) in events.iter().enumerate() {
        if let Event::Start(element) | Event::Empty(element) = event
            && element.name().as_ref().eq_ignore_ascii_case(b"Entity")
            && parents
                .last()
                .is_some_and(|name| name.eq_ignore_ascii_case(b"Entities"))
            && parents.iter().any(|name| {
                [b"Object".as_slice(), b"Entity", b"Brush", b"Prefab"]
                    .iter()
                    .any(|tag| name.eq_ignore_ascii_case(tag))
            })
        {
            let mut has_id = false;
            let mut id_only = true;
            for attr in element.attributes() {
                let attr = attr.map_err(|e| SceneError::Xml(e.to_string()))?;
                if attr.key.as_ref().eq_ignore_ascii_case(b"Id")
                    || attr.key.as_ref().eq_ignore_ascii_case(b"EntityId")
                {
                    has_id = true;
                } else {
                    id_only = false;
                }
            }
            if has_id && id_only {
                links.insert(index);
            }
        }
        match event {
            Event::Start(element) => parents.push(element.name().as_ref().to_vec()),
            Event::End(_) => {
                parents.pop();
            }
            _ => {}
        }
    }
    Ok(links)
}

fn find_object(events: &[Event<'static>], guid: &str) -> Result<(usize, usize), SceneError> {
    let links = entity_references(events)?;
    let mut match_range = None;
    for (i, event) in events.iter().enumerate() {
        if links.contains(&i) {
            continue;
        }
        let start = match event {
            Event::Start(e) | Event::Empty(e) if placement_tag(event) => e,
            _ => continue,
        };
        let found = value(start, b"Id")?.or(value(start, b"EntityId")?);
        if !found.is_some_and(|id| id.eq_ignore_ascii_case(guid)) {
            continue;
        }
        if match_range.is_some() {
            return Err(SceneError::DuplicateGuid(guid.to_owned()));
        }
        if matches!(event, Event::Empty(_)) {
            match_range = Some((i, i));
            continue;
        }
        let mut depth = 1usize;
        for (j, item) in events.iter().enumerate().skip(i + 1) {
            if matches!(item, Event::Start(_)) {
                depth += 1;
            } else if matches!(item, Event::End(_)) {
                depth -= 1;
                if depth == 0 {
                    match_range = Some((i, j));
                    break;
                }
            }
        }
        if match_range.is_none() {
            return Err(SceneError::Limits);
        }
    }
    match_range.ok_or(SceneError::NoMatchingGuid)
}

fn bool_value(b: bool) -> String {
    if b { "1" } else { "0" }.into()
}

fn transform_target(
    content: &mut [Event<'static>],
    edit: &LayerAuthoringEdit,
    object: &PlacedObject,
) -> Result<(), SceneError> {
    match edit {
        LayerAuthoringEdit::Spawn { settings, .. } => {
            if !object.kind.eq_ignore_ascii_case("CreatureSpawn")
                || !settings.spawn_weight.is_finite()
                || !(0.0..=1000.0).contains(&settings.spawn_weight)
                || !settings.initial_health_percent.is_finite()
                || !(0.0..=100.0).contains(&settings.initial_health_percent)
                || !bounded_name(&settings.group)
                || (!settings.home_area.is_empty() && !bounded_name(&settings.home_area))
                || settings
                    .populations
                    .iter()
                    .any(|p| !p.is_empty() && !bounded_name(p))
            {
                return Err(SceneError::Limits);
            }
            let mut props = false;
            let mut pops = false;
            let mut zookeeper = false;
            for event in content {
                if !opening(event) {
                    continue;
                }
                if tag(event, b"Properties") && !props {
                    *event = replace_start(
                        event,
                        &[
                            ("HomeAreaName", settings.home_area.clone()),
                            (
                                "bSpawnOnRoundStart",
                                bool_value(settings.spawn_on_round_start),
                            ),
                            (
                                "InitialHealthPercent",
                                settings.initial_health_percent.to_string(),
                            ),
                        ],
                    )?;
                    props = true;
                } else if tag(event, b"Populations") && !pops {
                    *event = replace_start(
                        event,
                        &[
                            ("Population1", settings.populations[0].clone()),
                            ("Population2", settings.populations[1].clone()),
                            ("Population3", settings.populations[2].clone()),
                            ("Population4", settings.populations[3].clone()),
                        ],
                    )?;
                    pops = true;
                } else if tag(event, b"ZooKeeper") && !zookeeper {
                    *event = replace_start(
                        event,
                        &[
                            ("fSpawnWeight", settings.spawn_weight.to_string()),
                            ("bZooKeeperEnabled", bool_value(settings.enabled)),
                            ("sZooKeeperGroup", settings.group.clone()),
                        ],
                    )?;
                    zookeeper = true;
                }
            }
            if !(props && pops && zookeeper) {
                return Err(SceneError::Xml("CreatureSpawn is missing authored Properties, Populations or ZooKeeper section".into()));
            }
        }
        LayerAuthoringEdit::Shape { points, .. } => {
            if !polygon_area(object)
                || points.len() != object.shape_points.len()
                || !(3..=8192).contains(&points.len())
                || points
                    .iter()
                    .flatten()
                    .any(|x| !x.is_finite() || x.abs() > 1e8)
            {
                return Err(SceneError::Limits);
            }
            let mut index = 0;
            let mut inside_points = false;
            for event in content {
                if tag(event, b"Points") {
                    if matches!(event, Event::Start(_)) {
                        inside_points = true;
                    } else if matches!(event, Event::End(_)) {
                        inside_points = false;
                    }
                } else if inside_points && opening(event) && tag(event, b"Point") {
                    if index >= points.len() {
                        return Err(SceneError::Limits);
                    }
                    let val = points[index]
                        .iter()
                        .map(f32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    *event = replace_start(event, &[("Pos", val)])?;
                    index += 1;
                }
            }
            if index != points.len() {
                return Err(SceneError::Limits);
            }
        }
        _ => (),
    }
    Ok(())
}

/// Placements whose authored `<Points>` polygon can be edited: ZooKeeper
/// areas and gameplay AreaShapes such as a map's OutOfBounds_Shape.
fn polygon_area(object: &PlacedObject) -> bool {
    object.kind.eq_ignore_ascii_case("ZooKeeperShape")
        || object.kind.eq_ignore_ascii_case("AreaShape")
}

/// Engine-exported mission XML (a Legacy-ported map's mission_*.xml) names an
/// Entity by a numeric EntityId plus a 64-bit hex EntityGuid, where editor
/// layers use one Id GUID.
fn mission_entity(element: &BytesStart<'_>) -> Result<bool, SceneError> {
    Ok(value(element, b"Id")?.is_none() && value(element, b"EntityId")?.is_some())
}

/// A new EntityGuid no entity in this file uses yet. Shipped missions' GUIDs
/// all begin with hex 4; new ones follow suit.
fn fresh_entity_guid(events: &[Event<'static>], seed: &str) -> Result<String, SceneError> {
    use sha2::{Digest, Sha256};
    let mut taken = HashSet::new();
    for event in events {
        if let Event::Start(element) | Event::Empty(element) = event
            && let Some(guid) = value(element, b"EntityGuid")?
        {
            taken.insert(guid.to_ascii_uppercase());
        }
    }
    for attempt in 0u32..256 {
        let digest = Sha256::digest(format!("{seed}:{attempt}:{}", events.len()));
        let raw = u64::from_le_bytes(digest[..8].try_into().map_err(|_| SceneError::Limits)?);
        let guid = format!(
            "{:016X}",
            (raw & 0x0FFF_FFFF_FFFF_FFFF) | 0x4000_0000_0000_0000
        );
        if !taken.contains(&guid) {
            return Ok(guid);
        }
    }
    Err(SceneError::Limits)
}

/// Attributes naming a copy of `element` as `new_id`, in its own ID scheme.
fn identity(
    events: &[Event<'static>],
    element: &BytesStart<'_>,
    new_id: &str,
) -> Result<Vec<(&'static str, String)>, SceneError> {
    if mission_entity(element)? {
        if new_id.is_empty() || !new_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SceneError::Xml(
                "A mission entity copy needs a new numeric EntityId".into(),
            ));
        }
        for event in events {
            if let Event::Start(other) | Event::Empty(other) = event
                && value(other, b"EntityId")?.is_some_and(|id| id == new_id)
            {
                return Err(SceneError::Xml(format!(
                    "EntityId {new_id} is already in use"
                )));
            }
        }
        Ok(vec![
            ("EntityId", new_id.to_owned()),
            ("EntityGuid", fresh_entity_guid(events, new_id)?),
        ])
    } else {
        Ok(vec![("Id", new_id.to_owned())])
    }
}

/// A GeomEntity for a mission: a copy of the file's first top-level
/// GeomEntity (keeping its Properties), or a bare one when there is none.
fn mission_geometry_template(events: &[Event<'static>]) -> Result<Vec<Event<'static>>, SceneError> {
    let links = entity_references(events)?;
    for (index, event) in events.iter().enumerate() {
        let (Event::Start(element) | Event::Empty(element)) = event else {
            continue;
        };
        if links.contains(&index)
            || !element.name().as_ref().eq_ignore_ascii_case(b"Entity")
            || !value(element, b"EntityClass")?
                .is_some_and(|class| class.eq_ignore_ascii_case("GeomEntity"))
            || value(element, b"ParentId")?.is_some()
        {
            continue;
        }
        let Some(id) = value(element, b"EntityId")? else {
            continue;
        };
        let (start, end) = find_object(events, &id)?;
        return Ok(events[start..=end].to_vec());
    }
    let mut bare = BytesStart::new("Entity");
    bare.push_attribute(("EntityClass", "GeomEntity"));
    bare.push_attribute(("CastShadowMinSpec", "1"));
    Ok(vec![Event::Empty(bare.into_owned())])
}

/// Patch a single original authored layer, not an entire flattened scene.
/// The caller must import the output through a managed Modding project.
/// A model-reference edit never modifies geometry assets and no source PAK
/// or read-only Asset Library path is accepted by this pure serializer.
pub fn edit_layer_bytes(source: &[u8], edit: &LayerAuthoringEdit) -> Result<Vec<u8>, SceneError> {
    let decoded =
        crate::cryxml::xml_bytes(source, MAX_BYTES).map_err(|e| SceneError::Xml(e.to_string()))?;
    let original = parse_level_scene(&[("original", &decoded)])?;
    let object = original
        .objects
        .iter()
        .find(|object| object.id.eq_ignore_ascii_case(edit.target()))
        .ok_or(SceneError::NoMatchingGuid)?;
    let mut events = parse_events(&decoded)?;
    let (start, end) = find_object(&events, edit.target())?;
    match edit {
        LayerAuthoringEdit::Insert {
            new_guid,
            name,
            model,
            transform,
            ..
        } => {
            if !bounded_guid(new_guid)
                || !authored_display_name(name)
                || model.is_empty()
                || model.len() > 256
                || model.starts_with('/')
                || model.contains('\\')
                || model.contains(':')
                || model.split('/').any(|component| {
                    component.is_empty()
                        || component == "."
                        || component == ".."
                        || !component
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
                })
                // The supplied Tutorial source confirms native Brush Prefab
                // declarations use .cgf, not a CDF/SKIN actor or guessed
                // generic geometry entry.
                || !model.to_ascii_lowercase().ends_with(".cgf")
                || original
                    .objects
                    .iter()
                    .any(|other| other.id.eq_ignore_ascii_case(new_guid))
            {
                return Err(SceneError::Limits);
            }
            let anchor_start = match &events[start] {
                Event::Start(element) | Event::Empty(element) => element.clone(),
                _ => return Err(SceneError::Limits),
            };
            if mission_entity(&anchor_start)? {
                // A compiled mission has no Brush records (static geometry is
                // baked into terrain.dat); geometry it adds is a GeomEntity.
                let normalized = transform.normalize()?;
                let fmt = |parts: &[f32]| {
                    parts
                        .iter()
                        .map(f32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let mut cloned = mission_geometry_template(&events)?;
                let template = match &cloned[0] {
                    Event::Start(element) | Event::Empty(element) => element.clone(),
                    _ => return Err(SceneError::Limits),
                };
                let mut changes = identity(&events, &anchor_start, new_guid)?;
                changes.extend([
                    ("Name", name.to_owned()),
                    ("Geometry", model.to_owned()),
                    ("Pos", fmt(&normalized.position)),
                    ("Rotate", fmt(&normalized.quaternion)),
                    ("Scale", fmt(&normalized.scale)),
                ]);
                if let Some(layer) = value(&anchor_start, b"Layer")? {
                    changes.push(("Layer", layer));
                }
                if let Some(parent) = value(&anchor_start, b"ParentId")? {
                    changes.push(("ParentId", parent));
                }
                if value(&template, b"Material")?.is_some() {
                    changes.push(("Material", String::new()));
                }
                if value(&template, b"HiddenInGame")?.is_some() {
                    changes.push(("HiddenInGame", "0".into()));
                }
                cloned[0] = replace_start(&cloned[0], &changes)?;
                events.splice(end + 1..end + 1, cloned);
            } else {
                // Evolve's authored map geometry is an <Object Type="Brush">
                // with a Prefab source-mesh reference, not an invented <Object
                // Geometry="..."/> record. Clone only a real same-layer Brush
                // template so native Layer and renderer/collision metadata survive.
                let source_start = match &events[start] {
                    Event::Start(element) | Event::Empty(element) => element,
                    _ => return Err(SceneError::Limits),
                };
                if value(source_start, b"Type")?
                    .is_none_or(|value| !value.eq_ignore_ascii_case("Brush"))
                    || value(source_start, b"Prefab")?.is_none()
                    || events
                        .iter()
                        .take(end)
                        .skip(start + 1)
                        .any(|event| placement_tag(event) && opening(event))
                {
                    return Err(SceneError::Xml(
                    "Select an authored Brush placement as the source template for geometry insertion".into(),
                ));
                }
                let normalized = transform.normalize()?;
                let fmt = |parts: &[f32]| {
                    parts
                        .iter()
                        .map(f32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let mut cloned = events[start..=end].to_vec();
                // Never carry an unrelated old mesh's explicit material override
                // to the new Prefab. Only clear it if the selected Brush had one:
                // a source with no Material attribute must remain that way.
                let mut changes = vec![
                    ("Id", new_guid.to_owned()),
                    ("Name", name.to_owned()),
                    ("Prefab", model.to_owned()),
                    ("Pos", fmt(&normalized.position)),
                    ("Rotate", fmt(&normalized.quaternion)),
                    ("Scale", fmt(&normalized.scale)),
                ];
                if value(source_start, b"Material")?.is_some() {
                    changes.push(("Material", String::new()));
                }
                cloned[0] = replace_start(&cloned[0], &changes)?;
                events.splice(end + 1..end + 1, cloned);
            }
        }
        LayerAuthoringEdit::Transform { guid, transform } => {
            let normalized = edit_level_transform(&original, guid, *transform)?
                .objects
                .into_iter()
                .find(|obj| obj.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?
                .local;
            let components = |parts: &[f32]| {
                parts
                    .iter()
                    .map(f32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            };
            events[start] = replace_start(
                &events[start],
                &[
                    ("Pos", components(&normalized.position)),
                    ("Rotate", components(&normalized.quaternion)),
                    ("Scale", components(&normalized.scale)),
                ],
            )?;
        }
        LayerAuthoringEdit::Delete { .. } => {
            if original.objects.iter().any(|child| {
                child.parent.eq_ignore_ascii_case(&object.id)
                    || (object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                        && !object.name.is_empty()
                        && child.home_area.eq_ignore_ascii_case(&object.name))
            }) {
                return Err(SceneError::Xml(
                    "Cannot delete a placement referenced by child objects or wildlife home areas"
                        .into(),
                ));
            }
            events.drain(start..=end);
        }
        LayerAuthoringEdit::Duplicate {
            new_guid,
            name,
            position,
            ..
        } => {
            if !bounded_guid(new_guid)
                || position.is_some_and(|p| p.iter().any(|v| !v.is_finite()))
                || !authored_display_name(name)
                || original
                    .objects
                    .iter()
                    .any(|o| o.id.eq_ignore_ascii_case(new_guid))
                || {
                    // An AreaShape's <Entities> list links other entities;
                    // those links are not nested placements.
                    let links = entity_references(&events)?;
                    events
                        .iter()
                        .enumerate()
                        .take(end)
                        .skip(start + 1)
                        .any(|(i, e)| placement_tag(e) && opening(e) && !links.contains(&i))
                }
            {
                return Err(SceneError::Limits);
            }
            let mut cloned = events[start..=end].to_vec();
            let offset = object.local.position;
            let new_pos = position
                .unwrap_or([offset[0] + 2.0, offset[1], offset[2]])
                .iter()
                .map(f32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let element = match &events[start] {
                Event::Start(element) | Event::Empty(element) => element.clone(),
                _ => return Err(SceneError::Limits),
            };
            let mut changes = identity(&events, &element, new_guid)?;
            changes.extend([("Name", name.to_owned()), ("Pos", new_pos)]);
            cloned[0] = replace_start(&cloned[0], &changes)?;
            events.splice(end + 1..end + 1, cloned);
        }
        LayerAuthoringEdit::Shape { points, .. } => {
            if !polygon_area(object)
                || !(3..=8192).contains(&points.len())
                || points
                    .iter()
                    .flatten()
                    .any(|x| !x.is_finite() || x.abs() > 1e8)
            {
                return Err(SceneError::Limits);
            }
            if points.len() == object.shape_points.len() {
                transform_target(&mut events[start..=end], edit, object)?;
            } else {
                let mut points_start = None;
                let mut points_end = None;
                for (offset, event) in events[start..=end].iter().enumerate() {
                    let idx = start + offset;
                    if tag(event, b"Points") {
                        match event {
                            Event::Start(_) => points_start = Some(idx),
                            Event::End(_) => {
                                points_end = Some(idx);
                                break;
                            }
                            Event::Empty(_) => {
                                points_start = Some(idx);
                                points_end = Some(idx);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                let new_point_events: Vec<Event<'static>> = points
                    .iter()
                    .map(|p| {
                        let pos = format!("{},{},{}", p[0], p[1], p[2]);
                        let mut elem = quick_xml::events::BytesStart::new("Point");
                        elem.push_attribute(("Pos", pos.as_str()));
                        Event::Empty(elem)
                    })
                    .collect();
                match (points_start, points_end) {
                    (Some(ps), Some(pe)) if ps < pe => {
                        events.splice(ps + 1..pe, new_point_events);
                    }
                    (Some(ps), Some(pe)) if ps == pe => {
                        let mut replacement = Vec::with_capacity(points.len() + 2);
                        replacement
                            .push(Event::Start(quick_xml::events::BytesStart::new("Points")));
                        replacement.extend(new_point_events);
                        replacement.push(Event::End(quick_xml::events::BytesEnd::new("Points")));
                        events.splice(ps..=ps, replacement);
                    }
                    _ => {
                        let mut replacement = Vec::with_capacity(points.len() + 2);
                        replacement
                            .push(Event::Start(quick_xml::events::BytesStart::new("Points")));
                        replacement.extend(new_point_events);
                        replacement.push(Event::End(quick_xml::events::BytesEnd::new("Points")));
                        events.splice(end..end, replacement);
                    }
                }
            }
        }
        _ => transform_target(&mut events[start..=end], edit, object)?,
    }
    let mut writer = Writer::new(Vec::new());
    for event in events {
        writer
            .write_event(event)
            .map_err(|e| SceneError::Xml(e.to_string()))?;
    }
    let output = writer.into_inner();
    let validated = parse_level_scene(&[("modified", &output)])?;
    match edit {
        LayerAuthoringEdit::Insert {
            new_guid,
            name,
            model,
            transform,
            ..
        } => {
            let added = validated
                .objects
                .iter()
                .filter(|object| object.id.eq_ignore_ascii_case(new_guid))
                .collect::<Vec<_>>();
            if added.len() != 1
                || added[0].name != *name
                || added[0].model != *model
                || added[0].local != transform.normalize()?
            {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Transform { guid, transform } => {
            let updated = validated
                .objects
                .iter()
                .find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            let expected = edit_level_transform(&original, guid, *transform)?
                .objects
                .into_iter()
                .find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            if updated.local != expected.local {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Delete { guid } => {
            if validated
                .objects
                .iter()
                .any(|o| o.id.eq_ignore_ascii_case(guid))
            {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Duplicate { new_guid, .. } => {
            if validated
                .objects
                .iter()
                .filter(|o| o.id.eq_ignore_ascii_case(new_guid))
                .count()
                != 1
            {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Spawn { guid, settings } => {
            let updated = validated
                .objects
                .iter()
                .find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            let expected = settings
                .populations
                .iter()
                .filter(|p| !p.is_empty())
                .cloned()
                .collect::<Vec<_>>();
            if updated.populations != expected
                || updated.home_area != settings.home_area
                || updated.wildlife.group != settings.group
                || updated.wildlife.enabled != Some(settings.enabled)
                || updated.wildlife.spawn_weight != Some(settings.spawn_weight)
                || updated.wildlife.spawn_on_round_start != Some(settings.spawn_on_round_start)
                || updated.wildlife.initial_health_percent != Some(settings.initial_health_percent)
            {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Shape { guid, points } => {
            let updated = validated
                .objects
                .iter()
                .find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            if updated.shape_points != *points {
                return Err(SceneError::Limits);
            }
        }
    }
    Ok(output)
}

/// Produce a native file replacement, preserving the original XML storage
/// representation. The game may expect CryXmlB even when the editor has
/// decoded the layer to UTF-8 for surgical entity edits.
pub fn edit_native_layer_bytes(
    source: &[u8],
    edit: &LayerAuthoringEdit,
) -> Result<Vec<u8>, SceneError> {
    let plain = edit_layer_bytes(source, edit)?;
    if !source.starts_with(b"CryXmlB\0") {
        return Ok(plain);
    }
    let native = crate::cryxml::encode_edited_xml(source, &plain, MAX_BYTES)
        .map_err(|error| SceneError::Xml(error.to_string()))?;
    let decoded = crate::cryxml::xml_bytes(&native, MAX_BYTES)
        .map_err(|error| SceneError::Xml(error.to_string()))?;
    parse_level_scene(&[("project-round-trip", &decoded)])?;
    Ok(native)
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &[u8] = br#"<ObjectLayer><LayerObjects><Object Type="Brush" Layer="Natural" Id="first" Name="first" Prefab="objects/rock.cgf" Pos="1,2,3" /><Object Id="spawn" Name="wild" EntityClass="CreatureSpawn" Pos="5,6,7"><Properties HomeAreaName="area" bSpawnOnRoundStart="0" InitialHealthPercent="100"><Populations Population1="Trapjaw" Population2="" Population3="" Population4=""/><ZooKeeper fSpawnWeight="1" bZooKeeperEnabled="1" sZooKeeperGroup="Creatures"/></Properties></Object><Object Id="shape" Name="area" EntityClass="ZooKeeperShape"><Area/><Points><Point Pos="0,0,0"/><Point Pos="5,0,0"/><Point Pos="5,5,0"/></Points></Object></LayerObjects></ObjectLayer>"#;

    #[test]
    fn edited_cryxml_source_remains_binary_and_matches_native_reader() {
        let original = crate::cryxml::encode_xml(XML, MAX_BYTES).unwrap();
        let mutation = LayerAuthoringEdit::Transform {
            guid: "first".into(),
            transform: Transform {
                position: [20.0, 30.0, 40.0],
                ..Transform::default()
            },
        };
        let changed = edit_native_layer_bytes(&original, &mutation).unwrap();
        assert!(changed.starts_with(b"CryXmlB\0"));
        let scene = parse_level_scene(&[("native", &changed)]).unwrap();
        assert_eq!(scene.objects[0].local.position, [20.0, 30.0, 40.0]);
        assert_eq!(scene.objects[1].populations, ["Trapjaw".to_owned()]);
        let plain = edit_native_layer_bytes(XML, &mutation).unwrap();
        assert!(!plain.starts_with(b"CryXmlB\0"));
        assert!(parse_level_scene(&[("plain", &plain)]).is_ok());
    }

    #[test]
    fn changing_actual_entity_ignores_id_only_nested_references() {
        let source = br#"<ObjectLayer><LayerObjects><Object Type="Entity" Id="actor" Name="Actor" Pos="1,2,3"/><Object Type="ReverbVolume" Id="reverb" Name="Volume"><Entities><Entity Id="actor"/></Entities></Object></LayerObjects></ObjectLayer>"#;
        let transformed = edit_layer_bytes(
            source,
            &LayerAuthoringEdit::Transform {
                guid: "actor".into(),
                transform: Transform {
                    position: [5.0, 6.0, 7.0],
                    ..Transform::default()
                },
            },
        )
        .unwrap();
        let scene = parse_level_scene(&[("edited", &transformed)]).unwrap();
        assert_eq!(scene.objects.len(), 2);
        assert_eq!(scene.objects[0].local.position, [5.0, 6.0, 7.0]);
        assert!(String::from_utf8_lossy(&transformed).contains("<Entity Id=\"actor\""));
    }

    #[test]
    fn native_brush_template_uses_prefab_and_brace_guid() {
        // Sanitized CryEngine Sandbox-style authoring structure, not game data.
        let original = br#"<ObjectLayer><LayerObjects><Object Type="Brush" Layer="Rocks" Id="{550E8400-E29B-41D4-A716-446655440000}" Name="Stone bridge" Pos="1,2,3" Rotate="1,0,0,0" Material="materials/old" Prefab="Objects/old.cgf"><CollisionFiltering/></Object></LayerObjects></ObjectLayer>"#;
        let fresh = "{A6EEAB04-D776-4C26-9B6D-7FED8DCD4E0C}";
        let insert = LayerAuthoringEdit::Insert {
            anchor_guid: "{550E8400-E29B-41D4-A716-446655440000}".into(),
            new_guid: fresh.into(),
            name: "New stone".into(),
            model: "Objects/new.cgf".into(),
            transform: Transform {
                position: [5., 6., 7.],
                ..Transform::default()
            },
        };
        let modified = edit_native_layer_bytes(original, &insert).unwrap();
        let scene = parse_level_scene(&[("native", &modified)]).unwrap();
        assert_eq!(scene.objects.len(), 2);
        assert_eq!(scene.objects[1].model, "Objects/new.cgf");
        assert_eq!(scene.objects[1].local.position, [5., 6., 7.]);
        let text = String::from_utf8(modified).unwrap();
        assert!(text.contains("Prefab=\"Objects/new.cgf\""));
        assert!(text.contains("Type=\"Brush\""));
        assert!(text.contains("Material=\"\""));
        assert!(!text.contains("Geometry="));
        let duplicate = LayerAuthoringEdit::Duplicate {
            guid: fresh.into(),
            new_guid: "{6DA3E510-0081-4321-AD45-111111111111}".into(),
            name: "New stone copy".into(),
            position: None,
        };
        let doubled = edit_layer_bytes(text.as_bytes(), &duplicate).unwrap();
        assert_eq!(
            parse_level_scene(&[("doubled", &doubled)])
                .unwrap()
                .objects
                .len(),
            3
        );
    }

    #[test]
    fn insert_model_has_exact_authored_path_and_safe_limits() {
        let edit = LayerAuthoringEdit::Insert {
            anchor_guid: "first".into(),
            new_guid: "550e8400-e29b-41d4-a716-446655440000".into(),
            name: "New rock".into(),
            model: "objects/rocks/stone.cgf".into(),
            transform: Transform {
                position: [15.0, 20.0, 30.0],
                ..Transform::default()
            },
        };
        let bytes = edit_layer_bytes(XML, &edit).unwrap();
        let scene = parse_level_scene(&[("source", &bytes)]).unwrap();
        assert_eq!(scene.objects.len(), 4);
        let added = scene
            .objects
            .iter()
            .find(|object| object.id == "550e8400-e29b-41d4-a716-446655440000")
            .unwrap();
        assert_eq!(added.model, "objects/rocks/stone.cgf");
        assert_eq!(added.local.position, [15.0, 20.0, 30.0]);
        assert!(String::from_utf8_lossy(&bytes).contains("Prefab=\"objects/rocks/stone.cgf\""));
        assert!(String::from_utf8_lossy(&bytes).contains("Type=\"Brush\""));
        let invalid = LayerAuthoringEdit::Insert {
            anchor_guid: "first".into(),
            new_guid: "another-id".into(),
            name: "Unsafe".into(),
            model: "../wrong/stone.cgf".into(),
            transform: Transform::default(),
        };
        assert!(edit_layer_bytes(XML, &invalid).is_err());
    }

    #[test]
    fn duplicate_and_delete_preserve_other_source_placements() {
        let copied = edit_layer_bytes(
            XML,
            &LayerAuthoringEdit::Duplicate {
                guid: "first".into(),
                new_guid: "new01".into(),
                name: "new_rock".into(),
                position: None,
            },
        )
        .unwrap();
        let scene = parse_level_scene(&[("modified", &copied)]).unwrap();
        assert_eq!(scene.objects.len(), 4);
        assert_eq!(scene.objects[0].local.position, [1., 2., 3.]);
        assert_eq!(scene.objects[1].local.position, [3., 2., 3.]);
        let removed = edit_layer_bytes(
            &copied,
            &LayerAuthoringEdit::Delete {
                guid: "new01".into(),
            },
        )
        .unwrap();
        assert_eq!(
            parse_level_scene(&[("modified", &removed)])
                .unwrap()
                .objects
                .len(),
            3
        );
        // A paste places the copy, with every property, at its target.
        let pasted = edit_layer_bytes(
            XML,
            &LayerAuthoringEdit::Duplicate {
                guid: "first".into(),
                new_guid: "new02".into(),
                name: "pasted_rock".into(),
                position: Some([40.0, 50.5, 6.25]),
            },
        )
        .unwrap();
        let scene = parse_level_scene(&[("pasted", &pasted)]).unwrap();
        assert_eq!(scene.objects[1].local.position, [40.0, 50.5, 6.25]);
        assert_eq!(scene.objects[1].model, scene.objects[0].model);
        assert!(
            edit_layer_bytes(
                XML,
                &LayerAuthoringEdit::Duplicate {
                    guid: "first".into(),
                    new_guid: "new03".into(),
                    name: "bad".into(),
                    position: Some([f32::NAN, 0.0, 0.0]),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn wildlife_fields_and_polygon_round_trip() {
        let props = SpawnPropertiesEdit {
            populations: [
                "TrapjawElite".into(),
                "MammothBird".into(),
                "".into(),
                "".into(),
            ],
            home_area: "area".into(),
            spawn_weight: 2.0,
            enabled: true,
            spawn_on_round_start: true,
            initial_health_percent: 80.0,
            group: "Creatures".into(),
        };
        let changed = edit_layer_bytes(
            XML,
            &LayerAuthoringEdit::Spawn {
                guid: "spawn".into(),
                settings: props,
            },
        )
        .unwrap();
        let scene = parse_level_scene(&[("edited", &changed)]).unwrap();
        assert_eq!(
            scene.objects[1].populations,
            ["TrapjawElite".to_owned(), "MammothBird".to_owned()]
        );
        assert_eq!(scene.objects[1].wildlife.spawn_weight, Some(2.0));
        let polygon = [[0., 0., 0.], [10., 0., 0.], [10., 10., 0.]];
        let updated = edit_layer_bytes(
            &changed,
            &LayerAuthoringEdit::Shape {
                guid: "shape".into(),
                points: polygon.to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            parse_level_scene(&[("new", &updated)]).unwrap().objects[2].shape_points,
            polygon
        );

        // Verify arbitrary polygon topology (adding and removing vertices)
        let expanded = [[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]];
        let with_extra_vertex = edit_layer_bytes(
            &updated,
            &LayerAuthoringEdit::Shape {
                guid: "shape".into(),
                points: expanded.to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            parse_level_scene(&[("expanded", &with_extra_vertex)])
                .unwrap()
                .objects[2]
                .shape_points,
            expanded
        );

        let native_shape = edit_native_layer_bytes(
            &crate::cryxml::encode_xml(&updated, MAX_BYTES).unwrap(),
            &LayerAuthoringEdit::Shape {
                guid: "shape".into(),
                points: expanded.to_vec(),
            },
        )
        .unwrap();
        assert!(native_shape.starts_with(b"CryXmlB\0"));
        assert_eq!(
            parse_level_scene(&[("native_shape", &native_shape)])
                .unwrap()
                .objects[2]
                .shape_points,
            expanded
        );

        let reduced = [[1., 1., 0.], [5., 1., 0.], [3., 5., 0.]];
        let with_fewer_vertices = edit_layer_bytes(
            &with_extra_vertex,
            &LayerAuthoringEdit::Shape {
                guid: "shape".into(),
                points: reduced.to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            parse_level_scene(&[("reduced", &with_fewer_vertices)])
                .unwrap()
                .objects[2]
                .shape_points,
            reduced
        );

        assert!(
            edit_layer_bytes(
                &with_extra_vertex,
                &LayerAuthoringEdit::Shape {
                    guid: "shape".into(),
                    points: vec![[0., 0., 0.], [1., 1., 1.]],
                },
            )
            .is_err()
        );
    }

    #[test]
    fn mission_out_of_bounds_area_polygon_can_grow() {
        // The shape of a compiled mission's OutOfBounds_Shape: an AreaShape
        // linking the reversed ForbiddenArea that damages players outside it.
        let mission = br#"<Mission><Objects><Entity Name="OutOfBounds_Shape" Pos="500,600,80" EntityClass="AreaShape" EntityId="111" EntityGuid="4000000000000111"><Properties Height="85.9"/><Points><Point Pos="0,0,0"/><Point Pos="10,0,0"/><Point Pos="10,10,0"/></Points><Entities><Entity Id="379"/></Entities></Entity><Entity Name="OutOfBounds_Damage" Pos="0,0,0" EntityClass="ForbiddenArea" EntityId="379" EntityGuid="4000000000000379"><Properties bReversed="1"/></Entity></Objects></Mission>"#;
        let scene = parse_level_scene(&[("mission", mission)]).unwrap();
        let area = scene
            .objects
            .iter()
            .find(|o| o.name == "OutOfBounds_Shape")
            .unwrap();
        assert_eq!(area.kind, "AreaShape");
        assert_eq!(area.shape_points.len(), 3);
        let grown = [[0., 0., 0.], [40., 0., 0.], [40., 40., 0.], [0., 40., 0.]];
        let edited = edit_layer_bytes(
            mission,
            &LayerAuthoringEdit::Shape {
                guid: area.id.clone(),
                points: grown.to_vec(),
            },
        )
        .unwrap();
        let scene = parse_level_scene(&[("edited", &edited)]).unwrap();
        let area = scene
            .objects
            .iter()
            .find(|o| o.name == "OutOfBounds_Shape")
            .unwrap();
        assert_eq!(area.shape_points, grown);
        let text = String::from_utf8(edited).unwrap();
        // The damage-area link and the forbidden area itself are untouched.
        assert!(text.contains(r#"<Entities><Entity Id="379"/></Entities>"#));
        assert!(text.contains(r#"bReversed="1""#));
    }

    #[test]
    fn referenced_home_area_is_not_deleted() {
        assert!(
            edit_layer_bytes(
                XML,
                &LayerAuthoringEdit::Delete {
                    guid: "shape".into()
                }
            )
            .is_err()
        );
        assert!(
            edit_layer_bytes(
                XML,
                &LayerAuthoringEdit::Duplicate {
                    guid: "shape".into(),
                    new_guid: "spawn".into(),
                    name: "bad".into(),
                    position: None
                }
            )
            .is_err()
        );
    }
}
