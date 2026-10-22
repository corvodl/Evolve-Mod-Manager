//! Source-verified dynamic CryEngine img:// menu pictures for the Ruffle preview.
//! The original main_menu.gfx requests these separately from its 1008/1009
//! Scaleform atlas tiles. Converted PNGs live only in the isolated scratch
//! directory. Neither the source movie nor any imported PAK is modified.
use evolve_core::LibraryReferenceStatus;
use evolve_core::dds::decode_dds_rgba;
use evolve_core::library_catalog::resolve_imported_reference_in_archive;
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
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
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
        if pixels == 0
            || pixels > MAX_PIXELS_PER_IMAGE
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
        if size == 0
            || size > MAX_PNG_BYTES
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
        assert_eq!(
            referenced_modes(data),
            vec!["goliathTutorialUnfocus", "hunt"]
        );
    }
    #[test]
    fn original_menu_modes_include_all_twenty_four_focus_variants() {
        // These are the exact filenames verified against the original
        // main_menu.gfx and the separately supplied MenuModeSelect archive.
        // The game assets themselves are intentionally not redistributed.
        let names = [
            "coop",
            "custom",
            "goliathTutorial",
            "hunt",
            "markovTutorial",
            "npe",
            "npeGriffin",
            "npeHank",
            "npeMarkov",
            "npeVal",
            "ranked",
            "training",
        ];
        let mut authored = Vec::new();
        for stem in names {
            for suffix in ["", "Unfocus"] {
                authored.extend_from_slice(VIRTUAL_PREFIX.as_bytes());
                authored.extend_from_slice(stem.as_bytes());
                authored.extend_from_slice(suffix.as_bytes());
                authored.extend_from_slice(b".dds\0");
            }
        }
        let all = referenced_modes(&authored);
        assert_eq!(all.len(), 24);
        assert_eq!(
            all.iter().filter(|name| name.ends_with("Unfocus")).count(),
            12
        );
        for stem in names {
            assert!(all.contains(&stem.to_owned()));
            assert!(all.contains(&format!("{stem}Unfocus")));
        }
    }
    #[test]
    fn dxt1_and_dxt5_mode_source_dimensions_match_original_assets() {
        // Actual source textures supplied in MenuModeSelect.zip were all
        // 340x600. Validate the Rust DDS->PNG path for both observed
        // compression families using generated surfaces, not game artwork.
        let temp = tempfile::tempdir().unwrap();
        for (fourcc, block_bytes) in [(b"DXT1", 8usize), (b"DXT5", 16usize)] {
            let mut dds = vec![0u8; 128];
            dds[..4].copy_from_slice(b"DDS ");
            dds[4..8].copy_from_slice(&124u32.to_le_bytes());
            // Matches supplied single-surface source headers (flags
            // 0x81007 with no mipmap-count flag and zero stored mip count).
            dds[8..12].copy_from_slice(&0x81007u32.to_le_bytes());
            dds[12..16].copy_from_slice(&600u32.to_le_bytes());
            dds[16..20].copy_from_slice(&340u32.to_le_bytes());
            dds[76..80].copy_from_slice(&32u32.to_le_bytes());
            dds[80..84].copy_from_slice(&4u32.to_le_bytes());
            dds[84..88].copy_from_slice(fourcc);
            // 85x150 four-by-four blocks (340x600).
            dds.resize(128 + 85 * 150 * block_bytes, 0);
            let file = temp.path().join(format!("mode-{block_bytes}.dds"));
            fs::write(&file, &dds).unwrap();
            let (_info, rgba) = decode_dds_rgba(&file).unwrap();
            assert_eq!(rgba.dimensions(), (340, 600));
            assert_eq!(dds.len(), if block_bytes == 8 { 102128 } else { 204128 });
            let output = temp.path().join(format!("mode-{block_bytes}.png"));
            image::DynamicImage::ImageRgba8(rgba)
                .save_with_format(&output, image::ImageFormat::Png)
                .unwrap();
            let png = fs::read(output).unwrap();
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
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
    fn imported_same_build_menu_dds_becomes_a_temporary_png() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let folder = root.join("UI_Assets.pak__11111111111111111111");
        let source = folder.join("files/images/MenuModeSelect/hunt.dds");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        // Minimal four-by-four DXT1 DDS, using the same header contract as
        // the existing core decoder fixtures. No proprietary source assets.
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend([0, 248, 224, 7, 0, 0, 0, 0]);
        fs::write(&source, &dds).unwrap();
        fs::write(
            folder.join("asset-library.json"),
            serde_json::to_vec(&json!({
                "version": 1, "name": PAK, "source": "/game/UI_Assets.pak",
                "entries": [{
                    "path": "images/MenuModeSelect/hunt.dds",
                    "bytes": dds.len()
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("build-collections.json"),
            serde_json::to_vec(&json!({
                "version": 1,
                "builds": {
                    "stage2": {
                        "label": "Modded Stage 2",
                        "members": {"11111111111111111111": {}}
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let movie = b"Libs/UI/FlashAssets/images/MenuModeSelect/hunt.dds\0";
        let result = stage(Some(root), Some("Modded Stage 2"), scratch.path(), movie);
        assert_eq!(result["requested"], 1, "{result:?}");
        assert_eq!(result["staged"], 1, "{result:?}");
        let png = fs::read(scratch.path().join("runtime-images/hunt.png")).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(fs::read(&source).unwrap(), dds);
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            stage(Some(root), Some("2012"), other.path(), movie)["staged"],
            0
        );
        assert!(!other.path().join("runtime-images/hunt.png").exists());
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
