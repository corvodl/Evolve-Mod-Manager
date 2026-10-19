//! CryEngine CAF/DBA references and verified converter-backed GLB timelines.
//! Raw CAF/DBA controller decoding is not guessed from chunk headers.
use crate::archive::safe_member_name;
use crate::signed_pak::SignedPakError;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_CONFIG: usize=2*1024*1024;
const MAX_GLTF: usize=256*1024*1024;
const MAX_FRAMES: usize=250000;
const MAX_NODES: usize=4096;
const MAX_CAF: usize=64*1024*1024;

#[derive(Debug,thiserror::Error)]
pub enum AnimationError {
    #[error("Unsupported or corrupt motion data: {0}")]
    Invalid(String),
    #[error("Missing compatible converter or animation track")]
    Unavailable,
    #[error("Converter failed or timed out: {0}")]
    Converter(String),
    #[error("File access: {0}")]
    Io(#[from] std::io::Error),
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct AnimationReference {
    pub name:String,
    pub path:String,
    pub kind:String,
    pub database:bool,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct CafInspection {
    pub version:u32,
    pub chunks:usize,
    pub additive:bool,
}
pub fn inspect_caf_bytes(bytes:&[u8])->Result<CafInspection,AnimationError>{
    if bytes.len()<32 || bytes.len()>MAX_CAF || bytes.get(..8)!=Some(b"CrChF\x07\0\0"){
        return Err(AnimationError::Invalid("not a recognized CrChF v7 CAF".into()));
    }
    let count=u32::from_le_bytes(bytes[8..12].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
    let table=u32::from_le_bytes(bytes[12..16].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
    let end=table.checked_add(count.checked_mul(16).ok_or(AnimationError::Unavailable)?).ok_or(AnimationError::Unavailable)?;
    if count==0 || count>10000 || table<16 || end>bytes.len(){return Err(AnimationError::Invalid("unsafe CAF chunk directory".into()));}
    let mut additive=false;
    for i in 0..count {
        let at=table+i*16;
        let ty=u32::from_le_bytes(bytes[at..at+4].try_into().map_err(|_|AnimationError::Unavailable)?);
        let length=u32::from_le_bytes(bytes[at+8..at+12].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
        let offset=u32::from_le_bytes(bytes[at+12..at+16].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
        if length==0 || offset<end || offset.checked_add(length).is_none_or(|n|n>bytes.len()){
            return Err(AnimationError::Invalid("CAF chunk boundaries invalid".into()));
        }
        if ty==0x09253002 {
            if length<40 {return Err(AnimationError::Invalid("CAF motion params truncated".into()));}
            additive=u32::from_le_bytes(bytes[offset..offset+4].try_into().map_err(|_|AnimationError::Unavailable)?) & 1 != 0;
        }
    }
    Ok(CafInspection{version:7,chunks:count,additive})
}
pub fn inspect_caf_file(path:&Path)->Result<CafInspection,AnimationError>{
    let meta=fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len()>MAX_CAF as u64 {return Err(AnimationError::Unavailable);}
    inspect_caf_bytes(&fs::read(path)?)
}
pub fn animation_references(bytes:&[u8])->Result<Vec<AnimationReference>,AnimationError>{
    if bytes.is_empty()||bytes.len()>MAX_CONFIG
        || bytes.windows(9).any(|w|w.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes.windows(8).any(|w|w.eq_ignore_ascii_case(b"<!ENTITY")){
        return Err(AnimationError::Invalid("unsafe chrparams".into()));
    }
    let mut reader=Reader::from_reader(bytes);
    let mut base=String::new();
    let mut parents=Vec::new();
    let mut items=Vec::new();
    loop {
        match reader.read_event().map_err(|e|AnimationError::Invalid(e.to_string()))? {
            Event::Start(e)|Event::Empty(e)=>{
                let tag=e.name().as_ref().to_vec();
                if parents.last().is_some_and(|p:&Vec<u8>|p.eq_ignore_ascii_case(b"AnimationList"))
                    && tag.eq_ignore_ascii_case(b"Animation") {
                    let mut name=String::new();
                    let mut reference=String::new();
                    for attr in e.attributes(){
                        let attr=attr.map_err(|e|AnimationError::Invalid(e.to_string()))?;
                        let v=attr.unescape_value().map_err(|e|AnimationError::Invalid(e.to_string()))?.into_owned();
                        match attr.key.as_ref(){
                            b"name"=>name=v,
                            b"path"=>reference=v.replace('\\',"/"),
                            _=>(),
                        }
                    }
                    if name.eq_ignore_ascii_case("#filepath"){
                        base=reference.trim_matches('/').to_owned();
                    }else{
                        let database=name.eq_ignore_ascii_case("$TracksDatabase");
                        if !name.is_empty() && !reference.is_empty() && !name.contains('*') && !reference.contains('*')
                            && (!name.starts_with(['#','$']) || database) {
                            if let Some(caf)=reference.to_ascii_lowercase().find(".caf (") {
                                reference.truncate(caf+4);
                            }
                            let full=if reference.to_ascii_lowercase().starts_with(["characters/","animations/"]){
                                reference
                            } else {format!("{}/{}",base,reference).trim_start_matches('/').to_string()};
                            if let Ok(path)=safe_member_name(&full) {
                                let kind=path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                                if matches!(kind.as_str(),"caf"|"dba"|"bspace"|"comb") && items.len()<MAX_FRAMES {
                                    items.push(AnimationReference{name,path,kind,database});
                                }
                            }
                        }
                    }
                }
                if matches!(reader.read_event(),Ok(Event::Eof)){} // no-op: avoid borrowing temporary event
                if tag.len()>0 && parents.len()<64 {
                    // Track direct root/AnimationList nesting only; nested Animation refs are not guessed.
                }
            }
            Event::End(_)=>{parents.pop();}
            Event::DocType(_) | Event::GeneralRef(_)=>return Err(AnimationError::Invalid("unsafe XML entities".into())),
            Event::Eof=>break,
            _=>(),
        }
    }
    Ok(items)
}

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct MotionClip {
    pub name:String,
    pub duration:f32,
    pub channels:usize,
}
#[derive(Clone,Debug)]
pub struct PoseSample {
    pub node:usize,
    pub path:String,
    /// Source glTF data: translations/scale float3, rotation xyzw float4.
    pub value:Vec<f32>,
}
#[derive(Clone,Debug)]
struct Track {
    target:usize,
    path:String,
    interp:String,
    times:Vec<f32>,
    values:Vec<Vec<f32>>,
}
#[derive(Clone,Debug)]
struct Clip {name:String,duration:f32,tracks:Vec<Track>}
#[derive(Clone,Debug)]
pub struct AnimatedGlb {
    clips:Vec<Clip>,
    pub node_names:Vec<String>,
}
fn json_u(v:&serde_json::Value,key:&str)->Result<usize,AnimationError>{
    v[key].as_u64().and_then(|v|usize::try_from(v).ok())
        .ok_or_else(||AnimationError::Invalid(format!("missing {key} integer")))
}
fn floats(doc:&serde_json::Value,binary:&[u8],index:usize,width:usize)->Result<Vec<Vec<f32>>,AnimationError>{
    let accessor=doc["accessors"].get(index).ok_or_else(||AnimationError::Invalid("invalid accessor".into()))?;
    if !accessor["sparse"].is_null() || accessor["componentType"]!=5126
        || accessor["type"]!=match width {1=>"SCALAR",3=>"VEC3",4=>"VEC4",_=>"invalid"} {
        return Err(AnimationError::Invalid("only dense float32 clip tracks supported".into()));
    }
    let count=json_u(accessor,"count")?;
    if count>MAX_FRAMES {return Err(AnimationError::Invalid("animation key count excessive".into()));}
    let view=doc["bufferViews"].get(json_u(accessor,"bufferView")?)
        .ok_or_else(||AnimationError::Invalid("invalid animation bufferView".into()))?;
    if view["buffer"].as_u64().unwrap_or(0)!=0 {return Err(AnimationError::Invalid("external buffers unsupported".into()));}
    let offset=json_u(view,"byteOffset").unwrap_or(0)
        .checked_add(json_u(accessor,"byteOffset").unwrap_or(0))
        .ok_or_else(||AnimationError::Invalid("accessor overflow".into()))?;
    let step=json_u(view,"byteStride").unwrap_or(width*4);
    if step<width*4 {return Err(AnimationError::Invalid("invalid animation stride".into()));}
    let mut result=Vec::with_capacity(count);
    for i in 0..count {
        let start=offset.checked_add(i.checked_mul(step).ok_or(AnimationError::Unavailable)?)
            .ok_or(AnimationError::Unavailable)?;
        let row=binary.get(start..start+width*4).ok_or(AnimationError::Unavailable)?;
        let mut values=Vec::with_capacity(width);
        for j in 0..width {
            let v=f32::from_le_bytes(row[j*4..j*4+4].try_into().map_err(|_|AnimationError::Unavailable)?);
            if !v.is_finite(){return Err(AnimationError::Invalid("non-finite keyframe".into()));}
            values.push(v);
        }
        result.push(values);
    }
    Ok(result)
}
impl AnimatedGlb {
    pub fn read(path:&Path)->Result<Self,AnimationError>{
        let meta=fs::symlink_metadata(path)?;
        if !meta.file_type().is_file() || meta.len()>MAX_GLTF as u64 || meta.len()<32 {
            return Err(AnimationError::Invalid("invalid GLB input file".into()));
        }
        let mut file=File::open(path)?;
        let mut header=[0u8;20];file.read_exact(&mut header)?;
        if &header[..4]!=b"glTF" || &header[16..20]!=b"JSON" ||
           u32::from_le_bytes(header[8..12].try_into().map_err(|_|AnimationError::Unavailable)?) as u64!=meta.len(){
            return Err(AnimationError::Invalid("invalid GLB header".into()));
        }
        let json_size=u32::from_le_bytes(header[12..16].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
        if json_size>12*1024*1024 || json_size+20>meta.len() as usize{return Err(AnimationError::Invalid("GLB JSON too large".into()));}
        let mut json=vec![0u8;json_size];file.read_exact(&mut json)?;
        let doc:serde_json::Value=serde_json::from_slice(&json).map_err(|e|AnimationError::Invalid(e.to_string()))?;
        let mut marker=[0u8;8];file.read_exact(&mut marker)?;
        let bytes=u32::from_le_bytes(marker[..4].try_into().map_err(|_|AnimationError::Unavailable)?) as usize;
        if &marker[4..]!=b"BIN\0" || bytes>MAX_GLTF{return Err(AnimationError::Invalid("GLB BIN chunk unsupported".into()));}
        let mut binary=vec![0u8;bytes];file.read_exact(&mut binary)?;
        let nodes=doc["nodes"].as_array().ok_or(AnimationError::Unavailable)?;
        if nodes.len()>MAX_NODES{return Err(AnimationError::Unavailable);}
        let names=nodes.iter().enumerate().map(|(i,n)|n["name"].as_str().map(str::to_owned).unwrap_or_else(||format!("Node_{i}"))).collect();
        let mut clips=Vec::new();
        if let Some(animations)=doc["animations"].as_array() {
            for animation in animations {
                let samples=animation["samplers"].as_array().ok_or(AnimationError::Unavailable)?;
                let channels=animation["channels"].as_array().ok_or(AnimationError::Unavailable)?;
                let mut tracks=Vec::new();
                let mut duration=0f32;
                for channel in channels {
                    let sampler=samples.get(json_u(channel,"sampler")?).ok_or(AnimationError::Unavailable)?;
                    let path=channel["target"]["path"].as_str().unwrap_or("");
                    let width=match path {"translation"|"scale"=>3,"rotation"=>4,_=>continue};
                    let target=json_u(&channel["target"],"node")?;
                    if target>=nodes.len(){return Err(AnimationError::Invalid("channel references unknown bone".into()));}
                    let times=floats(&doc,&binary,json_u(sampler,"input")?,1)?;
                    let output=floats(&doc,&binary,json_u(sampler,"output")?,width)?;
                    if times.len()!=output.len() || times.is_empty()
                        || times.windows(2).any(|w|w[0]>=w[1]) {return Err(AnimationError::Invalid("track times invalid".into()));}
                    let interpolation=sampler["interpolation"].as_str().unwrap_or("LINEAR");
                    if !matches!(interpolation,"LINEAR"|"STEP"){return Err(AnimationError::Invalid("unsupported interpolation".into()));}
                    duration=duration.max(*times.last().unwrap_or(&0.));
                    tracks.push(Track{
                        target,path:path.into(),interp:interpolation.into(),
                        times:times.into_iter().map(|v|v[0]).collect(),values:output,
                    });
                }
                clips.push(Clip{
                    name:animation["name"].as_str().unwrap_or("unnamed").to_owned(),
                    duration,tracks,
                });
            }
        }
        Ok(Self{clips,node_names:names})
    }
    pub fn list_clips(&self)->Vec<MotionClip>{
        self.clips.iter().map(|v|MotionClip{name:v.name.clone(),duration:v.duration,channels:v.tracks.len()}).collect()
    }
    pub fn sample(&self,clip_index:usize,seconds:f32)->Result<Vec<PoseSample>,AnimationError>{
        if !seconds.is_finite()||seconds<0. {return Err(AnimationError::Invalid("invalid time".into()));}
        let clip=self.clips.get(clip_index).ok_or(AnimationError::Unavailable)?;
        let seconds=seconds.min(clip.duration);
        let mut result=Vec::new();
        for track in &clip.tracks {
            let upper=track.times.partition_point(|v|*v<=seconds);
            let start=upper.saturating_sub(1).min(track.times.len()-1);
            let end=(start+1).min(track.times.len()-1);
            let alpha=if end>start && track.interp!="STEP" {
                ((seconds-track.times[start])/(track.times[end]-track.times[start])).clamp(0.,1.)
            }else{0.};
            let mut vector=track.values[start].iter().zip(&track.values[end])
                .map(|(a,b)|a*(1.-alpha)+b*alpha).collect::<Vec<_>>();
            if track.path=="rotation" {
                let first=&track.values[start];let other=&track.values[end];
                let dot=first.iter().zip(other).map(|(x,y)|x*y).sum::<f32>();
                let sign=if dot<0. {-1.}else{1.};
                for (i,value) in vector.iter_mut().enumerate() {
                    *value=first[i]*(1.-alpha)+other[i]*alpha*sign;
                }
                let norm=vector.iter().map(|v|v*v).sum::<f32>().sqrt();
                if norm<1e-6 {return Err(AnimationError::Invalid("invalid joint quaternion".into()));}
                for v in &mut vector {*v/=norm;}
            }
            result.push(PoseSample{node:track.target,path:track.path.clone(),value:vector});
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_corrupt_caf_metadata(){
        assert!(inspect_caf_bytes(b"CrChF\x07\0\0").is_err());
    }
    #[test]
    fn reads_exact_chrparams_catalog(){
        let xml=br#"<Params><AnimationList><Animation name="#filepath" path="characters/hunters/merc_tech/"/>
          <Animation name="idle" path="animations/tech_idle.caf"/>
          <Animation name="$TracksDatabase" path="animations/tech_3p.dba"/>
          <Animation name="walk*" path="animations/*.caf"/>
        </AnimationList></Params>"#;
        let catalog=animation_references(xml).unwrap();
        assert_eq!(catalog.len(),2);
        assert_eq!(catalog[0].name,"idle");
        assert!(catalog[1].database);
    }
}
