//! Independent external Cryengine-Converter bridge for original CAF/DBA
//! databases. The external GPL executable is not included in this Rust crate.
//! Actual decoded glTF tracks are validated before being listed as playable.
//! No reconstructed/dummy keyframes or cross-character retargeting.
use crate::animation::{AnimatedGlb, MotionClip};
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_PREVIEW_FILES: usize = 4000;
const MAX_TRACKS: usize = 15000;
const MAX_EXTRACT_BYTES: u64 = 512 * 1024 * 1024;
const CONVERTER_TIMEOUT: Duration = Duration::from_secs(1800);

#[derive(Debug, thiserror::Error)]
pub enum ConverterError {
    #[error("Invalid original asset, converter or output folder: {0}")]
    Invalid(String),
    #[error("Converter returned an error: {0}")]
    Failed(String),
    #[error("Converter produced no verified GLB animation clips")]
    NoTracks,
    #[error("Converter time limit exceeded")]
    Timeout,
    #[error("Output already exists")]
    Exists,
    #[error("File access: {0}")]
    Io(#[from] std::io::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvertedMotion {
    pub virtual_source: String,
    pub glb_files: Vec<String>,
    pub clips: Vec<MotionClip>,
    pub file_count: usize,
}
fn validate_path(
    exe: &Path,
    source: &Path,
    root: &Path,
    target: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), ConverterError> {
    let executable = exe.canonicalize()?;
    let origin = source.canonicalize()?;
    let folder = root.canonicalize()?;
    let target = target.to_path_buf();
    for p in [exe, source, root] {
        let stat = fs::symlink_metadata(p)?;
        if stat.file_type().is_symlink() {
            return Err(ConverterError::Invalid(
                "symbolic links not permitted".into(),
            ));
        }
    }
    if !executable.is_file()
        || !exe
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("exe"))
    {
        return Err(ConverterError::Invalid(
            "select bundled or explicitly installed Cryengine-Converter.exe".into(),
        ));
    }
    if !origin.is_file()
        || !folder.is_dir()
        || !origin.starts_with(&folder)
        || origin == folder
        || !matches!(
            source
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "caf" | "dba" | "chr" | "skin"
        )
    {
        return Err(ConverterError::Invalid(
            "original CAF/DBA/CHR/SKIN must lie inside exact extracted asset root".into(),
        ));
    }
    if !target.is_absolute() || target.exists() || fs::symlink_metadata(&target).is_ok() {
        return Err(ConverterError::Exists);
    }
    let parent = target.parent().ok_or(ConverterError::Exists)?;
    if !parent.is_dir()
        || fs::symlink_metadata(parent)?.file_type().is_symlink()
        || parent.canonicalize()?.starts_with(&folder)
    {
        return Err(ConverterError::Invalid(
            "output must be separate from original extracted assets".into(),
        ));
    }
    if executable.starts_with(&folder) {
        return Err(ConverterError::Invalid(
            "independent executable cannot reside inside assets".into(),
        ));
    }
    Ok((executable, origin, folder, target))
}
fn inspect_outputs(
    folder: &Path,
    source: &Path,
    root: &Path,
) -> Result<ConvertedMotion, ConverterError> {
    let mut waiting = vec![folder.to_path_buf()];
    let mut files = 0usize;
    let mut clips = Vec::new();
    let mut outputs = Vec::new();
    while let Some(dir) = waiting.pop() {
        for node in fs::read_dir(dir)? {
            files += 1;
            if files > MAX_PREVIEW_FILES {
                return Err(ConverterError::Invalid("too many converter outputs".into()));
            }
            let entry = node?;
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                return Err(ConverterError::Invalid(
                    "converter generated symlink".into(),
                ));
            }
            if meta.is_dir() {
                waiting.push(entry.path());
            } else if meta.is_file() {
                if meta.len() > MAX_EXTRACT_BYTES {
                    return Err(ConverterError::Invalid(
                        "output animation exceeds limits".into(),
                    ));
                }
                if entry
                    .path()
                    .extension()
                    .is_some_and(|v| v.eq_ignore_ascii_case("glb"))
                {
                    if let Ok(motion) = AnimatedGlb::read(&entry.path()) {
                        let names = motion.list_clips();
                        if !names.is_empty() {
                            if clips.len() + names.len() > MAX_TRACKS {
                                return Err(ConverterError::Invalid(
                                    "too many motion tracks".into(),
                                ));
                            }
                            clips.extend(names);
                            outputs.push(
                                entry
                                    .path()
                                    .strip_prefix(folder)
                                    .map_err(|_| {
                                        ConverterError::Invalid("untrusted output path".into())
                                    })?
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            );
                        }
                    }
                }
            }
        }
    }
    if outputs.is_empty() {
        return Err(ConverterError::NoTracks);
    }
    outputs.sort();
    let relative = source
        .strip_prefix(root)
        .map_err(|_| ConverterError::Invalid("source not in asset root".into()))?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(ConvertedMotion {
        virtual_source: relative,
        glb_files: outputs,
        clips,
        file_count: files,
    })
}

/// Executes only an explicitly chosen local converter with exact user paths.
/// The stage directory is removed on failure; publish only after validating
/// real animation tracks inside generated GLBs. No shell/remote downloads.
pub fn convert_original_motion(
    executable: &Path,
    source: &Path,
    asset_root: &Path,
    destination: &Path,
) -> Result<ConvertedMotion, ConverterError> {
    let (exe, original, root, out) = validate_path(executable, source, asset_root, destination)?;
    let parent = out.parent().ok_or(ConverterError::Exists)?;
    let stage = tempfile::Builder::new()
        .prefix(".evolve-convert-")
        .tempdir_in(parent)?;
    let mut child = Command::new(&exe)
        .arg(&original)
        .arg("-anim")
        .arg("-glb")
        .arg("-objectdir")
        .arg(&root)
        .arg("-out")
        .arg(stage.path())
        .arg("-maxthreads")
        .arg("2")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ConverterError::Failed(e.to_string()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > CONVERTER_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ConverterError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    if !status.success() {
        return Err(ConverterError::Failed(format!(
            "external Cryengine-Converter returned {status}"
        )));
    }
    let report = inspect_outputs(stage.path(), &original, &root)?;
    if fs::symlink_metadata(&out).is_ok() {
        return Err(ConverterError::Exists);
    }
    fs::rename(stage.path(), &out)?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_arbitrary_executables_and_source_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let assets = tmp.path().join("assets");
        fs::create_dir(&assets).unwrap();
        let source = assets.join("goliath_3p.dba");
        fs::write(&source, b"data").unwrap();
        let not_exe = tmp.path().join("script.py");
        fs::write(&not_exe, b"print('x')").unwrap();
        let dest = tmp.path().join("converted");
        assert!(convert_original_motion(&not_exe, &source, &assets, &dest).is_err());
        assert!(!dest.exists());
    }
}
