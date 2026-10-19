//! Safe historical (ZIP-based) PAK inspection and transactional extraction.
//! Stage 2 signed Twofish CryPAKs use methods 13/14 and are NOT ordinary ZIPs.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zip::{CompressionMethod, ZipArchive};

pub const MAX_ARCHIVE_ENTRIES: usize = 60_000;
pub const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PakMember {
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub method: u16,
    pub crc32: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PakInventory {
    pub schema: u32,
    pub source_file: String,
    pub kind: String,
    pub total_bytes: u64,
    pub entries: Vec<PakMember>,
}

#[derive(Debug, thiserror::Error)]
pub enum PakError {
    #[error("Select an existing regular .pak file (not a symbolic link)")]
    InvalidSource,
    #[error("Unsafe PAK member: {0}")]
    UnsafePath(String),
    #[error("Duplicate case-insensitive member path: {0}")]
    DuplicatePath(String),
    #[error("Unsupported PAK feature: {0}")]
    Unsupported(String),
    #[error("PAK exceeds bounded entry count or expanded size")]
    LimitsExceeded,
    #[error("Extraction destination already exists or is unsafe")]
    UnsafeDestination,
    #[error("Corrupt, truncated or unreadable ZIP/PAK: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

const WINDOWS_DEVICES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9",
];

/// Preserve original slash spelling but reject traversal, aliases, Windows device
/// names and invalid filename components before writing anything.
pub fn safe_member_name(raw: &str) -> Result<String, PakError> {
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') || raw.contains('\0') {
        return Err(PakError::UnsafePath(raw.into()));
    }
    let name = raw.replace('\\', "/");
    if name.len() > 2048 || name.ends_with('/') {
        return Err(PakError::UnsafePath(raw.into()));
    }
    for component in name.split('/') {
        let device = component.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if component.is_empty()
            || component == "." || component == ".."
            || component.ends_with(['.', ' '])
            || component.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || WINDOWS_DEVICES.contains(&device.as_str())
        {
            return Err(PakError::UnsafePath(raw.into()));
        }
    }
    Ok(name)
}

fn open_archive(source: &Path) -> Result<ZipArchive<File>, PakError> {
    let metadata = fs::symlink_metadata(source).map_err(|_| PakError::InvalidSource)?;
    if !metadata.file_type().is_file()
        || source.extension().and_then(|x| x.to_str())
            .is_none_or(|x| !x.eq_ignore_ascii_case("pak"))
    {
        return Err(PakError::InvalidSource);
    }
    Ok(ZipArchive::new(File::open(source)?)?)
}

fn inspect_open(archive: &mut ZipArchive<File>, source: &Path) -> Result<PakInventory, PakError> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(PakError::LimitsExceeded);
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        // Metadata-only access first; never decrypt Stage 2 entries.
        let member = archive.by_index_raw(i)?;
        let filename = member.name().to_owned();
        let is_directory = member.is_dir();
        if !is_directory && member.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            return Err(PakError::Unsupported("symlink members".into()));
        }
        if member.encrypted() {
            return Err(PakError::Unsupported("encrypted ZIP member".into()));
        }
        let method = match member.compression() {
            CompressionMethod::Stored => 0,
            CompressionMethod::Deflated => 8,
            _ => return Err(PakError::Unsupported(
                "only stored and DEFLATE methods are supported; Stage 2 method 13/14 needs signature and Twofish verification".into())),
        };
        if is_directory {
            let raw = filename.trim_end_matches('/');
            if !raw.is_empty() {
                safe_member_name(raw)?;
            }
            continue;
        }
        let name = safe_member_name(&filename)?;
        if !seen.insert(name.to_lowercase()) {
            return Err(PakError::DuplicatePath(name));
        }
        let size = member.size();
        let compressed_size = member.compressed_size();
        if size > MAX_ENTRY_BYTES
            || (size > 2 * 1024 * 1024 && size > compressed_size.saturating_mul(1000))
        {
            return Err(PakError::LimitsExceeded);
        }
        total = total.checked_add(size).ok_or(PakError::LimitsExceeded)?;
        if total > MAX_TOTAL_BYTES {
            return Err(PakError::LimitsExceeded);
        }
        entries.push(PakMember {
            path: name,
            size,
            compressed_size,
            method,
            crc32: member.crc32(),
        });
    }
    if entries.is_empty() {
        return Err(PakError::Unsupported("empty PAK archive".into()));
    }
    Ok(PakInventory {
        schema: 1,
        source_file: source.file_name().unwrap_or_default().to_string_lossy().to_string(),
        kind: "legacy-zip".into(),
        total_bytes: total,
        entries,
    })
}

/// Inspect metadata only, without decrypting or extracting entries.
pub fn inspect_legacy_pak(source: &Path) -> Result<PakInventory, PakError> {
    inspect_open(&mut open_archive(source)?, source)
}

/// Extract an entire verified ZIP-compatible PAK into a NEW directory.
/// A failed extraction removes its temporary directory; source PAKs are immutable.
/// No installed game folder is changed by the application.
pub fn extract_legacy_pak(source: &Path, destination: &Path) -> Result<PakInventory, PakError> {
    let mut archive = open_archive(source)?;
    let inventory = inspect_open(&mut archive, source)?;
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        return Err(PakError::UnsafeDestination);
    }
    let parent = destination.parent().ok_or(PakError::UnsafeDestination)?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(PakError::UnsafeDestination);
    }
    let staged = tempfile::Builder::new().prefix(".evolve-pak-").tempdir_in(parent)?;
    let mut extracted_index = 0usize;
    for i in 0..archive.len() {
        let mut member = archive.by_index(i)?;
        if member.is_dir() {
            continue;
        }
        let expected = &inventory.entries[extracted_index];
        extracted_index += 1;
        if safe_member_name(member.name())? != expected.path
            || member.size() != expected.size
            || member.crc32() != expected.crc32
        {
            return Err(PakError::Unsupported("member changed after validation".into()));
        }
        let target = staged.path().join(PathBuf::from(&expected.path));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new().create_new(true).write(true).open(&target)?;
        let mut limited = (&mut member).take(expected.size.saturating_add(1));
        let copied = io::copy(&mut limited, &mut output)?;
        if copied != expected.size {
            return Err(PakError::Unsupported(format!("length or CRC failure in {}", expected.path)));
        }
        // Force end-of-stream; ZIP's reader verifies CRC at EOF.
        let mut probe = [0u8; 1];
        if limited.read(&mut probe)? != 0 {
            return Err(PakError::Unsupported("extra uncompressed entry data".into()));
        }
        output.flush()?;
    }
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        return Err(PakError::UnsafeDestination);
    }
    fs::rename(staged.path(), destination)?;
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn sample(path: &Path) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("characters/tech_3p.cdf", opts).unwrap();
        zip.write_all(b"<Character/>").unwrap();
        zip.start_file("animations/idle.caf", opts).unwrap();
        zip.write_all(b"CAF!").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn inspects_and_extracts_legacy_archive() {
        let folder = tempfile::tempdir().unwrap();
        let pak = folder.path().join("Models.pak");
        sample(&pak);
        let original = fs::read(&pak).unwrap();
        let summary = inspect_legacy_pak(&pak).unwrap();
        assert_eq!(summary.entries.len(), 2);
        assert_eq!(summary.total_bytes, 16);
        let dest = folder.path().join("out");
        let extracted = extract_legacy_pak(&pak, &dest).unwrap();
        assert_eq!(extracted.total_bytes, summary.total_bytes);
        assert_eq!(fs::read(dest.join("characters/tech_3p.cdf")).unwrap(), b"<Character/>");
        assert_eq!(fs::read(dest.join("animations/idle.caf")).unwrap(), b"CAF!");
        assert_eq!(fs::read(&pak).unwrap(), original);
        assert!(extract_legacy_pak(&pak, &dest).is_err());
    }

    #[test]
    fn rejects_zip_slip_devices_duplicates_and_absolute_names() {
        for name in ["../bad", "C:/bad", "/bad", "a/../bad", "NUL.txt", "data/CON.xml",
                     "a//b", "trailing. ", "a:stream"] {
            assert!(safe_member_name(name).is_err(), "{name}");
        }
        assert_eq!(safe_member_name("models\\goliath.cdf").unwrap(), "models/goliath.cdf");
        let folder = tempfile::tempdir().unwrap();
        let pak = folder.path().join("dup.pak");
        let mut zip = zip::ZipWriter::new(File::create(&pak).unwrap());
        zip.start_file("Textures/Kala.dds", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hi").unwrap();
        zip.start_file("textures/kala.DDS", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"bye").unwrap();
        zip.finish().unwrap();
        assert!(matches!(inspect_legacy_pak(&pak), Err(PakError::DuplicatePath(_))));
    }

    #[test]
    fn rejects_non_archive_and_never_creates_destination() {
        let folder = tempfile::tempdir().unwrap();
        let pak = folder.path().join("bad.pak");
        fs::write(&pak, b"not an archive").unwrap();
        assert!(inspect_legacy_pak(&pak).is_err());
        let dest = folder.path().join("output");
        assert!(extract_legacy_pak(&pak, &dest).is_err());
        assert!(!dest.exists());
    }
}
