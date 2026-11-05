//! Source-preserving, bounded edits for managed Modding project .lyr/XML files.
//! The caller must use the verified project import transaction. This writer
//! never opens game PAKs or imports from the read-only Asset Library.
use crate::scene::{parse_level_scene, PlacedObject, SceneError};
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
    /// Duplicate an existing XML placement, including its original authored
    /// nested properties, into the same layer. Never duplicate nested objects.
    Duplicate { guid: String, new_guid: String, name: String },
    /// Remove one object only when it has no scene descendants or home areas.
    Delete { guid: String },
    /// Modify only the known source-authored CreatureSpawn fields.
    Spawn { guid: String, settings: SpawnPropertiesEdit },
    /// Modify source-authored ZooKeeperShape vertices in their original order.
    /// Count stays constant to avoid inventing engine-specific area topology.
    Shape { guid: String, points: Vec<[f32; 3]> },
}

impl LayerAuthoringEdit {
    fn target(&self) -> &str {
        match self {
            Self::Duplicate { guid, .. }
            | Self::Delete { guid }
            | Self::Spawn { guid, .. }
            | Self::Shape { guid, .. } => guid,
        }
    }
}

fn bounded_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.:/".contains(&b))
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

fn patched(event: &BytesStart<'_>, values: &[(&str, String)]) -> Result<BytesStart<'static>, SceneError> {
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

fn replace_start(event: &Event<'static>, fields: &[(&str, String)]) -> Result<Event<'static>, SceneError> {
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
        .iter().any(|name| tag(event, name))
}

fn parse_events(xml: &[u8]) -> Result<Vec<Event<'static>>, SceneError> {
    if xml.len() > MAX_BYTES || xml.windows(9).any(|s| s.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || xml.windows(8).any(|s| s.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(SceneError::Limits);
    }
    let mut reader = Reader::from_reader(xml);
    let mut events = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| SceneError::Xml(e.to_string()))?;
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

fn find_object(events: &[Event<'static>], guid: &str) -> Result<(usize, usize), SceneError> {
    let mut match_range = None;
    for (i, event) in events.iter().enumerate() {
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
                || settings.populations.iter().any(|p| !p.is_empty() && !bounded_name(p))
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
                    *event = replace_start(event, &[
                        ("HomeAreaName", settings.home_area.clone()),
                        ("bSpawnOnRoundStart", bool_value(settings.spawn_on_round_start)),
                        ("InitialHealthPercent", settings.initial_health_percent.to_string()),
                    ])?;
                    props = true;
                } else if tag(event, b"Populations") && !pops {
                    *event = replace_start(event, &[
                        ("Population1", settings.populations[0].clone()),
                        ("Population2", settings.populations[1].clone()),
                        ("Population3", settings.populations[2].clone()),
                        ("Population4", settings.populations[3].clone()),
                    ])?;
                    pops = true;
                } else if tag(event, b"ZooKeeper") && !zookeeper {
                    *event = replace_start(event, &[
                        ("fSpawnWeight", settings.spawn_weight.to_string()),
                        ("bZooKeeperEnabled", bool_value(settings.enabled)),
                        ("sZooKeeperGroup", settings.group.clone()),
                    ])?;
                    zookeeper = true;
                }
            }
            if !(props && pops && zookeeper) {
                return Err(SceneError::Xml("CreatureSpawn is missing authored Properties, Populations or ZooKeeper section".into()));
            }
        }
        LayerAuthoringEdit::Shape { points, .. } => {
            if !object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                || points.len() != object.shape_points.len()
                || !(3..=8192).contains(&points.len())
                || points.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1e8)
            {
                return Err(SceneError::Limits);
            }
            let mut index = 0;
            let mut inside_points = false;
            for event in content {
                if tag(event, b"Points") {
                    if matches!(event, Event::Start(_)) { inside_points = true; }
                    else if matches!(event, Event::End(_)) { inside_points = false; }
                } else if inside_points && opening(event) && tag(event, b"Point") {
                    if index >= points.len() { return Err(SceneError::Limits); }
                    let val = points[index].iter().map(f32::to_string).collect::<Vec<_>>().join(",");
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

/// Patch a single original authored layer, not an entire flattened scene.
/// The caller must import the output through a managed Modding project.
/// A model-reference edit never modifies geometry assets and no source PAK
/// or read-only Asset Library path is accepted by this pure serializer.
pub fn edit_layer_bytes(source: &[u8], edit: &LayerAuthoringEdit) -> Result<Vec<u8>, SceneError> {
    let decoded = crate::cryxml::xml_bytes(source, MAX_BYTES)
        .map_err(|e| SceneError::Xml(e.to_string()))?;
    let original = parse_level_scene(&[("original", &decoded)])?;
    let object = original.objects.iter()
        .find(|object| object.id.eq_ignore_ascii_case(edit.target()))
        .ok_or(SceneError::NoMatchingGuid)?;
    let mut events = parse_events(&decoded)?;
    let (start, end) = find_object(&events, edit.target())?;
    match edit {
        LayerAuthoringEdit::Delete { .. } => {
            if original.objects.iter().any(|child| {
                child.parent.eq_ignore_ascii_case(&object.id)
                    || (object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                        && !object.name.is_empty()
                        && child.home_area.eq_ignore_ascii_case(&object.name))
            }) {
                return Err(SceneError::Xml("Cannot delete a placement referenced by child objects or wildlife home areas".into()));
            }
            events.drain(start..=end);
        }
        LayerAuthoringEdit::Duplicate { new_guid, name, .. } => {
            if !bounded_name(new_guid) || !bounded_name(name)
                || original.objects.iter().any(|o| o.id.eq_ignore_ascii_case(new_guid))
                || events.iter().take(end).skip(start + 1).any(|e| placement_tag(e) && opening(e))
            {
                return Err(SceneError::Limits);
            }
            let mut cloned = events[start..=end].to_vec();
            let offset = object.local.position;
            let new_pos = [offset[0] + 2.0, offset[1], offset[2]]
                .iter().map(f32::to_string).collect::<Vec<_>>().join(",");
            cloned[0] = replace_start(&cloned[0], &[
                ("Id", new_guid.to_owned()),
                ("Name", name.to_owned()),
                ("Pos", new_pos),
            ])?;
            events.splice(end + 1..end + 1, cloned);
        }
        _ => transform_target(&mut events[start..=end], edit, object)?,
    }
    let mut writer = Writer::new(Vec::new());
    for event in events {
        writer.write_event(event).map_err(|e| SceneError::Xml(e.to_string()))?;
    }
    let output = writer.into_inner();
    let validated = parse_level_scene(&[("modified", &output)])?;
    match edit {
        LayerAuthoringEdit::Delete { guid } => {
            if validated.objects.iter().any(|o| o.id.eq_ignore_ascii_case(guid)) {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Duplicate { new_guid, .. } => {
            if validated.objects.iter().filter(|o| o.id.eq_ignore_ascii_case(new_guid)).count() != 1 {
                return Err(SceneError::Limits);
            }
        }
        LayerAuthoringEdit::Spawn { guid, settings } => {
            let updated = validated.objects.iter().find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            let expected = settings.populations.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>();
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
            let updated = validated.objects.iter().find(|o| o.id.eq_ignore_ascii_case(guid))
                .ok_or(SceneError::NoMatchingGuid)?;
            if updated.shape_points != *points { return Err(SceneError::Limits); }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &[u8] = br#"<ObjectLayer><LayerObjects><Object Id="first" Name="first" Geometry="objects/rock.cgf" Pos="1,2,3" /><Object Id="spawn" Name="wild" EntityClass="CreatureSpawn" Pos="5,6,7"><Properties HomeAreaName="area" bSpawnOnRoundStart="0" InitialHealthPercent="100"><Populations Population1="Trapjaw" Population2="" Population3="" Population4=""/><ZooKeeper fSpawnWeight="1" bZooKeeperEnabled="1" sZooKeeperGroup="Creatures"/></Properties></Object><Object Id="shape" Name="area" EntityClass="ZooKeeperShape"><Area/><Points><Point Pos="0,0,0"/><Point Pos="5,0,0"/><Point Pos="5,5,0"/></Points></Object></LayerObjects></ObjectLayer>"#;

    #[test]
    fn duplicate_and_delete_preserve_other_source_placements() {
        let copied = edit_layer_bytes(XML, &LayerAuthoringEdit::Duplicate {
            guid: "first".into(), new_guid: "new01".into(), name: "new_rock".into(),
        }).unwrap();
        let scene = parse_level_scene(&[("modified", &copied)]).unwrap();
        assert_eq!(scene.objects.len(), 4);
        assert_eq!(scene.objects[0].local.position, [1., 2., 3.]);
        assert_eq!(scene.objects[1].local.position, [3., 2., 3.]);
        let removed = edit_layer_bytes(&copied, &LayerAuthoringEdit::Delete {
            guid: "new01".into()
        }).unwrap();
        assert_eq!(parse_level_scene(&[("modified", &removed)]).unwrap().objects.len(), 3);
    }

    #[test]
    fn wildlife_fields_and_polygon_round_trip() {
        let props = SpawnPropertiesEdit {
            populations: ["TrapjawElite".into(), "MammothBird".into(), "".into(), "".into()],
            home_area: "area".into(), spawn_weight: 2.0, enabled: true,
            spawn_on_round_start: true, initial_health_percent: 80.0, group: "Creatures".into(),
        };
        let changed = edit_layer_bytes(XML, &LayerAuthoringEdit::Spawn {
            guid: "spawn".into(), settings: props
        }).unwrap();
        let scene = parse_level_scene(&[("edited", &changed)]).unwrap();
        assert_eq!(scene.objects[1].populations, ["TrapjawElite".to_owned(), "MammothBird".to_owned()]);
        assert_eq!(scene.objects[1].wildlife.spawn_weight, Some(2.0));
        let polygon = [[0.,0.,0.],[10.,0.,0.],[10.,10.,0.]];
        let updated = edit_layer_bytes(&changed, &LayerAuthoringEdit::Shape {
            guid: "shape".into(), points: polygon.to_vec()
        }).unwrap();
        assert_eq!(parse_level_scene(&[("new", &updated)]).unwrap().objects[2].shape_points, polygon);
    }

    #[test]
    fn referenced_home_area_is_not_deleted() {
        assert!(edit_layer_bytes(XML, &LayerAuthoringEdit::Delete { guid: "shape".into() }).is_err());
        assert!(edit_layer_bytes(XML, &LayerAuthoringEdit::Duplicate {
            guid: "shape".into(), new_guid: "spawn".into(), name: "bad".into()
        }).is_err());
    }
}
