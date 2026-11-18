//! Project-only Level Editor form values and validated source-file commands.
//! Read-only map previews never pass these edits to worker operations.
use evolve_core::TerrainBrushEdit;
use evolve_core::scene_authoring::{LayerAuthoringEdit, SpawnPropertiesEdit};
use evolve_core::wildlife_spawns::{WildlifeEntity, WildlifeSettings, normalize_keywords};
use evolve_core::{LevelScene, PlacedObject};

#[derive(Clone, Copy, Debug)]
pub(super) enum TextField {
    NewObjectName,
    NewObjectModel,
    Populations,
    HomeArea,
    SpawnWeight,
    SpawnGroup,
    SpawnHealth,
    WildlifeSpecies,
    WildlifeRadius,
    WildlifeCategory,
    ZoneKeywords,
    WildlifeLinks,
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
    pub wildlife_species: String,
    pub wildlife_radius: String,
    pub wildlife_category: String,
    pub zone_keywords: String,
    pub zone_terrain: bool,
    pub zone_restrict: bool,
    /// Linked entity names, or EntityIds where a name is not unique.
    pub wildlife_links: String,
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
            wildlife_species: String::new(),
            wildlife_radius: String::new(),
            wildlife_category: String::new(),
            zone_keywords: String::new(),
            zone_terrain: false,
            zone_restrict: false,
            wildlife_links: String::new(),
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
    pub fn sync(&mut self, scene: Option<&LevelScene>, selected: Option<&PlacedObject>) {
        self.new_object_name.clear();
        self.new_object_model.clear();
        self.populations.clear();
        self.home_area.clear();
        self.polygon.clear();
        self.wildlife_species.clear();
        self.wildlife_radius.clear();
        self.wildlife_category.clear();
        self.zone_keywords.clear();
        self.wildlife_links.clear();
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
        match WildlifeSettings::of(object) {
            Some(WildlifeSettings::SpawnPoint { species }) => self.wildlife_species = species,
            Some(WildlifeSettings::Group { radius }) => self.wildlife_radius = radius.to_string(),
            Some(WildlifeSettings::Node {
                enabled,
                category,
                radius,
            }) => {
                self.spawn_enabled = enabled;
                self.wildlife_category = category;
                self.wildlife_radius = radius.map_or(String::new(), |r| r.to_string());
            }
            Some(WildlifeSettings::Zone {
                keywords,
                use_terrain_height,
                restrict_all_species,
            }) => {
                self.zone_keywords = keywords;
                self.zone_terrain = use_terrain_height;
                self.zone_restrict = restrict_all_species;
            }
            None => {}
        }
        if let Some(scene) = scene {
            self.wildlife_links = object
                .wildlife
                .links
                .iter()
                .map(|id| link_label(scene, id))
                .collect::<Vec<_>>()
                .join(", ");
        }
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
            TextField::WildlifeSpecies => self.wildlife_species = value,
            TextField::WildlifeRadius => self.wildlife_radius = value,
            TextField::WildlifeCategory => self.wildlife_category = value,
            TextField::ZoneKeywords => self.zone_keywords = value,
            TextField::WildlifeLinks => self.wildlife_links = value,
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

    /// The selected wildlife entity's script properties, or a zone's keywords.
    pub fn wildlife(&self, object: &PlacedObject) -> Result<LayerAuthoringEdit, String> {
        let radius = |raw: &str| {
            raw.trim()
                .parse::<f32>()
                .map_err(|_| "Radius must be a number of metres".to_owned())
        };
        let settings = match WildlifeEntity::of(object) {
            Some(WildlifeEntity::SpawnPoint) => WildlifeSettings::SpawnPoint {
                species: self.wildlife_species.trim().to_owned(),
            },
            Some(WildlifeEntity::Group) => WildlifeSettings::Group {
                radius: radius(&self.wildlife_radius)?,
            },
            Some(WildlifeEntity::Node) => WildlifeSettings::Node {
                enabled: self.spawn_enabled,
                category: self.wildlife_category.trim().to_owned(),
                radius: if self.wildlife_radius.trim().is_empty() {
                    // The writer only sets attributes; it never removes one.
                    if object.wildlife.radius.is_some() {
                        return Err("This node authors a radius; enter one to keep it".into());
                    }
                    None
                } else {
                    Some(radius(&self.wildlife_radius)?)
                },
            },
            Some(WildlifeEntity::Zone) => WildlifeSettings::Zone {
                keywords: normalize_keywords(&self.zone_keywords)?,
                use_terrain_height: self.zone_terrain,
                restrict_all_species: self.zone_restrict,
            },
            _ => {
                return Err(format!(
                    "{} has no wildlife settings to edit here",
                    object.kind
                ));
            }
        };
        settings.validate()?;
        Ok(LayerAuthoringEdit::Wildlife {
            guid: object.id.clone(),
            settings,
        })
    }

    /// A node's candidate groups or a group's spawn points, from the links
    /// field: each entry is an EntityId or a name unique to its class.
    pub fn links(
        &self,
        scene: &LevelScene,
        object: &PlacedObject,
    ) -> Result<LayerAuthoringEdit, String> {
        let expected = WildlifeEntity::of(object)
            .and_then(WildlifeEntity::link_target)
            .ok_or("Only a WildlifeNode or WildlifeGroup links other wildlife")?;
        let mut targets = Vec::new();
        for entry in self
            .wildlife_links
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let candidates = scene
                .objects
                .iter()
                .filter(|other| WildlifeEntity::of(other) == Some(expected));
            let target = match candidates
                .clone()
                .find(|other| other.id.eq_ignore_ascii_case(entry))
            {
                Some(target) => target,
                None => {
                    let mut named =
                        candidates.filter(|other| other.name.eq_ignore_ascii_case(entry));
                    match (named.next(), named.next()) {
                        (Some(target), None) => target,
                        (None, _) => {
                            return Err(format!("No {} is named {entry}", expected.class_name()));
                        }
                        _ => {
                            return Err(format!(
                                "More than one {} is named {entry}; enter its EntityId",
                                expected.class_name()
                            ));
                        }
                    }
                }
            };
            targets.push(target.id.clone());
        }
        Ok(LayerAuthoringEdit::Links {
            guid: object.id.clone(),
            targets,
        })
    }

    pub fn shape(&self, guid: String) -> Result<LayerAuthoringEdit, String> {
        Ok(LayerAuthoringEdit::Shape {
            guid,
            points: self.points()?,
        })
    }

    /// Replace the polygon field with these vertices.
    pub fn set_points(&mut self, points: &[[f32; 3]]) {
        self.polygon = points
            .iter()
            .map(|p| p.iter().map(f32::to_string).collect::<Vec<_>>().join(","))
            .collect::<Vec<_>>()
            .join("; ");
    }

    /// The polygon field's vertices.
    pub fn points(&self) -> Result<Vec<[f32; 3]>, String> {
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
            return Err("An area polygon requires at least 3 vertices (x,y,z; ...)".into());
        }
        Ok(points)
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

/// A linked entity's name when no other entity of its class shares it,
/// otherwise its EntityId, so the links field reads back unambiguously.
fn link_label(scene: &LevelScene, id: &str) -> String {
    let Some(target) = scene.objects.iter().find(|object| object.id == id) else {
        return id.to_owned();
    };
    let shared = scene
        .objects
        .iter()
        .filter(|other| other.kind == target.kind && other.name.eq_ignore_ascii_case(&target.name))
        .count();
    if target.name.is_empty() || shared > 1 || target.name.contains(',') {
        id.to_owned()
    } else {
        target.name.clone()
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
    if let Some(settings) = wildlife_settings_editor(state, object) {
        column = column.push(settings);
    }
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
                    crate::ui_kit::switch(
                        &state.ui,
                        "spawn-enabled",
                        "Enabled",
                        forms.spawn_enabled,
                        Message::LevelAuthoringToggleEnabled,
                    ),
                    crate::ui_kit::switch(
                        &state.ui,
                        "spawn-round-start",
                        "Spawn on round start",
                        forms.spawn_on_round_start,
                        Message::LevelAuthoringToggleRoundStart,
                    ),
                ]
                .spacing(6),
            )
            .push(
                crate::design::button("Save wildlife to LYR")
                    .style(crate::design::primary)
                    .on_press(Message::LevelSaveWildlifeSpawn),
            );
    } else if object.kind.eq_ignore_ascii_case("ZooKeeperShape")
        || object.kind.eq_ignore_ascii_case("AreaShape")
    {
        column = column
            .push(
                text(if object.kind.eq_ignore_ascii_case("AreaShape") {
                    "Area vertices relative to its position (x,y,z; ...). An out-of-bounds area's outline is the playable space."
                } else {
                    "ZooKeeper area vertices (x,y,z; x,y,z; ...)"
                })
                .size(12),
            )
            .push(edit_field("Polygon vertices", &forms.polygon, TextField::Polygon))
            .push(crate::design::button("Apply polygon")
                .style(crate::design::primary)
                .on_press(Message::LevelSaveWildlifeShape))
            .push(text("Authored polygon vertices (minimum 3); verified against owning LYR/XML structure.")
                .size(11).style(crate::design::muted));
    }
    column.into()
}

/// Settings for the four Evolve spawn types: a static point's species, a
/// circle's radius and members, a random set's candidates, and a dynamic
/// zone's keywords. CreatureSpawn keeps its own form.
fn wildlife_settings_editor<'a>(
    state: &'a super::State,
    object: &PlacedObject,
) -> Option<iced::Element<'a, super::Message>> {
    use super::Message;
    use iced::widget::{column, row, text};
    let entity = WildlifeEntity::of(object).filter(|e| *e != WildlifeEntity::CreatureSpawn)?;
    let forms = &state.level_authoring;
    let mut form =
        column![text(format!("{} settings", entity.spawn_type().label())).size(12),].spacing(6);
    form = match entity {
        WildlifeEntity::SpawnPoint => form.push(edit_field(
            "Species (esWildlife), e.g. Reaver",
            &forms.wildlife_species,
            TextField::WildlifeSpecies,
        )),
        WildlifeEntity::Group => form.push(edit_field(
            "Spawn circle radius in metres (fRadius)",
            &forms.wildlife_radius,
            TextField::WildlifeRadius,
        )),
        WildlifeEntity::Node => form
            .push(edit_field(
                "Category (esWildlifeCategory), e.g. Default",
                &forms.wildlife_category,
                TextField::WildlifeCategory,
            ))
            .push(edit_field(
                "Node radius in metres (fRadius, optional)",
                &forms.wildlife_radius,
                TextField::WildlifeRadius,
            ))
            .push(crate::ui_kit::switch(
                &state.ui,
                "wildlife-node-enabled",
                "Node enabled",
                forms.spawn_enabled,
                Message::LevelAuthoringToggleEnabled,
            )),
        _ => form
            .push(edit_field(
                "Zone keywords, comma separated (ZooKeeperKeywords)",
                &forms.zone_keywords,
                TextField::ZoneKeywords,
            ))
            .push(
                row![
                    crate::ui_kit::switch(
                        &state.ui,
                        "zone-terrain-height",
                        "Follow terrain height",
                        forms.zone_terrain,
                        Message::LevelAuthoringToggleZoneTerrain,
                    ),
                    crate::ui_kit::switch(
                        &state.ui,
                        "zone-restrict-species",
                        "Keep all species inside",
                        forms.zone_restrict,
                        Message::LevelAuthoringToggleZoneRestrict,
                    ),
                ]
                .spacing(6),
            ),
    };
    form = form.push(
        crate::design::button("Save wildlife settings")
            .style(crate::design::primary)
            .on_press(Message::LevelSaveWildlifeSettings),
    );
    if let Some(target) = entity.link_target() {
        form = form
            .push(edit_field(
                if entity == WildlifeEntity::Node {
                    "Candidate spawn circles (WildlifeGroup names or ids)"
                } else {
                    "Member spawn points (WildlifeSpawnPoint names or ids)"
                },
                &forms.wildlife_links,
                TextField::WildlifeLinks,
            ))
            .push(
                crate::design::button("Save links")
                    .style(crate::design::secondary)
                    .on_press(Message::LevelSaveWildlifeLinks),
            )
            .push(
                text(format!(
                    "One to sixteen {}s from this mission file.",
                    target.class_name()
                ))
                .size(11)
                .style(crate::design::muted),
            );
    }
    Some(form.into())
}

pub(super) fn terrain_editor(state: &super::State) -> iced::Element<'_, super::Message> {
    use super::Message;
    use iced::widget::{column, row, text};
    let map = state
        .library_map
        .as_ref()
        .and_then(|(_, maps, index)| maps.get(*index));
    // A Legacy-ported map's heightmap is its compiled terrain.dat.
    let compiled = map.is_some_and(|map| map.terrain.is_none() && map.compiled_terrain.is_some());
    if state.level_read_only
        || state.level_project.is_none()
        || !map.is_some_and(|map| map.terrain.is_some() || map.compiled_terrain.is_some())
    {
        return text(
            "Terrain editing requires a Modding project with Heightmap.dat or terrain.dat.",
        )
        .size(11)
        .style(crate::design::muted)
        .into();
    }
    let form = &state.level_authoring;
    let sculpt = crate::design::button(if compiled {
        "Sculpt terrain.dat"
    } else {
        "Sculpt Heightmap.dat"
    })
    .on_press(Message::LevelSculptTerrain);
    let paint = crate::design::button(if compiled {
        "Paint surface type"
    } else {
        "Paint LayerId"
    })
    .on_press(Message::LevelPaintTerrain);
    column![
        text("Terrain source brush (project only)").size(13),
        row![
            edit_field("Sample X", &form.brush_x, TextField::BrushX),
            edit_field("Sample Y", &form.brush_y, TextField::BrushY),
            edit_field("Radius", &form.brush_radius, TextField::BrushRadius),
        ].spacing(6),
        row![
            edit_field("Height delta (world units)", &form.brush_delta_height, TextField::BrushDeltaHeight),
            sculpt,
        ].spacing(6),
        row![
            edit_field(
                if compiled { "SurfaceTypeID (leveldata.xml)" } else { "Authored LayerId" },
                &form.brush_layer_id,
                TextField::BrushLayerId,
            ),
            paint,
        ].spacing(6),
        text(if compiled {
            "Painting sets the detail material, footstep and physics surface; the baked ground colour is unchanged."
        } else {
            "LayerId edits preserve the original baked texture atlas; native material repaint is not yet complete."
        })
            .size(11).style(crate::design::muted),
    ].spacing(7).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::level_editor::level_validation::validate_deletion;

    const WILDLIFE: &[u8] = br#"<Mission><Objects>
<Entity Name="node" Pos="0,0,0" EntityClass="WildlifeNode" EntityId="1" Layer="WildlifeSpawns"><EntityLinks><Link TargetId="2" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/></EntityLinks><Properties><extensionfolderbeginWildlifeNode bEnabled="1" esWildlifeCategory="Default"/></Properties></Entity>
<Entity Name="circle" Pos="5,0,0" EntityClass="WildlifeGroup" EntityId="2" Layer="WildlifeSpawns"><Properties><extensionfolderbeginWildlifeGroup fRadius="10"/></Properties></Entity>
<Entity Name="twin" Pos="9,0,0" EntityClass="WildlifeGroup" EntityId="3" Layer="WildlifeSpawns"><Properties><extensionfolderbeginWildlifeGroup fRadius="8"/></Properties></Entity>
<Entity Name="twin" Pos="9,9,0" EntityClass="WildlifeGroup" EntityId="4" Layer="WildlifeSpawns"><Properties><extensionfolderbeginWildlifeGroup fRadius="8"/></Properties></Entity>
<Entity Name="zone" Pos="0,0,0" EntityClass="ZooKeeperShape" EntityId="5" Layer="ZooKeepers"><Area ZooKeeperKeywords="ArmadonBull" ZooKeeperUseTerrainHeight="1"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Area></Entity>
</Objects></Mission>"#;

    #[test]
    fn linked_wildlife_cannot_be_deleted() {
        let scene = evolve_core::parse_level_scene(&[("mission", WILDLIFE)]).unwrap();
        assert!(validate_deletion(&scene, "2").is_err());
        assert!(validate_deletion(&scene, "3").is_ok());
    }

    #[test]
    fn wildlife_form_reads_settings_and_names_links() {
        let scene = evolve_core::parse_level_scene(&[("mission", WILDLIFE)]).unwrap();
        let mut fields = Fields::default();
        fields.sync(Some(&scene), Some(&scene.objects[0]));
        assert_eq!(fields.wildlife_category, "Default");
        assert_eq!(fields.wildlife_links, "circle");
        fields.wildlife_links = "circle, 3".into();
        assert!(matches!(
            fields.links(&scene, &scene.objects[0]),
            Ok(LayerAuthoringEdit::Links { targets, .. }) if targets == ["2", "3"]
        ));
        fields.wildlife_links = "twin".into();
        assert!(
            fields
                .links(&scene, &scene.objects[0])
                .unwrap_err()
                .contains("EntityId")
        );
        fields.wildlife_links = "zone".into();
        assert!(fields.links(&scene, &scene.objects[0]).is_err());
        assert!(fields.links(&scene, &scene.objects[4]).is_err());

        fields.sync(Some(&scene), Some(&scene.objects[4]));
        assert!(fields.zone_terrain);
        fields.zone_keywords = " ArmadonBull ,NoElites ".into();
        assert!(matches!(
            fields.wildlife(&scene.objects[4]),
            Ok(LayerAuthoringEdit::Wildlife {
                settings: WildlifeSettings::Zone { keywords, .. },
                ..
            }) if keywords == "ArmadonBull, NoElites"
        ));

        fields.sync(Some(&scene), Some(&scene.objects[1]));
        assert_eq!(fields.wildlife_radius, "10");
        fields.wildlife_radius = "far".into();
        assert!(fields.wildlife(&scene.objects[1]).is_err());
        fields.wildlife_radius = "9000".into();
        assert!(fields.wildlife(&scene.objects[1]).is_err());
    }
}
