//! Optional video layer for the original Stage 2 main menu.
//! Source is always the exact-build, manifest-verified CRI USM in videos.pak.
//! The browser never receives a .usm or an arbitrary local file URL.
use evolve_core::LibraryReferenceStatus;
use evolve_core::library_catalog::resolve_imported_reference_in_archive;
use serde_json::{Value, json};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const ORIGINAL_MEMBER: &str = "Libs/UI/FlashAssets/videos/main_menu.usm";
const ORIGINAL_PAK: &str = "videos.pak";
const MAX_USM: u64 = 256 * 1024 * 1024;
const MAX_BROWSER_VIDEO: u64 = 64 * 1024 * 1024;
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(45);

fn plain_file(path: &Path, max_bytes: u64) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() < 12 || meta.len() > max_bytes {
        return Err("Not a regular bounded video file".into());
    }
    Ok(())
}

fn header(path: &Path, length: usize) -> Result<Vec<u8>, String> {
    let mut data = vec![0u8; length];
    File::open(path)
        .map_err(|e| e.to_string())?
        .read_exact(&mut data)
        .map_err(|e| e.to_string())?;
    Ok(data)
}
fn is_usm(path: &Path) -> Result<bool, String> {
    Ok(header(path, 4)? == b"CRID")
}
fn is_browser_video(path: &Path, ext: &str) -> Result<bool, String> {
    let bytes = header(path, 12)?;
    Ok(match ext {
        "mp4" => bytes[4..8] == *b"ftyp",
        "webm" => bytes[..4] == [0x1a, 0x45, 0xdf, 0xa3],
        _ => false,
    })
}

/// Browser-compatible sidecars are explicitly user-created derivatives of
/// the original USM, not replacements for the source movie or proof of game
/// codec parity. Never follow a symlink out of the verified USM parent.
fn sidecar(original: &Path, scratch: &Path) -> Result<Option<&'static str>, String> {
    let parent = original
        .parent()
        .ok_or("Original video has no parent folder")?;
    let safe_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    for (source_name, target_name, extension) in [
        ("main_menu.preview.mp4", "menu-background.mp4", "mp4"),
        ("main_menu.preview.webm", "menu-background.webm", "webm"),
    ] {
        let candidate = parent.join(source_name);
        if !candidate.exists() {
            continue;
        }
        plain_file(&candidate, MAX_BROWSER_VIDEO)?;
        let real = fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
        if !real.starts_with(&safe_parent) || !is_browser_video(&real, extension)? {
            return Err(format!(
                "Invalid original-video preview derivative: {source_name}"
            ));
        }
        let target = scratch.join(target_name);
        fs::copy(real, &target).map_err(|e| e.to_string())?;
        return Ok(Some(target_name));
    }
    Ok(None)
}

/// ffmpeg is NEVER launched from PATH, nor downloaded. A user must explicitly
/// point EVOLVE_GFX_FFMPEG to an absolute, regular executable they trust.
/// It is invoked without a shell and only on an exact-build verified source.
/// This preview derivative is muted by default; it does not alter game PAKs.
fn configured_ffmpeg() -> Result<Option<PathBuf>, String> {
    let Some(override_path) = env::var_os("EVOLVE_GFX_FFMPEG") else {
        return Ok(None);
    };
    let path = PathBuf::from(override_path);
    if !path.is_absolute() {
        return Err("EVOLVE_GFX_FFMPEG must be an absolute path".into());
    }
    if cfg!(windows)
        && !path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("ffmpeg.exe"))
    {
        return Err("The configured decoder must be ffmpeg.exe".into());
    }
    plain_file(&path, 512 * 1024 * 1024)?;
    let real = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if !real.is_absolute() {
        return Err("FFmpeg binary path did not resolve to an absolute path".into());
    }
    Ok(Some(real))
}
fn transcode(original: &Path, scratch: &Path, ffmpeg: &Path) -> Result<&'static str, String> {
    let target = scratch.join("menu-background.mp4");
    let mut child = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-nostdin")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(original)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "27",
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=1280:-2",
            "-t",
            "90",
            "-fs",
            "62914560",
            "-movflags",
            "+faststart",
        ])
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot launch selected FFmpeg binary: {e}"))?;
    let started = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < CONVERSION_TIMEOUT => {
                thread::sleep(Duration::from_millis(120));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&target);
                return Err(
                    "Background video conversion exceeded the 45-second preview limit".into(),
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&target);
                return Err(format!("Unable to monitor FFmpeg: {error}"));
            }
        }
    };
    if !result {
        let _ = fs::remove_file(&target);
        return Err("FFmpeg could not demux/decode the source CRI USM; use a verified compatible decoder to make main_menu.preview.mp4".into());
    }
    if plain_file(&target, MAX_BROWSER_VIDEO).is_err() || !is_browser_video(&target, "mp4")? {
        let _ = fs::remove_file(&target);
        return Err("FFmpeg produced an invalid or oversized MP4 preview".into());
    }
    Ok("menu-background.mp4")
}

/// Report only relative, known preview filenames to the browser. Missing
/// video never blocks GFX editing or fabricates static monster artwork.
pub fn stage_main_menu(root: Option<&Path>, build: Option<&str>, scratch: &Path) -> Value {
    let base = || {
        json!({
            "source_archive": ORIGINAL_PAK,
            "source_member": ORIGINAL_MEMBER,
            "playable": false
        })
    };
    let (Some(root), Some(build)) = (root, build) else {
        return json!({"source": base(), "status":
            "Import same-build videos.pak to the Asset Library and select the source build before previewing the original main-menu background."});
    };
    if build.trim().is_empty() {
        return json!({"source": base(), "status": "Select a verified source build for the original background video."});
    }
    let found = match resolve_imported_reference_in_archive(
        root,
        ORIGINAL_MEMBER,
        build,
        ORIGINAL_PAK,
    ) {
        Ok(LibraryReferenceStatus::Found(entry)) => entry,
        Ok(LibraryReferenceStatus::Missing) => {
            return json!({"source": base(), "status":
            "Original Libs/UI/FlashAssets/videos/main_menu.usm is not imported from videos.pak for this build."});
        }
        Ok(LibraryReferenceStatus::Ambiguous(_)) => {
            return json!({"source": base(), "status":
            "Ambiguous same-build original main-menu videos; choose one source PAK before rendering."});
        }
        Err(error) => {
            return json!({"source": base(), "status": format!("Original video source resolution failed: {error}")});
        }
    };
    let original = Path::new(&found.absolute_path);
    if let Err(error) = plain_file(original, MAX_USM) {
        return json!({"source": base(), "status": format!("Original main-menu video rejected: {error}")});
    }
    match is_usm(original) {
        Ok(true) => {}
        _ => {
            return json!({"source": base(), "status":
            "Original main-menu video does not have a CRI USM CRID header; refusing fallback artwork."});
        }
    }
    let playback = match sidecar(original, scratch) {
        Ok(Some(file)) => Ok((file, "user-provided browser video derived from original USM")),
        Ok(None) => match configured_ffmpeg() {
            Ok(Some(binary)) => transcode(original, scratch, &binary)
                .map(|file| (file, "locally converted original USM")),
            Ok(None) => Err("Original CRI USM found. To show its moving background in Ruffle, put main_menu.preview.mp4 beside extracted main_menu.usm, or set EVOLVE_GFX_FFMPEG to the absolute path of a compatible FFmpeg executable.".to_owned()),
            Err(error) => Err(format!("Original USM found but FFmpeg selection is invalid: {error}")),
        },
        Err(error) => Err(format!("Original USM found but preview derivative was rejected: {error}")),
    };
    match playback {
        Ok((file, mode)) => json!({
            "source": base(),
            "playable": true,
            "file": file,
            "mode": mode,
            "status": "Original main-menu background video staged separately behind transparent Ruffle UI."
        }),
        Err(status) => json!({"source": base(), "playable": false, "status": status}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_source_signatures_and_user_derivatives() {
        let folder = tempfile::tempdir().unwrap();
        let original = folder.path().join("main_menu.usm");
        fs::write(&original, b"CRID\0\0\0\0\0\0\0\0").unwrap();
        assert!(is_usm(&original).unwrap());
        let target = tempfile::tempdir().unwrap();
        assert!(sidecar(&original, target.path()).unwrap().is_none());
        fs::write(
            folder.path().join("main_menu.preview.mp4"),
            b"\0\0\0\x18ftypisom",
        )
        .unwrap();
        assert_eq!(
            sidecar(&original, target.path()).unwrap(),
            Some("menu-background.mp4")
        );
        assert_eq!(
            fs::read(target.path().join("menu-background.mp4")).unwrap(),
            b"\0\0\0\x18ftypisom"
        );
        fs::write(
            folder.path().join("main_menu.preview.mp4"),
            b"not-a-mp4-file",
        )
        .unwrap();
        assert!(sidecar(&original, target.path()).is_err());
    }
    #[test]
    fn missing_build_never_fabricates_a_background() {
        let temp = tempfile::tempdir().unwrap();
        let status = stage_main_menu(None, None, temp.path());
        assert_eq!(status["source"]["source_archive"], "videos.pak");
        assert_eq!(status["playable"], serde_json::Value::Null);
        assert!(!temp.path().join("menu-background.mp4").exists());
    }
}
