//! Read-only original-vs-modified asset comparison.
//! Exact hashes and verified DDS metadata rather than guessed visual parity.
use crate::{DdsInfo, inspect_dds};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum CompareError {
    #[error("Choose two existing regular files smaller than 16 GiB")]
    Unsafe,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetFingerprint {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub dds: Option<DdsInfo>,
    pub dds_diagnostic: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetComparison {
    pub schema: u32,
    pub original: AssetFingerprint,
    pub modified: AssetFingerprint,
    pub identical: bool,
    pub same_size: bool,
    pub same_dds_dimensions: Option<bool>,
}
fn fingerprint(path: &Path) -> Result<AssetFingerprint, CompareError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(CompareError::Unsafe);
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_FILE_BYTES {
            return Err(CompareError::Unsafe);
        }
        hash.update(&buffer[..count]);
    }
    if bytes != metadata.len() {
        return Err(CompareError::Unsafe);
    }
    let is_dds = path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy().to_lowercase();
        name.ends_with(".dds") || name.ends_with(".dds.0")
    });
    let (dds, dds_diagnostic) = if is_dds {
        match inspect_dds(path) {
            Ok(info) => (Some(info), None),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };
    Ok(AssetFingerprint {
        path: path.to_string_lossy().into_owned(),
        bytes,
        sha256: format!("{:x}", hash.finalize()),
        dds,
        dds_diagnostic,
    })
}
/// Neither argument is changed; hashes compare complete files, not thumbnails.
pub fn compare_assets(original: &Path, modified: &Path) -> Result<AssetComparison, CompareError> {
    let original = fingerprint(original)?;
    let modified = fingerprint(modified)?;
    let same_dds_dimensions = match (&original.dds, &modified.dds) {
        (Some(a), Some(b)) => Some(a.width == b.width && a.height == b.height),
        _ => None,
    };
    Ok(AssetComparison {
        schema: 1,
        identical: original.sha256 == modified.sha256 && original.bytes == modified.bytes,
        same_size: original.bytes == modified.bytes,
        original,
        modified,
        same_dds_dimensions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparisons_detect_changes_without_writing_source() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("before.pak");
        let modified = root.path().join("after.pak");
        fs::write(&original, b"example").unwrap();
        fs::write(&modified, b"example").unwrap();
        assert!(compare_assets(&original, &modified).unwrap().identical);
        fs::write(&modified, b"Example").unwrap();
        let report = compare_assets(&original, &modified).unwrap();
        assert!(!report.identical);
        assert!(report.same_size);
        assert_eq!(fs::read(&original).unwrap(), b"example");
    }
}
