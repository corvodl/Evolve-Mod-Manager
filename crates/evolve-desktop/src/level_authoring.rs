//! Project-only Level Editor form values and validated source-file commands.
//! Read-only map previews never pass these edits to worker operations.
use evolve_core::PlacedObject;
use evolve_core::scene_authoring::{LayerAuthoringEdit, SpawnPropertiesEdit};
use evolve_core::TerrainBrushEdit;

#[derive(Clone, Copy, Debug)]
pub(super) enum TextField {
    Populations, HomeArea, SpawnWeight, SpawnGroup, SpawnHealth, Polygon,
    BrushX, BrushY, BrushRadius, BrushDeltaHeight, BrushLayerId,
}

#[derive(Debug)]
pub(super) struct Fields {
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
            populations: String::new(), home_area: String::new(),
            spawn_weight: "1".into(), spawn_group: "CreatureSpawns".into(),
            spawn_health: "100".into(), spawn_enabled: true,
            spawn_on_round_start: false, polygon: String::new(),
            brush_x: "0".into(), brush_y: "0".into(), brush_radius: "2".into(),
            brush_delta_height: "1".into(), brush_layer_id: "0".into(),
        }
    }
}

impl Fields {
    pub fn sync(&mut self, selected: Option<&PlacedObject>) {
        self.populations.clear();
        self.home_area.clear();
        self.polygon.clear();
        let Some(object) = selected else { return; };
        self.populations = object.populations.join(", ");
        self.home_area.clone_from(&object.home_area);
        self.spawn_weight = object.wildlife.spawn_weight.unwrap_or(1.0).to_string();
        self.spawn_health = object.wildlife.initial_health_percent.unwrap_or(100.0).to_string();
        self.spawn_enabled = object.wildlife.enabled.unwrap_or(true);
        self.spawn_on_round_start = object.wildlife.spawn_on_round_start.unwrap_or(false);
        self.spawn_group = if object.wildlife.group.is_empty() {
            "CreatureSpawns".into()
        } else {
            object.wildlife.group.clone()
        };
        self.polygon = object.shape_points.iter()
            .map(|p| p.iter().map(f32::to_string).collect::<Vec<_>>().join(","))
            .collect::<Vec<_>>().join("; ");
    }

    pub fn change(&mut self, key: TextField, value: String) {
        match key {
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
        let names = self.populations.split(',')
            .map(str::trim).map(str::to_owned).collect::<Vec<_>>();
        if names.is_empty() || names.len() > 4 {
            return Err("Select one to four authored wildlife population names.".into());
        }
        let mut populations = std::array::from_fn(|_| String::new());
        for (slot, value) in populations.iter_mut().zip(names) {
            *slot = value;
        }
        let spawn_weight = self.spawn_weight.trim().parse::<f32>()
            .map_err(|_| "Spawn weight must be a number")?;
        let initial_health_percent = self.spawn_health.trim().parse::<f32>()
            .map_err(|_| "Initial health must be a percent")?;
        Ok(LayerAuthoringEdit::Spawn {
            guid,
            settings: SpawnPropertiesEdit {
                populations,
                home_area: self.home_area.trim().to_owned(),
                spawn_weight, enabled: self.spawn_enabled,
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
            if raw.is_empty() { continue; }
            let xyz = raw.split(',').map(|p| p.trim().parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Polygon coordinates must be numbers")?;
            let arr: [f32; 3] = xyz.try_into()
                .map_err(|_| "Each polygon vertex requires x,y,z")?;
            points.push(arr);
        }
        Ok(LayerAuthoringEdit::Shape { guid, points })
    }

    pub fn brush(&self, paint: bool) -> Result<TerrainBrushEdit, String> {
        let x = self.brush_x.trim().parse::<u32>().map_err(|_| "Invalid X terrain sample")?;
        let y = self.brush_y.trim().parse::<u32>().map_err(|_| "Invalid Y terrain sample")?;
        let radius = self.brush_radius.trim().parse::<u32>().map_err(|_| "Invalid brush radius")?;
        if paint {
            let layer_id = self.brush_layer_id.trim().parse::<u8>()
                .map_err(|_| "Terrain LayerId must be an unsigned byte")?;
            Ok(TerrainBrushEdit::Paint { x, y, radius, layer_id })
        } else {
            let delta_height = self.brush_delta_height.trim().parse::<f32>()
                .map_err(|_| "Invalid terrain height delta")?;
            Ok(TerrainBrushEdit::Sculpt { x, y, radius, delta_height })
        }
    }
}
