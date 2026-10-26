//! Read-only metadata for legacy Evolve FMOD Designer projects and FSB5 banks.
//! No proprietary tools, game assets, decoders, or executables are embedded.
use quick_xml::Reader;
use quick_xml::events::Event;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use thiserror::Error;

const MAX_PROJECT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ELEMENTS: usize = 2_000_000;

#[derive(Debug, Error)]
pub enum FmodError {
    #[error("FMOD source must be an existing regular file, not a link")]
    InvalidSource,
    #[error("FMOD file is truncated or has an invalid format header")]
    InvalidHeader,
    #[error("FMOD bank size fields do not fit the actual file")]
    InvalidBankSize,
    #[error("FMOD Designer project is too large to inspect safely")]
    OversizedProject,
    #[error("Unsupported audio extension; expected .fsb, .fev, or .fdp")]
    Unsupported,
    #[error("FMOD project XML is malformed: {0}")]
    ProjectXml(String),
    #[error("Could not inspect FMOD file: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FmodInfo {
    Bank {
        samples: u32,
        codec: &'static str,
        version: u32,
        size: u64,
    },
    Events {
        size: u64,
    },
    Project {
        name: String,
        events: usize,
        waveforms: usize,
        sound_banks: usize,
    },
}

impl FmodInfo {
    pub fn summary(&self) -> String {
        match self {
            Self::Bank {
                samples,
                codec,
                version,
                size,
            } => format!(
                "FMOD FSB5 v{version}: {samples} sound samples, {codec} encoding, {size} bytes. Read-only metadata; use a compatible bank builder to replace sounds."
            ),
            Self::Events { size } => format!(
                "FMOD FEV event definitions ({size} bytes). Keep event IDs and bank names unchanged for replacements."
            ),
            Self::Project {
                name,
                events,
                waveforms,
                sound_banks,
            } => format!(
                "FMOD project {name}: {events} events, {waveforms} waveform references and {sound_banks} sound-bank definitions. Source WAV files may be missing."
            ),
        }
    }
}

fn le32(bytes: &[u8], offset: usize) -> Result<u32, FmodError> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or(FmodError::InvalidHeader)?;
    Ok(u32::from_le_bytes(
        raw.try_into().map_err(|_| FmodError::InvalidHeader)?,
    ))
}

fn inspect_bank(path: &Path, size: u64) -> Result<FmodInfo, FmodError> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 60];
    file.read_exact(&mut header)
        .map_err(|_| FmodError::InvalidHeader)?;
    if &header[..4] != b"FSB5" {
        return Err(FmodError::InvalidHeader);
    }
    let version = le32(&header, 4)?;
    if version != 1 {
        // Other header revisions must be implemented before being treated as editable.
        return Err(FmodError::InvalidHeader);
    }
    let samples = le32(&header, 8)?;
    let sample_headers = u64::from(le32(&header, 12)?);
    let names = u64::from(le32(&header, 16)?);
    let audio_data = u64::from(le32(&header, 20)?);
    let required = 60u64
        .checked_add(sample_headers)
        .and_then(|n| n.checked_add(names))
        .and_then(|n| n.checked_add(audio_data))
        .ok_or(FmodError::InvalidBankSize)?;
    if samples > 1_000_000 || required > size {
        return Err(FmodError::InvalidBankSize);
    }
    let codec = match le32(&header, 24)? {
        1 => "PCM8",
        2 => "PCM16",
        5 => "float PCM",
        7 => "IMA ADPCM",
        11 => "MPEG",
        12 => "CELT",
        15 => "FMOD Vorbis",
        16 => "FADPCM",
        _ => "unknown codec",
    };
    Ok(FmodInfo::Bank {
        samples,
        codec,
        version,
        size,
    })
}

fn inspect_events(path: &Path, size: u64) -> Result<FmodInfo, FmodError> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .map_err(|_| FmodError::InvalidHeader)?;
    let riff_size = u64::from(le32(&header, 4)?)
        .checked_add(8)
        .ok_or(FmodError::InvalidHeader)?;
    if &header[..4] != b"RIFF" || &header[8..12] != b"FEV " || riff_size > size {
        return Err(FmodError::InvalidHeader);
    }
    Ok(FmodInfo::Events { size })
}

fn inspect_project(path: &Path, size: u64) -> Result<FmodInfo, FmodError> {
    if size > MAX_PROJECT_BYTES {
        return Err(FmodError::OversizedProject);
    }
    let file = File::open(path)?;
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut elements = 0usize;
    let mut name = String::new();
    let (mut events, mut waveforms, mut sound_banks) = (0, 0, 0);
    let mut root = false;
    let mut in_project_name = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(e)) => {
                depth += 1;
                elements += 1;
                if elements > MAX_ELEMENTS {
                    return Err(FmodError::OversizedProject);
                }
                let tag = e.name();
                let tag = tag.as_ref();
                if depth == 1 {
                    root = tag == b"project";
                }
                if !root {
                    return Err(FmodError::InvalidHeader);
                }
                if depth == 2 && tag == b"name" && name.is_empty() {
                    in_project_name = true;
                }
                match tag {
                    b"event" => events += 1,
                    b"waveform" => waveforms += 1,
                    b"soundbank" => sound_banks += 1,
                    _ => {}
                }
            }
            Ok(Event::Text(e)) if in_project_name => {
                name = String::from_utf8_lossy(e.as_ref())
                    .chars()
                    .take(160)
                    .collect();
            }
            Ok(Event::End(e)) => {
                if depth == 2 && e.name().as_ref() == b"name" {
                    in_project_name = false;
                }
                depth = depth.checked_sub(1).ok_or(FmodError::InvalidHeader)?;
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(FmodError::ProjectXml(error.to_string())),
            _ => {}
        }
        buffer.clear();
    }
    if !root || depth != 0 {
        return Err(FmodError::InvalidHeader);
    }
    Ok(FmodInfo::Project {
        name,
        events,
        waveforms,
        sound_banks,
    })
}

/// Inspect an indexed local file. Never runs an FMOD executable or modifies the source.
pub fn inspect_fmod_file(path: &Path) -> Result<FmodInfo, FmodError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(FmodError::InvalidSource);
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("fsb") {
        inspect_bank(path, meta.len())
    } else if ext.eq_ignore_ascii_case("fev") {
        inspect_events(path, meta.len())
    } else if ext.eq_ignore_ascii_case("fdp") {
        inspect_project(path, meta.len())
    } else {
        Err(FmodError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_fsb5_headers_without_decoding_audio() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("Goliath.fsb");
        let mut bytes = vec![0u8; 60 + 10 + 8 + 40];
        bytes[..4].copy_from_slice(b"FSB5");
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&10u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&8u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&40u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&15u32.to_le_bytes());
        fs::write(&source, &bytes).unwrap();
        assert!(
            inspect_fmod_file(&source)
                .unwrap()
                .summary()
                .contains("3 sound samples")
        );
        bytes[20..24].copy_from_slice(&400u32.to_le_bytes());
        fs::write(&source, bytes).unwrap();
        assert!(matches!(
            inspect_fmod_file(&source),
            Err(FmodError::InvalidBankSize)
        ));
    }
    #[test]
    fn checks_fev_riff_and_counts_fdp_events() {
        let temp = tempfile::tempdir().unwrap();
        let fev = temp.path().join("Goliath.fev");
        fs::write(&fev, b"RIFF\x04\x00\x00\x00FEV ").unwrap();
        assert!(matches!(
            inspect_fmod_file(&fev),
            Ok(FmodInfo::Events { .. })
        ));
        let fdp = temp.path().join("Goliath.fdp");
        fs::write(&fdp, "<project><name>Goliath</name><soundbank/><eventgroup><event><name>Roar</name></event></eventgroup><waveform/></project>").unwrap();
        match inspect_fmod_file(&fdp).unwrap() {
            FmodInfo::Project {
                name,
                events,
                waveforms,
                ..
            } => {
                assert_eq!(name, "Goliath");
                assert_eq!((events, waveforms), (1, 0)); // empty waveform is not a populated reference
            }
            _ => panic!("wrong type"),
        }
    }
}
