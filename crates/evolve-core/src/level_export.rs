//! Blender-compatible level scene from exact CryEngine source geometry.
//! Source terrain and level placements are imported into a single GLB, with
//! one separately editable Blender node per placement and original UV0.
//! Missing render dependencies fail closed, instead of faking a whole map.
use crate::archive::safe_member_name;
use crate::glb::{GlbError, geometry_glb_bytes};
use crate::model::{ModelError, ModelInfo, RenderMesh, read_model};
use crate::scene::{LevelScene, SceneError, load_level_scene};
use crate::terrain::{TerrainError, read_terrain};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const MAX_SCENE_GLB: usize=768*1024*1024;
const MAX_SCENE_INSTANCES: usize=50000;
const MAX_SCENE_GEOMETRIES: usize=4000;

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
    pub output_bytes: usize,
    /// Only recognized and verified render geometry is exported. Materials,
    /// vegetation instance payloads, collision and game scripting are separate.
    pub original_geometry_only: bool,
}

struct Assembler {
    document: Value,
    binary: Vec<u8>,
    meshes: usize,
    instances: usize,
}
impl Assembler {
    fn new(label: &str) -> Self {
        let document=json!({
            "asset":{"version":"2.0","generator":"Evolve Mod Manager Rust Level Editor"},
            "scene":0,
            "scenes":[{"name":label,"nodes":[0]}],
            "nodes":[{
                "name":"CryEngine world Z-up to Blender Y-up",
                "rotation":[-0.7071067811865476,0,0,0.7071067811865476],
                "children":[]
            }],
            "meshes":[], "bufferViews":[], "accessors":[],
            "buffers":[{"byteLength":0}],
            "extras":{"source":"CryEngine Stage 2 level","geometry_only":true}
        });
        Self {document,binary:Vec::new(),meshes:0,instances:0}
    }
    fn add_mesh(&mut self, mesh:&RenderMesh)->Result<usize,LevelExportError>{
        if self.meshes>=MAX_SCENE_GEOMETRIES{return Err(LevelExportError::Limits);}
        let data=geometry_glb_bytes(mesh)?;
        let json_len=u32::from_le_bytes(data[12..16].try_into().map_err(|_|LevelExportError::Limits)?) as usize;
        let source:Value=serde_json::from_slice(data.get(20..20+json_len).ok_or(LevelExportError::Limits)?)?;
        let bin_start=20+json_len+8;
        let raw=data.get(bin_start..).ok_or(LevelExportError::Limits)?;
        if self.binary.len().checked_add(raw.len()).is_none_or(|n|n>MAX_SCENE_GLB){
            return Err(LevelExportError::Limits);
        }
        while !self.binary.len().is_multiple_of(4){self.binary.push(0);}
        let offset=self.binary.len();
        self.binary.extend_from_slice(raw);
        let access_offset=self.document["accessors"].as_array().ok_or(LevelExportError::Limits)?.len();
        let view_offset=self.document["bufferViews"].as_array().ok_or(LevelExportError::Limits)?.len();
        let views=source["bufferViews"].as_array().ok_or(LevelExportError::Limits)?;
        for item in views {
            let mut v=item.clone();
            if v["buffer"].as_u64()!=Some(0){return Err(LevelExportError::Limits);}
            v["byteOffset"]=json!(v["byteOffset"].as_u64().unwrap_or(0)
                .checked_add(offset as u64).ok_or(LevelExportError::Limits)?);
            self.document["bufferViews"].as_array_mut().ok_or(LevelExportError::Limits)?.push(v);
        }
        let accessors=source["accessors"].as_array().ok_or(LevelExportError::Limits)?;
        for item in accessors {
            let mut v=item.clone();
            if !v["sparse"].is_null(){return Err(LevelExportError::Limits);}
            let n=v["bufferView"].as_u64().ok_or(LevelExportError::Limits)?;
            v["bufferView"]=json!(n+view_offset as u64);
            self.document["accessors"].as_array_mut().ok_or(LevelExportError::Limits)?.push(v);
        }
        let mut primitives=source["meshes"][0]["primitives"].as_array()
            .ok_or(LevelExportError::Limits)?.clone();
        for p in &mut primitives {
            let attrs=p["attributes"].as_object_mut().ok_or(LevelExportError::Limits)?;
            for index in attrs.values_mut() {
                *index=json!(index.as_u64().ok_or(LevelExportError::Limits)?+access_offset as u64);
            }
            p["indices"]=json!(p["indices"].as_u64().ok_or(LevelExportError::Limits)?+access_offset as u64);
        }
        let id=self.meshes;
        self.document["meshes"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!({"name":mesh.info.format,"primitives":primitives}));
        self.meshes+=1;
        Ok(id)
    }
    fn add_node(&mut self, name:&str, mesh:Option<usize>,
        transform:Option<crate::scene::Transform>, metadata:Value)->Result<(),LevelExportError>{
        if self.instances>=MAX_SCENE_INSTANCES{return Err(LevelExportError::Limits);}
        let mut value=json!({"name":name,"extras":metadata});
        if let Some(mesh)=mesh {value["mesh"]=json!(mesh);}
        if let Some(tf)=transform {
            value["translation"]=json!(tf.position);
            value["rotation"]=json!([tf.quaternion[1],tf.quaternion[2],tf.quaternion[3],tf.quaternion[0]]);
            value["scale"]=json!(tf.scale);
        }
        let index=self.document["nodes"].as_array().ok_or(LevelExportError::Limits)?.len();
        self.document["nodes"].as_array_mut().ok_or(LevelExportError::Limits)?.push(value);
        self.document["nodes"][0]["children"].as_array_mut().ok_or(LevelExportError::Limits)?
            .push(json!(index));
        self.instances+=1;
        Ok(())
    }
    fn finish(mut self)->Result<Vec<u8>,LevelExportError>{
        if self.meshes==0{return Err(LevelExportError::Missing("No verified level geometry or heightmap".into()));}
        if self.binary.len()>MAX_SCENE_GLB{return Err(LevelExportError::Limits);}
        self.document["buffers"][0]["byteLength"]=json!(self.binary.len());
        let mut json_data=serde_json::to_vec(&self.document)?;
        while !json_data.len().is_multiple_of(4){json_data.push(b' ');}
        while !self.binary.len().is_multiple_of(4){self.binary.push(0);}
        let total=12usize.checked_add(8).and_then(|v|v.checked_add(json_data.len()))
            .and_then(|v|v.checked_add(8))
            .and_then(|v|v.checked_add(self.binary.len()))
            .ok_or(LevelExportError::Limits)?;
        if total>MAX_SCENE_GLB || total>u32::MAX as usize{return Err(LevelExportError::Limits);}
        let mut data=Vec::with_capacity(total);
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
fn exact_geometry(root:&Path, virtual_path:&str)->Result<PathBuf,LevelExportError>{
    let normalized=safe_member_name(virtual_path)
        .map_err(|_|LevelExportError::Missing(virtual_path.into()))?;
    let mut candidate=root.to_owned();
    for part in normalized.split('/') {
        candidate.push(part);
        let meta=fs::symlink_metadata(&candidate)
            .map_err(|_|LevelExportError::Missing(normalized.clone()))?;
        if meta.file_type().is_symlink(){return Err(LevelExportError::Missing(normalized));}
    }
    let resolved=candidate.canonicalize()?;
    if !resolved.starts_with(root) || !resolved.is_file(){
        return Err(LevelExportError::Missing(virtual_path.into()));
    }
    Ok(resolved)
}
fn terrain_mesh(path:&Path)->Result<RenderMesh,LevelExportError>{
    let terrain=read_terrain(path,257)?;
    let points=terrain.vertices.len();let triangles=terrain.indices.len()/3;
    Ok(RenderMesh{
        info:ModelInfo{
            format:"CryEngine terrain surface".into(),
            version:1,chunk_count:0,vertex_count:points,
            triangle_count:triangles,original_uv0:true,original_normals:true,
        },
        positions:terrain.vertices,indices:terrain.indices,
        uv0:Some(terrain.uv),normals:Some(terrain.normals),
    })
}

/// Build a complete *verified geometry* scene from a level layer/mission XML,
/// an exact extraction root and optional source Heightmap.dat. Every referenced
/// render model must be present or the operation fails without creating a GLB.
pub fn export_level_scene_glb(
    placement: &Path, asset_root: &Path, terrain: Option<&Path>, destination: &Path,
)->Result<LevelExportReport,LevelExportError>{
    let root=asset_root.canonicalize()?;
    if !root.is_dir() || fs::symlink_metadata(asset_root)?.file_type().is_symlink(){
        return Err(LevelExportError::Missing("Select an actual extracted asset folder".into()));
    }
    if !placement.is_file() || fs::symlink_metadata(placement)?.file_type().is_symlink(){
        return Err(LevelExportError::Missing("Select original mission XML or LYR".into()));
    }
    if destination.extension().and_then(|e|e.to_str())!=Some("glb")
        || fs::symlink_metadata(destination).is_ok(){
        return Err(LevelExportError::Exists);
    }
    let parent=destination.parent().ok_or(LevelExportError::Exists)?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink(){
        return Err(LevelExportError::Exists);
    }
    let scene:LevelScene=load_level_scene(placement)?;
    let mut assembler=Assembler::new(placement.file_stem().and_then(|v|v.to_str()).unwrap_or("CryEngine Level"));
    let mut resources=HashMap::<String,usize>::new();
    let mut geometry_instances=0usize;
    let mut metadata_only=0usize;
    if let Some(source)=terrain{
        let mesh=terrain_mesh(source)?;
        let index=assembler.add_mesh(&mesh)?;
        assembler.add_node("CryEngine Terrain",Some(index),None,
            json!({"source":source.file_name().and_then(|n|n.to_str()).unwrap_or("Heightmap.dat")}))?;
        geometry_instances+=1;
    }
    for object in &scene.objects{
        let mesh_id=if object.model.is_empty(){None}else{
            let key=object.model.to_ascii_lowercase();
            if let Some(id)=resources.get(&key){Some(*id)}else{
                let path=exact_geometry(&root,&object.model)?;
                let mesh=read_model(&path)?;
                let id=assembler.add_mesh(&mesh)?;
                resources.insert(key,id);
                Some(id)
            }
        };
        if mesh_id.is_some(){geometry_instances+=1;}else{metadata_only+=1;}
        assembler.add_node(&object.name,mesh_id,Some(object.world),
            json!({"source_id":object.id,"layer":object.layer,
                   "type":object.kind,"parent_guid":object.parent,
                   "material_reference":object.material,"hidden":object.hidden}))?;
    }
    let bytes=assembler.finish()?;
    let mut stage=NamedTempFile::new_in(parent)?;
    stage.write_all(&bytes)?;
    stage.as_file().sync_all()?;
    stage.persist_noclobber(destination).map_err(|e|
        if e.error.kind()==io::ErrorKind::AlreadyExists {LevelExportError::Exists}
        else {LevelExportError::Io(e.error)}
    )?;
    Ok(LevelExportReport{
        source_objects:scene.objects.len(),
        exported_mesh_resources:resources.len()+usize::from(terrain.is_some()),
        exported_mesh_instances:geometry_instances,
        metadata_only_objects:metadata_only,
        included_terrain:terrain.is_some(),
        output_bytes:bytes.len(),
        original_geometry_only:true,
    })
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn creates_distinct_instance_nodes_under_common_zup_basis(){
        let model=RenderMesh{
            info:ModelInfo{
                format:"test".into(),version:7,chunk_count:1,
                vertex_count:3,triangle_count:1,original_uv0:true,original_normals:false,
            },
            positions:vec![[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]],
            indices:vec![0,1,2],uv0:Some(vec![[0.,0.],[1.,0.],[0.,1.]]),normals:None,
        };
        let mut scene=Assembler::new("test");
        let id=scene.add_mesh(&model).unwrap();
        scene.add_node("Object A",Some(id),None,json!({"id":"a"})).unwrap();
        scene.add_node("Object B",Some(id),None,json!({"id":"b"})).unwrap();
        let bytes=scene.finish().unwrap();
        let json_size=u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let json:Value=serde_json::from_slice(&bytes[20..20+json_size]).unwrap();
        assert_eq!(json["meshes"].as_array().unwrap().len(),1);
        assert_eq!(json["nodes"].as_array().unwrap().len(),3);
        assert_eq!(json["nodes"][1]["mesh"],json["nodes"][2]["mesh"]);
        assert!(json["nodes"][0]["rotation"].is_array());
    }
    #[test]
    fn refuses_missing_assets_without_creating_output(){
        let root=tempfile::tempdir().unwrap();
        let source=root.path().join("mission.xml");
        fs::write(&source,br#"<Objects><Object Id="A" Geometry="models/not-here.cgf"/></Objects>"#).unwrap();
        let output=root.path().join("scene.glb");
        let failure=export_level_scene_glb(&source,root.path(),None,&output);
        assert!(matches!(failure,Err(LevelExportError::Missing(_))));
        assert!(!output.exists());
    }
}
