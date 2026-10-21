//! Experimental interactive GFX preview using an independently installed Ruffle
//! desktop runtime. The original ActionScript bytecode remains in the movie;
//! only supported Scaleform DDS image tags are reconstructed for rendering.
//! This is intentionally NOT a CryEngine host callback implementation.
use crate::{gfx_editor::Session, gfx_external_textures};
use evolve_core::gfx::{MAX_DECOMPRESSED, MAX_SOURCE, inspect_gfx};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

const MAX_PLAYER_BYTES: u64 = 350 * 1024 * 1024;
const MAX_IMPORTS: usize = 128;

#[derive(Debug)]
struct ProcessOwner {
    child: Mutex<Child>,
    _scratch: tempfile::TempDir,
}
impl Drop for ProcessOwner {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[derive(Debug, Clone)]
pub struct RunningPreview {
    process: Arc<ProcessOwner>,
    pub restored_images: usize,
    pub skipped_images: usize,
    pub missing_imports: usize,
}
impl RunningPreview {
    pub fn is_running(&self) -> bool {
        self.process
            .child
            .lock()
            .is_ok_and(|mut child| child.try_wait().ok().flatten().is_none())
    }
}
fn executable(path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path.trim());
    if !candidate.is_absolute()
        || !candidate.file_name().is_some_and(|n| {
            matches!(
                n.to_string_lossy().to_ascii_lowercase().as_str(),
                "ruffle.exe" | "ruffle" | "ruffle_desktop.exe" | "ruffle_desktop"
            )
        })
    {
        return Err(
            "Choose the absolute path to an installed Ruffle desktop executable (ruffle.exe)."
                .into(),
        );
    }
    let meta = fs::symlink_metadata(candidate).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_PLAYER_BYTES {
        return Err("Ruffle must be a regular executable file (not a link).".into());
    }
    fs::canonicalize(candidate).map_err(|e| e.to_string())
}
fn safe_import(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 1024
        && !name.contains(['\\', ':'])
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && (name.to_ascii_lowercase().ends_with(".swf")
            || name.to_ascii_lowercase().ends_with(".gfx"))
}
/// Reheader to the SWF variant of the same compiled movie, without modifying
/// bytecode, frame instructions or authored display-list transformations.
fn flash_movie(movie: &[u8]) -> Result<Vec<u8>, String> {
    if movie.len() < 16 || movie.len() > MAX_DECOMPRESSED {
        return Err("Imported Scaleform movie exceeds the native source size limit".into());
    }
    let mut inspection = movie.to_vec();
    inspection[0] = match &movie[..3] {
        b"GFX" | b"FWS" => b'G',
        b"CFX" | b"CWS" => b'C',
        _ => return Err("Unsupported imported movie signature".into()),
    };
    inspection[1..3].copy_from_slice(b"FX");
    if inspection.len() <= MAX_SOURCE {
        inspect_gfx(&inspection).map_err(|e| e.to_string())?;
    } else {
        // Reconstructed preview-only bitmap characters can expand a checked
        // original beyond the game's 12 MiB source bound (up to 32 MiB).
        // Reject non-raw or inconsistent synthetic headers.
        let size = u32::from_le_bytes(inspection[4..8].try_into().unwrap()) as usize;
        if inspection[0] != b'G' || size != inspection.len() {
            return Err("Invalid oversized render-only SWF movie".into());
        }
    }
    let mut swf = movie.to_vec();
    swf[0] = if inspection[0] == b'G' { b'F' } else { b'C' };
    swf[1..3].copy_from_slice(b"WS");
    Ok(swf)
}
fn renderable_movie(
    bytes: &[u8],
    virtual_movie: &str,
    source: &Path,
    session: &Session,
) -> Result<(Vec<u8>, usize, usize), String> {
    // Use the exact same verified atlas reconstruction as the FFDec preview.
    // Ruffle can then rasterize preserved bitmap-fill shapes and run supported
    // AVM1/AVM2 actions in the original script tags.
    let mut canonical = bytes.to_vec();
    if canonical.len() < 3 {
        return Err("Movie signature is truncated".into());
    }
    match &canonical[..3] {
        b"FWS" => canonical[..3].copy_from_slice(b"GFX"),
        b"CWS" => canonical[..3].copy_from_slice(b"CFX"),
        _ => (),
    }
    let result = gfx_external_textures::prepare_preview(
        &canonical,
        virtual_movie,
        source,
        session.library_root.as_deref(),
        session.source_build.as_deref(),
    )?;
    Ok((
        flash_movie(&result.movie)?,
        result.converted,
        result.skipped,
    ))
}
fn stage_import(
    session: &Session,
    virtual_movie: &str,
    reference: &str,
    output: &Path,
) -> Result<Option<(usize, usize)>, String> {
    if !safe_import(reference) || reference.eq_ignore_ascii_case("movie.swf") {
        return Err("The movie contains an unsafe or colliding GFX/SWF import".into());
    }
    let source_root = fs::canonicalize(session.folder.path()).map_err(|e| e.to_string())?;
    let candidate = source_root.join(reference);
    let Ok(real) = fs::canonicalize(candidate) else {
        return Ok(None);
    };
    if !real.starts_with(&source_root) {
        return Err("Imported movie escaped the GFX scratch directory".into());
    }
    let metadata = fs::symlink_metadata(&real).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SOURCE as u64 {
        return Err("Imported movie is not a regular bounded file".into());
    }
    let bytes = fs::read(&real).map_err(|e| e.to_string())?;
    let virtual_path = virtual_movie
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/{reference}"))
        .unwrap_or_else(|| reference.to_owned());
    let (swf, converted, skipped) =
        renderable_movie(&bytes, &virtual_path, &session.source, session)?;
    let target = output.join(reference);
    let parent = target
        .parent()
        .ok_or("Invalid imported movie destination")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::write(target, swf).map_err(|e| e.to_string())?;
    Ok(Some((converted, skipped)))
}
/// Start a distinct Ruffle window on a temporary, validated copy of the GFX.
///
/// Host FSCommands and Evolve native Scaleform ExternalInterface are NOT
/// emulated; Ruffle's empty interface is used rather than routing any
/// original game action to an installed client or unrestricted filesystem.
/// The live process and preview directory are owned by RunningPreview.
pub fn launch(session: &Session, ruffle_exe: &str) -> Result<RunningPreview, String> {
    let binary = executable(ruffle_exe)?;
    let root = tempfile::Builder::new()
        .prefix("evolve-gfx-interactive-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let input = fs::read(&session.editable).map_err(|e| e.to_string())?;
    let (swf, mut converted, mut skipped) =
        renderable_movie(&input, &session.relative, &session.source, session)?;
    let root_movie = root.path().join("movie.swf");
    fs::write(&root_movie, swf).map_err(|e| e.to_string())?;
    let mut missing = 0usize;
    for name in session.info.imports.iter().take(MAX_IMPORTS) {
        match stage_import(session, &session.relative, name, root.path()) {
            Ok(Some((images, skip))) => {
                converted += images;
                skipped += skip;
            }
            Ok(None) => missing += 1,
            Err(e) => return Err(format!("Imported Scaleform movie {name}: {e}")),
        }
    }
    // The movie executes *only* in this external test player. No game DLLs,
    // registered game paths, credentials, or network bridges are handed off.
    let child = Command::new(binary)
        .arg("--dummy-external-interface")
        .arg("--open-url-mode")
        .arg("deny")
        .arg("--filesystem-access-mode")
        .arg("ask")
        .arg(&root_movie)
        .current_dir(root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Unable to launch Ruffle desktop: {e}"))?;
    Ok(RunningPreview {
        process: Arc::new(ProcessOwner {
            child: Mutex::new(child),
            _scratch: root,
        }),
        restored_images: converted,
        skipped_images: skipped,
        missing_imports: missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_untrusted_runtimes_and_import_paths() {
        assert!(executable("ruffle.exe").is_err());
        assert!(!safe_import("../ui.swf"));
        assert!(!safe_import("C:/evil.gfx"));
        assert!(safe_import("effects/modes.swf"));
        assert!(!safe_import("image.dds"));
        assert!(safe_import("movie.swf")); // Reject at staging: destination collision.
        assert_eq!("movie.swf".to_ascii_lowercase(), "movie.swf");
    }
    #[test]
    fn valid_gfx_keeps_bytecode_and_tags_when_converted_for_runtime() {
        // Minimal SWF frame header and one DoAction tag, deliberately without
        // invoking a player or requiring copyrighted game files.
        let mut bits = [0u8; 5 + 4 * 17];
        let mut put = |offset: usize, width: usize, value: u32| {
            for n in 0..width {
                bits[offset + n] = ((value >> (width - 1 - n)) & 1) as u8;
            }
        };
        put(0, 5, 17);
        for (i, number) in [0i32, 38400, 0, 21600].iter().enumerate() {
            put(5 + i * 17, 17, *number as u32);
        }
        let mut body = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            body[i / 8] |= bit << (7 - i % 8);
        }
        body.extend(7680u16.to_le_bytes());
        body.extend(1u16.to_le_bytes());
        body.extend(((12u16 << 6) | 1).to_le_bytes());
        body.push(0); // ActionEnd.
        body.extend(0u16.to_le_bytes());
        let mut original = b"GFX".to_vec();
        original.push(15);
        original.extend(((body.len() + 8) as u32).to_le_bytes());
        original.extend(body);
        let player = flash_movie(&original).unwrap();
        assert_eq!(&player[..3], b"FWS");
        assert_eq!(&player[3..], &original[3..]); // Preserved ActionScript.
    }
    #[test]
    fn rejects_invalid_flash_movie_without_executing_code() {
        assert!(flash_movie(b"FWS\x08").is_err());
        assert!(flash_movie(&[0u8; 24]).is_err());
    }
}
