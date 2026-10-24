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

/// The old exporter threw away both Blender streams and reduced every Python
/// traceback to "exit code 1". Keep output in a bounded, isolated file (not a
/// piped child process, which can deadlock on a full OS pipe). On failure,
/// extract the tail including the last Python traceback for user diagnostics.
pub(crate) fn blender_diagnostic_tail(path: &Path) -> String {
    const MAX_BYTES: u64 = 24 * 1024;
    const MAX_CHARS: usize = 3500;
    let Ok(mut file) = File::open(path) else {
        return "Blender produced no diagnostic log".into();
    };
    let Ok(size) = file.metadata().map(|m| m.len()) else {
        return "Unable to inspect Blender's diagnostic log".into();
    };
    if file
        .seek(SeekFrom::Start(size.saturating_sub(MAX_BYTES)))
        .is_err()
    {
        return "Unable to seek Blender's diagnostic log".into();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return "Unable to read Blender's diagnostic log".into();
    }
    let rendered = String::from_utf8_lossy(&bytes);
    let salient = rendered
        .rfind("Traceback (most recent call last):")
        .or_else(|| rendered.rfind("Error:"))
        .or_else(|| rendered.rfind("RuntimeError:"))
        .map(|pos| &rendered[pos..])
        .unwrap_or(&rendered);
    let diagnostic: String = salient
        .chars()
        .rev()
        .take(MAX_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if diagnostic.trim().is_empty() {
        "Blender exited without an explanatory Python error".into()
    } else {
        diagnostic.trim().to_string()
    }
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
    convert_glb_to_fbx_with_options(blender, source, destination, animation, true)
}

/// Keep geometry and UVs unchanged while controlling FBX-embedded textures.
pub fn convert_glb_to_fbx_with_options(
    blender: &Path,
    source: &Path,
    destination: &Path,
    animation: AnimationPolicy,
    embed_textures: bool,
) -> Result<FbxReport, FbxError> {
    convert_glb_to_fbx_with_layout(
        blender,
        source,
        destination,
        animation,
        embed_textures,
        false,
    )
}

/// Preserve original parts by default; optionally join meshes within their verified rig.
pub fn convert_glb_to_fbx_with_layout(
    blender: &Path,
    source: &Path,
    destination: &Path,
    animation: AnimationPolicy,
    embed_textures: bool,
    combine_meshes: bool,
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
    fs::write(
        stage.path().join("blender_uv0.py"),
        include_str!("blender_uv0.py"),
    )?;
    fs::write(
        stage.path().join("blender_mesh_shading.py"),
        include_str!("blender_mesh_shading.py"),
    )?;
    fs::write(
        stage.path().join("blender_fbx_safety.py"),
        include_str!("blender_fbx_safety.py"),
    )?;
    fs::write(
        stage.path().join("blender_material_ids.py"),
        include_str!("blender_material_ids.py"),
    )?;
    fs::write(
        stage.path().join("blender_static_fbx_basis.py"),
        include_str!("blender_static_fbx_basis.py"),
    )?;
    fs::write(
        stage.path().join("blender_export_hierarchy.py"),
        include_str!("blender_export_hierarchy.py"),
    )?;
    let staged_fbx = stage.path().join(
        destination
            .file_name()
            .ok_or_else(|| FbxError::Invalid("FBX destination is missing its filename".into()))?,
    );
    let log = stage.path().join("blender-export.log");
    let stdout = File::create(&log)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(blender);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
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
        .arg(if embed_textures { "1" } else { "0" })
        .arg(if combine_meshes { "1" } else { "0" })
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
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
            "Blender exit status: {status}. Blender diagnostic:\n{}",
            blender_diagnostic_tail(&log)
        )));
    }
    // Embedded normal-map Content is verified in the staged FBX by Blender's
    // bundled postprocessor. Never publish temporary normal PNG sidecars.
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
    fn error_tail_includes_blender_traceback_without_full_log_or_deadlock() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("blender-export.log");
        let huge = "noisy Blender startup output".repeat(5000);
        fs::write(
            &log,
            format!(
                "{huge}\nTraceback (most recent call last):\n  File \"export.py\", line 54\nValueError: FBX normal-map media reference missing\n"
            ),
        )
        .unwrap();
        let summary = blender_diagnostic_tail(&log);
        assert!(summary.contains("Traceback (most recent call last):"));
        assert!(summary.contains("ValueError: FBX normal-map media reference missing"));
        assert!(!summary.contains(&huge));
        assert!(summary.len() <= 4000);
    }

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
    fn embedded_normal_export_publishes_only_the_fbx_file() {
        let stage = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let source = stage.path().join("emet.fbx");
        let destination = output.path().join("emet.fbx");
        let mut body = FBX_HEADER.to_vec();
        body.extend(std::iter::repeat_n(0u8, 512));
        fs::write(&source, &body).unwrap();
        let textures = stage.path().join("textures");
        fs::create_dir(&textures).unwrap();
        fs::write(
            textures.join("emet_normal_123456789abc.png"),
            b"\x89PNG\r\n\x1a\nsource-normal-pixels",
        )
        .unwrap();
        assert_eq!(
            publish_verified_binary_fbx(&source, &destination).unwrap(),
            body.len() as u64
        );
        assert_eq!(fs::read(&destination).unwrap(), body);
        assert!(!output.path().join("textures").exists());
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
