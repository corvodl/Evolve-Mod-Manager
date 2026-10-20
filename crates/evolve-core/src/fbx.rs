//! Controlled Blender 4.2+ bridge for GLB -> binary FBX. It preserves
//! incoming skeletons, bone weights, materials and UV layers. Direct CryEngine
//! mesh/CAF conversion and original-UV reconstruction are separate parity gates.
use crate::policy::AnimationPolicy;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_GLB_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WAIT: Duration = Duration::from_secs(1800);
const FBX_HEADER: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";

#[derive(Debug, thiserror::Error)]
pub enum FbxError {
    #[error("Invalid or unsupported Blender / source / FBX destination: {0}")]
    Invalid(String),
    #[error("Blender process failed: {0}")]
    Converter(String),
    #[error("Blender timed out converting the model")]
    Timeout,
    #[error("FBX output already exists")]
    Exists,
    #[error("File I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FbxReport {
    pub output_size: u64,
    pub animations_included: bool,
    pub blender_glb_source: String,
    pub output_format: String,
}

pub fn validate_glb_source(source: &Path, animations: AnimationPolicy) -> Result<(), FbxError> {
    let meta = fs::symlink_metadata(source)?;
    if !meta.file_type().is_file()
        || meta.len() > MAX_GLB_BYTES
        || meta.len() < 36
        || !source
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("glb"))
    {
        return Err(FbxError::Invalid(
            "Select an existing regular GLB file under 1 GiB".into(),
        ));
    }
    let mut file = File::open(source)?;
    let mut header = [0u8; 20];
    file.read_exact(&mut header)?;
    if &header[..4] != b"glTF"
        || u32::from_le_bytes(header[4..8].try_into().unwrap_or([0; 4])) != 2
        || u64::from(u32::from_le_bytes(
            header[8..12].try_into().unwrap_or([0; 4]),
        )) != meta.len()
        || &header[16..20] != b"JSON"
    {
        return Err(FbxError::Invalid("Malformed glTF 2.0 GLB header".into()));
    }
    let json_size = u32::from_le_bytes(header[12..16].try_into().unwrap_or([0; 4])) as u64;
    if json_size == 0 || json_size > 24 * 1024 * 1024 || json_size + 20 > meta.len() {
        return Err(FbxError::Invalid("Oversized/missing GLB JSON".into()));
    }
    let mut json = vec![0u8; json_size as usize];
    file.read_exact(&mut json)?;
    let document: serde_json::Value = serde_json::from_slice(&json)
        .map_err(|e| FbxError::Invalid(format!("Invalid GLB JSON: {e}")))?;
    if document["asset"]["version"] != "2.0"
        || document["meshes"].as_array().is_none_or(|m| m.is_empty())
    {
        return Err(FbxError::Invalid("GLB lacks glTF 2.0 meshes".into()));
    }
    if animations.embed_verified_animations
        && (document["animations"]
            .as_array()
            .is_none_or(|v| v.is_empty())
            || document["skins"].as_array().is_none_or(|v| v.is_empty()))
    {
        return Err(FbxError::Invalid(
            "Animation-enabled export requires verified animated skin".into(),
        ));
    }
    Ok(())
}

/// Atomically publish a complete, verified binary FBX without overwriting an
/// existing target. Reading the 23-byte signature advances the file cursor:
/// rewind before copying or the published export is corrupt.
pub(crate) fn publish_verified_binary_fbx(
    staged: &Path,
    destination: &Path,
) -> Result<u64, FbxError> {
    if destination.exists() {
        return Err(FbxError::Exists);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| FbxError::Invalid("FBX output folder missing".into()))?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(FbxError::Invalid("Invalid FBX output directory".into()));
    }
    let meta = fs::symlink_metadata(staged)?;
    if !meta.file_type().is_file() || meta.len() < 256 {
        return Err(FbxError::Converter(
            "Blender produced no complete FBX".into(),
        ));
    }
    let mut source = File::open(staged)?;
    let mut signature = [0u8; 23];
    source.read_exact(&mut signature)?;
    if signature != FBX_HEADER {
        return Err(FbxError::Converter(
            "Blender output is not a binary FBX".into(),
        ));
    }
    source.seek(SeekFrom::Start(0))?;
    let mut published = tempfile::Builder::new()
        .prefix(".fbx-verified-")
        .tempfile_in(parent)?;
    let copied = std::io::copy(&mut source, &mut published)?;
    if copied != meta.len() {
        return Err(FbxError::Converter(
            "Blender FBX changed or was truncated while publishing".into(),
        ));
    }
    published.as_file_mut().sync_all()?;
    published.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            FbxError::Exists
        } else {
            FbxError::Io(e.error)
        }
    })?;
    Ok(copied)
}

/// Execute only an explicitly supplied installed Blender executable, with a
/// fixed bundled script. No shell, PATH lookup or downloads. Stage alongside
/// destination, validate actual FBX signature, then publish without overwrite.
pub fn convert_glb_to_fbx(
    blender: &Path,
    source: &Path,
    destination: &Path,
    animation: AnimationPolicy,
) -> Result<FbxReport, FbxError> {
    validate_glb_source(source, animation)?;
    let blender_meta = fs::symlink_metadata(blender)?;
    if !blender_meta.file_type().is_file()
        || !blender
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Err(FbxError::Invalid(
            "Select your explicitly installed Blender executable".into(),
        ));
    }
    if destination.exists()
        || !destination
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
    {
        return Err(FbxError::Exists);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| FbxError::Invalid("FBX output folder not specified".into()))?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(FbxError::Invalid(
            "Output parent must be existing non-linked directory".into(),
        ));
    }
    let stage = tempfile::Builder::new()
        .prefix(".evolve-fbx-")
        .tempdir_in(parent)?;
    let script = stage.path().join("convert_fbx.py");
    fs::write(&script, include_str!("blender_glb_fbx.py"))?;
    let staged_fbx = stage.path().join("converted.fbx");
    let mut child = Command::new(blender)
        .arg("--background")
        .arg("--factory-startup")
        .arg("--python-exit-code")
        .arg("1")
        .arg("--python")
        .arg(&script)
        .arg("--")
        .arg(source)
        .arg(&staged_fbx)
        .arg(if animation.embed_verified_animations {
            "1"
        } else {
            "0"
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| FbxError::Converter(e.to_string()))?;
    let begun = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if begun.elapsed() > MAX_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FbxError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    if !status.success() {
        return Err(FbxError::Converter(format!(
            "Blender exit status: {status}"
        )));
    }
    let size = publish_verified_binary_fbx(&staged_fbx, destination)?;
    Ok(FbxReport {
        output_size: size,
        animations_included: animation.embed_verified_animations,
        blender_glb_source: source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        output_format: "Binary FBX 7.x via Blender".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_fbx_publisher_preserves_binary_header_and_every_byte() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.fbx");
        let destination = directory.path().join("result.fbx");
        let mut original = FBX_HEADER.to_vec();
        original.extend((0..777).map(|i| (i % 251) as u8));
        fs::write(&source, &original).unwrap();
        assert_eq!(
            publish_verified_binary_fbx(&source, &destination).unwrap(),
            original.len() as u64
        );
        assert_eq!(fs::read(&destination).unwrap(), original);
        assert!(matches!(
            publish_verified_binary_fbx(&source, &destination),
            Err(FbxError::Exists)
        ));
    }

    #[test]
    fn verified_fbx_publisher_refuses_truncated_header() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("invalid.fbx");
        let destination = directory.path().join("not-created.fbx");
        fs::write(&source, vec![0u8; 512]).unwrap();
        assert!(matches!(
            publish_verified_binary_fbx(&source, &destination),
            Err(FbxError::Converter(_))
        ));
        assert!(!destination.exists());
    }
    #[test]
    fn animation_is_opt_in_and_validator_rejects_missing_skins() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("test.glb");
        let json = br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[]}]}"#;
        let padded = (json.len() + 3) & !3;
        let mut data = Vec::new();
        data.extend_from_slice(b"glTF");
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&((20 + padded) as u32).to_le_bytes());
        data.extend_from_slice(&(padded as u32).to_le_bytes());
        data.extend_from_slice(b"JSON");
        data.extend_from_slice(json);
        data.resize(20 + padded, b' ');
        fs::write(&source, data).unwrap();
        validate_glb_source(&source, AnimationPolicy::default()).unwrap();
        assert!(
            validate_glb_source(
                &source,
                AnimationPolicy {
                    embed_verified_animations: true
                }
            )
            .is_err()
        );
    }
}
