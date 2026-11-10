//! Guarded native RGB24 terrain tile persistence for a managed Modding project.
//!
//! This is the file-output half of terrain rebaking, not a shader/paint solver.
//! A matching, authenticated full-resolution baked atlas must already exist.
//! The original game PAK and imported Asset Library remain strictly read-only.
use crate::modding_projects::{self, ProjectRow};
use evolve_core::terrain_rebake::{
    decode_original_rgb24_grid, encode_original_rgb24_rebake,
};
use image::RgbaImage;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_TILE_BYTES: u64 = 48 * 1024 * 1024;
const MAX_GRID_SIDE: usize = 8;

fn validate_member(member: &str, index: usize, side: usize, parent: &Path) -> Result<(), String> {
    if member.is_empty()
        || member.contains('\\')
        || member.contains(':')
        || member.starts_with('/')
        || member.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Unsafe source terrain tile member path".into());
    }
    let path = Path::new(member);
    if path.parent() != Some(parent)
        || !path.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case(&format!("tile{}_{}.raw", index % side, index / side))
        })
    {
        return Err("Source tile names must match an exact complete native RGB24 grid".into());
    }
    Ok(())
}

/// Persist a complete, already-shaded RGB24 atlas to the original tile-member
/// layout of one managed, extracted terraintexture.pak Modding project.
///
/// This API never guesses terrain paint. Call it only after a genuine shader
/// path has independently produced full-resolution pixels. The whole map is
/// preflighted against the managed manifest *before* the first write. Each
/// changed tile uses the existing immutable EditorBackups + SHA CAS importer.
/// On a later failure, earlier writes are reverted in reverse order through
/// that same guarded importer; crash-consistent multi-file atomicity is still
/// not established, so this must not be called as a game-parity-complete bake.
pub(super) fn save_rebaked_rgb24_grid_checked(
    project: &ProjectRow,
    projects_root: &Path,
    members: &[String],
    side: usize,
    baked: &RgbaImage,
) -> Result<usize, String> {
    if !project.target.to_ascii_lowercase().ends_with("terraintexture.pak") {
        return Err("Native RGB24 rebake must target the terraintexture.pak Modding project".into());
    }
    if side == 0 || side > MAX_GRID_SIDE || side.checked_mul(side) != Some(members.len()) {
        return Err("Native RGB24 tile grid is incomplete".into());
    }
    let root = fs::canonicalize(projects_root).map_err(|e| e.to_string())?;
    let workspace = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if workspace == root || !workspace.starts_with(&root) {
        return Err("Native terrain rebake may only edit a managed Modding project".into());
    }
    let files_root = fs::canonicalize(workspace.join("files")).map_err(|e| e.to_string())?;
    let manifest_path = workspace.join(".evolve-pak-workspace.json");
    let meta = fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 8 * 1024 * 1024 {
        return Err("Missing or unsafe native rebake project manifest".into());
    }
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid terrain project manifest: {e}"))?;
    if manifest["version"].as_u64() != Some(1)
        || manifest["reference_only"].as_bool() == Some(true)
    {
        return Err("Terrain project manifest is not editable".into());
    }
    let entries = manifest["entries"]
        .as_array()
        .ok_or("Terrain project manifest entries are missing")?;
    let parent = Path::new(&members[0]).parent().unwrap_or(Path::new(""));
    let mut originals = Vec::<Vec<u8>>::with_capacity(members.len());
    for (index, member) in members.iter().enumerate() {
        validate_member(member, index, side, parent)?;
        if entries
            .iter()
            .filter(|entry| entry["path"].as_str() == Some(member.as_str())
                && entry["format"].as_str() == Some("raw"))
            .count() != 1
        {
            return Err("Terrain tile is missing or duplicated in editable project manifest".into());
        }
        let target = workspace.join("files").join(member);
        let metadata = fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_TILE_BYTES {
            return Err("Original terrain tile is not a bounded regular project member".into());
        }
        let canonical = fs::canonicalize(&target).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&files_root) {
            return Err("Native rebake tile escaped the managed project root".into());
        }
        originals.push(fs::read(canonical).map_err(|e| e.to_string())?);
    }

    let refs: Vec<_> = originals.iter().map(Vec::as_slice).collect();
    let encoded = encode_original_rgb24_rebake(&refs, side, baked).map_err(|e| e.to_string())?;
    let patched = originals
        .iter()
        .zip(encoded.changed_tiles.iter())
        .map(|(before, after)| after.as_deref().unwrap_or(before))
        .collect::<Vec<_>>();
    if decode_original_rgb24_grid(&patched, side).map_err(|e| e.to_string())? != *baked {
        return Err("RGB24 tile rebake failed exact atlas decode verification".into());
    }
    let mut committed = Vec::<usize>::new();
    for (index, output) in encoded.changed_tiles.iter().enumerate() {
        let Some(output) = output else { continue };
        let result = modding_projects::save_level_member_checked(
            project, projects_root, &members[index], &originals[index], output,
        );
        match result {
            Ok(true) => committed.push(index),
            Ok(false) | Err(_) => {
                let cause = match result {
                    Ok(false) => "Native terrain tile unexpectedly matched original".to_owned(),
                    Err(error) => error,
                    Ok(true) => unreachable!(),
                };
                let mut rollback_errors = Vec::new();
                for &prior in committed.iter().rev() {
                    if let Some(previous_output) = &encoded.changed_tiles[prior] {
                        if let Err(error) = modding_projects::save_level_member_checked(
                            project, projects_root, &members[prior],
                            previous_output, &originals[prior],
                        ) {
                            rollback_errors.push(format!("{}: {error}", members[prior]));
                        }
                    }
                }
                return Err(if rollback_errors.is_empty() {
                    format!("Native RGB24 rebake failed; prior tiles restored: {cause}")
                } else {
                    format!(
                        "Native RGB24 rebake failed; project rollback incomplete: {cause}; {}. Recover from EditorBackups.",
                        rollback_errors.join("; ")
                    )
                });
            }
        }
    }
    Ok(committed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn native_tile(index: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[index, index + 1, index + 2].repeat(16));
        bytes
    }

    fn project_fixture() -> (tempfile::TempDir, ProjectRow, Vec<String>, Vec<Vec<u8>>) {
        let folder = tempfile::tempdir().unwrap();
        let workspace = folder.path().join("Projects").join("TutorialTerrain");
        let mut members = Vec::new();
        let mut originals = Vec::new();
        let mut entries = Vec::new();
        for i in 0..4 {
            let name = format!("tile{}_{}.raw", i % 2, i / 2);
            let bytes = native_tile(i as u8);
            let path = workspace.join("files").join(&name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &bytes).unwrap();
            entries.push(serde_json::json!({
                "path": name, "format": "raw",
                "sha256": format!("{:x}", Sha256::digest(&bytes))
            }));
            members.push(name);
            originals.push(bytes);
        }
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "entries": entries
            }))
            .unwrap(),
        )
        .unwrap();
        let project = ProjectRow {
            label: "TutorialTerrain".into(),
            target: "terraintexture.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace,
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        (folder, project, members, originals)
    }

    #[test]
    fn native_bake_changes_only_touched_project_tile_and_is_backup_recoverable() {
        let (folder, project, members, originals) = project_fixture();
        let refs: Vec<_> = originals.iter().map(Vec::as_slice).collect();
        let mut image = decode_original_rgb24_grid(&refs, 2).unwrap();
        image.get_pixel_mut(1, 6).0 = [23, 65, 220, 255];
        let count = save_rebaked_rgb24_grid_checked(
            &project, &folder.path().join("Projects"), &members, 2, &image,
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            fs::read(project.workspace.join("files").join(&members[0])).unwrap(),
            originals[0]
        );
        assert_eq!(
            fs::read(project.workspace.join("files").join(&members[2])).unwrap(),
            originals[2]
        );
        let saved = members
            .iter()
            .map(|path| fs::read(project.workspace.join("files").join(path)).unwrap())
            .collect::<Vec<_>>();
        let saved_refs: Vec<_> = saved.iter().map(Vec::as_slice).collect();
        assert_eq!(decode_original_rgb24_grid(&saved_refs, 2).unwrap(), image);
        assert_eq!(
            fs::read_dir(project.workspace.join("EditorBackups")).unwrap().count(),
            1
        );
        assert!(modding_projects::restore_original_raw_member(
            &project, &folder.path().join("Projects"), &members[1],
        )
        .unwrap());
        assert_eq!(
            fs::read(project.workspace.join("files").join(&members[1])).unwrap(),
            originals[1]
        );
    }

    #[test]
    fn missing_tile_or_wrong_manifest_fails_before_any_project_write() {
        let (folder, project, mut members, originals) = project_fixture();
        let refs: Vec<_> = originals.iter().map(Vec::as_slice).collect();
        let mut image = decode_original_rgb24_grid(&refs, 2).unwrap();
        image.get_pixel_mut(1, 6).0 = [23, 65, 220, 255];
        members[3] = "missing.raw".into();
        assert!(save_rebaked_rgb24_grid_checked(
            &project, &folder.path().join("Projects"), &members, 2, &image,
        ).is_err());
        for (index, member) in ["tile0_0.raw", "tile1_0.raw", "tile0_1.raw", "tile1_1.raw"].iter().enumerate() {
            assert_eq!(
                fs::read(project.workspace.join("files").join(member)).unwrap(), originals[index]
            );
        }
        assert!(!project.workspace.join("EditorBackups").exists());
    }
}
