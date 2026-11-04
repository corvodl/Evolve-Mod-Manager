//! Blender-compatible level scene from exact CryEngine source geometry.
//! Source terrain and level placements are imported into a single GLB, with
//! one separately editable Blender node per placement and original UV0.
//! Missing render dependencies fail closed, instead of faking a whole map.
use crate::archive::safe_member_name;
use crate::dds::decode_dds_rgba;
use crate::glb::{GlbError, geometry_glb_bytes};
use crate::materials::{MaterialDocument, MaterialSlot, read_material};
use crate::model::{ModelError, ModelInfo, RenderMesh, original_mesh_subsets, read_model};
use crate::scene::{LevelScene, SceneError, load_level_scene};
use crate::terrain::{TerrainError, read_terrain};
use image::{DynamicImage, ImageFormat};
use quick_xml::{Reader, events::{BytesStart, Event}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
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
    node.attributes().flatten()
        .find(|attr| attr.key.as_ref().eq_ignore_ascii_case(key))
        .and_then(|attr| attr.unescape_value().ok().map(|text| text.into_owned()))
}
fn light_value(node: &BytesStart<'_>, keys: &[&[u8]]) -> Option<f32> {
    keys.iter().find_map(|key| light_attr(node,key))
        .and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite())
}
fn light_color(node: &BytesStart<'_>) -> Option<[f32; 3]> {
    let text = [b"DiffuseColor".as_slice(), b"clrDiffuse", b"Color", b"color"]
        .iter().find_map(|key| light_attr(node,key))?;
    let rgb = text.split(',').map(|v| v.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    if rgb.len() != 3 { return None; }
    let c = [rgb[0]?, rgb[1]?, rgb[2]?];
    c.iter().all(|v| v.is_finite() && (0.0..=16.0).contains(v)).then_some(c)
}
impl PendingLight {
    fn read_props(&mut self, node: &BytesStart<'_>) {
        if light_attr(node,b"bFakeLight").is_some_and(|x| x == "1") {
            self.enabled = false;
        }
        if light_attr(node,b"bActive").or_else(|| light_attr(node,b"Active"))
            .is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        {
            self.enabled = false;
        }
        if let Some(radius) = light_value(node,&[b"Radius",b"fRadius",b"LightRadius"])
            .filter(|v| (0.1..=100_000.0).contains(v))
        { self.radius=Some(radius); }
        if let Some(intensity) = light_value(node,&[
            b"DiffuseMultiplier", b"fDiffuseMultiplier", b"LightIntensity",b"Intensity"
        ]).filter(|v| (0.0..=1_000_000.0).contains(v)) {
            self.intensity=Some(intensity);
        }
        if let Some(color) = light_color(node) { self.color=Some(color); }
        if let Some(fov) = light_value(node,&[
            b"ProjectorFov", b"fProjectorFov", b"FOV",b"fFov"
        ]).filter(|fov| (1.0..179.0).contains(fov)) {
            self.cone_degrees=Some(fov);
        }
        if let Some(size) = light_value(node,&[b"fAttenuationBulbSize"])
            .filter(|v| *v > 0.0 && *v < 100_000.0)
        { self.bulb_size=Some(size); }
    }
}
fn complete_light(
    current: &mut Option<PendingLight>,
    objects: &HashMap<String, &crate::scene::PlacedObject>,
    lights: &mut Vec<ExportLight>,
) {
    let Some(source) = current.take() else { return; };
    if !source.enabled { return; }
    let Some(actor)=objects.get(&source.id) else { return; };
    let (Some(radius),Some(intensity),Some(color))=
        (source.radius,source.intensity,source.color) else { return; };
    if !actor.world.position.iter().all(|v| v.is_finite())
        || !actor.world.quaternion.iter().all(|v| v.is_finite())
    { return; }
    lights.push(ExportLight{
        id: actor.id.clone(), name: actor.name.clone(),
        world:actor.world, color, intensity, radius,
        cone_degrees:source.cone_degrees, bulb_size:source.bulb_size
    });
}
/// Match GUIDs rather than guessing from names. Only positively authored
/// lights become Blender objects; flare-only, disabled, or incomplete records
/// are not replaced with artificial defaults.
fn authored_lights(source: &[u8], scene: &LevelScene) -> Result<Vec<ExportLight>, LevelExportError> {
    if source.len()>MAX_LIGHT_LAYER_BYTES {
        return Err(LevelExportError::Material("Light layer exceeds supported size".into()));
    }
    let data=crate::cryxml::xml_bytes(source,MAX_LIGHT_LAYER_BYTES)
        .map_err(|e| LevelExportError::Material(format!("Light XML: {e}")))?;
    let objects=scene.objects.iter().filter(|o| !o.hidden)
        .map(|o|(o.id.to_ascii_lowercase(),o)).collect::<HashMap<_,_>>();
    let mut reader=Reader::from_reader(data.as_ref());
    let (mut depth,mut pending,mut result)=(0usize,None,Vec::new());
    loop {
        let event=reader.read_event()
            .map_err(|e| LevelExportError::Material(format!("Light XML: {e}")))?;
        let starts=matches!(&event,Event::Start(_));
        match event {
            Event::Start(node)|Event::Empty(node) => {
                let current_depth=depth+1;
                if current_depth>128 { return Err(LevelExportError::Limits); }
                let tag=node.name();
                let entity=tag.as_ref().eq_ignore_ascii_case(b"Object")
                    || tag.as_ref().eq_ignore_ascii_case(b"Entity");
                if entity {
                    let class=light_attr(&node,b"EntityClass")
                        .or_else(||light_attr(&node,b"Class"))
                        .unwrap_or_default().to_ascii_lowercase();
                    let kind=light_attr(&node,b"Type").unwrap_or_default().to_ascii_lowercase();
                    let is_light=matches!(class.as_str(),
                        "light"|"lightentity"|"pointlight"|"spotlight")
                        || (kind=="light" && class.is_empty());
                    if is_light {
                        if let Some(id)=light_attr(&node,b"Id")
                            .or_else(||light_attr(&node,b"EntityId")) {
                            let mut light=PendingLight{
                                id:id.to_ascii_lowercase(), depth:current_depth,
                                enabled:true, ..Default::default()
                            };
                            light.read_props(&node);
                            pending=Some(light);
                        }
                    }
                } else if let Some(light)=pending.as_mut()
                    && current_depth>light.depth && current_depth<=light.depth+7
                { light.read_props(&node); }
                if starts { depth+=1; }
                else if entity && pending.as_ref().is_some_and(|p|p.depth==current_depth) {
                    complete_light(&mut pending,&objects,&mut result);
                }
                if result.len()>MAX_LIGHT_SOURCES {return Err(LevelExportError::Limits);}
            }
            Event::End(_)=>{
                if pending.as_ref().is_some_and(|p|p.depth==depth) {
                    complete_light(&mut pending,&objects,&mut result);
                }
                depth=depth.saturating_sub(1);
            }
            Event::DocType(_)|Event::GeneralRef(_)=>{
                return Err(LevelExportError::Material("Unsafe light XML entity".into()));
            }
            Event::Eof=>break,
            _=>{}
        }
    }
    Ok(result)
}
/// CryEngine spotlights shine along local +X; glTF punctual spots shine
/// along local -Z. Rotate only the light node; the source mesh remains intact.
fn light_spot_rotation(source: [f32; 4]) -> [f32; 4] {
    let [w,x,y,z]=source;
    let h=std::f32::consts::FRAC_1_SQRT_2;
    let [p,q,r,t]=[h,0.0,-h,0.0];
    [w*p-x*q-y*r-z*t,w*q+x*p+y*t-z*r,w*r-x*t+y*p+z*q,w*t+x*r-y*q+z*p]
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
    fn source_texture(
        &mut self,
        root: &Path,
        virtual_path: &str,
    ) -> Result<Option<usize>, LevelExportError> {
        let cache_key = virtual_path.to_ascii_lowercase();
        if let Some(index) = self.cached_images.get(&cache_key) {
            return Ok(Some(*index));
        }
        let source = exact_geometry(root, virtual_path).or_else(|_| {
            if virtual_path.to_ascii_lowercase().ends_with(".dds") {
                exact_geometry(root, &format!("{virtual_path}.0"))
            } else {
                Err(LevelExportError::Missing(virtual_path.into()))
            }
        });
        let Ok(source) = source else {
            self.missing_texture_references += 1;
            return Ok(None);
        };
        let Ok((_, rgba)) = decode_dds_rgba(&source) else {
            self.missing_texture_references += 1;
            return Ok(None);
        };
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

    /// Preserve authored leaf slot order, including NoDraw holes. glTF maps
    /// established texture channels directly; other CryEngine channels remain
    /// explicitly identified in extras, rather than assigned guessed semantics.
    fn source_material(
        &mut self,
        slot: &MaterialSlot,
        root: &Path,
    ) -> Result<usize, LevelExportError> {
        let mut pbr = json!({"metallicFactor":0.0,"roughnessFactor":1.0});
        if let Some(tint) = slot.diffuse_tint {
            pbr["baseColorFactor"] =
                json!([tint[0], tint[1], tint[2], slot.opacity.unwrap_or(1.0)]);
        } else if let Some(opacity) = slot.opacity {
            pbr["baseColorFactor"] = json!([1.0, 1.0, 1.0, opacity]);
        }
        let mut value = json!({
            "name": slot.name,
            "doubleSided": true,
            "pbrMetallicRoughness": pbr,
            "extras": {
                "cryengine_shader": slot.shader,
                "cryengine_original_maps": {},
                "cryengine_shininess": slot.shininess,
                "cryengine_glow_amount": slot.glow_amount
            }
        });
        if let Some(alpha_test) = slot.alpha_test {
            value["alphaMode"] = json!("MASK");
            value["alphaCutoff"] = json!(alpha_test);
        } else if slot.opacity.is_some_and(|x| x < 1.0) {
            value["alphaMode"] = json!("BLEND");
        }
        if let Some(color) = slot.emissive_color {
            value["emissiveFactor"] = json!(color);
        }
        for reference in &slot.maps {
            let Some(texture) = self.source_texture(root, &reference.path)? else {
                continue;
            };
            let role = reference.role.as_str();
            value["extras"]["cryengine_original_maps"][role] = json!({
                "texture":texture,"source":reference.path
            });
            match role {
                "diffuse" => {
                    value["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":texture})
                }
                "normals" | "normal" | "bumpmap" => {
                    // BC5 X/Y normal reconstruction and shader tangent orientation
                    // must be validated on real assets before claiming FBX parity.
                    value["normalTexture"] = json!({"index":texture});
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
        let id = self.add_mesh(mesh)?;
        let Some(document) = material else {
            self.missing_material_sets += 1;
            return Ok(id);
        };
        let mut ids = Vec::with_capacity(document.slots.len());
        for slot in &document.slots {
            ids.push(self.source_material(slot, root)?);
        }
        let old = self.document["meshes"][id]["primitives"][0].clone();
        let ranges = original_mesh_subsets(source_bytes, mesh.indices.len())
            .ok()
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
    fn add_light(&mut self, light: &ExportLight) -> Result<(), LevelExportError> {
        let is_spot=light.cone_degrees.is_some();
        let mut definition=json!({
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
        if let Some(degrees)=light.cone_degrees {
            definition["spot"]=json!({
                "innerConeAngle":0.0,
                "outerConeAngle":(degrees.to_radians()*0.5)
            });
        }
        if self.document.get("extensions").is_none() {
            self.document["extensions"]=json!({"KHR_lights_punctual":{"lights":[]}});
            self.document["extensionsUsed"]=json!(["KHR_lights_punctual"]);
        }
        let index=self.document["extensions"]["KHR_lights_punctual"]["lights"]
            .as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["extensions"]["KHR_lights_punctual"]["lights"]
            .as_array_mut().ok_or(LevelExportError::Limits)?.push(definition);
        self.add_node(
            &light.name,None,Some(light.world),
            json!({"source_id":light.id,"kind":"CryEngineLight",
                "original_rotation_wxyz":light.world.quaternion}),
        )?;
        let last=self.document["nodes"].as_array().ok_or(LevelExportError::Limits)?.len()-1;
        self.document["nodes"][last]["extensions"]=json!({"KHR_lights_punctual":{"light":index}});
        if is_spot {
            let q=light_spot_rotation(light.world.quaternion);
            self.document["nodes"][last]["rotation"]=json!([q[1],q[2],q[3],q[0]]);
        }
        self.lights+=1;
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
fn terrain_mesh(path: &Path) -> Result<RenderMesh, LevelExportError> {
    let terrain = read_terrain(path, 257)?;
    let points = terrain.vertices.len();
    let triangles = terrain.indices.len() / 3;
    Ok(RenderMesh {
        vertex_alpha: None,
        info: ModelInfo {
            format: "CryEngine terrain surface".into(),
            version: 1,
            chunk_count: 0,
            vertex_count: points,
            triangle_count: triangles,
            original_uv0: true,
            original_normals: true,
        },
        positions: terrain.vertices,
        indices: terrain.indices,
        uv0: Some(terrain.uv),
        normals: Some(terrain.normals),
    })
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
    let root = asset_root.canonicalize()?;
    if !root.is_dir() || fs::symlink_metadata(asset_root)?.file_type().is_symlink() {
        return Err(LevelExportError::Missing(
            "Select an actual extracted asset folder".into(),
        ));
    }
    if !placement.is_file() || fs::symlink_metadata(placement)?.file_type().is_symlink() {
        return Err(LevelExportError::Missing(
            "Select original mission XML or LYR".into(),
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
    let scene: LevelScene = load_level_scene(placement)?;
    let lights = authored_lights(&fs::read(placement)?, &scene)?;
    let mut assembler = Assembler::new(
        placement
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("CryEngine Level"),
    );
    let mut resources = HashMap::<(String, String), usize>::new();
    let mut geometry_instances = 0usize;
    let mut metadata_only = 0usize;
    if let Some(source) = terrain {
        let mesh = terrain_mesh(source)?;
        let index = assembler.add_mesh(&mesh)?;
        assembler.add_node(
            "CryEngine Terrain",
            Some(index),
            None,
            json!({"source":source.file_name().and_then(|n|n.to_str()).unwrap_or("Heightmap.dat")}),
        )?;
        geometry_instances += 1;
    }
    for object in &scene.objects {
        let mesh_id = if object.model.is_empty() {
            None
        } else {
            let key = (
                object.model.to_ascii_lowercase(),
                object.material.to_ascii_lowercase(),
            );
            if let Some(id) = resources.get(&key) {
                Some(*id)
            } else {
                let path = exact_geometry(&root, &object.model)?;
                let mesh = read_model(&path)?;
                let bytes = fs::read(&path)?;
                let virtual_material = if object.material.trim().is_empty() {
                    let stem = object
                        .model
                        .rsplit_once('.')
                        .map_or(object.model.as_str(), |(stem, _)| stem);
                    format!("{stem}.mtl")
                } else if object.material.to_ascii_lowercase().ends_with(".mtl") {
                    object.material.clone()
                } else {
                    format!("{}.mtl", object.material)
                };
                let authored = exact_geometry(&root, &virtual_material)
                    .ok()
                    .and_then(|mtl| read_material(&mtl).ok());
                let id = assembler.add_textured_mesh(&mesh, &bytes, authored.as_ref(), &root)?;
                resources.insert(key, id);
                Some(id)
            }
        };
        if mesh_id.is_some() {
            geometry_instances += 1;
        } else {
            metadata_only += 1;
        }
        assembler.add_node(
            &object.name,
            mesh_id,
            Some(object.world),
            json!({"source_id":object.id,"layer":object.layer,
                   "type":object.kind,"parent_guid":object.parent,
                   "material_reference":object.material,"hidden":object.hidden}),
        )?;
    }
    for light in &lights {
        assembler.add_light(light)?;
    }
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
        exported_mesh_resources: resources.len() + usize::from(terrain.is_some()),
        exported_mesh_instances: geometry_instances,
        metadata_only_objects: metadata_only,
        included_terrain: terrain.is_some(),
        exported_lights: lights.len(),
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
    fn source_lights_export_as_moveable_gltf_punctual_nodes() {
        let data=br#"<Objects>
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
        let scene=crate::scene::parse_level_scene(&[("fixture",data)]).unwrap();
        let lights=authored_lights(data,&scene).unwrap();
        assert_eq!(lights.len(),1);
        assert_eq!(lights[0].radius,20.);
        let mut exporter=Assembler::new("Lamp fixture");
        exporter.add_light(&lights[0]).unwrap();
        assert_eq!(exporter.document["extensionsUsed"][0],"KHR_lights_punctual");
        assert_eq!(exporter.document["extensions"]["KHR_lights_punctual"]["lights"][0]["type"],"spot");
        assert_eq!(exporter.document["nodes"][1]["extensions"]["KHR_lights_punctual"]["light"],0);
        assert_eq!(exporter.document["nodes"][1]["translation"],json!([11.,12.,13.]));
    }
    #[test]
    fn spotlight_basis_maps_gltf_negative_z_to_cryengine_positive_x() {
        let [w,x,y,z]=light_spot_rotation([1.,0.,0.,0.]);
        let direction=[
            -2.0*(x*z+w*y),
            -2.0*(y*z-w*x),
            -(1.0-2.0*(x*x+y*y)),
        ];
        assert!((direction[0]-1.).abs()<1e-5);
        assert!(direction[1].abs()<1e-5);
        assert!(direction[2].abs()<1e-5);
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
