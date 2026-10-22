//! Optional video layer for the original Stage 2 main menu.
//! Source is always the exact-build, manifest-verified CRI USM in videos.pak.
//! The browser never receives a .usm or an arbitrary local file URL.
use evolve_core::library_catalog::resolve_imported_reference_in_archive;
use evolve_core::{
    LibraryReference, LibraryReferenceStatus, imported_archive_files, imported_library,
};
use serde_json::{Value, json};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const ORIGINAL_MEMBER: &str = "front_end/mainmenu_goliath.usm";
const ORIGINAL_PAK: &str = "videos.pak";
// Stage 2 videos.pak contains individual front_end/mainmenu_*.usm backgrounds.
// The Goliath movie is the default; the other character videos are not fallback
// candidates, since selecting any of them would change the intended background.
// Resolve only exact, source-build-qualified manifest entries (some imports
// retain an additional "files/" prefix in their manifest paths).
const VIDEO_MEMBERS: &[&str] = &[ORIGINAL_MEMBER, "files/front_end/mainmenu_goliath.usm"];
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
        ("mainmenu_goliath.preview.mp4", "menu-background.mp4", "mp4"),
        (
            "mainmenu_goliath.preview.webm",
            "menu-background.webm",
            "webm",
        ),
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
fn configured_ffmpeg(configured: &str) -> Result<Option<PathBuf>, String> {
    let path = if !configured.trim().is_empty() {
        PathBuf::from(configured.trim())
    } else if let Some(override_path) = env::var_os("EVOLVE_GFX_FFMPEG") {
        PathBuf::from(override_path)
    } else {
        return Ok(None);
    };
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
    let mut command = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
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
        return Err("FFmpeg could not demux/decode the source CRI USM; use a verified compatible decoder to make mainmenu_goliath.preview.mp4".into());
    }
    if plain_file(&target, MAX_BROWSER_VIDEO).is_err() || !is_browser_video(&target, "mp4")? {
        let _ = fs::remove_file(&target);
        return Err("FFmpeg produced an invalid or oversized MP4 preview".into());
    }
    Ok("menu-background.mp4")
}

/// Enumerate only verified video members from the selected source build's
/// imported videos.pak. No untrusted filesystem scan or cross-build fallback.
fn imported_video_members(root: &Path, build: &str) -> Result<Option<Vec<String>>, String> {
    let library = imported_library(root).map_err(|e| e.to_string())?;
    let mut archives = library.archives.iter().filter(|archive| {
        archive.name.eq_ignore_ascii_case(ORIGINAL_PAK)
            && archive.builds.iter().any(|label| label == build)
    });
    let Some(archive) = archives.next() else {
        return Ok(None);
    };
    if archives.next().is_some() {
        return Err("Multiple same-build videos.pak archives: choose one verified source".into());
    }
    let inventory = imported_archive_files(archive)
        .map_err(|e| format!("Cannot validate the imported videos.pak manifest: {e}"))?;
    Ok(Some(
        inventory
            .entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .filter(|path| {
                let lower = path.to_ascii_lowercase();
                [".usm", ".bik", ".mp4", ".webm"]
                    .iter()
                    .any(|suffix| lower.ends_with(*suffix))
            })
            .collect(),
    ))
}

fn is_goliath_video(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("mainmenu_goliath.usm"))
}

/// Accept only unambiguous source-verified representations inside videos.pak.
/// No global basename searching, no game directory probing, no guessed assets.
fn original_video(root: &Path, build: &str) -> Result<LibraryReferenceStatus, String> {
    let mut matches = Vec::<LibraryReference>::new();
    for &member in VIDEO_MEMBERS {
        match resolve_imported_reference_in_archive(root, member, build, ORIGINAL_PAK)
            .map_err(|e| format!("Cannot inspect source videos.pak: {e}"))?
        {
            LibraryReferenceStatus::Missing => {}
            LibraryReferenceStatus::Found(found) => {
                if !matches
                    .iter()
                    .any(|previous| previous.absolute_path == found.absolute_path)
                {
                    matches.push(found);
                }
            }
            LibraryReferenceStatus::Ambiguous(_) => {
                return Err(
                    "Multiple same-build copies of the Goliath main-menu video are ambiguous"
                        .into(),
                );
            }
        }
    }
    // Other videos.pak versions can place the Goliath clip under a deeper
    // mount root. Resolve an exact, unique Goliath filename from the original
    // verified archive, never a similar-looking monster background.
    if matches.is_empty()
        && let Some(paths) = imported_video_members(root, build)?
    {
        for path in paths.into_iter().filter(|path| is_goliath_video(path)) {
            match resolve_imported_reference_in_archive(root, &path, build, ORIGINAL_PAK)
                .map_err(|e| format!("Cannot verify original Goliath video member {path}: {e}"))?
            {
                LibraryReferenceStatus::Found(found) => {
                    if !matches
                        .iter()
                        .any(|entry| entry.absolute_path == found.absolute_path)
                    {
                        matches.push(found);
                    }
                }
                LibraryReferenceStatus::Ambiguous(_) => {
                    return Err("Ambiguous source-qualified Goliath video copies".into());
                }
                LibraryReferenceStatus::Missing => {
                    return Err(format!(
                        "Original Goliath video member {path} is missing or modified"
                    ));
                }
            }
        }
    }
    Ok(match matches.len() {
        0 => LibraryReferenceStatus::Missing,
        1 => LibraryReferenceStatus::Found(matches.remove(0)),
        _ => LibraryReferenceStatus::Ambiguous(matches),
    })
}

fn missing_video_status(root: &Path, build: &str) -> String {
    match imported_video_members(root, build) {
        Ok(None) => "Import the original videos.pak into the selected build in Asset Library; the Goliath background movie is not available.".into(),
        Ok(Some(paths)) if paths.is_empty() => {
            "The selected build's videos.pak has no verified video files. Inspect its extracted archive manifest.".into()
        }
        Ok(Some(paths)) => {
            let candidates = paths
                .iter()
                .take(8)
                .map(|path| path.chars().take(160).collect::<String>())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "The selected build's videos.pak contains {} verified video members but none resolved as mainmenu_goliath.usm. Original filenames (first 8): {candidates}. Check the source archive rather than selecting a different monster video.",
                paths.len()
            )
        }
        Err(error) => format!("Imported videos.pak could not be safely indexed: {error}"),
    }
}

/// Report only relative, known preview filenames to the browser. Missing
/// video never blocks GFX editing or fabricates static monster artwork.
pub fn stage_main_menu(
    root: Option<&Path>,
    build: Option<&str>,
    scratch: &Path,
    configured_decoder: &str,
) -> Value {
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
    let found = match original_video(root, build) {
        Ok(LibraryReferenceStatus::Found(entry)) => entry,
        Ok(LibraryReferenceStatus::Missing) => {
            return json!({"source": base(), "playable": false, "status": missing_video_status(root, build)});
        }
        Ok(LibraryReferenceStatus::Ambiguous(_)) => {
            return json!({"source": base(), "status":
                "More than one source-qualified Goliath menu video found in same-build videos.pak; resolve the duplicate instead of guessing."});
        }
        Err(error) => {
            return json!({"source": base(), "status": format!("Original video source resolution failed: {error}")});
        }
    };
    // Report the actual manifest entry; a verified import may retain "files/".
    let verified_source = json!({
        "source_archive": ORIGINAL_PAK,
        "source_member": &found.relative_path,
        "playable": false
    });
    let original = Path::new(&found.absolute_path);
    if let Err(error) = plain_file(original, MAX_USM) {
        return json!({"source": verified_source.clone(), "status": format!("Original main-menu video rejected: {error}")});
    }
    match is_usm(original) {
        Ok(true) => {}
        _ => {
            return json!({"source": verified_source.clone(), "status":
            "Original main-menu video does not have a CRI USM CRID header; refusing fallback artwork."});
        }
    }
    let playback = match sidecar(original, scratch) {
        Ok(Some(file)) => Ok((file, "user-provided browser video derived from original USM")),
        Ok(None) => match configured_ffmpeg(configured_decoder) {
            Ok(Some(binary)) => transcode(original, scratch, &binary)
                .map(|file| (file, "locally converted original USM")),
            Ok(None) => Err("Original CRI USM found. To show its moving background in Ruffle, put mainmenu_goliath.preview.mp4 beside extracted mainmenu_goliath.usm, or select a compatible FFmpeg executable in Settings > Integrations.".to_owned()),
            Err(error) => Err(format!("Original USM found but FFmpeg selection is invalid: {error}")),
        },
        Err(error) => Err(format!("Original USM found but preview derivative was rejected: {error}")),
    };
    match playback {
        Ok((file, mode)) => json!({
            "source": verified_source.clone(),
            "playable": true,
            "file": file,
            "mode": mode,
            "status": "Original main-menu background video staged separately behind transparent Ruffle UI."
        }),
        Err(status) => {
            json!({"source": verified_source.clone(), "playable": false, "status": status})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_decoder_requires_explicit_absolute_bounded_executable() {
        let temp = tempfile::tempdir().unwrap();
        let decoder = temp.path().join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        fs::write(&decoder, b"synthetic-executable").unwrap();
        assert_eq!(
            configured_ffmpeg(decoder.to_str().unwrap()).unwrap(),
            Some(fs::canonicalize(&decoder).unwrap())
        );
        assert!(configured_ffmpeg("relative/ffmpeg.exe").is_err());
        assert!(configured_ffmpeg(temp.path().to_str().unwrap()).is_err());
        assert!(
            configured_ffmpeg(temp.path().join("absent-ffmpeg.exe").to_str().unwrap()).is_err()
        );
    }
    #[test]
    fn validates_source_signatures_and_user_derivatives() {
        let folder = tempfile::tempdir().unwrap();
        let original = folder.path().join("mainmenu_goliath.usm");
        fs::write(&original, b"CRID\0\0\0\0\0\0\0\0").unwrap();
        assert!(is_usm(&original).unwrap());
        let target = tempfile::tempdir().unwrap();
        assert!(sidecar(&original, target.path()).unwrap().is_none());
        fs::write(
            folder.path().join("mainmenu_goliath.preview.mp4"),
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
            folder.path().join("mainmenu_goliath.preview.mp4"),
            b"not-a-mp4-file",
        )
        .unwrap();
        assert!(sidecar(&original, target.path()).is_err());
    }
    #[test]
    fn video_source_can_be_verified_from_relative_video_pak_member() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let id = "11111111111111111111";
        let folder = root.join(format!("videos.pak__{id}"));
        let virtual_path = "front_end/mainmenu_goliath.usm";
        let source = folder.join("files").join(virtual_path);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let data = b"CRID\0\0\0\0\0\0\0\0";
        fs::write(&source, data).unwrap();
        // Multiple character backgrounds are normal. The Goliath default
        // must not become ambiguous or silently fall back to another movie.
        let kraken_path = "front_end/mainmenu_kraken.usm";
        let kraken = folder.join("files").join(kraken_path);
        fs::write(&kraken, data).unwrap();
        fs::write(
            folder.join("asset-library.json"),
            serde_json::to_vec(&json!({
                "version": 1,
                "name": "videos.pak",
                "source": "/game/videos.pak",
                "entries": [
                    {"path": virtual_path, "bytes": data.len()},
                    {"path": kraken_path, "bytes": data.len()}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("build-collections.json"),
            serde_json::to_vec(&json!({
                "version": 1,
                "builds": {
                    "stage2": {"label": "Modded Stage 2", "members": {"11111111111111111111": {}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            original_video(root, "Modded Stage 2").unwrap(),
            LibraryReferenceStatus::Found(found) if found.relative_path == virtual_path
        ));
        assert!(matches!(
            original_video(root, "2012").unwrap(),
            LibraryReferenceStatus::Missing
        ));
        let scratch = tempfile::tempdir().unwrap();
        let result = stage_main_menu(Some(root), Some("Modded Stage 2"), scratch.path(), "");
        assert!(
            result["status"].as_str().unwrap().contains("FFmpeg")
                || result["status"]
                    .as_str()
                    .unwrap()
                    .contains("preview derivative")
        );
        assert_eq!(result["source"]["source_archive"], ORIGINAL_PAK);
        assert_eq!(result["source"]["source_member"], virtual_path);
        assert_eq!(result["playable"], false);
    }
    #[test]
    fn discovers_deep_goliath_video_and_reports_source_movie_names() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let id = "11111111111111111111";
        let folder = root.join(format!("videos.pak__{id}"));
        let deep = "Movies/Stage2/front_end/mainmenu_goliath.usm";
        let intro = "Movies/Stage2/front_end/intro.usm";
        let source = folder.join("files").join(deep);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let video = b"CRID\0\0\0\0\0\0\0\0";
        fs::write(&source, video).unwrap();
        fs::write(
            folder.join("asset-library.json"),
            serde_json::to_vec(&json!({
                "version":1,
                "name":"videos.pak",
                "source":"/game/videos.pak",
                "entries":[{"path":deep,"bytes":video.len()}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("build-collections.json"),
            serde_json::to_vec(&json!({
                "version":1,
                "builds":{"stage2":{"label":"Modded Stage 2","members":{"11111111111111111111":{}}}}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            original_video(root, "Modded Stage 2").unwrap(),
            LibraryReferenceStatus::Found(found) if found.relative_path == deep
        ));
        let scratch = tempfile::tempdir().unwrap();
        let preview = stage_main_menu(Some(root), Some("Modded Stage 2"), scratch.path(), "");
        assert_eq!(preview["source"]["source_member"], deep);
        assert_eq!(preview["playable"], false);
        // A different movie must only appear in diagnostics; it cannot
        // silently become a replacement for the default Goliath background.
        fs::rename(&source, folder.join("files").join(intro)).unwrap();
        fs::write(
            folder.join("asset-library.json"),
            serde_json::to_vec(&json!({
                "version":1,
                "name":"videos.pak",
                "source":"/game/videos.pak",
                "entries":[{"path":intro,"bytes":video.len()}]
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            original_video(root, "Modded Stage 2").unwrap(),
            LibraryReferenceStatus::Missing
        ));
        let missing = stage_main_menu(Some(root), Some("Modded Stage 2"), scratch.path(), "");
        assert_eq!(missing["playable"], false);
        assert!(missing["status"].as_str().unwrap().contains(intro));
    }
    #[test]
    fn missing_build_never_fabricates_a_background() {
        let temp = tempfile::tempdir().unwrap();
        let status = stage_main_menu(None, None, temp.path(), "");
        assert_eq!(status["source"]["source_archive"], "videos.pak");
        assert_eq!(status["playable"], serde_json::Value::Null);
        assert!(!temp.path().join("menu-background.mp4").exists());
    }
}
