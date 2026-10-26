//! Conservative FSB5 v1 multi-sample bank writer for *existing* sound slots.
//! Original names, sample ordering, unrelated bytes and unknown metadata survive.
//! FMOD Vorbis encoding is deliberately NOT implemented here: the donor must
//! already be a verified single-subsound FSB5 Vorbis bank.
use std::fmt;

const MAX_BANK: usize = 512 * 1024 * 1024;
const MAX_SAMPLES: usize = 100_000;
const OFFSET_MASK: u64 = ((1u64 << 28) - 1) << 6;
const COUNT_MASK: u64 = ((1u64 << 30) - 1) << 34;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildError {
    Invalid(&'static str),
    Unsupported(&'static str),
}
impl fmt::Display for RebuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "Malformed FSB5 bank: {reason}"),
            Self::Unsupported(reason) => write!(f, "Unsupported FSB5 replacement: {reason}"),
        }
    }
}
impl std::error::Error for RebuildError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Extra {
    kind: u8,
    bytes: Vec<u8>,
}
#[derive(Clone)]
struct Sample {
    header: u64,
    extras: Vec<Extra>,
    data: Vec<u8>,
}
impl Sample {
    fn count(&self) -> u64 { self.header >> 34 }
    fn frequency_code(&self) -> u64 { (self.header >> 1) & 15 }
    fn channels(&self) -> u32 {
        self.extras.iter().find(|c| c.kind == 1)
            .and_then(|c| c.bytes.first()).map_or(((self.header >> 5) & 1) as u32 + 1, |v| u32::from(*v))
    }
    fn frequency(&self) -> u32 {
        if let Some(chunk) = self.extras.iter().find(|c| c.kind == 2)
            && let Ok(raw) = <[u8;4]>::try_from(chunk.bytes.as_slice()) {
                return u32::from_le_bytes(raw);
            }
        match self.frequency_code() {
            1 => 8000, 2 => 11000, 3 => 11025, 4 => 16000, 5 => 22050,
            6 => 24000, 7 => 32000, 8 => 44100, 9 => 48000, _ => 0
        }
    }
}
struct Bank {
    header: [u8; 60],
    names: Vec<u8>,
    samples: Vec<Sample>,
    mode: u32,
}
fn invalid(reason: &'static str) -> RebuildError { RebuildError::Invalid(reason) }
fn le32(bytes: &[u8], pos: usize) -> Result<u32, RebuildError> {
    let array: [u8; 4] = bytes.get(pos..pos+4).ok_or_else(|| invalid("truncated u32"))?
        .try_into().map_err(|_| invalid("invalid u32"))?;
    Ok(u32::from_le_bytes(array))
}
fn le64(bytes: &[u8], pos: usize) -> Result<u64, RebuildError> {
    let array: [u8; 8] = bytes.get(pos..pos+8).ok_or_else(|| invalid("truncated sample"))?
        .try_into().map_err(|_| invalid("invalid sample"))?;
    Ok(u64::from_le_bytes(array))
}
fn align16(value: usize) -> Result<usize, RebuildError> {
    value.checked_add(15).map(|v| v & !15).ok_or_else(|| invalid("size overflow"))
}
impl Bank {
    fn parse(raw: &[u8]) -> Result<Self, RebuildError> {
        if raw.len() < 60 || raw.len() > MAX_BANK || &raw[0..4] != b"FSB5" {
            return Err(invalid("missing FSB5 v1 header or bank too large"));
        }
        if le32(raw, 4)? != 1 {
            return Err(RebuildError::Unsupported("only FSB5 header version 1 is supported"));
        }
        let count = le32(raw, 8)? as usize;
        if count == 0 || count > MAX_SAMPLES {
            return Err(invalid("invalid sample count"));
        }
        let header_size = le32(raw, 12)? as usize;
        let names_size = le32(raw, 16)? as usize;
        let data_size = le32(raw, 20)? as usize;
        let mode = le32(raw, 24)?;
        let end_headers = 60usize.checked_add(header_size).ok_or_else(|| invalid("header overflow"))?;
        let end_names = end_headers.checked_add(names_size).ok_or_else(|| invalid("name overflow"))?;
        let end_data = end_names.checked_add(data_size).ok_or_else(|| invalid("data overflow"))?;
        if end_data > raw.len() || header_size < count * 8 {
            return Err(invalid("section extends beyond file"));
        }
        let mut samples = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        let mut cursor = 60;
        for _ in 0..count {
            if cursor.checked_add(8).is_none_or(|end| end > end_headers) {
                return Err(invalid("sample header outside section"));
            }
            let word = le64(raw, cursor)?;
            cursor += 8;
            let mut next = word & 1 == 1;
            let mut extras = Vec::new();
            while next {
                if cursor.checked_add(4).is_none_or(|end| end > end_headers) {
                    return Err(invalid("extra metadata header outside section"));
                }
                let packed = le32(raw, cursor)?;
                cursor += 4;
                next = packed & 1 != 0;
                let size = ((packed >> 1) & 0x00ff_ffff) as usize;
                let kind = (packed >> 25) as u8;
                if cursor.checked_add(size).is_none_or(|end| end > end_headers) {
                    return Err(invalid("extra metadata data outside section"));
                }
                extras.push(Extra { kind, bytes: raw[cursor..cursor + size].to_vec() });
                cursor += size;
                if extras.len() > 64 { return Err(invalid("too many metadata chunks")); }
            }
            offsets.push(((word >> 6) & ((1u64 << 28) - 1)) as usize * 16);
            samples.push(Sample { header: word, extras, data: Vec::new() });
        }
        // Legacy banks may end the sample-header table exactly at the last
        // metadata chunk. Standalone Ogg remuxers can append zero alignment
        // bytes inside the declared header section instead.
        if cursor > end_headers || offsets[0] != 0
            || raw[cursor..end_headers].iter().any(|&byte| byte != 0)
        {
            return Err(invalid("sample metadata, padding or first sample offset mismatch"));
        }
        for index in 0..count {
            let start = offsets[index];
            let end = if index + 1 < count { offsets[index+1] } else { data_size };
            if start >= end || end > data_size || (index + 1 < count && end % 16 != 0) {
                return Err(invalid("sample payload offset is invalid"));
            }
            samples[index].data = raw[end_names + start .. end_names + end].to_vec();
        }
        let header: [u8;60] = raw[..60].try_into().map_err(|_| invalid("header"))?;
        let names = raw[end_headers..end_names].to_vec();
        Ok(Self { header, names, samples, mode })
    }
    fn encode(&self) -> Result<Vec<u8>, RebuildError> {
        if self.samples.is_empty() || self.samples.len() > MAX_SAMPLES {
            return Err(invalid("invalid sample count"));
        }
        let mut headers = Vec::new();
        let mut data = Vec::new();
        for sample in &self.samples {
            let offset = align16(data.len())?;
            data.resize(offset, 0);
            let encoded_offset = (offset / 16) as u64;
            if encoded_offset >= (1u64 << 28) || sample.count() >= (1u64 << 30)
                || sample.data.is_empty() { return Err(invalid("sample offset/count too large")); }
            let mut word = sample.header & !OFFSET_MASK;
            word &= !1u64;
            word |= encoded_offset << 6;
            if !sample.extras.is_empty() { word |= 1; }
            headers.extend_from_slice(&word.to_le_bytes());
            for (i, extra) in sample.extras.iter().enumerate() {
                if extra.bytes.len() > 0x00ff_ffff { return Err(invalid("metadata too large")); }
                let packed = (u32::from(extra.kind) << 25)
                    | ((extra.bytes.len() as u32) << 1)
                    | u32::from(i + 1 < sample.extras.len());
                headers.extend_from_slice(&packed.to_le_bytes());
                headers.extend_from_slice(&extra.bytes);
            }
            data.extend_from_slice(&sample.data);
        }
        if headers.len() > u32::MAX as usize || data.len() > u32::MAX as usize
            || self.names.len() > u32::MAX as usize
            || 60usize.saturating_add(headers.len()).saturating_add(self.names.len()).saturating_add(data.len()) > MAX_BANK
        {
            return Err(invalid("rebuilt bank exceeds size limits"));
        }
        let mut header = self.header;
        header[8..12].copy_from_slice(&(self.samples.len() as u32).to_le_bytes());
        header[12..16].copy_from_slice(&(headers.len() as u32).to_le_bytes());
        header[16..20].copy_from_slice(&(self.names.len() as u32).to_le_bytes());
        header[20..24].copy_from_slice(&(data.len() as u32).to_le_bytes());
        let mut out = Vec::with_capacity(60 + headers.len() + self.names.len() + data.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&headers);
        out.extend_from_slice(&self.names);
        out.extend_from_slice(&data);
        Ok(out)
    }
}

/// Read original sample channels and sample rate so the encoder cannot
/// silently change the monster's playback pitch or spatialization.
pub fn sample_parameters(original: &[u8], one_based: u32) -> Result<(u32, u32), RebuildError> {
    let bank = Bank::parse(original)?;
    let index = one_based.checked_sub(1).ok_or_else(|| invalid("sample indices start at 1"))? as usize;
    let sample = bank.samples.get(index).ok_or_else(|| invalid("sample index is out of range"))?;
    let rate = sample.frequency();
    let channels = sample.channels();
    if rate == 0 || !(1..=8).contains(&channels) {
        return Err(invalid("unsupported sample rate or channel count"));
    }
    Ok((rate, channels))
}

/// Replace a *one-based* Vorbis subsound using a single-subsound FSB5 generated
/// by an independent encoder/remuxer. This is native bank compilation, not
/// Vorbis packet encoding or a guarantee the target game accepts the bank.
pub fn replace_vorbis_sample(
    original: &[u8],
    one_based: u32,
    single_sample_fsb: &[u8],
) -> Result<Vec<u8>, RebuildError> {
    let mut bank = Bank::parse(original)?;
    let mut donor_bank = Bank::parse(single_sample_fsb)?;
    if bank.mode != 15 || donor_bank.mode != 15 {
        return Err(RebuildError::Unsupported("only matching FMOD Vorbis (mode 15) is supported; IMA ADPCM needs a separate encoder"));
    }
    let index = one_based.checked_sub(1).ok_or_else(|| invalid("sample indices start at 1"))? as usize;
    if index >= bank.samples.len() || donor_bank.samples.len() != 1 {
        return Err(invalid("sample index or donor sample count mismatch"));
    }
    let old = &mut bank.samples[index];
    let replacement = donor_bank.samples.pop().ok_or_else(|| invalid("missing donor sample"))?;
    if old.channels() != replacement.channels() || old.frequency() != replacement.frequency() {
        return Err(RebuildError::Unsupported("replacement channels and sample rate must match the original"));
    }
    if replacement.count() == 0 { return Err(invalid("zero-length donor")); }
    if old.extras.iter().filter(|e| e.kind == 11).count() != 1
        || replacement.extras.iter().filter(|e| e.kind == 11).count() != 1
    {
        return Err(invalid("Vorbis setup metadata missing or ambiguous"));
    }
    if let Some(loop_chunk) = old.extras.iter().find(|e| e.kind == 3) {
        if loop_chunk.bytes.len() != 8 || le32(&loop_chunk.bytes, 4)? as u64 > replacement.count() {
            return Err(RebuildError::Unsupported("replacement is shorter than the original loop endpoint"));
        }
    }
    let vorbis_data = replacement.extras.iter().find(|e| e.kind == 11)
        .ok_or_else(|| invalid("donor missing setup"))?.bytes.clone();
    if vorbis_data.len() < 4 {
        return Err(invalid("invalid Vorbis metadata"));
    }
    for extra in &mut old.extras {
        if extra.kind == 11 { extra.bytes = vorbis_data.clone(); }
    }
    let donor_sample_count = replacement.count();
    old.header = (old.header & !COUNT_MASK) | (replacement.header & COUNT_MASK);
    old.data = replacement.data;
    let output = bank.encode()?;
    let check = Bank::parse(&output)?;
    if check.samples.len() != bank.samples.len()
        || check.names != bank.names
        || check.mode != bank.mode
        || check.samples[index].count() != donor_sample_count
    {
        return Err(invalid("rebuilt bank failed read-after-write verification"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(sample_count: usize) -> Vec<u8> {
        let mut header = [0u8;60];
        header[..4].copy_from_slice(b"FSB5");
        header[4..8].copy_from_slice(&1u32.to_le_bytes());
        header[24..28].copy_from_slice(&15u32.to_le_bytes());
        let samples = (0..sample_count).map(|i| Sample {
            header: ((160u64 + i as u64) << 34) | (9u64 << 1) | (1u64 << 5),
            extras: vec![
                Extra {kind:11,bytes:vec![0x44,0x33,0x22,0x11,0,0,0,0]},
                Extra {kind:17,bytes:vec![0xa0,i as u8,0,0]}
            ],
            data: vec![i as u8 + 1; 35 + i],
        }).collect();
        Bank {header,mode:15,names:b"sample name table".to_vec(),samples}.encode().unwrap()
    }
    #[test]
    fn replaces_one_sample_without_changing_others_or_names() {
        let original = fixture(3);
        let donor = fixture(1);
        let rebuilt = replace_vorbis_sample(&original,2,&donor).unwrap();
        let a = Bank::parse(&original).unwrap();
        let b = Bank::parse(&rebuilt).unwrap();
        assert_eq!(a.names,b.names);
        assert_eq!(a.samples[0].data,b.samples[0].data);
        assert_eq!(a.samples[2].data,b.samples[2].data);
        assert_eq!(a.samples[0].extras,b.samples[0].extras);
        assert_eq!(a.samples[1].extras[1],b.samples[1].extras[1]);
        assert_eq!(b.samples[1].count(), 160);
    }
    #[test]
    fn refuses_zero_index_and_bad_donor() {
        let bank = fixture(2);
        assert!(replace_vorbis_sample(&bank,0,&fixture(1)).is_err());
        assert!(replace_vorbis_sample(&bank,3,&fixture(1)).is_err());
        assert!(replace_vorbis_sample(&bank,1,&fixture(2)).is_err());
        let mut corrupt=fixture(1);
        corrupt[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(replace_vorbis_sample(&bank,1,&corrupt).is_err());
    }
}
