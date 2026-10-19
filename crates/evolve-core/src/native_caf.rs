//! Native CryEngine CAF controller decoding (0x829, 0x831).
//! Matches the production manager's controller/time/quantized quaternion
//! families without guessing unsupported DBA bitset sequences.
use serde::{Deserialize,Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const MAX_CAF:usize=64*1024*1024;
const MAX_CONTROLLER:usize=1500;
const MAX_KEYS:usize=8192;

#[derive(Debug,thiserror::Error)]
pub enum NativeCafError {
    #[error("Invalid CryEngine CAF header, data layout, or source value: {0}")]
    Format(String),
    #[error("Unsupported or truncated controller stream version/encoding")]
    Unsupported,
    #[error("File I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct CafController {
    pub bone_crc32:u32,
    pub rotation_times:Vec<f32>,
    /// CryEngine quaternion XYZW in source coordinates.
    pub rotations:Vec<[f32;4]>,
    pub position_times:Vec<f32>,
    pub positions:Vec<[f32;3]>,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct NativeCafMotion {
    pub schema:u32,
    pub controllers:Vec<CafController>,
    pub start_frame:f32,
    pub end_frame:f32,
    pub fps:f32,
    pub additive:bool,
    pub cyclic:bool,
}
fn u16_at(raw:&[u8],at:usize)->Result<u16,NativeCafError>{
    let v=raw.get(at..at+2).ok_or(NativeCafError::Unsupported)?;
    Ok(u16::from_le_bytes([v[0],v[1]]))
}
fn u32_at(raw:&[u8],at:usize)->Result<u32,NativeCafError>{
    let v=raw.get(at..at+4).ok_or(NativeCafError::Unsupported)?;
    Ok(u32::from_le_bytes(v.try_into().map_err(|_|NativeCafError::Unsupported)?))
}
fn float_at(raw:&[u8],at:usize)->Result<f32,NativeCafError>{
    Ok(f32::from_bits(u32_at(raw,at)?))
}
fn quaternion(raw:&[u8],at:usize,format:u8)->Result<[f32;4],NativeCafError>{
    let mut q=[0f32;4];
    if format==1 {
        for (j,item) in q.iter_mut().enumerate(){*item=float_at(raw,at+j*4)?;}
    }else if format==3 {
        for (j,item) in q.iter_mut().enumerate().take(3){
            *item=i16::from_le_bytes(raw.get(at+j*2..at+j*2+2)
                .ok_or(NativeCafError::Unsupported)?.try_into().map_err(|_|NativeCafError::Unsupported)?) as f32/32767.;
        }
        q[3]=(1.-q.iter().take(3).map(|v|v*v).sum::<f32>()).max(0.).sqrt();
    }else{
        let (widths,size):([usize;3],usize)=match format{
            4=>([10,10,10],4),
            5=>([15,15,15],6),
            6=>([20,20,20],8),
            8=>([21,21,20],8),
            _=>return Err(NativeCafError::Unsupported),
        };
        let mut packed=0u128;
        for (i,value) in raw.get(at..at+size).ok_or(NativeCafError::Unsupported)?.iter().enumerate(){
            packed|=u128::from(*value) <<(i*8);
        }
        let largest=((packed>> (size*8-2))&3) as usize;
        let mut components=[0f32;3];
        let mut shift=0usize;
        for (i,bits) in widths.into_iter().enumerate(){
            let value=(packed>>shift)&((1u128<<bits)-1);
            let scale=match bits {10=>723.,15=>23170.,20=>741454.,21=>1482909.,_=>return Err(NativeCafError::Unsupported)};
            components[i]=value as f32/scale-std::f32::consts::FRAC_1_SQRT_2;
            shift+=bits;
        }
        let mut k=0usize;
        for (i,item) in q.iter_mut().enumerate(){
            if i==largest{
                *item=(1.-components.iter().map(|v|v*v).sum::<f32>()).max(0.).sqrt();
            }else{
                *item=components[k];
                k+=1;
            }
        }
    }
    let norm=q.iter().map(|v|v*v).sum::<f32>().sqrt();
    if !norm.is_finite() || !(0.25..=1.75).contains(&norm) {
        return Err(NativeCafError::Format("unrealistic quaternion".into()));
    }
    for value in &mut q {*value/=norm;}
    Ok(q)
}
fn rotation_size(id:u8)->Option<usize>{
    match id{1=>Some(16),3|5=>Some(6),4=>Some(4),6|8=>Some(8),_=>None}
}
fn times(raw:&[u8],offset:usize,count:usize,ty:u8)->Result<(Vec<f32>,usize),NativeCafError>{
    let stride=match ty{0|3=>4,1|4=>2,2|5=>1,_=>return Err(NativeCafError::Unsupported)};
    let end=offset.checked_add(count.checked_mul(stride).ok_or(NativeCafError::Unsupported)?)
        .ok_or(NativeCafError::Unsupported)?;
    if end>raw.len(){return Err(NativeCafError::Unsupported);}
    let mut samples=Vec::with_capacity(count);
    for i in 0..count {
        let at=offset+i*stride;
        let value=match stride{
            1=>raw[at] as f32,
            2=>f32::from(u16_at(raw,at)?),
            4=>float_at(raw,at)?,
            _=>unreachable!(),
        };
        if !value.is_finite() || samples.last().is_some_and(|v|value<*v){
            return Err(NativeCafError::Format("nonmonotonic or nonfinite timestamps".into()));
        }
        samples.push(value);
    }
    Ok((samples,end))
}
fn align4(n:usize)->usize{(n+3)&!3}
fn controller(raw:&[u8],version:u16)->Result<CafController,NativeCafError>{
    let (cid,nr,np,rot_fmt,rot_time,pos_fmt,pos_shared,pos_time,align,mut at)=match version{
        0x0829=>{
            if raw.len()<16{return Err(NativeCafError::Unsupported);}
            (u32_at(raw,0)?,u16_at(raw,4)? as usize,u16_at(raw,6)? as usize,
                raw[8],raw[9],raw[10],raw[11],raw[12],raw[13],16usize)
        },
        0x0831=>{
            if raw.len()<20{return Err(NativeCafError::Unsupported);}
            (u32_at(raw,0)?,u16_at(raw,8)? as usize,u16_at(raw,10)? as usize,
                raw[12],raw[13],raw[14],raw[15],raw[16],raw[17],20usize)
        },
        _=>return Err(NativeCafError::Unsupported)
    };
    if nr+np==0 || nr>MAX_KEYS || np>MAX_KEYS {
        return Err(NativeCafError::Format("unsupported keyframe count".into()));
    }
    let mut rotation_times=Vec::new();
    let mut rotations=Vec::new();
    if nr>0 {
        let width=rotation_size(rot_fmt).ok_or(NativeCafError::Unsupported)?;
        let end=at.checked_add(nr.checked_mul(width).ok_or(NativeCafError::Unsupported)?)
            .ok_or(NativeCafError::Unsupported)?;
        if end>raw.len(){return Err(NativeCafError::Unsupported);}
        for i in 0..nr {rotations.push(quaternion(raw,at+i*width,rot_fmt)?);}
        at=end;
        if align!=0{at=align4(at);}
        let (decoded,offset)=times(raw,at,nr,rot_time)?;
        rotation_times=decoded;
        at=offset;
        if align!=0 && np>0{at=align4(at);}
    }
    let mut position_times=Vec::new();
    let mut positions=Vec::new();
    if np>0 {
        if pos_fmt!=0 && pos_fmt!=2{return Err(NativeCafError::Unsupported);}
        let end=at.checked_add(np.checked_mul(12).ok_or(NativeCafError::Unsupported)?)
            .ok_or(NativeCafError::Unsupported)?;
        if end>raw.len(){return Err(NativeCafError::Unsupported);}
        for i in 0..np {
            let at=at+i*12;
            let xyz=[float_at(raw,at)?,float_at(raw,at+4)?,float_at(raw,at+8)?];
            if xyz.iter().any(|v|!v.is_finite()||v.abs()>10000.){
                return Err(NativeCafError::Format("invalid bone position".into()));
            }
            positions.push(xyz);
        }
        at=end;
        if pos_shared!=0 {
            if align!=0{at=align4(at);}
            let (decoded,offset)=times(raw,at,np,pos_time)?;
            position_times=decoded;
            at=offset;
        }else{
            if rotation_times.len()!=np{
                return Err(NativeCafError::Format("position key times missing".into()));
            }
            position_times=rotation_times.clone();
        }
    }
    if at>raw.len(){return Err(NativeCafError::Unsupported);}
    Ok(CafController{bone_crc32:cid,rotation_times,rotations,position_times,positions})
}

pub fn decode_native_caf(bytes:&[u8])->Result<NativeCafMotion,NativeCafError>{
    if bytes.len()<32 || bytes.len()>MAX_CAF || bytes.get(..8)!=Some(b"CrChF\x07\0\0"){
        return Err(NativeCafError::Format("unrecognized CAF magic/version".into()));
    }
    let count=u32_at(bytes,8)? as usize;
    let table=u32_at(bytes,12)? as usize;
    let end=table.checked_add(count.checked_mul(16).ok_or(NativeCafError::Unsupported)?)
        .ok_or(NativeCafError::Unsupported)?;
    if count==0 || count>MAX_CONTROLLER+20 || table<16 || end>bytes.len() {
        return Err(NativeCafError::Format("invalid chunk table".into()));
    }
    let mut tracks=Vec::new();
    let mut used=HashSet::new();
    let mut flags=0u32;
    let mut start=0f32;
    let mut last=0f32;
    let mut fps=30f32;
    for i in 0..count {
        let at=table+i*16;
        let kind=u32_at(bytes,at)?;
        let size=u32_at(bytes,at+8)? as usize;
        let offset=u32_at(bytes,at+12)? as usize;
        if size==0 || offset<end || offset.checked_add(size).is_none_or(|n|n>bytes.len()){
            return Err(NativeCafError::Format("invalid chunk boundaries".into()));
        }
        if kind & 0xffff == 0x100d {
            let decoded=controller(&bytes[offset..offset+size],(kind>>16) as u16)?;
            if !used.insert(decoded.bone_crc32){return Err(NativeCafError::Format("duplicate joint controller".into()));}
            tracks.push(decoded);
            if tracks.len()>MAX_CONTROLLER {return Err(NativeCafError::Unsupported);}
        }else if kind==0x09253002 && size>=40 {
            flags=u32_at(bytes,offset)?;
            let ticks=u32_at(bytes,offset+8)? as i32;
            let secs=float_at(bytes,offset+12)?;
            let first=u32_at(bytes,offset+16)? as i32;
            let endframe=u32_at(bytes,offset+20)? as i32;
            if ticks>0 && secs>0. && secs<1. {
                fps=1.0/(ticks as f32*secs);
                start=first as f32/ticks as f32;
                last=endframe as f32/ticks as f32;
            }
        }
    }
    if tracks.is_empty() {return Err(NativeCafError::Format("no playable bone controller".into()));}
    if !fps.is_finite() || !(1.0..=240.).contains(&fps){fps=30.;}
    if last<=start {
        last=tracks.iter().flat_map(|track|
            track.rotation_times.iter().chain(&track.position_times).copied())
            .fold(0f32,f32::max);
    }
    Ok(NativeCafMotion{
        schema:1,controllers:tracks,start_frame:start,end_frame:last,fps,
        additive:flags&1!=0,cyclic:flags&2!=0,
    })
}
pub fn read_native_caf(path:&Path)->Result<NativeCafMotion,NativeCafError>{
    let stat=fs::symlink_metadata(path)?;
    if !stat.file_type().is_file() || stat.len()>MAX_CAF as u64{return Err(NativeCafError::Unsupported);}
    decode_native_caf(&fs::read(path)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample()->Vec<u8>{
        let mut ctrl=Vec::new();
        ctrl.extend_from_slice(&123u32.to_le_bytes());
        ctrl.extend_from_slice(&2u16.to_le_bytes()); // two rotation keys
        ctrl.extend_from_slice(&2u16.to_le_bytes()); // two position keys
        ctrl.extend_from_slice(&[1,0,0,0,0,0]); // rot float4, times float, position uses rot times
        ctrl.extend_from_slice(&0u16.to_le_bytes());
        for _ in 0..2{for f in [0.,0.,0.,1.] {ctrl.extend_from_slice(&f32::to_le_bytes(f));}}
        for f in [0f32,30f32]{ctrl.extend_from_slice(&f.to_le_bytes());}
        for xyz in [[0.,0.,0.],[5.,0.,0.]]{for f in xyz{ctrl.extend_from_slice(&f32::to_le_bytes(f));}}
        assert_eq!(ctrl.len(),80);
        let mut data=vec![0u8;32];
        data[..8].copy_from_slice(b"CrChF\x07\0\0");
        data[8..12].copy_from_slice(&1u32.to_le_bytes());
        data[12..16].copy_from_slice(&16u32.to_le_bytes());
        data[16..20].copy_from_slice(&0x0829100du32.to_le_bytes());
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data[24..28].copy_from_slice(&(ctrl.len() as u32).to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());
        data.extend_from_slice(&ctrl);
        data
    }
    #[test]
    fn decodes_crchf_native_caf_quaternion_and_position_tracks(){
        let motion=decode_native_caf(&sample()).unwrap();
        assert_eq!(motion.controllers.len(),1);
        let track=&motion.controllers[0];
        assert_eq!(track.bone_crc32,123);
        assert_eq!(track.position_times,[0.,30.]);
        assert_eq!(track.rotations[1],[0.,0.,0.,1.]);
        assert_eq!(track.positions[1],[5.,0.,0.]);
        assert_eq!(motion.end_frame,30.);
    }
    #[test]
    fn refuses_unsupported_or_truncated_controller(){
        let mut data=sample();
        data[16..20].copy_from_slice(&0x0800100du32.to_le_bytes());
        assert!(decode_native_caf(&data).is_err());
        let mut data=sample();
        data.truncate(data.len()-5);
        assert!(decode_native_caf(&data).is_err());
    }
}
