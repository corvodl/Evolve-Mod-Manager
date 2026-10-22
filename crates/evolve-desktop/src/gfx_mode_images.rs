//! Source-verified dynamic CryEngine img:// menu pictures for the Ruffle preview.
//! The original main_menu.gfx requests these separately from its 1008/1009
//! Scaleform atlas tiles. Converted PNGs live only in the isolated scratch
//! directory. Neither the source movie nor any imported PAK is modified.
use evolve_core::dds::decode_dds_rgba;
use evolve_core::library_catalog::resolve_imported_reference_in_archive;
use evolve_core::LibraryReferenceStatus;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const VIRTUAL_PREFIX: &str = "Libs/UI/FlashAssets/images/MenuModeSelect/";
const PAK: &str = "UI_Assets.pak";
const MAX_MODES: usize = 64;
const MAX_PIXELS_PER_IMAGE: u64 = 6_000_000;
const MAX_TOTAL_PIXELS: u64 = 48_000_000;
const MAX_PNG_BYTES: u64 = 12 * 1024 * 1024;
const MAX_TOTAL_PNG_BYTES: u64 = 64 * 1024 * 1024;

fn safe_image_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Find full virtual paths authored in the original ActionScript constants.
/// This is intentionally not a directory-wide discovery or a basename search.
pub(crate) fn referenced_modes(movie: &[u8]) -> Vec<String> {
    let mut found = BTreeSet::new();
    let prefix = VIRTUAL_PREFIX.as_bytes();
    for start in 0..movie.len().saturating_sub(prefix.len()) {
        if &movie[start..start + prefix.len()] != prefix {
            continue;
        }
        let tail = &movie[start + prefix.len()..];
        let Some(end) = tail.iter().take(70).position(|&c| c == 0) else {
            continue;
        };
        let Ok(value) = std::str::from_utf8(&tail[..end]) else {
            continue;
        };
        let Some((name, ext)) = value.rsplit_once('.') else {
            continue;
        };
        if ext.eq_ignore_ascii_case("dds") && safe_image_name(name) {
            found.insert(name.to_owned());
        }
    }
    found.into_iter().take(MAX_MODES).collect()
}

fn candidates(name: &str) -> [String; 4] {
    let relative = format!("images/MenuModeSelect/{name}.dds");
    [
        relative.clone(),
        format!("files/{relative}"),
        format!("Libs/UI/FlashAssets/{relative}"),
        format!("files/Libs/UI/FlashAssets/{relative}"),
    ]
}

/// Stage only exact source-build-qualified UI_Assets entries; no synthesized
/// artwork and no loading assets from unrelated game builds or PAKs.
pub(crate) fn stage(
    root: Option<&Path>,
    build: Option<&str>,
    scratch: &Path,
    authored_movie: &[u8],
) -> Value {
    let names = referenced_modes(authored_movie);
    let requested = names.len();
    let mut staged = 0usize;
    let mut missing = Vec::new();
    let mut ambiguous = Vec::new();
    let mut failed = Vec::new();
    let mut total_pixels = 0u64;
    let mut total_bytes = 0u64;
    let (Some(root), Some(build)) = (root, build) else {
        return json!({"requested": requested, "staged": 0, "status":
            "A verified Asset Library source build is required for dynamic menu DDS images."});
    };
    if build.trim().is_empty() {
        return json!({"requested": requested, "staged": 0, "status": "No selected source build."});
    }
    let destination = scratch.join("runtime-images");
    // Do not create an empty directory when this movie does not use mode images.
    if names.is_empty() {
        return json!({"requested": 0, "staged": 0, "status": "No authored MenuModeSelect DDS references."});
    }
    if let Err(error) = fs::create_dir_all(&destination) {
        return json!({"requested": requested, "staged": 0, "status": format!("Cannot create isolated image preview: {error}")});
    }
    for name in names {
        let mut matched = None;
        let mut duplicate = false;
        for candidate in candidates(&name) {
            match resolve_imported_reference_in_archive(root, &candidate, build, PAK) {
                Ok(LibraryReferenceStatus::Found(hit)) => {
                    if matched.is_some() {
                        duplicate = true;
                        break;
                    }
                    matched = Some(hit);
                }
                Ok(LibraryReferenceStatus::Ambiguous(_)) => {
                    duplicate = true;
                    break;
                }
                Ok(LibraryReferenceStatus::Missing) => {}
                Err(_) => {
                    duplicate = true; // Fail closed for bad/unsafe source manifests.
                    break;
                }
            }
        }
        if duplicate {
            ambiguous.push(name);
            continue;
        }
        let Some(found) = matched else {
            missing.push(name);
            continue;
        };
        let Ok((_info, rgba)) = decode_dds_rgba(Path::new(&found.absolute_path)) else {
            failed.push(name);
            continue;
        };
        let pixels = u64::from(rgba.width()) * u64::from(rgba.height());
        if pixels == 0 || pixels > MAX_PIXELS_PER_IMAGE
            || total_pixels.saturating_add(pixels) > MAX_TOTAL_PIXELS
        {
            failed.push(name);
            continue;
        }
        let target = destination.join(format!("{name}.png"));
        if image::DynamicImage::ImageRgba8(rgba)
            .save_with_format(&target, image::ImageFormat::Png)
            .is_err()
        {
            failed.push(name);
            continue;
        }
        let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(u64::MAX);
        if size == 0 || size > MAX_PNG_BYTES
            || total_bytes.saturating_add(size) > MAX_TOTAL_PNG_BYTES
        {
            let _ = fs::remove_file(&target);
            failed.push(name);
            continue;
        }
        total_pixels += pixels;
        total_bytes += size;
        staged += 1;
    }
    json!({
        "source_archive": PAK,
        "requested": requested,
        "staged": staged,
        "missing": missing,
        "ambiguous_or_invalid_manifest": ambiguous,
        "decode_or_budget_failures": failed,
        "status": "Only original same-build MenuModeSelect DDS converted to scratch PNG; source movies and PAKs unchanged.",
        "runtime_url_rewrite": "img://Libs/UI/FlashAssets/images/MenuModeSelect/<name>.dds -> runtime-images/<name>.png"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_only_original_virtual_mode_images() {
        let data = b"prefix\0Libs/UI/FlashAssets/images/MenuModeSelect/hunt.dds\0junk\0Libs/UI/FlashAssets/images/MenuModeSelect/goliathTutorialUnfocus.DDS\0Libs/UI/FlashAssets/images/MenuModeSelect/../secret.dds\0Libs/UI/FlashAssets/images/Other/hunt.dds\0";
        assert_eq!(referenced_modes(data), vec!["goliathTutorialUnfocus", "hunt"]);
    }
    #[test]
    fn rejects_unsafe_mode_names_and_empty_movielists() {
        for bad in ["", "..", "file.png", "a/b", "a\\b", "a:b"] {
            assert!(!safe_image_name(bad));
        }
        let temp = tempfile::tempdir().unwrap();
        let result = stage(None, None, temp.path(), b"unrelated gfx");
        assert_eq!(result["requested"], 0);
        assert_eq!(result["staged"], 0);
        assert!(!temp.path().join("runtime-images").exists());
    }
    #[test]
    fn no_unverified_source_is_ever_staged() {
        let temp = tempfile::tempdir().unwrap();
        let movie = b"Libs/UI/FlashAssets/images/MenuModeSelect/hunt.dds\0";
        let result = stage(None, None, temp.path(), movie);
        assert_eq!(result["requested"], 1);
        assert_eq!(result["staged"], 0);
        assert!(!temp.path().join("runtime-images").exists());
    }
}
