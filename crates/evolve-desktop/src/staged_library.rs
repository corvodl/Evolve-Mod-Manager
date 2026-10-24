//! Promote an imported, authenticated PAK to an isolated editable baseline.
//! An imported archive is never an implicit launcher target. Only an exact
//! source-root + relative-path + SHA match to rekey_plan.json can reuse a
//! launchable staged archive.
use crate::{modding_keys, modding_projects};
use evolve_core::{
    ImportedPak, imported_archive_files, imported_library, inspect_signed_pak,
    library_import::import_signed_library_pak_with_keys,
    signed_pak::resign_signed_pak_with_original_name,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn sha(path: &Path) -> Result<String, String> {
    let mut digest = Sha256::new();
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut buf = [0; 128 * 1024];
    loop {
        let count = input.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buf[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 32 * 1024 * 1024 {
        return Err(format!("Unsafe or oversized metadata: {}", path.display()));
    }
    serde_json::from_reader(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid metadata at {}: {e}", path.display()))
}

fn signed_source(pak: &ImportedPak) -> Result<(PathBuf, String, bool), String> {
    if pak.id.len() != 20 || !pak.id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid imported archive identity".into());
    }
    let filename = modding_keys::signed_basename(Path::new(&pak.name))?;
    if filename != pak.name {
        return Err("Imported archive name disagrees with file name".into());
    }
    let folder = PathBuf::from(&pak.directory);
    let info = read_json(&folder.join("asset-library.json"))?;
    // Older private-manager imports may not record a source SHA. They still
    // identify the exact source path and have independently verified extracted
    // files. Hash the current regular PAK and cryptographically verify it at
    // staging time instead of forcing a costly reimport of every archive.
    // Never ignore or replace a checksum when a modern manifest supplies one.
    let expected = info["source_sha256"].as_str();
    if info.get("source_sha256").is_some() && expected.is_none() {
        return Err("Imported source checksum field is malformed".into());
    }
    if let Some(value) = expected
        && (value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("Imported source checksum is invalid".into());
    }
    if info["name"].as_str() != Some(pak.name.as_str())
        || info["source"].as_str() != Some(pak.source.as_str())
    {
        return Err("Imported manifest and selected archive disagree".into());
    }
    let source = PathBuf::from(&pak.source);
    let meta = fs::symlink_metadata(&source).map_err(|_| {
        "Original imported PAK was moved or removed; re-import the current file".to_owned()
    })?;
    if !meta.file_type().is_file() {
        return Err("Original imported PAK is not a regular file".into());
    }
    let source = fs::canonicalize(&source).map_err(|e| e.to_string())?;
    let current = sha(&source)?;
    Ok((
        source,
        current.clone(),
        expected.is_some_and(|recorded| !current.eq_ignore_ascii_case(recorded)),
    ))
}

/// Only read the signed name authenticated during the original import.
/// A trusted import may record ASCII-lowercase RSA signing even when the
/// physical filename has mixed case.
fn signed_name(pak: &ImportedPak) -> Result<String, String> {
    let doc = read_json(&Path::new(&pak.directory).join("asset-library.json"))?;
    let name = doc["signed_name"].as_str().unwrap_or(&pak.name);
    if name != pak.name && name != pak.name.to_ascii_lowercase() {
        return Err("Imported PAK manifest has an unsafe signed filename".into());
    }
    Ok(name.to_owned())
}

fn exact_planned_target(
    stage: &Path,
    source: &Path,
    checksum: &str,
) -> Result<Option<PathBuf>, String> {
    let plan = read_json(&stage.join("rekey_plan.json"))?;
    let root = plan["source_root"]
        .as_str()
        .ok_or("Signing stage is missing its original game root")?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let Ok(relative) = source.strip_prefix(root) else {
        return Ok(None);
    };
    let name = relative
        .to_str()
        .ok_or("Imported game-relative PAK path is not UTF-8")?
        .replace('\\', "/");
    if name
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("Unsafe game-relative PAK path".into());
    }
    let Some(entries) = plan["entries"].as_array() else {
        return Err("Signing stage lacks an archive inventory".into());
    };
    let record = entries
        .iter()
        .find(|e| e["path"].as_str() == Some(name.as_str()));
    let Some(record) = record else {
        return Ok(None);
    };
    if !record["sha256"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case(checksum))
    {
        // A newer authenticated import can still be edited in its own
        // isolated project. It cannot replace an out-of-date game baseline:
        // never map this PAK to the original stage by filename alone.
        return Ok(None);
    }
    if record["kind"].as_str() != Some("signed-encrypted") {
        return Ok(None);
    }
    let target = stage.join("paks").join(&name);
    let staged_name = modding_keys::staged_target(stage, &target)?;
    if staged_name != name {
        return Err("Staged archive path differs from original game inventory".into());
    }
    Ok(Some(target))
}

/// Returns a new editable project. On an exact installed-game match it reuses
/// the already-authenticated launcher stage. Otherwise it creates a distinct
/// non-launchable isolated baseline, never inventing a game destination.
pub fn create_project(
    pak: &ImportedPak,
    projects: &Path,
    stage: &Path,
) -> Result<(PathBuf, usize, Option<String>, bool), String> {
    imported_archive_files(pak).map_err(|e| e.to_string())?;
    // An imported Asset Library extraction is already a valid *editing*
    // baseline. Prefer it immediately unless a perfectly matching signed
    // stage can be reused. Old game builds, missing source PAKs and absent
    // signing stages are compatibility warnings, not editing blockers.
    let ready_stage = signed_source(pak)
        .ok()
        .is_some_and(|(source, digest, changed)| {
            !changed
                && modding_keys::signing_keys(stage).is_ok()
                && matches!(exact_planned_target(stage, &source, &digest), Ok(Some(_)))
        });
    if !ready_stage {
        let (workspace, count) = modding_projects::edit_imported_snapshot(pak, projects)?;
        return Ok((workspace, count, None, false));
    }
    let (source, source_sha, changed) = signed_source(pak)?;
    // Refresh ONLY the selected archive when the original changed. Import
    // verifies its RSA signature and every extracted file before publishing a
    // separate content-addressed library snapshot. Never overwrite previous
    // imports or silently assume that a game-update PAK is deployable.
    let refreshed = if changed {
        let folder = fs::canonicalize(&pak.directory).map_err(|e| e.to_string())?;
        let root = folder
            .parent()
            .ok_or("Imported archive has no library root")?;
        if !fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_dir()) {
            return Err("Saved Asset Library root is unsafe".into());
        }
        let keys = modding_keys::archive_verification_keys(stage)?;
        let imported = import_signed_library_pak_with_keys(root, &source, &keys.paths)
            .map_err(|e| format!("Selected PAK changed since import; automatic single-PAK verification failed: {e}"))?;
        let catalog = imported_library(root).map_err(|e| e.to_string())?;
        let latest = catalog
            .archives
            .into_iter()
            .find(|candidate| {
                candidate.id == imported.id
                    && candidate.source == pak.source
                    && candidate.name == pak.name
            })
            .ok_or("Verified updated PAK was not found in the saved Asset Library")?;
        let (_, checked, still_changed) = signed_source(&latest)?;
        if still_changed || checked != source_sha {
            return Err("Source PAK changed again while preparing the updated import; retry after the game is closed".into());
        }
        Some(latest)
    } else {
        None
    };
    let pak = refreshed.as_ref().unwrap_or(pak);
    let (source, original_sha, stale) = signed_source(pak)?;
    if stale {
        return Err("Selected source PAK changed again during preparation".into());
    }
    let basename = modding_keys::signed_basename(&source)?;
    let authenticated_name = signed_name(pak)?;
    if basename != pak.name {
        return Err("Source archive basename changed after import".into());
    }
    let keys = modding_keys::signing_keys(stage)?;
    let planned = exact_planned_target(stage, &source, &original_sha)?;
    let (staged, launchable) = if let Some(path) = planned {
        inspect_signed_pak(&path, &keys.public, &authenticated_name)
            .map_err(|e| format!("Existing signed stage is invalid: {e}"))?;
        (path, true)
    } else {
        let candidates = modding_keys::archive_verification_keys(stage)?;
        let original_key = candidates.paths.iter()
            .find(|key| inspect_signed_pak(&source, key, &authenticated_name).is_ok())
            .ok_or("Imported PAK does not authenticate as a signed CryPAK under configured keys; legacy/plain ZIP editing is not supported by the signed project builder")?;
        let root = stage.join("paks").join("library-imports");
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        if !fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_dir()) {
            return Err("Imported stage folder is not a real directory".into());
        }
        let dest_dir = root.join(&pak.id);
        let dest = dest_dir.join(&basename);
        if dest_dir.exists() {
            let record = read_json(&dest_dir.join("origin.json"))?;
            if record["original_sha256"].as_str() != Some(original_sha.as_str())
                || record["source"].as_str() != Some(pak.source.as_str())
                || record["staged_sha256"].as_str() != Some(sha(&dest)?.as_str())
            {
                return Err(
                    "Imported stage identity changed; refusing to overwrite an existing baseline"
                        .into(),
                );
            }
            inspect_signed_pak(&dest, &keys.public, &authenticated_name)
                .map_err(|e| format!("Staged imported PAK failed verification: {e}"))?;
        } else {
            let temp = tempfile::Builder::new()
                .prefix(".pak-import-")
                .tempdir_in(&root)
                .map_err(|e| e.to_string())?;
            let authored = temp.path().join(&basename);
            resign_signed_pak_with_original_name(
                &source,
                original_key,
                &keys.private,
                &keys.public,
                &authenticated_name,
                &basename,
                &authored,
            )
            .map_err(|e| format!("Could not sign isolated editing copy: {e}"))?;
            if sha(&source)? != original_sha {
                return Err(
                    "Source changed while preparing staged copy; nothing was published".into(),
                );
            }
            let manifest = json!({
                "version": 1,
                "archive_id": pak.id,
                "source": pak.source,
                "original_sha256": original_sha,
                "staged_sha256": sha(&authored)?,
                "deployment": "unmapped",
                "note": "Editing baseline only; no installed game target has been selected"
            });
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.path().join("origin.json"))
                .map_err(|e| e.to_string())?;
            file.write_all(
                serde_json::to_string_pretty(&manifest)
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            if dest_dir.exists() {
                return Err("Another import created this staged identity; retry".into());
            }
            fs::rename(temp.path(), &dest_dir).map_err(|e| e.to_string())?;
        }
        (dest, false)
    };
    let (workspace, count) = modding_projects::unpack_raw_project_with_signed_name(
        projects,
        stage,
        &staged,
        &authenticated_name,
    )?;
    // Preserve source provenance and explicitly distinguish non-deployable
    // imported archives from PAKs mapped by the exact game manifest.
    let manifest_path = workspace.join(".evolve-pak-workspace.json");
    let mut manifest = read_json(&manifest_path)?;
    manifest["imported_source"] = json!(pak.source);
    manifest["imported_source_sha256"] = json!(original_sha);
    manifest["imported_archive_id"] = json!(pak.id);
    manifest["deployment_mapped"] = json!(launchable);
    manifest["source_was_refreshed"] = json!(changed);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok((
        workspace,
        count,
        refreshed.map(|archive| archive.id),
        launchable,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edited_main_menu_snapshot_requires_no_game_source_or_stage() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("AssetLibrary");
        let imported = library.join("UI_Assets.pak__0123456789abcdefabcd");
        let original = imported.join("files/Libs/UI/main_menu.gfx");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, b"original menu").unwrap();
        let manifest = json!({
            "version": 1,
            "name": "UI_Assets.pak",
            "source": dir.path().join("missing-game/Game/UI_Assets.pak"),
            "source_sha256": "a".repeat(64),
            "entries": [{
                "path": "Libs/UI/main_menu.gfx", "bytes": 13,
                "sha256": sha(&original).unwrap()
            }]
        });
        fs::write(imported.join("asset-library.json"), manifest.to_string()).unwrap();
        let archive = ImportedPak {
            id: "0123456789abcdefabcd".into(),
            name: "UI_Assets.pak".into(),
            source: dir
                .path()
                .join("missing-game/Game/UI_Assets.pak")
                .to_string_lossy()
                .into_owned(),
            directory: imported.to_string_lossy().into_owned(),
            group: "Other".into(),
            description: String::new(),
            entries: 1,
            extracted_bytes: 13,
            warnings: 0,
            builds: vec![],
        };
        let projects = dir.path().join("Projects");
        let stage = dir.path().join("nonexistent-stage");
        let (workspace, count, refreshed, mapped) =
            create_project(&archive, &projects, &stage).unwrap();
        assert_eq!(count, 1);
        assert!(refreshed.is_none());
        assert!(!mapped);
        let member = workspace.join("files/Libs/UI/main_menu.gfx");
        assert_eq!(fs::read(&member).unwrap(), b"original menu");
        fs::write(&member, b"Test B new main menu").unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"original menu");
        let found = modding_projects::discover_projects(&projects, &stage).unwrap();
        assert!(found.iter().any(|p| {
            p.target == "UI_Assets.pak"
                && p.modified == ["Libs/UI/main_menu.gfx"]
                && p.status == "Modified"
        }));
    }

    #[test]
    fn legacy_import_without_source_sha_reuses_original_without_reimport() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("models.pak");
        fs::write(&source, b"original signed source placeholder").unwrap();
        let imported = dir.path().join("old-import");
        fs::create_dir_all(&imported).unwrap();
        let archive = ImportedPak {
            id: "c".repeat(20),
            name: "models.pak".into(),
            source: source.to_string_lossy().into_owned(),
            directory: imported.to_string_lossy().into_owned(),
            group: String::new(),
            description: String::new(),
            entries: 1,
            extracted_bytes: 10,
            warnings: 0,
            builds: vec![],
        };
        let manifest = json!({
            "version": 1, "name": archive.name, "source": archive.source,
            "entries": [{"path": "models/test.cdf", "bytes": 1}]
        });
        fs::write(imported.join("asset-library.json"), manifest.to_string()).unwrap();
        let (verified, checksum, changed) = signed_source(&archive).unwrap();
        assert_eq!(verified, fs::canonicalize(&source).unwrap());
        assert_eq!(checksum, sha(&source).unwrap());
        assert!(!changed);

        let with_sha = json!({
            "version": 1, "name": archive.name, "source": archive.source,
            "source_sha256": "0".repeat(64),
            "entries": [{"path": "models/test.cdf", "bytes": 1}]
        });
        fs::write(imported.join("asset-library.json"), with_sha.to_string()).unwrap();
        assert!(signed_source(&archive).unwrap().2);
    }

    #[test]
    fn changed_installed_pak_does_not_get_mapped_by_filename() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let stage = root.path().join("stage");
        let source = game.join("Game/UI_Assets.pak");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(&source, b"new original after game update").unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            json!({
                "source_root": game,
                "entries": [{ "path": "Game/UI_Assets.pak",
                    "kind": "signed-encrypted", "sha256": "0".repeat(64) }]
            })
            .to_string(),
        )
        .unwrap();
        assert!(
            exact_planned_target(&stage, &source, &sha(&source).unwrap())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn foreign_pak_cannot_infer_target_from_filename() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        fs::create_dir_all(&stage).unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            json!({
                "source_root": dir.path().join("game"),
                "entries": [{
                    "path": "Game/duplicate.pak", "kind": "signed-encrypted",
                    "sha256": "aa".repeat(32),
                }]
            })
            .to_string(),
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("game")).unwrap();
        let foreign = dir
            .path()
            .join("foreign")
            .join("Game")
            .join("duplicate.pak");
        assert!(
            exact_planned_target(&stage, &foreign, &"aa".repeat(32))
                .unwrap()
                .is_none()
        );
    }
}
