//! Verified same-build Scaleform ImportAssets lookup for FFDec scratch copies.
//! Mirrors the private manager's no-cross-build/ambiguity policies. GFX/SWF
//! movie references are rendered by FFDec only when the selected Asset Library
//! has an exact, manifest-verified file in the requested build.
use evolve_core::gfx::MAX_SOURCE;
use evolve_core::{LibraryReferenceStatus, resolve_imported_reference};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Dependency {
    pub reference: String,
    pub outcome: String,
}
fn valid(relative: &str) -> bool {
    !relative.is_empty()
        && relative.len() < 1024
        && !relative.contains(['\\', ':'])
        && !relative.starts_with('/')
        && relative
            .split('/')
            .all(|piece| !piece.is_empty() && piece != "." && piece != "..")
}
fn verified(path: &Path, base: Option<&Path>) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > MAX_SOURCE as u64 {
        return Err("Referenced movie is linked, oversized or not a regular file".into());
    }
    let real = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if let Some(parent) = base
        && !real.starts_with(parent)
    {
        return Err("Referenced movie escaped its verified extraction root".into());
    }
    let blob = fs::read(real).map_err(|e| e.to_string())?;
    if !matches!(&blob[..3], b"GFX" | b"CFX" | b"FWS" | b"CWS") {
        return Err("Referenced movie has unsupported Scaleform signature".into());
    }
    Ok(blob)
}
fn variants(virtual_path: &str) -> Vec<String> {
    let mut options = vec![virtual_path.to_string()];
    let stem = virtual_path
        .strip_suffix(".swf")
        .or_else(|| virtual_path.strip_suffix(".SWF"));
    if let Some(stem) = stem {
        options.push(format!("{stem}.gfx"));
    }
    options
}
fn swf_import(blob: &[u8], import: &str) -> Vec<u8> {
    let mut payload = blob.to_vec();
    if import.to_ascii_lowercase().ends_with(".swf") {
        if &payload[..3] == b"GFX" {
            payload[..3].copy_from_slice(b"FWS");
        }
        if &payload[..3] == b"CFX" {
            payload[..3].copy_from_slice(b"CWS");
        }
    }
    payload
}
pub fn stage_imports(
    imports: &[String],
    virtual_movie: &str,
    source: &Path,
    destination: &Path,
    library_root: Option<&Path>,
    build: Option<&str>,
) -> Result<Vec<Dependency>, String> {
    let virtual_parent = Path::new(virtual_movie)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let parent = fs::canonicalize(source.parent().ok_or("Missing original GFX parent")?)
        .map_err(|e| e.to_string())?;
    let mut diagnostics = Vec::new();
    for import in imports.iter().take(128) {
        if !valid(import)
            || !["gfx", "swf"].contains(
                &Path::new(import)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
            )
        {
            return Err("GFX imported movie reference contains unsafe path or extension".into());
        }
        let movie_path = virtual_parent
            .join(import)
            .to_string_lossy()
            .replace('\\', "/");
        if !valid(&movie_path) {
            return Err("Unsafe compiled movie virtual path".into());
        }
        let mut payload = None;
        let mut origin = String::new();
        for variant in variants(import) {
            let candidate = parent.join(&variant);
            if let Ok(data) = verified(&candidate, Some(&parent)) {
                payload = Some(data);
                origin = "Verified same-folder import".into();
                break;
            }
        }
        if payload.is_none() {
            if let (Some(root), Some(label)) = (library_root, build) {
                if label.trim().is_empty() {
                    return Err("GFX imported library build cannot be empty".into());
                }
                for variant in variants(&movie_path) {
                    let lookup = resolve_imported_reference(root, &variant, Some(label))
                        .map_err(|e| e.to_string())?;
                    match lookup {
                        LibraryReferenceStatus::Found(found) => {
                            let path = PathBuf::from(found.absolute_path);
                            if let Ok(data) = verified(&path, None) {
                                origin = format!("{} · {}", label, found.archive_name);
                                payload = Some(data);
                                break;
                            }
                        }
                        LibraryReferenceStatus::Ambiguous(_) => {
                            origin = format!("Ambiguous import across same-build PAKs ({label})");
                            break;
                        }
                        LibraryReferenceStatus::Missing => (),
                    }
                }
            } else {
                origin = "Library lookup unavailable without a verified source build".into();
            }
        }
        let Some(data) = payload else {
            diagnostics.push(Dependency {
                reference: import.clone(),
                outcome: if origin.is_empty() {
                    "Missing verified source".into()
                } else {
                    origin
                },
            });
            continue;
        };
        let target = destination.join(import);
        if let Some(directory) = target.parent() {
            fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        }
        fs::write(&target, swf_import(&data, import)).map_err(|e| e.to_string())?;
        diagnostics.push(Dependency {
            reference: import.clone(),
            outcome: origin,
        });
    }
    Ok(diagnostics)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiled_gfx_import_is_reheadered_as_swf_only_for_preview() {
        let mut bytes = b"GFX".to_vec();
        bytes.extend([0u8; 10]);
        assert_eq!(&swf_import(&bytes, "effects.swf")[..3], b"FWS");
        assert_eq!(&bytes[..3], b"GFX");
        assert_eq!(&swf_import(&bytes, "effects.gfx")[..3], b"GFX");
    }
    #[test]
    fn refuses_unsafe_imports_and_fails_closed_without_build() {
        assert!(!valid("../other.gfx"));
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("main.gfx");
        fs::write(&original, b"GFX movie").unwrap();
        let scratch = dir.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        assert!(
            stage_imports(
                &["../other.gfx".into()],
                "Libs/UI/main.gfx",
                &original,
                &scratch,
                None,
                None
            )
            .is_err()
        );
        let status = stage_imports(
            &["gfxfontlib_locfont.gfx".into()],
            "Libs/UI/main.gfx",
            &original,
            &scratch,
            None,
            None,
        )
        .unwrap();
        assert_eq!(status.len(), 1);
        assert!(
            status[0]
                .outcome
                .contains("without a verified source build")
        );
    }
}
