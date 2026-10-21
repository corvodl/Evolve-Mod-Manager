//! Conservative, native Scaleform root first-frame layout editor.
//! Ported from private gfx_layout_backend.py: never infer runtime ActionScript
//! state, duplicate depths, or dimensions for sprite/image class instances.
use crate::gfx::{GfxError, MAX_DECOMPRESSED, inspect_gfx};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

fn invalid(reason: &'static str) -> GfxError { GfxError::Invalid(reason) }

#[derive(Clone, Debug, PartialEq)]
pub struct StaticLayer {
    pub depth: u16,
    pub character: u16,
    pub label: String,
    pub bounds: Option<[f32; 4]>, // SWF local shape x_min,x_max,y_min,y_max, in pixels.
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
    pub editable: bool,
    pub reason: &'static str,
}
#[derive(Clone, Copy, Debug)]
pub struct LayoutEdit {
    pub depth: u16,
    pub character: u16,
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
    pub visible: bool,
}

struct Bits<'a> { data: &'a [u8], bit: usize }
impl Bits<'_> {
    fn take(&mut self, count: usize) -> Result<u32, GfxError> {
        if count > 31 || self.bit.checked_add(count).is_none_or(|end| end > self.data.len() * 8) {
            return Err(invalid("Truncated Scaleform matrix/rectangle"));
        }
        let mut v = 0u32;
        for _ in 0..count {
            v = (v << 1) | u32::from(self.data[self.bit / 8] >> (7 - self.bit % 8) & 1);
            self.bit += 1;
        }
        Ok(v)
    }
    fn signed(&mut self, count: usize) -> Result<i32, GfxError> {
        if count == 0 { return Ok(0); }
        let v = self.take(count)?;
        let sign = 1 << (count-1);
        Ok(if v & sign != 0 { (i64::from(v) - (1i64 << count)) as i32 } else { v as i32 })
    }
    fn size(&self) -> usize { self.bit.div_ceil(8) }
}
fn rect(data: &[u8]) -> Result<([f32;4], usize), GfxError> {
    let mut bits = Bits { data, bit: 0 };
    let count = bits.take(5)? as usize;
    if !(2..=31).contains(&count) { return Err(invalid("Unsupported SWF rectangle encoding")); }
    let mut out = [0f32;4];
    for item in &mut out { *item = bits.signed(count)? as f32 / 20.; }
    Ok((out, bits.size()))
}
fn matrix(data: &[u8]) -> Result<([f32;6],usize),GfxError> {
    let mut b=Bits { data,bit:0 };
    let (mut a,mut d,mut c,mut m)=(1f32,1f32,0f32,0f32);
    if b.take(1)? != 0 {
        let n=b.take(5)? as usize;
        a=b.signed(n)? as f32 / 65536.;
        d=b.signed(n)? as f32 / 65536.;
    }
    if b.take(1)? != 0 {
        let n=b.take(5)? as usize;
        c=b.signed(n)? as f32 / 65536.;
        m=b.signed(n)? as f32 / 65536.;
    }
    let n=b.take(5)? as usize;
    let x=b.signed(n)? as f32 / 20.;
    let y=b.signed(n)? as f32 / 20.;
    Ok(([a,c,m,d,x,y],b.size()))
}
#[derive(Clone)]
struct Tag { code: u16, payload: Vec<u8> }
fn raw_movie(data: &[u8]) -> Result<Vec<u8>,GfxError> {
    inspect_gfx(data)?;
    if &data[..3]!=b"CFX" { return Ok(data.to_vec()); }
    let mut out=data[..8].to_vec();
    out[0]=b'G';
    let mut decoder=ZlibDecoder::new(&data[8..]);
    decoder.take((MAX_DECOMPRESSED+1) as u64).read_to_end(&mut out)?;
    if out.len()!=u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize {
        return Err(invalid("Compressed GFX decompression size mismatch"));
    }
    Ok(out)
}
fn root_tags(data: &[u8]) -> Result<(Vec<u8>, Vec<Tag>), GfxError> {
    let raw=raw_movie(data)?;
    let (_,rect_size)=rect(&raw[8..])?;
    let mut at=8+rect_size+4;
    if at>raw.len() { return Err(invalid("Truncated GFX tag header")); }
    let head=raw[..at].to_vec();
    let mut tags=Vec::new();
    while at+2<=raw.len() {
        let word=u16::from_le_bytes(raw[at..at+2].try_into().unwrap());
        at+=2;
        let code=word >> 6;
        let mut len=usize::from(word&63);
        if len==63 {
            if at+4>raw.len() { return Err(invalid("Truncated long GFX tag")); }
            len=u32::from_le_bytes(raw[at..at+4].try_into().unwrap()) as usize;
            at+=4;
        }
        if len>MAX_DECOMPRESSED || len>raw.len()-at {
            return Err(invalid("Truncated GFX tag payload"));
        }
        tags.push(Tag { code, payload: raw[at..at+len].to_vec() });
        at+=len;
        if tags.len()>80_000 { return Err(invalid("Too many Scaleform tags")); }
        if code==0 { break; }
    }
    Ok((head,tags))
}
#[derive(Clone)]
struct Placement { character:u16,depth:u16, transform:[f32;6], offset:usize, len:usize, safe:bool }
fn placement(code:u16,data:&[u8])->Option<Placement> {
    let (character,depth,offset,len,safe,transform)=match code {
        4 if data.len()>=5 => {
            let id=u16::from_le_bytes([data[0],data[1]]);
            let depth=u16::from_le_bytes([data[2],data[3]]);
            let (matrix,size)=matrix(&data[4..]).ok()?;
            (id,depth,4,size,true,matrix)
        },
        26 if data.len()>=5 && data[0]&2!=0 => {
            let depth=u16::from_le_bytes([data[1],data[2]]);
            let id=u16::from_le_bytes([data[3],data[4]]);
            let (matrix,size)=if data[0]&4!=0 { matrix(&data[5..]).ok()? } else {([1.,0.,0.,1.,0.,0.],0)};
            (id,depth,5,size,true,matrix)
        },
        70 if data.len()>=6 && data[0]&2!=0 && data[1]&8==0 => {
            let depth=u16::from_le_bytes([data[2],data[3]]);
            let id=u16::from_le_bytes([data[4],data[5]]);
            let matrix=if data[0]&4!=0 { matrix(&data[6..]).ok()?.0 } else { [1.,0.,0.,1.,0.,0.] };
            (id,depth,6,0,false,matrix)
        },
        _=>return None,
    };
    Some(Placement{character,depth,transform,offset,len,safe})
}
fn props(m:[f32;6])->(f32,f32,f32,f32,f32,bool) {
    let [a,c,b,d,x,y]=m;
    let (sx,sy)=(a.hypot(b),c.hypot(d));
    if sx<0.01||sy<0.01 {return (x,y,sx,sy,0.,false);}
    let dot=(a*c+b*d)/(sx*sy);
    (x,y,sx,sy,b.atan2(a).to_degrees(),dot.abs()<=0.002 && a*d-b*c>0.)
}
fn symbols(tags:&[Tag])->HashMap<u16,String> {
    let mut names=HashMap::new();
    for tag in tags.iter().filter(|t|t.code==56) {
        if tag.payload.len()<2 {continue;}
        let count=usize::from(u16::from_le_bytes([tag.payload[0],tag.payload[1]]));
        let mut index=2;
        for _ in 0..count.min(2048) {
            if index+2>tag.payload.len() { break; }
            let id=u16::from_le_bytes([tag.payload[index],tag.payload[index+1]]);
            index+=2;
            let Some(end)=tag.payload[index..].iter().position(|v|*v==0) else {break};
            if end<=256 {names.insert(id,String::from_utf8_lossy(&tag.payload[index..index+end]).to_string());}
            index+=end+1;
        }
    }
    names
}
fn first_frame_layers(tags:&[Tag])->Vec<StaticLayer> {
    let names=symbols(tags);
    let mut shapes=HashMap::new();
    let mut placed:HashMap<u16,Option<Placement>>=HashMap::new();
    let mut seen=HashSet::new();
    let mut first=true;
    for tag in tags {
        if matches!(tag.code,2|22|32|83) && tag.payload.len()>2 {
            if let Ok((bounds,_))=rect(&tag.payload[2..]){
                shapes.insert(u16::from_le_bytes([tag.payload[0],tag.payload[1]]),bounds);
            }
        }
        if tag.code==1 {first=false;}
        if !first {continue;}
        if let Some(item)=placement(tag.code,&tag.payload){
            if !seen.insert(item.depth){ placed.insert(item.depth,None); }
            else {placed.insert(item.depth,Some(item));}
        }else if tag.code==28 && tag.payload.len()>=2 {
            let depth=u16::from_le_bytes([tag.payload[0],tag.payload[1]]);
            placed.remove(&depth);seen.remove(&depth);
        }else if tag.code==5 && tag.payload.len()>=4 {
            let depth=u16::from_le_bytes([tag.payload[2],tag.payload[3]]);
            placed.remove(&depth);seen.remove(&depth);
        }
    }
    let mut elements:Vec<_>=placed.into_iter().filter_map(|(_,item)|item).map(|item|{
        let (x,y,sx,sy,angle,supported)=props(item.transform);
        let bounds=shapes.get(&item.character).copied();
        let editable=item.safe&&supported;
        StaticLayer {
            depth:item.depth,character:item.character,
            label:names.get(&item.character).cloned()
                .unwrap_or_else(||format!("Character #{}",item.character)),
            bounds,x,y,scale_x:sx,scale_y:sy,rotation:angle,editable,
            reason:if !editable {"Unsupported PlaceObject3 or skewed/reflected transform"}
                else if bounds.is_some(){"Verified first-frame shape"}
                else {"No verified shape bounds — numeric transform only"}
        }
    }).collect();
    elements.sort_by_key(|item|item.depth);
    elements.truncate(512);
    elements
}
pub fn static_layers(movie:&[u8])->Result<Vec<StaticLayer>,GfxError> {
    let (_,tags)=root_tags(movie)?;
    Ok(first_frame_layers(&tags))
}
fn signed_width(n:i32)->usize {
    // Include the negative bound and avoid lossy positive sign bits.
    for width in 2..=31 {
        let low=-(1i64<<(width-1));
        let high=(1i64<<(width-1))-1;
        if i64::from(n)>=low && i64::from(n)<=high {return width;}
    }
    32
}
fn encoded_matrix(edit:LayoutEdit)->Result<Vec<u8>,GfxError> {
    let values=[edit.x,edit.y,edit.scale_x,edit.scale_y,edit.rotation];
    if values.iter().any(|v|!v.is_finite()) || edit.x.abs()>100_000. ||
        edit.y.abs()>100_000. || !(0.01..=20.).contains(&edit.scale_x) ||
        !(0.01..=20.).contains(&edit.scale_y) || edit.rotation.abs()>360. {
        return Err(invalid("Layout transform is non-finite or outside safe bounds"));
    }
    let radians=edit.rotation.to_radians();
    let (sin,cos)=radians.sin_cos();
    let parts=[(edit.scale_x*cos*65536.).round() as i32,
        (edit.scale_y*cos*65536.).round() as i32,
        (-edit.scale_y*sin*65536.).round() as i32,
        (edit.scale_x*sin*65536.).round() as i32];
    let n=parts.iter().map(|v|signed_width(*v)).max().unwrap_or(0);
    let tx=(edit.x*20.).round() as i32;
    let ty=(edit.y*20.).round() as i32;
    let nt=signed_width(tx).max(signed_width(ty));
    if n>31||nt>31 {return Err(invalid("Transform exceeds SWF MATRIX bit width"));}
    let mut bits=Vec::with_capacity(192);
    let mut put=|v:i64,count:usize| {
        for bit in (0..count).rev(){bits.push(((v>>bit)&1) as u8);}
    };
    put(1,1);put(n as i64,5);
    for value in parts.iter().take(2){put(i64::from(*value),n);}
    put(1,1);put(n as i64,5);
    for value in parts.iter().skip(2){put(i64::from(*value),n);}
    put(nt as i64,5);put(i64::from(tx),nt);put(i64::from(ty),nt);
    bits.resize(bits.len().div_ceil(8)*8,0);
    Ok(bits.chunks(8).map(|chunk|chunk.iter().fold(0u8,|v,b|(v<<1)|b)).collect())
}
fn rebuild(head:&[u8],tags:&[Tag],compressed:bool)->Result<Vec<u8>,GfxError> {
    let mut source=head.to_vec();
    for tag in tags {
        let len=tag.payload.len();
        if len<63 {source.extend(((tag.code<<6)|(len as u16)).to_le_bytes());}
        else {
            source.extend(((tag.code<<6)|63).to_le_bytes());
            source.extend((len as u32).to_le_bytes());
        }
        source.extend(&tag.payload);
    }
    if source.len()>MAX_DECOMPRESSED {return Err(invalid("Edited GFX exceeds decoded size limit"));}
    let len=source.len() as u32;
    source[4..8].copy_from_slice(&len.to_le_bytes());
    if !compressed {inspect_gfx(&source)?;return Ok(source);}
    let mut result=source[..8].to_vec();result[0]=b'C';
    let mut zip=ZlibEncoder::new(Vec::new(),Compression::default());
    zip.write_all(&source[8..])?;
    result.extend(zip.finish()?);
    if result.len()>crate::gfx::MAX_SOURCE {return Err(invalid("Edited compressed GFX exceeds source limit"));}
    inspect_gfx(&result)?;
    Ok(result)
}
pub fn edit_first_frame(movie:&[u8],edit:LayoutEdit)->Result<Vec<u8>,GfxError> {
    let (head,mut tags)=root_tags(movie)?;
    let layers=first_frame_layers(&tags);
    if layers.iter().filter(|item| item.depth==edit.depth && item.character==edit.character && item.editable).count()!=1{
        return Err(invalid("Selected root first-frame placement is ambiguous or not editable"));
    }
    let matrix=encoded_matrix(edit)?;
    let mut first=true;
    let mut indices=Vec::new();
    for (i,tag) in tags.iter().enumerate(){
        if tag.code==1 {first=false;}
        if !first {continue;}
        if let Some(p)=placement(tag.code,&tag.payload)
            && p.depth==edit.depth && p.character==edit.character {indices.push((i,p));}
    }
    if indices.len()!=1 {return Err(invalid("Root placement is ambiguous"));}
    let (index,p)=indices.pop().unwrap();
    if !edit.visible {tags.remove(index);}
    else {
        let tag=&mut tags[index];
        if tag.code==26 {tag.payload[0]|=4;}
        tag.payload.splice(p.offset..p.offset+p.len,matrix);
    }
    rebuild(&head,&tags,movie.starts_with(b"CFX"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn put(bits:&mut Vec<u8>,value:u32,len:usize) {
        for i in (0..len).rev(){bits.push(((value>>i)&1) as u8);}
    }
    fn rectangle(values:[i32;4])->Vec<u8>{
        let mut bits=Vec::new();put(&mut bits,16,5);
        for x in values {put(&mut bits,x as u32,16);}
        bits.resize(bits.len().div_ceil(8)*8,0);
        bits.chunks(8).map(|p|p.iter().fold(0,|n,v|(n<<1)|v)).collect()
    }
    fn movie()->Vec<u8>{
        let mut frame=rectangle([0,19200,0,10800]);
        frame.extend(7680u16.to_le_bytes());frame.extend(1u16.to_le_bytes());
        let shape=rectangle([0,2000,0,1000]);
        let mut data=7u16.to_le_bytes().to_vec();data.extend(shape);
        frame.extend(((2u16<<6)|(data.len() as u16)).to_le_bytes());frame.extend(data);
        let initial=LayoutEdit{depth:3,character:7,x:100.,y:50.,scale_x:1.,scale_y:1.,rotation:0.,visible:true};
        let mut payload=7u16.to_le_bytes().to_vec();payload.extend(3u16.to_le_bytes());
        payload.extend(encoded_matrix(initial).unwrap());
        frame.extend(((4u16<<6)|(payload.len() as u16)).to_le_bytes());frame.extend(payload);
        frame.extend(0u16.to_le_bytes());
        let mut file=b"GFX".to_vec();file.push(15);
        file.extend(((frame.len()+8) as u32).to_le_bytes());file.extend(frame);file
    }
    #[test] fn layer_edit_and_hide_rebuild_only_root_tag(){
        let original=movie();
        let layers=static_layers(&original).unwrap();
        assert_eq!(layers.len(),1);
        assert_eq!(layers[0].bounds,Some([0.,100.,0.,50.]));
        assert!((layers[0].x-100.).abs()<0.1);
        let edited=edit_first_frame(&original,LayoutEdit{
            depth:3,character:7,x:210.,y:144.,scale_x:2.,scale_y:0.5,rotation:45.,visible:true
        }).unwrap();
        let after=static_layers(&edited).unwrap();
        assert!((after[0].x-210.).abs()<0.1);
        assert!((after[0].rotation-45.).abs()<0.2);
        assert!((after[0].scale_x-2.).abs()<0.01);
        assert!((after[0].scale_y-0.5).abs()<0.01);
        assert!(static_layers(&edit_first_frame(&edited,LayoutEdit{
            depth:3,character:7,x:210.,y:144.,scale_x:2.,scale_y:0.5,rotation:45.,visible:false
        }).unwrap()).unwrap().is_empty());
    }
    #[test] fn compressed_roundtrip_and_invalid_transform() {
        let raw=movie();
        let mut zip=ZlibEncoder::new(Vec::new(),Compression::default());
        zip.write_all(&raw[8..]).unwrap();
        let mut cfx=b"CFX".to_vec();cfx.extend(&raw[3..8]);cfx.extend(zip.finish().unwrap());
        let edit=LayoutEdit{depth:3,character:7,x:12.,y:8.,scale_x:1.,scale_y:1.,rotation:0.,visible:true};
        assert_eq!(static_layers(&edit_first_frame(&cfx,edit).unwrap()).unwrap()[0].x,12.);
        assert!(edit_first_frame(&raw,LayoutEdit{x:f32::NAN,..edit}).is_err());
        assert!(edit_first_frame(&raw,LayoutEdit{depth:8,..edit}).is_err());
    }
}
