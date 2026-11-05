//! Project-only Level Editor form values and validated source-file commands.
//! Read-only map previews never pass these edits to worker operations.
use evolve_core::PlacedObject;
use evolve_core::TerrainBrushEdit;
use evolve_core::scene_authoring::{LayerAuthoringEdit, SpawnPropertiesEdit};

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
            "Exact model path (.cgf/.cdf/...)",
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
            .push(text("Preserves the authored vertex count; arbitrary topology changes require verification.")
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
