//! Blender-compatible level scene from exact CryEngine source geometry.
//! Source terrain and level placements are imported into a single GLB, with
//! one separately editable Blender node per placement and original UV0.
//! Missing render dependencies fail closed, instead of faking a whole map.
use crate::archive::safe_member_name;
use crate::dds::{DdsFormat, decode_dds_rgba, decode_dds_rgba_with_parts};
use crate::glb::{GlbError, geometry_glb_bytes};
use crate::materials::{MaterialDocument, MaterialSlot, read_material};
use crate::model::{ModelError, ModelInfo, RenderMesh, original_mesh_subsets, read_model, read_world_model_parts_with_subsets, WorldModelPart};
use crate::scene::{LevelScene, SceneError, load_level_scene};
use crate::terrain::{MAX_RENDER_GRID, TerrainError, TerrainMesh, TerrainPaintMap, read_terrain, read_terrain_paint};
use image::{DynamicImage, ImageFormat};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const MAX_SCENE_GLB: usize = 768 * 1024 * 1024;
const MAX_SCENE_INSTANCES: usize = 50000;
const MAX_SCENE_GEOMETRIES: usize = 4000;

#[derive(Debug, thiserror::Error)]
pub enum LevelExportError {
    #[error("Level scene invalid: {0}")]
    Scene(#[from] SceneError),
    #[error("Source terrain not decodable: {0}")]
    Terrain(#[from] TerrainError),
    #[error("One of the source geometry files is invalid: {0}")]
    Model(#[from] ModelError),
    #[error("Mesh GLB failed validation: {0}")]
    Mesh(#[from] GlbError),
    #[error("Missing or unsafe level asset: {0}")]
    Missing(String),
    #[error("Source material/texture export failed: {0}")]
    Material(String),
    #[error("Level scene exceeds GLB export limits")]
    Limits,
    #[error("Output already exists")]
    Exists,
    #[error("Level scene export I/O: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid glTF intermediate: {0}")]
    Json(#[from] serde_json::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelExportReport {
    pub source_objects: usize,
    pub exported_mesh_resources: usize,
    pub exported_mesh_instances: usize,
    pub metadata_only_objects: usize,
    pub included_terrain: bool,
    /// Actual source-authored point/spot light objects exported via KHR_lights_punctual.
    #[serde(default)]
    pub exported_lights: usize,
    /// Number of separately selectable terrain tile meshes.
    #[serde(default)]
    pub terrain_tiles: usize,
    /// Authored categorical paint LayerIds represented as selectable materials.
    #[serde(default)]
    pub terrain_painted_layer_ids: usize,
    /// Full source sample fidelity; false when the terrain decoder had to stride.
    #[serde(default)]
    pub terrain_source_resolution: bool,
    pub output_bytes: usize,
    /// Only recognized and verified render geometry is exported. Materials,
    /// vegetation instance payloads, collision and game scripting are separate.
    pub original_geometry_only: bool,
    #[serde(default)]
    pub authored_material_slots: usize,
    #[serde(default)]
    pub embedded_textures: usize,
    #[serde(default)]
    pub missing_material_sets: usize,
    #[serde(default)]
    pub missing_texture_references: usize,
}

const MAX_LIGHT_SOURCES: usize = 2_048;
const MAX_LIGHT_LAYER_BYTES: usize = 48 * 1024 * 1024;

/// Export candidate extracted exclusively from an original Light/LightEntity
/// XML element and matched to the scene's resolved world-transform GUID.
#[derive(Clone, Debug)]
struct ExportLight {
    id: String,
    name: String,
    world: crate::scene::Transform,
    color: [f32; 3],
    intensity: f32,
    radius: f32,
    cone_degrees: Option<f32>,
    bulb_size: Option<f32>,
}

#[derive(Default)]
struct PendingLight {
    id: String,
    depth: usize,
    radius: Option<f32>,
    color: Option<[f32; 3]>,
    intensity: Option<f32>,
    cone_degrees: Option<f32>,
    bulb_size: Option<f32>,
    enabled: bool,
}
fn light_attr(node: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    node.attributes()
        .flatten()
        .find(|attr| attr.key.as_ref().eq_ignore_ascii_case(key))
        .and_then(|attr| attr.unescape_value().ok().map(|text| text.into_owned()))
}
fn light_value(node: &BytesStart<'_>, keys: &[&[u8]]) -> Option<f32> {
    keys.iter()
        .find_map(|key| light_attr(node, key))
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
}
fn light_color(node: &BytesStart<'_>) -> Option<[f32; 3]> {
    let text = [
        b"DiffuseColor".as_slice(),
        b"clrDiffuse",
        b"Color",
        b"color",
    ]
    .iter()
    .find_map(|key| light_attr(node, key))?;
    let rgb = text
        .split(',')
        .map(|v| v.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    if rgb.len() != 3 {
        return None;
    }
    let c = [rgb[0]?, rgb[1]?, rgb[2]?];
    c.iter()
        .all(|v| v.is_finite() && (0.0..=16.0).contains(v))
        .then_some(c)
}
impl PendingLight {
    fn read_props(&mut self, node: &BytesStart<'_>) {
        if light_attr(node, b"bFakeLight").is_some_and(|x| x == "1") {
            self.enabled = false;
        }
        if light_attr(node, b"bActive")
            .or_else(|| light_attr(node, b"Active"))
            .is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        {
            self.enabled = false;
        }
        if let Some(radius) = light_value(node, &[b"Radius", b"fRadius", b"LightRadius"])
            .filter(|v| (0.1..=100_000.0).contains(v))
        {
            self.radius = Some(radius);
        }
        if let Some(intensity) = light_value(
            node,
            &[
                b"DiffuseMultiplier",
                b"fDiffuseMultiplier",
                b"LightIntensity",
                b"Intensity",
            ],
        )
        .filter(|v| (0.0..=1_000_000.0).contains(v))
        {
            self.intensity = Some(intensity);
        }
        if let Some(color) = light_color(node) {
            self.color = Some(color);
        }
        if let Some(fov) = light_value(node, &[b"ProjectorFov", b"fProjectorFov", b"FOV", b"fFov"])
            .filter(|fov| (1.0..179.0).contains(fov))
        {
            self.cone_degrees = Some(fov);
        }
        if let Some(size) =
            light_value(node, &[b"fAttenuationBulbSize"]).filter(|v| *v > 0.0 && *v < 100_000.0)
        {
            self.bulb_size = Some(size);
        }
    }
}
fn complete_light(
    current: &mut Option<PendingLight>,
    objects: &HashMap<String, &crate::scene::PlacedObject>,
    lights: &mut Vec<ExportLight>,
) {
    let Some(source) = current.take() else {
        return;
    };
    if !source.enabled {
        return;
    }
    let Some(actor) = objects.get(&source.id) else {
        return;
    };
    let (Some(radius), Some(intensity), Some(color)) =
        (source.radius, source.intensity, source.color)
    else {
        return;
    };
    if !actor.world.position.iter().all(|v| v.is_finite())
        || !actor.world.quaternion.iter().all(|v| v.is_finite())
    {
        return;
    }
    lights.push(ExportLight {
        id: actor.id.clone(),
        name: actor.name.clone(),
        world: actor.world,
        color,
        intensity,
        radius,
        cone_degrees: source.cone_degrees,
        bulb_size: source.bulb_size,
    });
}
/// Match GUIDs rather than guessing from names. Only positively authored
/// lights become Blender objects; flare-only, disabled, or incomplete records
/// are not replaced with artificial defaults.
fn authored_lights(
    source: &[u8],
    scene: &LevelScene,
) -> Result<Vec<ExportLight>, LevelExportError> {
    if source.len() > MAX_LIGHT_LAYER_BYTES {
        return Err(LevelExportError::Material(
            "Light layer exceeds supported size".into(),
        ));
    }
    let data = crate::cryxml::xml_bytes(source, MAX_LIGHT_LAYER_BYTES)
        .map_err(|e| LevelExportError::Material(format!("Light XML: {e}")))?;
    let objects = scene
        .objects
        .iter()
        .filter(|o| !o.hidden)
        .map(|o| (o.id.to_ascii_lowercase(), o))
        .collect::<HashMap<_, _>>();
    let mut reader = Reader::from_reader(data.as_ref());
    let (mut depth, mut pending, mut result) = (0usize, None, Vec::new());
    loop {
        let event = reader
            .read_event()
            .map_err(|e| LevelExportError::Material(format!("Light XML: {e}")))?;
        let starts = matches!(&event, Event::Start(_));
        match event {
            Event::Start(node) | Event::Empty(node) => {
                let current_depth = depth + 1;
                if current_depth > 128 {
                    return Err(LevelExportError::Limits);
                }
                let tag = node.name();
                let entity = tag.as_ref().eq_ignore_ascii_case(b"Object")
                    || tag.as_ref().eq_ignore_ascii_case(b"Entity");
                if entity {
                    let class = light_attr(&node, b"EntityClass")
                        .or_else(|| light_attr(&node, b"Class"))
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    let kind = light_attr(&node, b"Type")
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    let is_light = matches!(
                        class.as_str(),
                        "light" | "lightentity" | "pointlight" | "spotlight"
                    ) || (kind == "light" && class.is_empty());
                    if is_light {
                        if let Some(id) =
                            light_attr(&node, b"Id").or_else(|| light_attr(&node, b"EntityId"))
                        {
                            let mut light = PendingLight {
                                id: id.to_ascii_lowercase(),
                                depth: current_depth,
                                enabled: true,
                                ..Default::default()
                            };
                            light.read_props(&node);
                            pending = Some(light);
                        }
                    }
                } else if let Some(light) = pending.as_mut()
                    && current_depth > light.depth
                    && current_depth <= light.depth + 7
                {
                    light.read_props(&node);
                }
                if starts {
                    depth += 1;
                } else if entity && pending.as_ref().is_some_and(|p| p.depth == current_depth) {
                    complete_light(&mut pending, &objects, &mut result);
                }
                if result.len() > MAX_LIGHT_SOURCES {
                    return Err(LevelExportError::Limits);
                }
            }
            Event::End(_) => {
                if pending.as_ref().is_some_and(|p| p.depth == depth) {
                    complete_light(&mut pending, &objects, &mut result);
                }
                depth = depth.saturating_sub(1);
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(LevelExportError::Material("Unsafe light XML entity".into()));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}
/// CryEngine spotlights shine along local +X; glTF punctual spots shine
/// along local -Z. Rotate only the light node; the source mesh remains intact.
fn light_spot_rotation(source: [f32; 4]) -> [f32; 4] {
    let [w, x, y, z] = source;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let [p, q, r, t] = [h, 0.0, -h, 0.0];
    [
        w * p - x * q - y * r - z * t,
        w * q + x * p + y * t - z * r,
        w * r - x * t + y * p + z * q,
        w * t + x * r - y * q + z * p,
    ]
}

/// Reconstruct the positive tangent-space Z channel for the two authored
/// normal components in BC5. This changes only the PNG for Blender's
/// tangent-space normal slot: diffuse/packed R-G data is never reclassified.
/// No Y inversion is applied without in-game shader verification.
fn reconstruct_bc5_normal(rgba:&mut image::RgbaImage) {
    for pixel in rgba.pixels_mut() {
        let x=f32::from(pixel[0])/127.5-1.0;
        let y=f32::from(pixel[1])/127.5-1.0;
        let z=(1.0-x*x-y*y).max(0.0).sqrt();
        let len=(x*x+y*y+z*z).sqrt().max(f32::EPSILON);
        pixel[2]=((z/len+1.0)*127.5).round().clamp(0.0,255.0) as u8;
        pixel[3]=255;
    }
}

struct Assembler {
    document: Value,
    binary: Vec<u8>,
    meshes: usize,
    instances: usize,
    cached_images: HashMap<String, usize>,
    missing_material_sets: usize,
    missing_texture_references: usize,
    lights: usize,
    paint_materials: HashMap<u8,usize>,
    paint_map_texture: Option<usize>,
}
impl Assembler {
    fn new(label: &str) -> Self {
        let document = json!({
            "asset":{"version":"2.0","generator":"Evolve Mod Manager Rust Level Editor"},
            "scene":0,
            "scenes":[{"name":label,"nodes":[0]}],
            "nodes":[{
                "name":"CryEngine world Z-up to Blender Y-up",
                "rotation":[-std::f32::consts::FRAC_1_SQRT_2,0,0,std::f32::consts::FRAC_1_SQRT_2],
                "children":[]
            }],
            "meshes":[], "bufferViews":[], "accessors":[],
            "materials":[], "images":[], "textures":[],
            "buffers":[{"byteLength":0}],
            "extras":{"source":"CryEngine Stage 2 level","geometry_only":true}
        });
        Self {
            document,
            binary: Vec::new(),
            meshes: 0,
            instances: 0,
            cached_images: HashMap::new(),
            missing_material_sets: 0,
            missing_texture_references: 0,
            lights: 0,
            paint_materials: HashMap::new(),
            paint_map_texture: None,
        }
    }
    fn add_mesh(&mut self, mesh: &RenderMesh) -> Result<usize, LevelExportError> {
        if self.meshes >= MAX_SCENE_GEOMETRIES {
            return Err(LevelExportError::Limits);
        }
        let data = geometry_glb_bytes(mesh)?;
        let json_len = u32::from_le_bytes(
            data[12..16]
                .try_into()
                .map_err(|_| LevelExportError::Limits)?,
        ) as usize;
        let source: Value = serde_json::from_slice(
            data.get(20..20 + json_len)
                .ok_or(LevelExportError::Limits)?,
        )?;
        let bin_start = 20 + json_len + 8;
        let raw = data.get(bin_start..).ok_or(LevelExportError::Limits)?;
        if self
            .binary
            .len()
            .checked_add(raw.len())
            .is_none_or(|n| n > MAX_SCENE_GLB)
        {
            return Err(LevelExportError::Limits);
        }
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        let offset = self.binary.len();
        self.binary.extend_from_slice(raw);
        let access_offset = self.document["accessors"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        let view_offset = self.document["bufferViews"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        let views = source["bufferViews"]
            .as_array()
            .ok_or(LevelExportError::Limits)?;
        for item in views {
            let mut v = item.clone();
            if v["buffer"].as_u64() != Some(0) {
                return Err(LevelExportError::Limits);
            }
            v["byteOffset"] = json!(
                v["byteOffset"]
                    .as_u64()
                    .unwrap_or(0)
                    .checked_add(offset as u64)
                    .ok_or(LevelExportError::Limits)?
            );
            self.document["bufferViews"]
                .as_array_mut()
                .ok_or(LevelExportError::Limits)?
                .push(v);
        }
        let accessors = source["accessors"]
            .as_array()
            .ok_or(LevelExportError::Limits)?;
        for item in accessors {
            let mut v = item.clone();
            if !v["sparse"].is_null() {
                return Err(LevelExportError::Limits);
            }
            let n = v["bufferView"].as_u64().ok_or(LevelExportError::Limits)?;
            v["bufferView"] = json!(n + view_offset as u64);
            self.document["accessors"]
                .as_array_mut()
                .ok_or(LevelExportError::Limits)?
                .push(v);
        }
        let mut primitives = source["meshes"][0]["primitives"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .clone();
        for p in &mut primitives {
            let attrs = p["attributes"]
                .as_object_mut()
                .ok_or(LevelExportError::Limits)?;
            for index in attrs.values_mut() {
                *index =
                    json!(index.as_u64().ok_or(LevelExportError::Limits)? + access_offset as u64);
            }
            p["indices"] = json!(
                p["indices"].as_u64().ok_or(LevelExportError::Limits)? + access_offset as u64
            );
        }
        let id = self.meshes;
        self.document["meshes"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"name":mesh.info.format,"primitives":primitives}));
        self.meshes += 1;
        Ok(id)
    }
    /// Each source map is embedded as PNG at its original decoded mip dimensions.
    /// We deliberately do not fabricate unknown maps or borrow same-name DDS
    /// files from a different extraction root.
    fn source_texture<R>(
        &mut self,
        resolve: &mut R,
        virtual_path: &str,
        normal_map: bool,
    ) -> Result<Option<usize>, LevelExportError>
    where
        R: FnMut(&str) -> Result<Option<PathBuf>, LevelExportError>,
    {
        let cache_key = format!("{}:{}",virtual_path.to_ascii_lowercase(),normal_map);
        if let Some(index) = self.cached_images.get(&cache_key) {
            return Ok(Some(*index));
        }
        let source = if let Some(source) = resolve(virtual_path)? {
            Some(source)
        } else if virtual_path.to_ascii_lowercase().ends_with(".dds") {
            resolve(&format!("{virtual_path}.0"))?
        } else {
            None
        };
        let Some(source) = source else {
            self.missing_texture_references += 1;
            return Ok(None);
        };
        // The top mip may reside in .dds.1..N, including another PAK
        // from the exact same installed game. Ask the verified resolver for
        // every numbered candidate; the DDS decoder checks contiguity and
        // byte lengths before permitting the reconstructed image.
        let decoded=match decode_dds_rgba(&source) {
            Ok(decoded)=>Ok(decoded),
            Err(initial) if source.file_name().and_then(|n|n.to_str())
                .is_some_and(|n|n.to_ascii_lowercase().ends_with(".dds.0")) => {
                let mut external=Vec::new();
                for n in 1..20 {
                    let candidate=format!("{virtual_path}.{n}");
                    if let Some(part)=resolve(&candidate)? {
                        if part.parent()!=source.parent() {
                            external.push(part);
                        }
                    }
                }
                if external.is_empty(){Err(initial)}
                else {decode_dds_rgba_with_parts(&source,&external)}
            }
            Err(error)=>Err(error),
        };
        let Ok((info,mut rgba))=decoded else {
            self.missing_texture_references+=1;
            return Ok(None);
        };
        if normal_map && info.format==DdsFormat::Bc5 {
            reconstruct_bc5_normal(&mut rgba);
        }
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|error| LevelExportError::Material(error.to_string()))?;
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        if self
            .binary
            .len()
            .checked_add(png.len())
            .is_none_or(|n| n > MAX_SCENE_GLB)
        {
            return Err(LevelExportError::Limits);
        }
        let offset = self.binary.len();
        self.binary.extend_from_slice(&png);
        let view = self.document["bufferViews"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["bufferViews"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":png.len()}));
        let image = self.document["images"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["images"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"name":virtual_path,"bufferView":view,"mimeType":"image/png"}));
        let index = self.document["textures"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["textures"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"source":image}));
        self.cached_images.insert(cache_key, index);
        Ok(Some(index))
    }

    fn require_extension(&mut self,name:&str)->Result<(),LevelExportError> {
        if self.document.get("extensionsUsed").is_none() {
            self.document["extensionsUsed"]=json!([]);
        }
        let extensions=self.document["extensionsUsed"].as_array_mut()
            .ok_or(LevelExportError::Limits)?;
        if !extensions.iter().any(|entry|entry.as_str()==Some(name)) {
            extensions.push(json!(name));
        }
        Ok(())
    }

    /// Preserve authored leaf slot order, including NoDraw holes. glTF maps
    /// established texture channels directly; other CryEngine channels remain
    /// explicitly identified in extras, rather than assigned guessed semantics.
    fn source_material<R>(
        &mut self,
        slot: &MaterialSlot,
        resolve: &mut R,
    ) -> Result<usize, LevelExportError>
    where
        R: FnMut(&str) -> Result<Option<PathBuf>, LevelExportError>,
    {
        let mut pbr = json!({"metallicFactor":0.0,"roughnessFactor":1.0});
        if let Some(tint) = slot.diffuse_tint {
            pbr["baseColorFactor"] =
                json!([tint[0].clamp(0.,1.),tint[1].clamp(0.,1.),tint[2].clamp(0.,1.),slot.opacity.unwrap_or(1.0)]);
        } else if let Some(opacity) = slot.opacity {
            pbr["baseColorFactor"] = json!([1.0, 1.0, 1.0, opacity]);
        }
        let mut value = json!({
            "name": slot.name,
            "pbrMetallicRoughness": pbr,
            "extras": {
                "cryengine_shader": slot.shader,
                "cryengine_original_maps": {},
                "cryengine_shininess": slot.shininess,
                "cryengine_glow_amount": slot.glow_amount,
                "cryengine_original_slot_name":slot.name,
                "cryengine_alpha_glow":slot.alpha_glow,
                "cryengine_ambient_multiplier":slot.ambient_multiplier,
                "cryengine_shader_gen_mask":slot.shader_gen_mask,
                "cryengine_specular_tint":slot.specular_tint,
                "cryengine_diffuse_tint":slot.diffuse_tint,
                "cryengine_alpha_test":slot.alpha_test,
                "cryengine_opacity":slot.opacity,
                "cryengine_diffuse_tex_mod":slot.diffuse_tex_mod,
                "cryengine_decal_tex_mod":slot.decal_tex_mod,
                "cryengine_layer_params":slot.layer_params,
                "cryengine_detail_tile":slot.detail_tile,
                "cryengine_foliage_transmit":slot.foliage_transmit,
                "cryengine_indirect_color":slot.indirect_color,
                "cryengine_pbr_mapping":"glTF is a neutral approximation of CryEngine parameters"
            }
        });
        // CryEngine's authored NoDraw leaf still occupies an original
        // material slot. Preserve its polygons for editing, but never render
        // them as an opaque white placeholder in Blender.
        if slot.shader.eq_ignore_ascii_case("NoDraw") {
            value["alphaMode"] = json!("BLEND");
            value["pbrMetallicRoughness"]["baseColorFactor"] =
                json!([1.0, 1.0, 1.0, 0.0]);
            value["extras"]["cryengine_no_draw"] = json!(true);
        } else if let Some(alpha_test) = slot.alpha_test {
            value["alphaMode"] = json!("MASK");
            value["alphaCutoff"] = json!(alpha_test);
        } else if slot.opacity.is_some_and(|x| x < 1.0) {
            value["alphaMode"] = json!("BLEND");
        }
        if let Some(color)=slot.emissive_color {
            let strength=color.iter().copied().fold(1f32,f32::max);
            value["emissiveFactor"]=json!(color.map(|channel|channel/strength));
            if strength>1. {
                value["extensions"]["KHR_materials_emissive_strength"]=
                    json!({"emissiveStrength":strength});
                self.require_extension("KHR_materials_emissive_strength")?;
            }
        }
        if let Some(tint)=slot.specular_tint {
            value["extensions"]["KHR_materials_specular"]=
                json!({"specularColorFactor":tint.map(|value|value.clamp(0.,1.))});
            self.require_extension("KHR_materials_specular")?;
        }
        for reference in &slot.maps {
            let Some(texture) = self.source_texture(
                resolve,&reference.path,
                matches!(reference.role.as_str(),"normals"|"normal"|"bumpmap")
            )? else {
                continue;
            };
            let role = reference.role.as_str();
            value["extras"]["cryengine_original_maps"][role] = json!({
                "texture":texture,"source":reference.path
            });
            match role {
                "diffuse" => {
                    value["pbrMetallicRoughness"]["baseColorTexture"]=json!({"index":texture});
                    if let Some(tf)=slot.diffuse_tex_mod {
                        let [su,sv,ou,ov]=tf.sample(0.);
                        value["pbrMetallicRoughness"]["baseColorTexture"]
                            ["extensions"]["KHR_texture_transform"]=
                            json!({"scale":[su,sv],"offset":[ou,ov]});
                        self.require_extension("KHR_texture_transform")?;
                    }
                }
                "normals" | "normal" | "bumpmap" => {
                    // BC5 X/Y normal reconstruction and shader tangent orientation
                    // must be validated on real assets before claiming FBX parity.
                    value["normalTexture"] = json!({"index":texture});
                }
                "specular" | "specularcolor" => {
                    value["extensions"]["KHR_materials_specular"]
                        ["specularColorTexture"]=json!({"index":texture});
                    self.require_extension("KHR_materials_specular")?;
                }
                "emissive" | "emittance" | "emission" | "glow" | "emissivecolor"
                | "emissionmap" | "glowmap" | "selfillum" | "selfillumination" => {
                    value["emissiveTexture"] = json!({"index":texture});
                    if value.get("emissiveFactor").is_none() {
                        value["emissiveFactor"] = json!([1.0, 1.0, 1.0]);
                    }
                }
                _ => {} // Extra channels remain traceable, never silently reclassified.
            }
        }
        let index = self.document["materials"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        // A unique material name lets Blender recover the exact MTL slot
        // metadata even when two models reuse a generic "Default" leaf.
        value["name"]=json!(format!("Evolve_MTL_{index:06} - {}",slot.name));
        self.document["materials"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(value);
        Ok(index)
    }

    /// Split source triangles by their verified CryEngine material subset IDs.
    /// In the absence of a verified subset table only single-leaf materials may
    /// be bound; multi-leaf guesses would silently color the wrong polygons.
    fn add_textured_mesh(
        &mut self,
        mesh: &RenderMesh,
        source_bytes: &[u8],
        material: Option<&MaterialDocument>,
        root: &Path,
    ) -> Result<usize, LevelExportError> {
        self.add_textured_mesh_resolved(mesh,source_bytes,material,None,&mut |path| {
            Ok(exact_geometry(root,path).ok())
        })
    }
    fn add_textured_mesh_resolved<R>(
        &mut self,
        mesh: &RenderMesh,
        source_bytes: &[u8],
        material: Option<&MaterialDocument>,
        original_ranges: Option<&[(u32,u32,u32)]>,
        resolve: &mut R,
    ) -> Result<usize, LevelExportError>
    where
        R: FnMut(&str) -> Result<Option<PathBuf>, LevelExportError>,
    {
        let id = self.add_mesh(mesh)?;
        let Some(document) = material else {
            self.missing_material_sets += 1;
            return Ok(id);
        };
        let mut ids = Vec::with_capacity(document.slots.len());
        for slot in &document.slots {
            ids.push(self.source_material(slot, resolve)?);
        }
        let old = self.document["meshes"][id]["primitives"][0].clone();
        let ranges = original_ranges.map(|ranges|ranges.to_vec())
            .or_else(|| original_mesh_subsets(source_bytes, mesh.indices.len()).ok())
            .filter(|ranges| {
                !ranges.is_empty()
                    && ranges
                        .iter()
                        .all(|(_, count, _)| *count > 0 && count.is_multiple_of(3))
                    && ranges.iter().try_fold(0u32, |cursor, (first, count, _)| {
                        (*first == cursor)
                            .then(|| cursor.checked_add(*count))
                            .flatten()
                    }) == Some(mesh.indices.len() as u32)
            });
        let mut primitives = Vec::new();
        if let Some(ranges) = ranges {
            let accessor = old["indices"].as_u64().ok_or(LevelExportError::Limits)? as usize;
            let original = self.document["accessors"][accessor].clone();
            let base = original["byteOffset"].as_u64().unwrap_or(0);
            for (first, count, material_id) in ranges {
                let mut primitive = old.clone();
                let id = self.document["accessors"]
                    .as_array()
                    .ok_or(LevelExportError::Limits)?
                    .len();
                self.document["accessors"]
                    .as_array_mut()
                    .ok_or(LevelExportError::Limits)?
                    .push(json!({
                        "bufferView": original["bufferView"],
                        "byteOffset": base + u64::from(first)*4,
                        "componentType": 5125,
                        "count":count,
                        "type":"SCALAR"
                    }));
                primitive["indices"] = json!(id);
                if let Some(slot) = ids.get(material_id as usize) {
                    primitive["material"] = json!(slot);
                } else {
                    self.missing_material_sets += 1;
                }
                primitives.push(primitive);
            }
        } else {
            let mut primitive = old;
            if ids.len() == 1 {
                primitive["material"] = json!(ids[0]);
            } else {
                self.missing_material_sets += 1;
            }
            primitives.push(primitive);
        }
        self.document["meshes"][id]["primitives"] = json!(primitives);
        Ok(id)
    }

    fn add_node(
        &mut self,
        name: &str,
        mesh: Option<usize>,
        transform: Option<crate::scene::Transform>,
        metadata: Value,
    ) -> Result<(), LevelExportError> {
        if self.instances >= MAX_SCENE_INSTANCES {
            return Err(LevelExportError::Limits);
        }
        let mut value = json!({"name":name,"extras":metadata});
        if let Some(mesh) = mesh {
            value["mesh"] = json!(mesh);
        }
        if let Some(tf) = transform {
            value["translation"] = json!(tf.position);
            value["rotation"] = json!([
                tf.quaternion[1],
                tf.quaternion[2],
                tf.quaternion[3],
                tf.quaternion[0]
            ]);
            value["scale"] = json!(tf.scale);
        }
        let index = self.document["nodes"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["nodes"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(value);
        self.document["nodes"][0]["children"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!(index));
        self.instances += 1;
        Ok(())
    }
    /// glTF KHR_lights_punctual is supported by Blender's glTF importer.
    /// The raw CryEngine multiplier is kept in extras: glTF photometric
    /// intensity is not an assertion of native illumination parity.
    /// A source CGF can contain multiple render meshes. The original actor
    /// world transform belongs to one parent node, so moving one placement
    /// does not transform a different instance of that model.
    fn add_placement(
        &mut self,object:&crate::scene::PlacedObject,meshes:&[usize],
    ) -> Result<(),LevelExportError> {
        let metadata=json!({"source_id":object.id,"layer":object.layer,
            "type":object.kind,"parent_guid":object.parent,
            "material_reference":object.material,"hidden":object.hidden});
        if meshes.len()<=1 {
            return self.add_node(&object.name,meshes.first().copied(),Some(object.world),metadata);
        }
        self.add_node(&object.name,None,Some(object.world),metadata)?;
        let parent=self.document["nodes"].as_array().ok_or(LevelExportError::Limits)?.len()-1;
        self.document["nodes"][parent]["children"]=json!([]);
        for (i,mesh) in meshes.iter().copied().enumerate() {
            self.add_node(
                &format!("{} | Render part {}",object.name,i),
                Some(mesh),None,
                json!({"original_part":i,"source_id":object.id}),
            )?;
            let child=self.document["nodes"].as_array().ok_or(LevelExportError::Limits)?.len()-1;
            let detached=self.document["nodes"][0]["children"]
                .as_array_mut().ok_or(LevelExportError::Limits)?.pop();
            if detached!=Some(json!(child)) {return Err(LevelExportError::Limits);}
            self.document["nodes"][parent]["children"]
                .as_array_mut().ok_or(LevelExportError::Limits)?.push(json!(child));
        }
        Ok(())
    }

    /// Embed the *complete* authored LayerId bitmap as a nearest-neighbor
    /// PNG, with each original category byte in R. The original X-major
    /// CryEngine bitmap is transposed into ordinary PNG row-major coordinates
    /// only; no color mixing or palette guesses are performed.
    fn embed_paint_map(&mut self,paint:&TerrainPaintMap)
        ->Result<usize,LevelExportError> {
        if let Some(id)=self.paint_map_texture{return Ok(id);}
        let rgba=image::RgbaImage::from_fn(paint.width,paint.height,|x,y|{
            image::Rgba([paint.id_at(x,y).unwrap_or(255),0,0,255])
        });
        let mut png=Vec::new();
        DynamicImage::ImageRgba8(rgba).write_to(
            &mut Cursor::new(&mut png),ImageFormat::Png
        ).map_err(|e|LevelExportError::Material(e.to_string()))?;
        while !self.binary.len().is_multiple_of(4) {self.binary.push(0);}
        if self.binary.len().checked_add(png.len()).is_none_or(|n|n>MAX_SCENE_GLB) {
            return Err(LevelExportError::Limits);
        }
        let offset=self.binary.len();
        self.binary.extend_from_slice(&png);
        let view=self.document["bufferViews"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["bufferViews"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":png.len()}));
        let image=self.document["images"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["images"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"name":"Original CryEngine LayerId (categorical)",
                "bufferView":view,"mimeType":"image/png"}));
        if self.document.get("samplers").is_none() {
            self.document["samplers"]=json!([]);
        }
        let sampler=self.document["samplers"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["samplers"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"magFilter":9728,"minFilter":9728,"wrapS":33071,"wrapT":33071}));
        let texture=self.document["textures"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["textures"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"source":image,"sampler":sampler}));
        self.document["extras"]["original_categorical_terrain_paint"]=json!({
            "texture":texture,"width":paint.width,"height":paint.height,
            "encoding":"PNG R = exact original LayerId, row-major pixels",
            "layer_ids_not_surface_type_ids":true,
            "blending":"native CryEngine transition not applied"
        });
        self.paint_map_texture=Some(texture);
        Ok(texture)
    }

    /// Source LayerId is categorically distinct from SurfaceTypeID. This
    /// assigns original IDs to editable face material groups; it does NOT
    /// purport to replicate CryEngine's paint boundary blending.
    fn paint_material<R>(
        &mut self,id:u8,paint:&TerrainPaintMap,baked:Option<usize>,resolve:&mut R
    )->Result<usize,LevelExportError>
    where R:FnMut(&str)->Result<Option<PathBuf>,LevelExportError>
    {
        if let Some(&at)=self.paint_materials.get(&id) {return Ok(at);}
        let paint_texture=self.embed_paint_map(paint)?;
        let candidates=paint.layers.iter().filter(|l|l.id==id).collect::<Vec<_>>();
        let layer=(candidates.len()==1).then(||candidates[0]);
        let mut value=baked.map(|at|self.document["materials"][at].clone())
            .unwrap_or_else(||json!({"pbrMetallicRoughness":{"metallicFactor":0.,"roughnessFactor":1.}}));
        // Occupy the standard glTF occlusion slot with zero strength so
        // importers retain the exact categorical PNG without changing the
        // original baked terrain color or pretending it is ambient occlusion.
        value["occlusionTexture"]=json!({"index":paint_texture,"strength":0.0});
        value["name"]=json!(format!("Terrain LayerId {id} - {}",
            layer.map_or("unresolved",|l|l.name.as_str())));
        value["extras"]["cryengine_terrain_paint"]=json!({
            "source_layer_id":id,
            "authored_bindings":candidates.len(),
            "layer":layer.map(|l|json!({
                "name":l.name,"surface_type":l.surface_type,
                "filter_color":l.filter_color,"brightness":l.brightness,
                "tiling":l.tiling
            })),
            "face_assignment":"original categorical LayerId sampled at triangle center",
            "native_height_and_detail_blending":"not reproduced"
        });
        // Resolve only the authored Terrain.Layer MTL for this exact paint ID.
        // The baked map remains the base RGB; detail images and parameters
        // are preserved without assuming the game's high-pass blend equation.
        let authored_path=layer
            .map(|l|l.surface_type.trim().replace('\\',"/"))
            .filter(|path|!path.is_empty())
            .map(|path|if path.to_ascii_lowercase().ends_with(".mtl"){
                path
            }else{format!("{path}.mtl")});
        value["extras"]["cryengine_terrain_paint"]["mtl_reference"]=json!(authored_path);
        if let Some(path)=authored_path {
            if let Some(source)=resolve(&path)? {
                match read_material(&source) {
                    Ok(mtl)=>{
                        let slots=mtl.slots.iter().filter(|s|
                            s.shader.eq_ignore_ascii_case("Terrain.Layer")
                        ).collect::<Vec<_>>();
                        value["extras"]["cryengine_terrain_paint"]["mtl_layer_slot_count"]=
                            json!(slots.len());
                        if slots.len()==1 {
                            let slot=slots[0];
                            value["extras"]["cryengine_terrain_paint"]["layer_params"]=
                                json!(slot.layer_params);
                            value["extras"]["cryengine_terrain_paint"]["detail_tiling"]=
                                json!(slot.detail_tile);
                            value["extras"]["cryengine_terrain_paint"]["shininess"]=
                                json!(slot.shininess);
                            value["extras"]["cryengine_terrain_paint"]["native_shader"]=
                                json!(slot.shader);
                            for map in &slot.maps {
                                let normal=matches!(map.role.as_str(),
                                    "normals"|"normal"|"bumpmap");
                                if let Some(texture)=self.source_texture(resolve,&map.path,normal)?{
                                    value["extras"]["cryengine_terrain_paint"]["source_maps"]
                                        [map.role.as_str()]=json!({
                                            "texture":texture,"source":map.path
                                        });
                                    if normal {
                                        value["normalTexture"]=json!({"index":texture});
                                    }
                                }
                            }
                        }
                    }
                    Err(error)=>{
                        value["extras"]["cryengine_terrain_paint"]["mtl_error"]=
                            json!(error.to_string());
                    }
                }
            } else {
                value["extras"]["cryengine_terrain_paint"]["mtl_error"]=
                    json!("Missing exact-build authored terrain MTL");
            }
        }
        let at=self.document["materials"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["materials"].as_array_mut().ok_or(LevelExportError::Limits)?.push(value);
        self.paint_materials.insert(id,at);
        Ok(at)
    }

    /// Reorder this tile's index buffer into categorical paint-material face
    /// groups, without moving vertices, changing UV0, or adding guesswork.
    fn painted_tile(
        &mut self,mesh_id:usize,mesh:&RenderMesh,
        paint:&TerrainPaintMap,baked:Option<usize>,resolve:&mut impl FnMut(&str)->Result<Option<PathBuf>,LevelExportError>
    )->Result<(),LevelExportError> {
        let uv=mesh.uv0.as_ref().ok_or(LevelExportError::Limits)?;
        if paint.width<2 || paint.height<2 || uv.len()!=mesh.positions.len()
            || mesh.indices.len()%3!=0 {return Err(LevelExportError::Limits);}
        let mut groups=BTreeMap::<u8,Vec<u32>>::new();
        for tri in mesh.indices.chunks_exact(3) {
            let mut center=[0f32;2];
            for &vertex in tri {
                let point=uv.get(vertex as usize).ok_or(LevelExportError::Limits)?;
                center[0]+=point[0]/3.; center[1]+=point[1]/3.;
            }
            let x=(center[0].clamp(0.,1.)*(paint.width-1) as f32).round() as u32;
            let y=(center[1].clamp(0.,1.)*(paint.height-1) as f32).round() as u32;
            let id=paint.id_at(x,y).ok_or(LevelExportError::Limits)?;
            groups.entry(id).or_default().extend_from_slice(tri);
        }
        let base=self.document["meshes"][mesh_id]["primitives"][0].clone();
        let accessor=base["indices"].as_u64().ok_or(LevelExportError::Limits)? as usize;
        let info=&self.document["accessors"][accessor];
        if info["componentType"]!=5125 {return Err(LevelExportError::Limits);}
        let view=info["bufferView"].as_u64().ok_or(LevelExportError::Limits)? as usize;
        let offset=self.document["bufferViews"][view]["byteOffset"].as_u64()
            .and_then(|at|at.checked_add(info["byteOffset"].as_u64().unwrap_or(0)))
            .ok_or(LevelExportError::Limits)? as usize;
        if offset.checked_add(mesh.indices.len()*4).is_none_or(|end|end>self.binary.len()) {
            return Err(LevelExportError::Limits);
        }
        let info_offset=info["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut cursor=0usize;
        let mut prims=Vec::new();
        for (id,triangles) in groups {
            let material=self.paint_material(id,paint,baked,resolve)?;
            for (i,index) in triangles.iter().enumerate() {
                let at=offset+(cursor+i)*4;
                self.binary[at..at+4].copy_from_slice(&index.to_le_bytes());
            }
            let next=self.document["accessors"].as_array().ok_or(LevelExportError::Limits)?.len();
            self.document["accessors"].as_array_mut().ok_or(LevelExportError::Limits)?.push(json!({
                "bufferView":view,"byteOffset":info_offset+cursor*4,
                "componentType":5125,"type":"SCALAR","count":triangles.len()
            }));
            let mut p=base.clone();
            p["indices"]=json!(next);
            p["material"]=json!(material);
            prims.push(p);
            cursor+=triangles.len();
        }
        self.document["meshes"][mesh_id]["primitives"]=json!(prims);
        Ok(())
    }

    /// Use the authored baked terrain color, never a synthetic 1x1 texture.
    /// An absent or undecodable terrain package leaves untextured geometry.
    fn add_terrain_color(&mut self, rgba: image::RgbaImage) -> Result<usize, LevelExportError> {
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|e| LevelExportError::Material(e.to_string()))?;
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        if self
            .binary
            .len()
            .checked_add(png.len())
            .is_none_or(|n| n > MAX_SCENE_GLB)
        {
            return Err(LevelExportError::Limits);
        }
        let at = self.binary.len();
        self.binary.extend_from_slice(&png);
        let view = self.document["bufferViews"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["bufferViews"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"buffer":0,"byteOffset":at,"byteLength":png.len()}));
        let img = self.document["images"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["images"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"name":"Source baked TerrainTexture.pak","bufferView":view,"mimeType":"image/png"}));
        let tex = self.document["textures"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["textures"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(json!({"source":img}));
        let id = self.document["materials"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["materials"].as_array_mut().ok_or(LevelExportError::Limits)?.push(json!({
            "name":"CryEngine baked terrain color",
            "pbrMetallicRoughness":{"baseColorTexture":{"index":tex},"metallicFactor":0.0,"roughnessFactor":1.0},
            "extras":{"source":"TerrainTexture.pak","terrain_layers":"Not baked into the original atlas exporter"}
        }));
        Ok(id)
    }

    fn add_light(&mut self, light: &ExportLight) -> Result<(), LevelExportError> {
        let is_spot = light.cone_degrees.is_some();
        let mut definition = json!({
            "name":light.name,
            "type":if is_spot {"spot"} else {"point"},
            "color":light.color,
            "intensity":light.intensity,
            "range":light.radius,
            "extras":{
                "source_id":light.id,
                "cryengine_diffuse_multiplier":light.intensity,
                "cryengine_radius":light.radius,
                "cryengine_bulb_size":light.bulb_size,
                "conversion_note":"glTF attenuation and photometry are approximate"
            }
        });
        if let Some(degrees) = light.cone_degrees {
            definition["spot"] = json!({
                "innerConeAngle":0.0,
                "outerConeAngle":(degrees.to_radians()*0.5)
            });
        }
        if self.document.get("extensions").is_none() {
            self.document["extensions"] = json!({"KHR_lights_punctual":{"lights":[]}});
            self.document["extensionsUsed"] = json!(["KHR_lights_punctual"]);
        }
        let index = self.document["extensions"]["KHR_lights_punctual"]["lights"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len();
        self.document["extensions"]["KHR_lights_punctual"]["lights"]
            .as_array_mut()
            .ok_or(LevelExportError::Limits)?
            .push(definition);
        self.add_node(
            &light.name,
            None,
            Some(light.world),
            json!({"source_id":light.id,"kind":"CryEngineLight",
                "original_rotation_wxyz":light.world.quaternion}),
        )?;
        let last = self.document["nodes"]
            .as_array()
            .ok_or(LevelExportError::Limits)?
            .len()
            - 1;
        self.document["nodes"][last]["extensions"] = json!({"KHR_lights_punctual":{"light":index}});
        if is_spot {
            let q = light_spot_rotation(light.world.quaternion);
            self.document["nodes"][last]["rotation"] = json!([q[1], q[2], q[3], q[0]]);
        }
        self.lights += 1;
        Ok(())
    }
    fn finish(mut self) -> Result<Vec<u8>, LevelExportError> {
        if self.meshes == 0 {
            return Err(LevelExportError::Missing(
                "No verified level geometry or heightmap".into(),
            ));
        }
        if self.binary.len() > MAX_SCENE_GLB {
            return Err(LevelExportError::Limits);
        }
        self.document["extras"]["geometry_only"] = json!(
            self.document["materials"]
                .as_array()
                .is_none_or(|m| m.is_empty())
        );
        self.document["extras"]["missing_material_sets"] = json!(self.missing_material_sets);
        self.document["extras"]["missing_texture_references"] =
            json!(self.missing_texture_references);
        self.document["buffers"][0]["byteLength"] = json!(self.binary.len());
        let mut json_data = serde_json::to_vec(&self.document)?;
        while !json_data.len().is_multiple_of(4) {
            json_data.push(b' ');
        }
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        let total = 12usize
            .checked_add(8)
            .and_then(|v| v.checked_add(json_data.len()))
            .and_then(|v| v.checked_add(8))
            .and_then(|v| v.checked_add(self.binary.len()))
            .ok_or(LevelExportError::Limits)?;
        if total > MAX_SCENE_GLB || total > u32::MAX as usize {
            return Err(LevelExportError::Limits);
        }
        let mut data = Vec::with_capacity(total);
        data.extend_from_slice(b"glTF");
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&(total as u32).to_le_bytes());
        data.extend_from_slice(&(json_data.len() as u32).to_le_bytes());
        data.extend_from_slice(b"JSON");
        data.extend_from_slice(&json_data);
        data.extend_from_slice(&(self.binary.len() as u32).to_le_bytes());
        data.extend_from_slice(b"BIN\0");
        data.extend_from_slice(&self.binary);
        Ok(data)
    }
}
/// Author-declared CrChF MtlName (0x08021014) references only.
fn embedded_mtl_names(bytes:&[u8])->Vec<String> {
    if bytes.len()<32 || bytes.get(..5)!=Some(b"CrChF"){return Vec::new();}
    let read=|at:usize|->Option<usize>{
        Some(u32::from_le_bytes(bytes.get(at..at+4)?.try_into().ok()?) as usize)
    };
    let (Some(count),Some(start))=(read(8),read(12)) else{return Vec::new();};
    if count==0 || count>10_000 || start<16
        || start.checked_add(count*16).is_none_or(|n|n>bytes.len()) {return Vec::new();}
    let mut names=Vec::<String>::new();
    for i in 0..count {
        let at=start+i*16;
        if read(at)!=Some(0x08021014){continue;}
        let (Some(len),Some(offset))=(read(at+8),read(at+12)) else{continue;};
        if len<132 || offset<start+count*16
            || offset.checked_add(len).is_none_or(|n|n>bytes.len()){continue;}
        let Some(field)=bytes.get(offset..offset+128) else{continue;};
        let Some(zero)=field.iter().position(|b|*b==0) else{continue;};
        let Ok(raw)=std::str::from_utf8(&field[..zero]) else{continue;};
        let name=raw.trim().replace('\\',"/");
        let name=if name.to_ascii_lowercase().ends_with(".mtl"){name}else{format!("{name}.mtl")};
        if safe_member_name(&name).is_err(){continue;}
        if !names.iter().any(|known|known.eq_ignore_ascii_case(&name)){names.push(name);}
    }
    names
}

/// Resolve explicit or embedded model MtlName dependencies with selected
/// build provenance. If multiple distinct original sources match, never
/// silently pick the first archive or a similar model basename.
fn source_model_material<R>(
    virtual_model:&str,explicit:&str,bytes:&[u8],resolve:&mut R
)->Result<Option<PathBuf>,LevelExportError>
where R:FnMut(&str)->Result<Option<PathBuf>,LevelExportError>
{
    if !explicit.trim().is_empty() {
        let key=explicit.trim().replace('\\',"/");
        let key=if key.to_ascii_lowercase().ends_with(".mtl"){key}else{format!("{key}.mtl")};
        return resolve(&key);
    }
    let hints=embedded_mtl_names(bytes);
    if !hints.is_empty() {
        let parent=virtual_model.rsplit_once('/').map_or("",|(folder,_)|folder);
        let mut unique=Vec::new();
        for hint in hints {
            let rooted=["objects/","materials/","characters/","levels/"]
                .iter().any(|prefix|hint.to_ascii_lowercase().starts_with(prefix));
            let mut candidates=Vec::new();
            if !rooted && !parent.is_empty() {
                candidates.push(format!("{parent}/{hint}"));
            }
            candidates.push(hint);
            for virtual_path in candidates {
                if let Some(path)=resolve(&virtual_path)?
                    && !unique.contains(&path) {
                    unique.push(path);
                }
            }
        }
        if unique.len()>1 {
            return Err(LevelExportError::Material(format!(
                "Conflicting authored MtlName references for {virtual_model}"
            )));
        }
        // Explicit MtlName exists. Missing means missing, not permission
        // to borrow an unrelated same-stem material.
        return Ok(unique.pop());
    }
    let stem=virtual_model.rsplit_once('.').map_or(virtual_model,|(stem,_)|stem);
    resolve(&format!("{stem}.mtl"))
}

fn exact_geometry(root: &Path, virtual_path: &str) -> Result<PathBuf, LevelExportError> {
    let normalized = safe_member_name(virtual_path)
        .map_err(|_| LevelExportError::Missing(virtual_path.into()))?;
    let mut candidate = root.to_owned();
    for part in normalized.split('/') {
        candidate.push(part);
        let meta = fs::symlink_metadata(&candidate)
            .map_err(|_| LevelExportError::Missing(normalized.clone()))?;
        if meta.file_type().is_symlink() {
            return Err(LevelExportError::Missing(normalized));
        }
    }
    let resolved = candidate.canonicalize()?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(LevelExportError::Missing(virtual_path.into()));
    }
    Ok(resolved)
}
const TERRAIN_TILE_QUADS: usize = 256;

/// Split the maximum decoded terrain grid into bounded source-UV0 mesh tiles.
/// All tile vertices stay in original absolute world coordinates, so adjacent
/// tiles are exactly coincident and Blender can edit them independently.
fn terrain_mesh_tiles(source: &TerrainMesh) -> Result<Vec<RenderMesh>, LevelExportError> {
    let stride = source.info.stride as usize;
    if stride == 0 {
        return Err(LevelExportError::Limits);
    }
    let sampled = |total: u32| {
        let mut positions = (0..total).step_by(stride).collect::<Vec<_>>();
        if positions.last().copied() != Some(total - 1) {
            positions.push(total - 1);
        }
        positions.len()
    };
    let nx = sampled(source.info.width);
    let ny = sampled(source.info.height);
    if nx.checked_mul(ny) != Some(source.vertices.len())
        || nx < 2
        || ny < 2
        || source.normals.len() != source.vertices.len()
        || source.uv.len() != source.vertices.len()
    {
        return Err(LevelExportError::Limits);
    }
    let mut result = Vec::new();
    for x0 in (0..nx - 1).step_by(TERRAIN_TILE_QUADS) {
        let x1 = (x0 + TERRAIN_TILE_QUADS).min(nx - 1);
        for y0 in (0..ny - 1).step_by(TERRAIN_TILE_QUADS) {
            let y1 = (y0 + TERRAIN_TILE_QUADS).min(ny - 1);
            let rows = y1 - y0 + 1;
            let cols = x1 - x0 + 1;
            let mut positions = Vec::with_capacity(rows * cols);
            let mut normals = Vec::with_capacity(rows * cols);
            let mut uv = Vec::with_capacity(rows * cols);
            for x in x0..=x1 {
                for y in y0..=y1 {
                    let index = x * ny + y;
                    positions.push(source.vertices[index]);
                    normals.push(source.normals[index]);
                    uv.push(source.uv[index]);
                }
            }
            let mut indices = Vec::with_capacity((cols - 1) * (rows - 1) * 6);
            for x in 0..cols - 1 {
                for y in 0..rows - 1 {
                    let a = (x * rows + y) as u32;
                    let b = a + rows as u32;
                    let c = a + 1;
                    indices.extend_from_slice(&[a, b, c, b, b + 1, c]);
                }
            }
            result.push(RenderMesh {
                vertex_alpha: None,
                info: ModelInfo {
                    format: format!("CryEngine Terrain Tile ({x0},{y0})"),
                    version: 1,
                    chunk_count: 0,
                    vertex_count: positions.len(),
                    triangle_count: indices.len() / 3,
                    original_uv0: true,
                    original_normals: true,
                },
                positions,
                indices,
                uv0: Some(uv),
                normals: Some(normals),
            });
        }
    }
    Ok(result)
}
fn original_terrain_color(source: &Path) -> Option<image::RgbaImage> {
    // The map's own TerrainTexture.pak is the only admissible baked color.
    // Other installed maps or similarly-named global DDS files are not used.
    let folder = source.parent()?;
    let map = if folder
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("LevelData")
    {
        folder.parent()?
    } else {
        folder
    };
    let color = map.join("TerrainTexture.pak");
    let meta = fs::symlink_metadata(&color).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    crate::terrain_tiles::read_terrain_texture_atlas(&color)
        .ok()
        .filter(|atlas| !atlas.incomplete)
        .map(|atlas| atlas.image)
}

/// Build a complete *verified geometry* scene from a level layer/mission XML,
/// an exact extraction root and optional source Heightmap.dat. Every referenced
/// render model must be present or the operation fails without creating a GLB.
pub fn export_level_scene_glb(
    placement: &Path,
    asset_root: &Path,
    terrain: Option<&Path>,
    destination: &Path,
) -> Result<LevelExportReport, LevelExportError> {
    if !placement.is_file() || fs::symlink_metadata(placement)?.file_type().is_symlink() {
        return Err(LevelExportError::Missing(
            "Select original mission XML or LYR".into(),
        ));
    }
    let scene = load_level_scene(placement)?;
    let bytes = fs::read(placement)?;
    let sources = vec![(
        placement
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        bytes,
    )];
    let label = placement
        .file_stem()
        .and_then(|p| p.to_str())
        .unwrap_or("CryEngine Level");
    let root = asset_root.canonicalize()?;
    export_level_scene_glb_with_scene(
        &scene,
        &sources,
        label,
        asset_root,
        terrain,
        destination,
        |virtual_path| Ok(exact_geometry(&root, virtual_path).ok()),
    )
}

/// Exports the exact multi-layer editor scene including its already-decoded
/// VegetationMap.dat placements. `resolve` must use verified same-install
/// asset-library paths; no project-wide basename fallback is allowed.
pub fn export_level_scene_glb_with_scene<R>(
    scene: &LevelScene,
    sources: &[(String, Vec<u8>)],
    label: &str,
    asset_root: &Path,
    terrain: Option<&Path>,
    destination: &Path,
    mut resolve: R,
) -> Result<LevelExportReport, LevelExportError>
where
    R: FnMut(&str) -> Result<Option<PathBuf>, LevelExportError>,
{
    let root = asset_root.canonicalize()?;
    if !root.is_dir() || fs::symlink_metadata(asset_root)?.file_type().is_symlink() {
        return Err(LevelExportError::Missing(
            "Select an actual extracted asset folder".into(),
        ));
    }
    if destination.extension().and_then(|e| e.to_str()) != Some("glb")
        || fs::symlink_metadata(destination).is_ok()
    {
        return Err(LevelExportError::Exists);
    }
    let parent = destination.parent().ok_or(LevelExportError::Exists)?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(LevelExportError::Exists);
    }
    let mut source_lights = Vec::new();
    for (_, bytes) in sources {
        source_lights.extend(authored_lights(bytes, scene)?);
    }
    let mut used = std::collections::HashSet::new();
    let lights = source_lights
        .into_iter()
        .filter(|light| used.insert(light.id.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    let mut assembler = Assembler::new(label);
    let mut resources = HashMap::<(String, String), Vec<usize>>::new();
    let mut geometry_instances = 0usize;
    let mut metadata_only = 0usize;
    let mut terrain_tiles = 0usize;
    let mut terrain_source_resolution = true;
    if let Some(source) = terrain {
        let terrain_data = read_terrain(source, MAX_RENDER_GRID)?;
        let paint=read_terrain_paint(source)?;
        terrain_source_resolution = terrain_data.info.stride == 1;
        let color = original_terrain_color(source)
            .map(|image| assembler.add_terrain_color(image))
            .transpose()?;
        for mesh in terrain_mesh_tiles(&terrain_data)? {
            let index = assembler.add_mesh(&mesh)?;
            if let Some(paint)=paint.as_ref() {
                assembler.painted_tile(index,&mesh,paint,color,&mut resolve)?;
            } else if let Some(material) = color {
                assembler.document["meshes"][index]["primitives"][0]["material"] = json!(material);
            }
            assembler.add_node(
                &mesh.info.format,
                Some(index),
                None,
                json!({"source":"Heightmap.dat","terrain_tile":true,
                    "source_sample_stride":terrain_data.info.stride}),
            )?;
            terrain_tiles += 1;
            geometry_instances += 1;
        }
    }
    for object in &scene.objects {
        let mesh_ids = if object.model.is_empty() {
            Vec::new()
        } else {
            let key=(object.model.to_ascii_lowercase(),object.material.to_ascii_lowercase());
            if let Some(ids)=resources.get(&key) {
                ids.clone()
            } else {
                let name=&object.model;
                let mut candidates=vec![name.clone()];
                let ext=Path::new(name).extension().and_then(|x|x.to_str())
                    .unwrap_or_default().to_ascii_lowercase();
                if matches!(ext.as_str(),"cgf"|"cga"|"chr"|"skin") {
                    candidates.push(Path::new(name).with_extension(format!("{ext}m"))
                        .to_string_lossy().to_string());
                }
                let mut loaded=None;
                let mut errors=Vec::new();
                for candidate in candidates {
                    if let Some(path)=resolve(&candidate)? {
                        if let Ok(parts)=read_world_model_parts_with_subsets(&path)
                            && !parts.is_empty()
                        {
                            loaded=Some((parts,path,candidate));
                            break;
                        }
                        match read_model(&path) {
                            Ok(mesh)=>{
                                loaded=Some((vec![WorldModelPart{
                                    mesh,subsets:Vec::new(),has_subset_table:false
                                }],path,candidate));
                                break;
                            }
                            Err(error)=>errors.push(format!("{candidate}: {error}")),
                        }
                    }
                }
                let (parts,path,virtual_source)=loaded.ok_or_else(||LevelExportError::Missing(
                    format!("{}: {}",object.model,errors.join("; "))
                ))?;
                let bytes=fs::read(&path)?;
                let authored=source_model_material(
                    &virtual_source,&object.material,&bytes,&mut resolve
                )?.and_then(|material|read_material(&material).ok());
                let mut ids=Vec::new();
                for part in parts {
                    // An explicit empty ranges slice means the source part
                    // has no table; do not borrow another mesh's subset IDs.
                    let known=Some(part.subsets.as_slice());
                    let index=assembler.add_textured_mesh_resolved(
                        &part.mesh,&bytes,authored.as_ref(),known,&mut resolve
                    )?;
                    ids.push(index);
                }
                resources.insert(key,ids.clone());
                ids
            }
        };
        if mesh_ids.is_empty() {metadata_only+=1;}
        else {geometry_instances+=1;}
        assembler.add_placement(object,&mesh_ids)?;
    }
    for light in &lights {
        assembler.add_light(light)?;
    }
    let assembler_mesh_count=assembler.meshes;
    let terrain_painted_layer_ids=assembler.paint_materials.len();
    let authored_material_slots = assembler.document["materials"]
        .as_array()
        .map_or(0, Vec::len);
    let embedded_textures = assembler.document["textures"]
        .as_array()
        .map_or(0, Vec::len);
    let missing_material_sets = assembler.missing_material_sets;
    let missing_texture_references = assembler.missing_texture_references;
    let bytes = assembler.finish()?;
    let mut stage = NamedTempFile::new_in(parent)?;
    stage.write_all(&bytes)?;
    stage.as_file().sync_all()?;
    stage.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == io::ErrorKind::AlreadyExists {
            LevelExportError::Exists
        } else {
            LevelExportError::Io(e.error)
        }
    })?;
    Ok(LevelExportReport {
        source_objects: scene.objects.len(),
        exported_mesh_resources: assembler_mesh_count,
        exported_mesh_instances: geometry_instances,
        metadata_only_objects: metadata_only,
        included_terrain: terrain.is_some(),
        exported_lights: lights.len(),
        terrain_tiles,
        terrain_painted_layer_ids,
        terrain_source_resolution,
        output_bytes: bytes.len(),
        original_geometry_only: authored_material_slots == 0,
        authored_material_slots,
        embedded_textures,
        missing_material_sets,
        missing_texture_references,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crchf_source_mtlname_is_preferred_over_lookalike_stem() {
        let mut chunk=vec![0u8;16+16+132];
        chunk[..5].copy_from_slice(b"CrChF");
        chunk[8..12].copy_from_slice(&1u32.to_le_bytes());
        chunk[12..16].copy_from_slice(&16u32.to_le_bytes());
        chunk[16..20].copy_from_slice(&0x08021014u32.to_le_bytes());
        chunk[24..28].copy_from_slice(&132u32.to_le_bytes());
        chunk[28..32].copy_from_slice(&32u32.to_le_bytes());
        let hint=b"materials/maps/verified.mtl";
        chunk[32..32+hint.len()].copy_from_slice(hint);
        assert_eq!(embedded_mtl_names(&chunk),["materials/maps/verified.mtl"]);
        let path=std::path::PathBuf::from("/verified/maps/material.mtl");
        let resolved=source_model_material(
            "objects/rock.cgf","",&chunk,&mut |virtual_path|{
                if virtual_path=="materials/maps/verified.mtl" {
                    Ok(Some(path.clone()))
                } else {Ok(None)}
            }
        ).unwrap();
        assert_eq!(resolved,Some(path));
        assert_eq!(source_model_material(
            "objects/rock.cgf","",&chunk,&mut |_key|Ok(None)
        ).unwrap(),None);
        // An exact authored MtlName cannot be replaced with a similarly
        // spelled basename or a same-stem MTL when it is unresolved.
        let malicious=b"../wrong/build.mtl";
        chunk[32..32+hint.len()].fill(0);
        chunk[32..32+malicious.len()].copy_from_slice(malicious);
        assert!(embedded_mtl_names(&chunk).is_empty());
    }
    #[test]
    fn painted_terrain_retains_source_categorical_layer_face_groups() {
        let mesh=RenderMesh {
            vertex_alpha:None,
            info:ModelInfo{
                format:"terrain fixture".into(),version:1,chunk_count:1,
                vertex_count:4,triangle_count:2,
                original_uv0:true,original_normals:true
            },
            positions:vec![[0.,0.,0.],[1.,0.,0.],[0.,1.,0.],[1.,1.,0.]],
            indices:vec![0,1,2,1,3,2],
            uv0:Some(vec![[0.,0.],[1.,0.],[0.,1.],[1.,1.]]),
            normals:Some(vec![[0.,0.,1.];4]),
        };
        let paint=TerrainPaintMap {
            width:2,height:2,
            layer_names:std::collections::BTreeMap::from([
                (3,"Grass".into()),(7,"Rock".into())
            ]),
            layers:vec![
                crate::terrain::TerrainLayerDefinition {
                    id:3,name:"Grass".into(),surface_type:"Materials/Grass".into(),
                    filter_color:None,brightness:None,tiling:None
                },
                crate::terrain::TerrainLayerDefinition {
                    id:7,name:"Rock".into(),surface_type:"Materials/Rock".into(),
                    filter_color:None,brightness:None,tiling:None
                },
            ],
            surface_types:vec![],
            ids:vec![3,3,7,7]
        };
        let mut result=Assembler::new("terrain");
        let id=result.add_mesh(&mesh).unwrap();
        result.painted_tile(id,&mesh,&paint,None).unwrap();
        let primitives=result.document["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(),2);
        assert_eq!(result.paint_materials.len(),2);
        assert_eq!(primitives.iter().map(|p|{
            result.document["accessors"][p["indices"].as_u64().unwrap() as usize]["count"]
                .as_u64().unwrap()
        }).sum::<u64>(),6);
        assert_eq!(result.document["materials"][0]["extras"]
            ["cryengine_terrain_paint"]["source_layer_id"],3);
        assert_eq!(result.document["materials"][1]["extras"]
            ["cryengine_terrain_paint"]["source_layer_id"],7);
    }
    #[test]
    fn bc5_reconstruction_keeps_xy_and_restores_positive_z_for_blender() {
        let mut image=image::RgbaImage::from_pixel(2,1,image::Rgba([128,128,0,74]));
        image.put_pixel(1,0,image::Rgba([255,128,0,9]));
        reconstruct_bc5_normal(&mut image);
        let center=image.get_pixel(0,0);
        assert_eq!(center[0],128);
        assert_eq!(center[1],128);
        assert_eq!(center[2],255);
        assert_eq!(center[3],255);
        let edge=image.get_pixel(1,0);
        assert_eq!(edge[0],255);
        assert!(edge[2]>=124 && edge[2]<=130);
    }
    #[test]
    fn source_nodraw_is_invisible_but_keeps_its_original_material_slot() {
        let material = crate::materials::parse_material(
            br#"<Material Name="Root"><SubMaterials>
                <Material Name="Visible" Shader="Illum" />
                <Material Name="Helper" Shader="NoDraw" />
            </SubMaterials></Material>"#,
        ).unwrap();
        assert_eq!(material.slots.len(), 2);
        let folder=tempfile::tempdir().unwrap();
        let mut exporter=Assembler::new("NoDraw test");
        for slot in &material.slots {
            exporter.source_material(slot,&mut |_path|Ok(None)).unwrap();
        }
        assert_eq!(exporter.document["materials"][0]["alphaMode"], serde_json::Value::Null);
        assert_eq!(exporter.document["materials"][1]["alphaMode"], "BLEND");
        assert_eq!(
            exporter.document["materials"][1]["pbrMetallicRoughness"]["baseColorFactor"][3],0.0
        );
        assert_eq!(exporter.document["materials"][1]["extras"]["cryengine_no_draw"],true);
        drop(folder);
    }
    #[test]
    fn multi_part_actor_has_one_movable_transform_parent() {
        let xml=br#"<Objects><Object Id="actor-a" Name="Tree"
            Pos="15,25,35" Scale="2,2,2"/></Objects>"#;
        let scene=crate::scene::parse_level_scene(&[("fixture",xml)]).unwrap();
        let obj=&scene.objects[0];
        let mesh=RenderMesh {
            vertex_alpha:None,
            info:ModelInfo{format:"fixture".into(),version:1,chunk_count:1,
                vertex_count:3,triangle_count:1,
                original_uv0:true,original_normals:false},
            positions:vec![[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]],
            indices:vec![0,1,2],uv0:Some(vec![[0.,0.],[1.,0.],[0.,1.]]),
            normals:None,
        };
        let mut export=Assembler::new("fixture");
        let a=export.add_mesh(&mesh).unwrap();
        let b=export.add_mesh(&mesh).unwrap();
        export.add_placement(obj,&[a,b]).unwrap();
        assert_eq!(export.document["nodes"][0]["children"],json!([1]));
        assert_eq!(export.document["nodes"][1]["translation"],json!([15.,25.,35.]));
        assert_eq!(export.document["nodes"][1]["scale"],json!([2.,2.,2.]));
        assert_eq!(export.document["nodes"][1]["children"],json!([2,3]));
        assert!(export.document["nodes"][2].get("translation").is_none());
        assert!(export.document["nodes"][3].get("translation").is_none());
    }
    #[test]
    fn full_grid_terrain_is_split_without_changing_original_uvs() {
        let w = 260usize;
        let h = 260usize;
        let size = w * h;
        let mut verts = Vec::with_capacity(size);
        let mut normals = Vec::with_capacity(size);
        let mut uv = Vec::with_capacity(size);
        for x in 0..w {
            for y in 0..h {
                verts.push([x as f32, y as f32, (x + y) as f32 * 0.1]);
                normals.push([0., 0., 1.]);
                uv.push([x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32]);
            }
        }
        let terrain = TerrainMesh {
            info: crate::terrain::TerrainInfo {
                width: w as u32,
                height: h as u32,
                unit: 1.,
                max_height: 100.,
                decoded_samples: size,
                stride: 1,
            },
            vertices: verts,
            indices: vec![],
            normals,
            uv,
        };
        let tiles = terrain_mesh_tiles(&terrain).unwrap();
        assert_eq!(tiles.len(), 4);
        assert_eq!(
            tiles
                .iter()
                .map(|tile| tile.indices.len() / 3)
                .sum::<usize>(),
            2 * (w - 1) * (h - 1)
        );
        for tile in &tiles {
            assert!(tile.positions.len() <= 257 * 257);
            for (position, texture) in tile.positions.iter().zip(tile.uv0.as_ref().unwrap()) {
                assert!((position[0] / (w - 1) as f32 - texture[0]).abs() < 1e-6);
                assert!((position[1] / (h - 1) as f32 - texture[1]).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn source_lights_export_as_moveable_gltf_punctual_nodes() {
        let data = br#"<Objects>
            <Object Id="LIGHT-1" Name="Lamp" Type="Entity" EntityClass="Light"
                Pos="11,12,13">
                <Properties Radius="20" DiffuseMultiplier="4"
                    DiffuseColor="0.25,0.5,1" ProjectorFov="80"
                    fAttenuationBulbSize="0.4" />
            </Object>
            <Object Id="FAKE" Name="Flare" Type="Entity" EntityClass="Light"
                Pos="0,0,0">
                <Properties Radius="10" DiffuseMultiplier="2"
                    DiffuseColor="1,1,1" bFakeLight="1"/>
            </Object>
        </Objects>"#;
        let scene = crate::scene::parse_level_scene(&[("fixture", data)]).unwrap();
        let lights = authored_lights(data, &scene).unwrap();
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].radius, 20.);
        let mut exporter = Assembler::new("Lamp fixture");
        exporter.add_light(&lights[0]).unwrap();
        assert_eq!(
            exporter.document["extensionsUsed"][0],
            "KHR_lights_punctual"
        );
        assert_eq!(
            exporter.document["extensions"]["KHR_lights_punctual"]["lights"][0]["type"],
            "spot"
        );
        assert_eq!(
            exporter.document["nodes"][1]["extensions"]["KHR_lights_punctual"]["light"],
            0
        );
        assert_eq!(
            exporter.document["nodes"][1]["translation"],
            json!([11., 12., 13.])
        );
    }
    #[test]
    fn spotlight_basis_maps_gltf_negative_z_to_cryengine_positive_x() {
        let [w, x, y, z] = light_spot_rotation([1., 0., 0., 0.]);
        let direction = [
            -2.0 * (x * z + w * y),
            -2.0 * (y * z - w * x),
            -(1.0 - 2.0 * (x * x + y * y)),
        ];
        assert!((direction[0] - 1.).abs() < 1e-5);
        assert!(direction[1].abs() < 1e-5);
        assert!(direction[2].abs() < 1e-5);
    }
    #[test]
    fn creates_distinct_instance_nodes_under_common_zup_basis() {
        let model = RenderMesh {
            vertex_alpha: None,
            info: ModelInfo {
                format: "test".into(),
                version: 7,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: false,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: None,
        };
        let mut scene = Assembler::new("test");
        let id = scene.add_mesh(&model).unwrap();
        scene
            .add_node("Object A", Some(id), None, json!({"id":"a"}))
            .unwrap();
        scene
            .add_node("Object B", Some(id), None, json!({"id":"b"}))
            .unwrap();
        let bytes = scene.finish().unwrap();
        let json_size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let json: Value = serde_json::from_slice(&bytes[20..20 + json_size]).unwrap();
        assert_eq!(json["meshes"].as_array().unwrap().len(), 1);
        assert_eq!(json["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(json["nodes"][1]["mesh"], json["nodes"][2]["mesh"]);
        assert!(json["nodes"][0]["rotation"].is_array());
    }
    #[test]
    fn single_source_leaf_becomes_a_real_gltf_material() {
        let model = RenderMesh {
            vertex_alpha: None,
            info: ModelInfo {
                format: "fixture".into(),
                version: 1,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: true,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: Some(vec![[0., 0., 1.]; 3]),
        };
        let root = tempfile::tempdir().unwrap();
        let authored = crate::materials::parse_material(
            br#"<Material Name="verified" Shader="Illum" Diffuse="1,1,1" Opacity="0.5"/>"#,
        )
        .unwrap();
        let mut exporter = Assembler::new("fixture");
        exporter
            .add_textured_mesh(&model, b"unsupported-subset", Some(&authored), root.path())
            .unwrap();
        let binary = exporter.finish().unwrap();
        let json_size = u32::from_le_bytes(binary[12..16].try_into().unwrap()) as usize;
        let document: Value = serde_json::from_slice(&binary[20..20 + json_size]).unwrap();
        assert_eq!(document["materials"].as_array().unwrap().len(), 1);
        assert_eq!(document["meshes"][0]["primitives"][0]["material"], 0);
        assert_eq!(document["materials"][0]["alphaMode"], "BLEND");
        assert_eq!(document["extras"]["geometry_only"], false);
    }
    #[test]
    fn ambiguous_multi_leaf_without_original_subsets_is_unassigned() {
        let mesh = RenderMesh {
            vertex_alpha: None,
            info: ModelInfo {
                format: "fixture".into(),
                version: 1,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: false,
            },
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0., 0.], [1., 0.], [0., 1.]]),
            normals: None,
        };
        let authored = crate::materials::parse_material(
            br#"<Material Name="root"><SubMaterials><Material Name="a"/><Material Name="b"/></SubMaterials></Material>"#
        ).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut exporter = Assembler::new("fixture");
        exporter
            .add_textured_mesh(&mesh, b"invalid-subsets", Some(&authored), root.path())
            .unwrap();
        let binary = exporter.finish().unwrap();
        let json_size = u32::from_le_bytes(binary[12..16].try_into().unwrap()) as usize;
        let document: Value = serde_json::from_slice(&binary[20..20 + json_size]).unwrap();
        assert!(
            document["meshes"][0]["primitives"][0]
                .get("material")
                .is_none()
        );
        assert_eq!(document["extras"]["missing_material_sets"], 1);
    }
    #[test]
    fn refuses_missing_assets_without_creating_output() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("mission.xml");
        fs::write(
            &source,
            br#"<Objects><Object Id="A" Geometry="models/not-here.cgf"/></Objects>"#,
        )
        .unwrap();
        let output = root.path().join("scene.glb");
        let failure = export_level_scene_glb(&source, root.path(), None, &output);
        assert!(matches!(failure, Err(LevelExportError::Missing(_))));
        assert!(!output.exists());
    }
}
