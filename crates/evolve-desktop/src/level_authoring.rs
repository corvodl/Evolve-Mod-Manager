//! Project-only Level Editor form values and validated source-file commands.
//! Read-only map previews never pass these edits to worker operations.
use evolve_core::TerrainBrushEdit;
use evolve_core::scene_authoring::{LayerAuthoringEdit, SpawnPropertiesEdit};
use evolve_core::{LevelScene, PlacedObject};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum TextField {
    NewObjectName,
    NewObjectModel,
    Populations,
    HomeArea,
    SpawnWeight,
    SpawnGroup,
    SpawnHealth,
    Polygon,
    BrushX,
    BrushY,
    BrushRadius,
    BrushDeltaHeight,
    BrushLayerId,
}

#[derive(Debug)]
pub(super) struct Fields {
    pub new_object_name: String,
    pub new_object_model: String,
    pub populations: String,
    pub home_area: String,
    pub spawn_weight: String,
    pub spawn_group: String,
    pub spawn_health: String,
    pub spawn_enabled: bool,
    pub spawn_on_round_start: bool,
    pub polygon: String,
    pub brush_x: String,
    pub brush_y: String,
    pub brush_radius: String,
    pub brush_delta_height: String,
    pub brush_layer_id: String,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            new_object_name: String::new(),
            new_object_model: String::new(),
            populations: String::new(),
            home_area: String::new(),
            spawn_weight: "1".into(),
            spawn_group: "CreatureSpawns".into(),
            spawn_health: "100".into(),
            spawn_enabled: true,
            spawn_on_round_start: false,
            polygon: String::new(),
            brush_x: "0".into(),
            brush_y: "0".into(),
            brush_radius: "2".into(),
            brush_delta_height: "1".into(),
            brush_layer_id: "0".into(),
        }
    }
}

impl Fields {
    pub fn sync(&mut self, selected: Option<&PlacedObject>) {
        self.new_object_name.clear();
        self.new_object_model.clear();
        self.populations.clear();
        self.home_area.clear();
        self.polygon.clear();
        let Some(object) = selected else {
            return;
        };
        self.new_object_name = format!("{}_new", object.name);
        self.new_object_model.clone_from(&object.model);
        self.populations = object.populations.join(", ");
        self.home_area.clone_from(&object.home_area);
        self.spawn_weight = object.wildlife.spawn_weight.unwrap_or(1.0).to_string();
        self.spawn_health = object
            .wildlife
            .initial_health_percent
            .unwrap_or(100.0)
            .to_string();
        self.spawn_enabled = object.wildlife.enabled.unwrap_or(true);
        self.spawn_on_round_start = object.wildlife.spawn_on_round_start.unwrap_or(false);
        self.spawn_group = if object.wildlife.group.is_empty() {
            "CreatureSpawns".into()
        } else {
            object.wildlife.group.clone()
        };
        self.polygon = object
            .shape_points
            .iter()
            .map(|p| p.iter().map(f32::to_string).collect::<Vec<_>>().join(","))
            .collect::<Vec<_>>()
            .join("; ");
    }

    pub fn change(&mut self, key: TextField, value: String) {
        match key {
            TextField::NewObjectName => self.new_object_name = value,
            TextField::NewObjectModel => self.new_object_model = value,
            TextField::Populations => self.populations = value,
            TextField::HomeArea => self.home_area = value,
            TextField::SpawnWeight => self.spawn_weight = value,
            TextField::SpawnGroup => self.spawn_group = value,
            TextField::SpawnHealth => self.spawn_health = value,
            TextField::Polygon => self.polygon = value,
            TextField::BrushX => self.brush_x = value,
            TextField::BrushY => self.brush_y = value,
            TextField::BrushRadius => self.brush_radius = value,
            TextField::BrushDeltaHeight => self.brush_delta_height = value,
            TextField::BrushLayerId => self.brush_layer_id = value,
        }
    }

    pub fn spawn(&self, guid: String) -> Result<LayerAuthoringEdit, String> {
        let names = self
            .populations
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if names.is_empty() || names.len() > 4 {
            return Err("Select one to four authored wildlife population names.".into());
        }
        let mut populations = std::array::from_fn(|_| String::new());
        for (slot, value) in populations.iter_mut().zip(names) {
            *slot = value;
        }
        let spawn_weight = self
            .spawn_weight
            .trim()
            .parse::<f32>()
            .map_err(|_| "Spawn weight must be a number")?;
        let initial_health_percent = self
            .spawn_health
            .trim()
            .parse::<f32>()
            .map_err(|_| "Initial health must be a percent")?;
        Ok(LayerAuthoringEdit::Spawn {
            guid,
            settings: SpawnPropertiesEdit {
                populations,
                home_area: self.home_area.trim().to_owned(),
                spawn_weight,
                enabled: self.spawn_enabled,
                spawn_on_round_start: self.spawn_on_round_start,
                initial_health_percent,
                group: self.spawn_group.trim().to_owned(),
            },
        })
    }

    pub fn shape(&self, guid: String) -> Result<LayerAuthoringEdit, String> {
        let mut points = Vec::new();
        for item in self.polygon.split(';') {
            let raw = item.trim();
            if raw.is_empty() {
                continue;
            }
            let xyz = raw
                .split(',')
                .map(|p| p.trim().parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Polygon coordinates must be numbers")?;
            let arr: [f32; 3] = xyz
                .try_into()
                .map_err(|_| "Each polygon vertex requires x,y,z")?;
            points.push(arr);
        }
        if points.len() < 3 {
            return Err("Wildlife polygon requires at least 3 vertices (x,y,z; ...)".into());
        }
        Ok(LayerAuthoringEdit::Shape { guid, points })
    }

    pub fn brush(&self, paint: bool) -> Result<TerrainBrushEdit, String> {
        let x = self
            .brush_x
            .trim()
            .parse::<u32>()
            .map_err(|_| "Invalid X terrain sample")?;
        let y = self
            .brush_y
            .trim()
            .parse::<u32>()
            .map_err(|_| "Invalid Y terrain sample")?;
        let radius = self
            .brush_radius
            .trim()
            .parse::<u32>()
            .map_err(|_| "Invalid brush radius")?;
        if paint {
            let layer_id = self
                .brush_layer_id
                .trim()
                .parse::<u8>()
                .map_err(|_| "Terrain LayerId must be an unsigned byte")?;
            Ok(TerrainBrushEdit::Paint {
                x,
                y,
                radius,
                layer_id,
            })
        } else {
            let delta_height = self
                .brush_delta_height
                .trim()
                .parse::<f32>()
                .map_err(|_| "Invalid terrain height delta")?;
            Ok(TerrainBrushEdit::Sculpt {
                x,
                y,
                radius,
                delta_height,
            })
        }
    }
}

/// The per-layer writer cannot see references in other project members.
/// Require the combined loaded scene to be free of references before sending
/// a deletion to the source-file transaction.
pub(super) fn validate_deletion(scene: &LevelScene, guid: &str) -> Result<(), String> {
    let mut found = scene
        .objects
        .iter()
        .filter(|object| object.id.eq_ignore_ascii_case(guid));
    let object = found
        .next()
        .ok_or("Selected placement is no longer in the loaded level")?;
    if found.next().is_some() {
        return Err("Cannot delete an object with a repeated GUID across layers".into());
    }
    if scene.objects.iter().any(|other| {
        other.id != object.id
            && (other.parent.eq_ignore_ascii_case(guid)
                || (object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                    && !object.name.is_empty()
                    && other.home_area.eq_ignore_ascii_case(&object.name)))
    }) {
        return Err(
            "Cannot delete a placement referenced by a child or wildlife home area in another layer"
                .into(),
        );
    }
    Ok(())
}

/// CryEngine also stores ID-only child links inside <Entities>. The scene
/// parser deliberately excludes these from placements, so search them before
/// deleting a target whose reference could reside in a different layer.
fn contains_entity_link(bytes: &[u8], guid: &str) -> Result<bool, String> {
    use quick_xml::{Reader, events::Event};
    let xml = evolve_core::cryxml::xml_bytes(bytes, 48 * 1024 * 1024).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut parents = Vec::<Vec<u8>>::new();
    let mut events = 0usize;
    loop {
        events += 1;
        if events > 500_000 {
            return Err("Map layer exceeds XML reference-check limit".into());
        }
        match reader.read_event().map_err(|e| e.to_string())? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let element = match &event {
                    Event::Start(element) | Event::Empty(element) => element,
                    _ => unreachable!(),
                };
                if element.name().as_ref().eq_ignore_ascii_case(b"Entity")
                    && parents
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(b"Entities"))
                {
                    let mut reference = None;
                    let mut id_only = true;
                    for attr in element.attributes() {
                        let attr = attr.map_err(|e| e.to_string())?;
                        if attr.key.as_ref().eq_ignore_ascii_case(b"Id")
                            || attr.key.as_ref().eq_ignore_ascii_case(b"EntityId")
                        {
                            reference = Some(
                                attr.unescape_value()
                                    .map_err(|e| e.to_string())?
                                    .into_owned(),
                            );
                        } else {
                            id_only = false;
                        }
                    }
                    if id_only && reference.is_some_and(|id| id.eq_ignore_ascii_case(guid)) {
                        return Ok(true);
                    }
                }
                if matches!(event, Event::Start(_)) {
                    parents.push(element.name().as_ref().to_vec());
                }
            }
            Event::End(_) => {
                parents.pop();
            }
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}

/// Re-read every original layer in the owning map on the save worker, rather
/// than trusting a potentially stale UI preview. The transaction checks this
/// snapshot again immediately before replacing the selected project member.
#[derive(PartialEq, Eq)]
pub(super) struct ProjectLayerSnapshot(Vec<(PathBuf, [u8; 32])>);

pub(super) fn validate_project_deletion(
    files_root: &Path,
    source: &Path,
    guid: &str,
) -> Result<ProjectLayerSnapshot, String> {
    const MAX_LAYERS: usize = 1024;
    const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
    let files_root = fs::canonicalize(files_root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let relative = source
        .strip_prefix(&files_root)
        .map_err(|_| "Source escaped the project files root")?;
    let mut map = files_root.clone();
    let mut layers_found = false;
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err("Unsafe project layer path".into());
        };
        if name.to_string_lossy().eq_ignore_ascii_case("layers") {
            layers_found = true;
            break;
        }
        map.push(name);
    }
    if !layers_found || map == files_root {
        return Err("Cannot establish the full owning map for this deletion".into());
    }
    let mut pending = vec![map];
    let mut names = Vec::new();
    let mut visited = 0usize;
    while let Some(folder) = pending.pop() {
        let meta = fs::symlink_metadata(&folder).map_err(|e| e.to_string())?;
        if !meta.file_type().is_dir()
            || !fs::canonicalize(&folder)
                .map_err(|e| e.to_string())?
                .starts_with(&files_root)
        {
            return Err("Linked or invalid map directory cannot be checked for references".into());
        }
        for entry in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            visited += 1;
            if visited > MAX_LAYERS * 16 {
                return Err("Map directory exceeds reference-check limit".into());
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Linked map member cannot be checked for references".into());
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry.path().extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("lyr") || ext.eq_ignore_ascii_case("xml")
                })
            {
                names.push(entry.path());
                if names.len() > MAX_LAYERS {
                    return Err("Map has too many XML layers for a safe deletion".into());
                }
            }
        }
    }
    names.sort();
    if !names.iter().any(|name| name == &source) {
        return Err("Owning source layer was not included in the reference check".into());
    }
    let mut total = 0u64;
    let mut data = Vec::with_capacity(names.len());
    let mut snapshot = Vec::with_capacity(names.len());
    for path in names {
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        total = total
            .checked_add(meta.len())
            .ok_or("Map layer size overflow")?;
        if !meta.file_type().is_file() || meta.len() > 48 * 1024 * 1024 || total > MAX_TOTAL_BYTES {
            return Err("Map layers exceed the bounded reference-check size".into());
        }
        if !fs::canonicalize(&path)
            .map_err(|e| e.to_string())?
            .starts_with(&files_root)
        {
            return Err("Project map member escaped the managed files root".into());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != meta.len() {
            return Err("A project layer changed during reference checking".into());
        }
        snapshot.push((path.to_owned(), Sha256::digest(&bytes).into()));
        data.push((path.to_string_lossy().into_owned(), bytes));
    }
    let refs = data
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    let recovered = evolve_core::parse_level_scene_read_only(&refs).map_err(|e| e.to_string())?;
    if recovered
        .duplicate_guids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(guid))
    {
        return Err("Cannot delete a placement with duplicate GUID declarations".into());
    }
    validate_deletion(&recovered.scene, guid)?;
    for (_, bytes) in &data {
        if contains_entity_link(bytes, guid)? {
            return Err("Cannot delete a placement referenced by a nested Entity link".into());
        }
    }
    let snapshot = ProjectLayerSnapshot(snapshot);
    snapshot.verify()?;
    Ok(snapshot)
}

impl ProjectLayerSnapshot {
    pub(super) fn verify(&self) -> Result<(), String> {
        for (path, expected) in &self.0 {
            let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file()
                || meta.len() > 48 * 1024 * 1024
                || <[u8; 32]>::from(Sha256::digest(fs::read(path).map_err(|e| e.to_string())?))
                    != *expected
            {
                return Err(
                    "A project map layer changed during deletion; reload before retrying".into(),
                );
            }
        }
        Ok(())
    }
}

fn edit_field<'a>(
    label: &'static str,
    value: &'a str,
    field: TextField,
) -> iced::Element<'a, super::Message> {
    crate::design::text_input(label, value)
        .on_input(move |v| super::Message::LevelAuthoringChanged(field, v))
        .into()
}

pub(super) fn placement_editor<'a>(
    state: &'a super::State,
    object: &'a PlacedObject,
) -> iced::Element<'a, super::Message> {
    use super::Message;
    use iced::widget::{column, row, text};
    let forms = &state.level_authoring;
    if state.level_read_only || state.level_project.is_none() {
        return text("Editing requires File → Open as Modding Project.")
            .size(11)
            .style(crate::design::muted)
            .into();
    }
    if object.kind.eq_ignore_ascii_case("Vegetation") {
        return column![
            text("VegetationMap.dat (project copy)").size(12),
            crate::design::button("Save vegetation position")
                .on_press(Message::LevelMoveVegetation),
            crate::design::button("Save uniform vegetation scale")
                .on_press(Message::LevelScaleVegetation),
            row![
                crate::design::button("Duplicate vegetation")
                    .on_press(Message::LevelDuplicateVegetation),
                crate::design::button("Delete vegetation").on_press(Message::LevelDeleteVegetation),
            ]
            .spacing(6),
            text("Uses the original packed record and its selected prototype.")
                .size(11)
                .style(crate::design::muted),
        ]
        .spacing(6)
        .into();
    }
    let mut column = column![
        text("Project placement authoring").size(12),
        row![
            crate::design::button("Duplicate placement").on_press(Message::LevelDuplicateSelected),
            crate::design::button("Delete placement").on_press(Message::LevelDeleteSelected),
        ]
        .spacing(6),
        text("Add a geometry instance in this same source layer").size(12),
        edit_field(
            "New object name",
            &forms.new_object_name,
            TextField::NewObjectName
        ),
        edit_field(
            "Exact CGF model path (.cgf)",
            &forms.new_object_model,
            TextField::NewObjectModel
        ),
        crate::design::button("Add geometry to Modding project")
            .on_press(Message::LevelAddGeometry),
        text("Each action saves the owning LYR/XML project member with a verified backup.")
            .size(11)
            .style(crate::design::muted),
    ]
    .spacing(6);
    if object.kind.eq_ignore_ascii_case("CreatureSpawn") {
        column = column
            .push(text("Wildlife spawn settings").size(12))
            .push(edit_field(
                "Populations: up to four, comma separated",
                &forms.populations,
                TextField::Populations,
            ))
            .push(edit_field(
                "HomeAreaName",
                &forms.home_area,
                TextField::HomeArea,
            ))
            .push(edit_field(
                "Spawn weight",
                &forms.spawn_weight,
                TextField::SpawnWeight,
            ))
            .push(edit_field(
                "ZooKeeper group",
                &forms.spawn_group,
                TextField::SpawnGroup,
            ))
            .push(edit_field(
                "Initial health (0–100)",
                &forms.spawn_health,
                TextField::SpawnHealth,
            ))
            .push(
                row![
                    crate::design::button(if forms.spawn_enabled {
                        "☑ Enabled"
                    } else {
                        "☐ Enabled"
                    })
                    .on_press(Message::LevelAuthoringToggleEnabled),
                    crate::design::button(if forms.spawn_on_round_start {
                        "☑ Spawn on round start"
                    } else {
                        "☐ Spawn on round start"
                    })
                    .on_press(Message::LevelAuthoringToggleRoundStart),
                ]
                .spacing(6),
            )
            .push(
                crate::design::button("Save wildlife to LYR")
                    .style(crate::design::primary)
                    .on_press(Message::LevelSaveWildlifeSpawn),
            );
    } else if object.kind.eq_ignore_ascii_case("ZooKeeperShape") {
        column = column
            .push(text("ZooKeeper area vertices (x,y,z; x,y,z; ...)").size(12))
            .push(edit_field("Polygon vertices", &forms.polygon, TextField::Polygon))
            .push(crate::design::button("Save area polygon to LYR")
                .style(crate::design::primary)
                .on_press(Message::LevelSaveWildlifeShape))
            .push(text("Authored polygon vertices (minimum 3); verified against owning LYR/XML structure.")
                .size(11).style(crate::design::muted));
    }
    column.into()
}

pub(super) fn terrain_editor(state: &super::State) -> iced::Element<'_, super::Message> {
    use super::Message;
    use iced::widget::{column, row, text};
    if state.level_read_only
        || state.level_project.is_none()
        || !state
            .library_map
            .as_ref()
            .and_then(|(_, maps, index)| maps.get(*index))
            .is_some_and(|map| map.terrain.is_some())
    {
        return text("Terrain editing requires a Modding project with Heightmap.dat.")
            .size(11)
            .style(crate::design::muted)
            .into();
    }
    let form = &state.level_authoring;
    column![
        text("Terrain source brush (project only)").size(13),
        row![
            edit_field("Sample X", &form.brush_x, TextField::BrushX),
            edit_field("Sample Y", &form.brush_y, TextField::BrushY),
            edit_field("Radius", &form.brush_radius, TextField::BrushRadius),
        ].spacing(6),
        row![
            edit_field("Height delta (world units)", &form.brush_delta_height, TextField::BrushDeltaHeight),
            crate::design::button("Sculpt Heightmap.dat")
                .on_press(Message::LevelSculptTerrain),
        ].spacing(6),
        row![
            edit_field("Authored LayerId", &form.brush_layer_id, TextField::BrushLayerId),
            crate::design::button("Paint LayerId")
                .on_press(Message::LevelPaintTerrain),
        ].spacing(6),
        text("LayerId edits preserve the original baked texture atlas; native material repaint is not yet complete.")
            .size(11).style(crate::design::muted),
    ].spacing(7).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_checks_parent_and_home_area_across_layers() {
        let area = br#"<ObjectLayer><LayerObjects><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Area/><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object><Object Id="parent" Name="parent" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#;
        let children = br#"<ObjectLayer><LayerObjects><Object Id="spawn" Name="spawn" EntityClass="CreatureSpawn" Parent="parent"><Properties HomeAreaName="den"/></Object><Object Id="free" Name="free" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#;
        let scene =
            evolve_core::parse_level_scene(&[("area.lyr", area), ("children.lyr", children)])
                .unwrap();
        assert!(validate_deletion(&scene, "area").is_err());
        assert!(validate_deletion(&scene, "parent").is_err());
        assert!(validate_deletion(&scene, "free").is_ok());
        assert!(validate_deletion(&scene, "missing").is_err());
    }

    #[test]
    fn worker_rechecks_all_project_layers_before_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let files = temp.path().join("files");
        let layers = files.join("levels/Tutorial/layers");
        fs::create_dir_all(&layers).unwrap();
        let source = layers.join("shapes.lyr");
        let other = layers.join("spawns.lyr");
        fs::write(&source, br#"<ObjectLayer><LayerObjects><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object></LayerObjects></ObjectLayer>"#).unwrap();
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(validate_project_deletion(&files, &source, "area").is_err());
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="host" Type="Brush" Prefab="objects/rock.cgf"><Entities><Entity Id="area"/></Entities></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(validate_project_deletion(&files, &source, "area").is_err());
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName=""/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        let snapshot = validate_project_deletion(&files, &source, "area").unwrap();
        assert!(snapshot.verify().is_ok());
        let added = layers.join("late.lyr");
        fs::write(&added, br#"<ObjectLayer><LayerObjects><Object Id="new" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(snapshot != validate_project_deletion(&files, &source, "area").unwrap());
        fs::remove_file(&added).unwrap();
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(snapshot.verify().is_err());
        assert!(validate_project_deletion(&files, &source, "area").is_err());
    }

    #[test]
    fn managed_level_delete_saves_backup_and_reopens_without_touching_other_layer() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("Projects");
        let workspace = projects.join("Tutorial");
        let member = "levels/Tutorial/layers/objects.lyr";
        let source = workspace.join("files").join(member);
        let other = workspace.join("files/levels/Tutorial/layers/spawns.lyr");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let original = br#"<ObjectLayer><LayerObjects><Object Id="rock" Name="rock" Type="Brush" Prefab="objects/rock.cgf"/><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object></LayerObjects></ObjectLayer>"#;
        let other_bytes = br#"<ObjectLayer><LayerObjects><Object Id="spawn" Name="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#;
        fs::write(&source, original).unwrap();
        fs::write(&other, other_bytes).unwrap();
        fs::write(workspace.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version": 1, "entries": [{"path": member, "format": "raw", "sha256": format!("{:x}", Sha256::digest(original))}]
        })).unwrap()).unwrap();
        let project = crate::modding_projects::ProjectRow {
            label: "Tutorial".into(),
            target: "levels_tutorial.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        let snapshot =
            validate_project_deletion(&workspace.join("files"), &source, "rock").unwrap();
        let edited = evolve_core::scene_authoring::edit_native_layer_bytes(
            original,
            &LayerAuthoringEdit::Delete {
                guid: "rock".into(),
            },
        )
        .unwrap();
        assert!(snapshot.verify().is_ok());
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project, &projects, member, original, &edited
            )
            .unwrap()
        );
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project, &projects, member, original, &edited
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), edited);
        assert_eq!(fs::read(&other).unwrap(), other_bytes);
        let before = evolve_core::parse_level_scene_read_only(&[
            (member, original.as_slice()),
            ("spawns.lyr", other_bytes.as_slice()),
        ])
        .unwrap();
        let saved_bytes = fs::read(&source).unwrap();
        let after = evolve_core::parse_level_scene_read_only(&[
            (member, saved_bytes.as_slice()),
            ("spawns.lyr", other_bytes.as_slice()),
        ])
        .unwrap();
        assert_eq!(before.scene.objects.len(), after.scene.objects.len() + 1);
        assert!(
            after
                .scene
                .objects
                .iter()
                .any(|obj| obj.id == "spawn" && obj.home_area == "den")
        );
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(
            crate::modding_projects::restore_original_raw_member(&project, &projects, member)
                .unwrap()
        );
        assert_eq!(fs::read(&source).unwrap(), original);
    }

    #[test]
    #[ignore = "requires local read-only Tutorial imports (EVOLVE_BENCH_LIBRARY)"]
    fn imported_tutorial_placement_round_trips_across_all_layers() {
        use std::{fs, path::Path};
        let library = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let imports = evolve_core::imported_library(Path::new(&library)).unwrap();
        let archive = imports
            .archives
            .iter()
            .find(|pak| {
                pak.name.eq_ignore_ascii_case("levels_tutorial.pak")
                    && pak.source.to_ascii_lowercase().contains("moddedevolve")
            })
            .unwrap();
        let inventory = evolve_core::imported_archive_files(archive).unwrap();
        let mut layer_paths = inventory
            .entries
            .iter()
            .filter(|entry| entry.relative_path.to_ascii_lowercase().ends_with(".lyr"))
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();
        layer_paths.sort();
        assert_eq!(
            layer_paths.len(),
            39,
            "expected the complete Tutorial layer set"
        );
        let layers = layer_paths
            .iter()
            .map(|name| fs::read(Path::new(&inventory.root).join(name)).unwrap())
            .collect::<Vec<_>>();
        let before = layer_paths
            .iter()
            .zip(&layers)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let combined = evolve_core::parse_level_scene_read_only(&before).unwrap();
        let object = combined
            .scene
            .objects
            .iter()
            .find(|obj| {
                obj.model.to_ascii_lowercase().ends_with(".cgf")
                    && !combined.duplicate_guids.contains(&obj.id)
                    && obj.id.starts_with('{')
                    && obj.id.ends_with('}')
            })
            .expect("unique authored Tutorial Brush");
        let index = layer_paths
            .iter()
            .position(|name| *name == object.layer)
            .unwrap();
        let unreferenced = combined
            .scene
            .objects
            .iter()
            .find(|candidate| {
                candidate.model.to_ascii_lowercase().ends_with(".cgf")
                    && !combined.duplicate_guids.contains(&candidate.id)
                    && validate_deletion(&combined.scene, &candidate.id).is_ok()
            })
            .expect("unreferenced Tutorial placement");
        let unreferenced_source = Path::new(&inventory.root).join(&unreferenced.layer);
        validate_project_deletion(
            Path::new(&inventory.root),
            &unreferenced_source,
            &unreferenced.id,
        )
        .unwrap()
        .verify()
        .unwrap();
        let mut position = object.local;
        position.position[0] += 1.0;
        let updated = evolve_core::scene_authoring::edit_native_layer_bytes(
            &layers[index],
            &LayerAuthoringEdit::Transform {
                guid: object.id.clone(),
                transform: position,
            },
        )
        .unwrap();
        assert_eq!(
            updated.starts_with(b"CryXmlB\0"),
            layers[index].starts_with(b"CryXmlB\0")
        );
        let mut changed = layers.clone();
        changed[index] = updated.clone();
        let after = layer_paths
            .iter()
            .zip(&changed)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let reparsed = evolve_core::parse_level_scene_read_only(&after).unwrap();
        assert_eq!(reparsed.scene.objects.len(), combined.scene.objects.len());
        assert_eq!(reparsed.duplicate_guids, combined.duplicate_guids);
        for (previous, current) in combined.scene.objects.iter().zip(&reparsed.scene.objects) {
            assert_eq!(previous.id, current.id);
            if previous.id != object.id {
                assert_eq!(previous.local, current.local);
            }
        }
        assert_eq!(
            reparsed
                .scene
                .objects
                .iter()
                .find(|obj| obj.id == object.id)
                .unwrap()
                .local
                .position,
            position.position
        );
        // Stage all 39 original Tutorial layers in an isolated managed project.
        // Exercise the same backup/CAS transaction as editor commits and verify
        // the native result across the complete map, never writing to imports.
        let temp = tempfile::tempdir().unwrap();
        let projects_root = temp.path().join("Projects");
        let workspace = projects_root.join("Tutorial");
        for (name, bytes) in layer_paths.iter().zip(&layers) {
            let destination = workspace.join("files").join(name);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        let member = object.layer.as_str();
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "entries": [{
                    "path": member,
                    "format": "raw",
                    "sha256": format!("{:x}", Sha256::digest(&layers[index]))
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let project = crate::modding_projects::ProjectRow {
            label: "Tutorial".into(),
            target: "levels_tutorial.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project,
                &projects_root,
                member,
                &layers[index],
                &updated
            )
            .unwrap()
        );
        let reread = layer_paths
            .iter()
            .map(|name| fs::read(workspace.join("files").join(name)).unwrap())
            .collect::<Vec<_>>();
        let references = layer_paths
            .iter()
            .zip(&reread)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let reopened = evolve_core::parse_level_scene_read_only(&references).unwrap();
        assert_eq!(reopened.scene.objects.len(), combined.scene.objects.len());
        assert_eq!(reopened.duplicate_guids, combined.duplicate_guids);
        assert_eq!(
            reopened
                .scene
                .objects
                .iter()
                .find(|candidate| candidate.id == object.id)
                .unwrap()
                .local
                .position,
            position.position
        );
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(
            crate::modding_projects::restore_original_raw_member(&project, &projects_root, member)
                .unwrap()
        );
        assert_eq!(
            fs::read(workspace.join("files").join(member)).unwrap(),
            layers[index]
        );
        for (name, bytes) in layer_paths.iter().zip(&layers) {
            assert_eq!(
                fs::read(Path::new(&inventory.root).join(name)).unwrap(),
                *bytes
            );
        }
    }
}
