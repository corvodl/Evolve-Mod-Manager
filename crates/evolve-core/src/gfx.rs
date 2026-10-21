//! Source-only Scaleform GFX/CFX inspection. No scripts or game callbacks run.
//! Matches the private manager's bounded FFDec editing preflight.
use flate2::read::ZlibDecoder;
use std::io::Read;
use thiserror::Error;

pub const MAX_SOURCE: usize = 12 * 1024 * 1024;
pub const MAX_DECOMPRESSED: usize = 32 * 1024 * 1024;
const MAX_TAGS: usize = 80_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GfxResource {
    pub kind: &'static str,
    pub id: u16,
}

#[derive(Clone, Debug)]
pub struct GfxInfo {
    pub version: u8,
    pub compressed: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub frames: u16,
    pub root_placements: u32,
    pub script_present: bool,
    pub sprites: Vec<(u16, u16)>,
    pub resources: Vec<GfxResource>,
    pub imports: Vec<String>,
    pub tags: usize,
}

#[derive(Debug, Error)]
pub enum GfxError {
    #[error("Invalid GFX/CFX movie: {0}")]
    Invalid(&'static str),
    #[error("Unable to decompress GFX: {0}")]
    Decode(#[from] std::io::Error),
}
use GfxError::Invalid;

fn bits(data: &[u8], start: usize, width: usize) -> Result<i32, GfxError> {
    if width == 0 || width > 31 || start + width > data.len() * 8 {
        return Err(Invalid("Truncated frame rectangle"));
    }
    let mut number = 0u32;
    for bit in start..start + width {
        number = (number << 1) | u32::from(data[bit / 8] >> (7 - bit % 8) & 1);
    }
    let mask = 1u32 << (width - 1);
    Ok(if number & mask != 0 {
        (number as i64 - (1_i64 << width)) as i32
    } else {
        number as i32
    })
}

fn clean_import(value: &[u8]) -> Result<String, GfxError> {
    let end = value
        .iter()
        .position(|b| *b == 0)
        .ok_or(Invalid("Unterminated imported movie name"))?;
    if end == 0 || end > 1024 {
        return Err(Invalid("Invalid imported movie name"));
    }
    let movie = std::str::from_utf8(&value[..end])
        .map_err(|_| Invalid("Invalid imported movie encoding"))?;
    // Only file-local, relative imports; never open an arbitrary URL or path.
    if movie.contains('\\')
        || movie.contains(':')
        || movie.starts_with('/')
        || movie
            .split('/')
            .any(|piece| piece.is_empty() || piece == "." || piece == "..")
    {
        return Err(Invalid("Unsafe external GFX import reference"));
    }
    Ok(movie.to_owned())
}

pub fn inspect_gfx(original: &[u8]) -> Result<GfxInfo, GfxError> {
    if original.len() < 16 || original.len() > MAX_SOURCE {
        return Err(Invalid("GFX movie missing or exceeds 12 MiB"));
    }
    let compressed = match &original[..3] {
        b"GFX" => false,
        b"CFX" => true,
        _ => return Err(Invalid("Expected GFX or CFX signature")),
    };
    let version = original[3];
    let declared = u32::from_le_bytes(original[4..8].try_into().unwrap()) as usize;
    if !(5..=50).contains(&version) || !(16..=MAX_DECOMPRESSED).contains(&declared) {
        return Err(Invalid("Unsupported version or decompressed length"));
    }
    let decoded;
    let source = if compressed {
        let mut decoder = ZlibDecoder::new(&original[8..]);
        let mut expanded = Vec::new();
        decoder
            .by_ref()
            .take((MAX_DECOMPRESSED + 1) as u64)
            .read_to_end(&mut expanded)?;
        if expanded.len() != declared - 8 {
            return Err(Invalid(
                "Decompressed length differs from declared GFX length",
            ));
        }
        let mut data = original[..8].to_vec();
        data[0] = b'G';
        data.extend(expanded);
        decoded = data;
        decoded.as_slice()
    } else {
        original
    };
    if source.len() != declared {
        return Err(Invalid("GFX length differs from declared size"));
    }
    let body = &source[8..];
    let nbits = ((body[0] >> 3) & 31) as usize;
    if nbits == 0 || nbits > 31 {
        return Err(Invalid("Invalid frame bounds bit width"));
    }
    let mut values = [0i32; 4];
    for (index, value) in values.iter_mut().enumerate() {
        *value = bits(body, 5 + index * nbits, nbits)?;
    }
    let offset = 8 + (5 + 4 * nbits).div_ceil(8);
    if offset + 4 > source.len() {
        return Err(Invalid("Truncated movie frame rate/count"));
    }
    let frame_rate = u16::from_le_bytes(source[offset..offset + 2].try_into().unwrap());
    let frames = u16::from_le_bytes(source[offset + 2..offset + 4].try_into().unwrap());
    let width = (values[1] - values[0]) / 20;
    let height = (values[3] - values[2]) / 20;
    if !(1..=8192).contains(&width)
        || !(1..=8192).contains(&height)
        || i64::from(width) * i64::from(height) > 36_000_000
        || !(1..=2048).contains(&frames)
    {
        return Err(Invalid("Invalid GFX stage dimensions or frames"));
    }
    let mut info = GfxInfo {
        version,
        compressed,
        width: width as u32,
        height: height as u32,
        fps: f32::from(frame_rate) / 256.0,
        frames,
        root_placements: 0,
        script_present: false,
        sprites: Vec::new(),
        resources: Vec::new(),
        imports: Vec::new(),
        tags: 0,
    };
    let mut offset = offset + 4;
    while offset + 2 <= source.len() {
        let header = u16::from_le_bytes(source[offset..offset + 2].try_into().unwrap());
        offset += 2;
        let tag = header >> 6;
        let mut size = usize::from(header & 63);
        if size == 63 {
            if offset + 4 > source.len() {
                return Err(Invalid("Truncated long tag length"));
            }
            size = u32::from_le_bytes(source[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
        }
        if size > MAX_DECOMPRESSED || size > source.len() - offset {
            return Err(Invalid("Truncated GFX tag"));
        }
        let payload = &source[offset..offset + size];
        offset += size;
        info.tags += 1;
        if info.tags > MAX_TAGS {
            return Err(Invalid("Too many movie tags"));
        }
        if tag == 39 && payload.len() >= 4 {
            let id = u16::from_le_bytes([payload[0], payload[1]]);
            let count = u16::from_le_bytes([payload[2], payload[3]]);
            if info.sprites.len() >= 1024 || count > 2048 {
                return Err(Invalid("Sprite list exceeds safe bounds"));
            }
            info.sprites.push((id, count));
        }
        if [4, 26, 70].contains(&tag) {
            info.root_placements += 1;
        }
        if [12, 59, 72, 82].contains(&tag) {
            info.script_present = true;
        }
        if [57, 71].contains(&tag) {
            if info.imports.len() >= 128 {
                return Err(Invalid("Too many imported movies"));
            }
            info.imports.push(clean_import(payload)?);
        }
        let kind = match tag {
            6 | 20 | 21 | 35 | 36 | 90 => Some("image"),
            2 | 22 | 32 | 83 => Some("shape"),
            11 | 33 | 37 => Some("text"),
            14 => Some("sound"),
            87 => Some("binary"),
            7 | 34 => Some("button"),
            _ => None,
        };
        if let Some(kind) = kind
            && payload.len() >= 2
        {
            let id = u16::from_le_bytes([payload[0], payload[1]]);
            if info.resources.len() >= 10_000 {
                return Err(Invalid("Too many GFX resource tags"));
            }
            info.resources.push(GfxResource { kind, id });
        }
        if tag == 0 {
            break;
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tag: Option<(u16, u16)>) -> Vec<u8> {
        // RECT: 12-bit signed values (0,1920*20,0,1080*20).
        let numbers = [0i32, 38400, 0, 21600];
        let mut bits = vec![0u8; 5 + 4 * 17];
        let mut put = |offset: usize, width: usize, v: u32| {
            for i in 0..width {
                bits[offset + i] = ((v >> (width - 1 - i)) & 1) as u8;
            }
        };
        put(0, 5, 17);
        for (index, n) in numbers.into_iter().enumerate() {
            put(5 + index * 17, 17, n as u32);
        }
        let mut frame = vec![0u8; bits.len().div_ceil(8)];
        for (i, b) in bits.iter().enumerate() {
            frame[i / 8] |= b << (7 - i % 8);
        }
        frame.extend(7680u16.to_le_bytes()); // 30fps
        frame.extend(1u16.to_le_bytes());
        if let Some((code, id)) = tag {
            frame.extend(((code << 6) | 2).to_le_bytes());
            frame.extend(id.to_le_bytes());
        }
        frame.extend(0u16.to_le_bytes());
        let mut movie = b"GFX".to_vec();
        movie.push(15);
        movie.extend(((frame.len() + 8) as u32).to_le_bytes());
        movie.extend(frame);
        movie
    }
    #[test]
    fn reads_real_movie_dimensions_and_resource_tag() {
        let info = inspect_gfx(&fixture(Some((21, 7)))).unwrap();
        assert_eq!(
            (info.width, info.height, info.fps, info.frames),
            (1920, 1080, 30.0, 1)
        );
        assert_eq!(
            info.resources,
            vec![GfxResource {
                kind: "image",
                id: 7
            }]
        );
    }
    #[test]
    fn cfx_decompression_and_malformed_inputs_are_bounded() {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let raw = fixture(None);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw[8..]).unwrap();
        let mut compressed = b"CFX".to_vec();
        compressed.extend(&raw[3..8]);
        compressed.extend(encoder.finish().unwrap());
        assert_eq!(inspect_gfx(&compressed).unwrap().width, 1920);
        assert!(inspect_gfx(b"FWS\x09").is_err());
        let mut truncated = raw.clone();
        truncated.pop();
        assert!(inspect_gfx(&truncated).is_err());
    }
    #[test]
    fn reject_import_escape() {
        assert!(clean_import(b"../external.gfx\0").is_err());
        assert!(clean_import(b"https://evil.invalid\0").is_err());
        assert_eq!(
            clean_import(b"gfxfontlib_locfont.gfx\0").unwrap(),
            "gfxfontlib_locfont.gfx"
        );
    }
}
