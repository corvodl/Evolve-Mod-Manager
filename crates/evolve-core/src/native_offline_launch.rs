//! Native offline game launch preflight — read-only. Validates that an
//! independently staged and signed mod profile corresponds to the EXACT game
//! PAKs it will replace, with no orphaned backup. This does not install,
//! patch DLLs, start game processes or claim recovery parity.
use crate::profiles::plan_mod_profile;
use crate::signed_pak::inspect_signed_pak;
use crate::staging::stage_status;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeOfflineReadiness {
    pub game_root: PathBuf,
    pub stage_root: PathBuf,
    pub profile_id: String,
    pub validated_targets: Vec<String>,
    pub total_mod_bytes: u64,
    /// Deliberately false until atomic installed-game mount, process and
    /// independently recoverable watchdog semantics have been implemented.
    pub native_launch_armed: bool,
}

fn regular(path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0 {
        return Err(format!("Unsafe or missing regular file {}", path.display()));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}
fn folder(path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() {
        return Err(format!("Unsafe or missing directory {}", path.display()));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}
fn unlinked_game_member(root: &Path, virtual_path: &str) -> Result<PathBuf, String> {
    let mut parent = root.to_path_buf();
    for (index, component) in virtual_path.split('/').enumerate() {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(':')
            || component.contains('\\')
        {
            return Err("Unsafe game PAK target".into());
        }
        parent.push(component);
        let meta = fs::symlink_metadata(&parent).map_err(|e| e.to_string())?;
        let final_component = index == virtual_path.split('/').count() - 1;
        if (final_component && !meta.file_type().is_file())
            || (!final_component && !meta.file_type().is_dir())
        {
            return Err("Game PAK target is not an ordinary, unlinked path".into());
        }
    }
    let canonical = fs::canonicalize(&parent).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("Game PAK target escaped game root".into());
    }
    let backup = PathBuf::from(format!("{}.customkey-original", canonical.display()));
    if fs::symlink_metadata(&backup).is_ok() {
        return Err("Unrestored installed-game PAK backup blocks native launch".into());
    }
    Ok(canonical)
}

/// Read-only preflight for future native offline mounting. Uses exact verified
/// source inventory; never guesses an installed PAK from a same-named file.
/// The returned plan is NOT permission to execute or change installed files.
pub fn inspect_native_offline_readiness(
    game_root: &Path,
    isolated_stage: &Path,
    profile_id: &str,
    original_public_key: &Path,
    mod_public_key: &Path,
) -> Result<NativeOfflineReadiness, String> {
    let game = folder(game_root)?;
    let stage = folder(isolated_stage)?;
    if game == stage || game.starts_with(&stage) || stage.starts_with(&game) {
        return Err("Game installation and offline stage overlap".into());
    }
    let executable = regular(&game.join("bin64_SteamRetail/Evolve.exe"))?;
    if !executable.starts_with(&game) {
        return Err("Installed executable escaped game root".into());
    }
    if stage_status(&stage).map_err(|e| e.to_string())?.is_some() {
        return Err("Restore staged changes before native offline launch".into());
    }
    let original_key = regular(original_public_key)?;
    let mod_key = regular(mod_public_key)?;
    if original_key == mod_key
        || fs::read(&original_key).map_err(|e| e.to_string())?
            == fs::read(&mod_key).map_err(|e| e.to_string())?
    {
        return Err("Original and offline signing public keys must differ".into());
    }
    let planned = plan_mod_profile(&stage, profile_id, &original_key, &mod_key)
        .map_err(|e| format!("Profile is not ready: {e}"))?;
    if planned.mods.is_empty() {
        return Err("No enabled signed mods in offline profile".into());
    }
    let mut seen = HashSet::new();
    for item in &planned.mods {
        if !seen.insert(item.target.to_ascii_lowercase()) {
            return Err("Case-colliding game PAK targets".into());
        }
        let installed = unlinked_game_member(&game, &item.target)?;
        let basename = item.target.rsplit('/').next().unwrap_or_default();
        // Independently verify the exact *installed* signed original. Matching
        // staged member names alone cannot establish this game's provenance.
        let verified = inspect_signed_pak(&installed, &original_key, basename).map_err(|e| {
            format!(
                "Installed PAK {} failed original signature: {e}",
                item.target
            )
        })?;
        if verified.entries.is_empty() {
            return Err("Installed PAK has no signed source entries".into());
        }
    }
    Ok(NativeOfflineReadiness {
        game_root: game,
        stage_root: stage,
        profile_id: planned.profile_id,
        validated_targets: planned.mods.iter().map(|m| m.target.clone()).collect(),
        total_mod_bytes: planned.total_bytes,
        native_launch_armed: false,
    })
}

/// Immutable, hash-bound preparation record for a future native offline
/// game swap. Deliberately read-only; an independent recovery worker and
/// durable atomic journal are mandatory before mounting installed game files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeOfflineSwapPlan {
    pub schema: u32,
    pub game_root: PathBuf,
    pub stage_root: PathBuf,
    pub profile_id: String,
    pub targets: Vec<NativeOfflineTarget>,
    pub native_launch_armed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeOfflineTarget {
    pub relative_path: String,
    pub original_sha256: String,
    pub replacement_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeRecoveryState {
    /// Exact installed original with no independent backup.
    OriginalReady,
    /// Original moved to an exact backup but custom PAK not yet mounted.
    InterruptedBeforeMount,
    /// Exact original backup and exact replacement installed.
    OfflineReplacementMounted,
    /// Original restored but the backup has not yet been removed.
    StrandedOriginalBackup,
}

/// Pair the signed game original with its exact isolated staging original.
/// The previous read-only gate checked both signatures but did not prove
/// the *installed bytes* matched the stage used to build the custom PAK.
pub fn plan_native_offline_swap(
    game_root: &Path,
    isolated_stage: &Path,
    profile_id: &str,
    original_public_key: &Path,
    mod_public_key: &Path,
) -> Result<NativeOfflineSwapPlan, String> {
    let ready = inspect_native_offline_readiness(
        game_root, isolated_stage, profile_id, original_public_key, mod_public_key,
    )?;
    let profile = plan_mod_profile(
        &ready.stage_root, profile_id, original_public_key, mod_public_key,
    ).map_err(|e| e.to_string())?;
    let mut targets = Vec::with_capacity(profile.mods.len());
    for item in &profile.mods {
        let original = unlinked_game_member(&ready.game_root, &item.target)?;
        let staged = regular(&ready.stage_root.join("paks").join(&item.target))?;
        if !staged.starts_with(ready.stage_root.join("paks")) {
            return Err("Original staged PAK escaped the isolated workspace".into());
        }
        let installed_digest = crate::staging::digest(&original)
            .map_err(|e| format!("Installed original PAK cannot be hashed: {e}"))?;
        let stage_digest = crate::staging::digest(&staged)
            .map_err(|e| format!("Stage original PAK cannot be hashed: {e}"))?;
        if installed_digest != stage_digest {
            return Err(format!(
                "Installed original differs from the exact signed staging base: {}",
                item.target
            ));
        }
        // plan_mod_profile has already verified signed mod pairs, member
        // inventories and item.sha256. Recheck the replacement hash, since
        // any bytes written after preflight invalidate this manifest.
        let replacement = regular(Path::new(&item.source))?;
        let replacement_sha256 = crate::staging::digest(&replacement)
            .map_err(|e| format!("Replacement PAK cannot be hashed: {e}"))?;
        if replacement_sha256 != item.sha256 {
            return Err("Modified PAK changed while building launch plan".into());
        }
        targets.push(NativeOfflineTarget {
            relative_path: item.target.clone(),
            original_sha256: installed_digest,
            replacement_sha256,
        });
    }
    Ok(NativeOfflineSwapPlan {
        schema: 1,
        game_root: ready.game_root,
        stage_root: ready.stage_root,
        profile_id: ready.profile_id,
        targets,
        native_launch_armed: false,
    })
}

/// Read only: classify the exact on-disk recovery phase for every planned
/// target. Any third-party modifications or unexpected links fail CLOSED.
/// This is NOT a restore operation and never deletes or renames game data.
pub fn audit_native_offline_swap(
    plan: &NativeOfflineSwapPlan,
) -> Result<Vec<(String, NativeRecoveryState)>, String> {
    if plan.schema != 1 || plan.native_launch_armed || plan.targets.is_empty()
        || plan.targets.len() > 512
    {
        return Err("Untrusted or unsupported native swap manifest".into());
    }
    let root = folder(&plan.game_root)?;
    let stage = folder(&plan.stage_root)?;
    if root == stage || root.starts_with(&stage) || stage.starts_with(&root) {
        return Err("Native offline swap roots overlap".into());
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for item in &plan.targets {
        if item.relative_path.is_empty() || !seen.insert(item.relative_path.to_ascii_lowercase())
            || item.relative_path.starts_with('/')
            || item.relative_path.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.contains('\\') || part.contains(':')
            })
            || !item.relative_path.to_ascii_lowercase().ends_with(".pak")
            || item.original_sha256.len() != 64
            || item.replacement_sha256.len() != 64
            || !item.original_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !item.replacement_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || item.original_sha256.eq_ignore_ascii_case(&item.replacement_sha256)
        {
            return Err("Invalid or colliding native offline target".into());
        }
        let target = root.join(&item.relative_path);
        let mut parent = root.clone();
        for component in item.relative_path.split('/').take(item.relative_path.split('/').count() - 1) {
            parent.push(component);
            if !fs::symlink_metadata(&parent).map_err(|e| e.to_string())?.file_type().is_dir() {
                return Err("Symlink or invalid installed game subfolder".into());
            }
        }
        let backup = PathBuf::from(format!("{}.customkey-original", target.display()));
        let ordinary_exists = |path: &Path| -> Result<bool, String> {
            match fs::symlink_metadata(path) {
                Ok(meta) => {
                    if !meta.file_type().is_file() {
                        return Err("Unexpected linked/non-file game or recovery target".into());
                    }
                    Ok(true)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.to_string()),
            }
        };
        let original_exists = ordinary_exists(&target)?;
        let backup_exists = ordinary_exists(&backup)?;
        let original_hash = if original_exists {
            let path = regular(&target)?;
            Some(crate::staging::digest(&path).map_err(|e| e.to_string())?)
        } else { None };
        let backup_hash = if backup_exists {
            let path = regular(&backup)?;
            Some(crate::staging::digest(&path).map_err(|e| e.to_string())?)
        } else { None };
        if backup_hash.as_deref().is_some_and(|value| value != item.original_sha256) {
            return Err(format!("Recovery backup changed: {}", item.relative_path));
        }
        let state = match (original_hash.as_deref(), backup_hash.as_deref()) {
            (Some(value), None) if value == item.original_sha256 => NativeRecoveryState::OriginalReady,
            (None, Some(_)) => NativeRecoveryState::InterruptedBeforeMount,
            (Some(value), Some(_)) if value == item.replacement_sha256 =>
                NativeRecoveryState::OfflineReplacementMounted,
            (Some(value), Some(_)) if value == item.original_sha256 =>
                NativeRecoveryState::StrandedOriginalBackup,
            _ => return Err(format!("Unrecognized or changed game PAK state: {}", item.relative_path)),
        };
        result.push((item.relative_path.clone(), state));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::create_mod_profile;
    use crate::staging::initialize_stage;

    #[test]
    fn native_launch_preflight_never_writes_game_or_stage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        let bin = root.join("bin64_SteamRetail");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("Evolve.exe"), b"demo").unwrap();
        let stage = temp.path().join("stage");
        initialize_stage(&stage).unwrap();
        let profile = create_mod_profile(&stage, "Offline").unwrap();
        let old = temp.path().join("original.bin");
        let new = temp.path().join("mod.bin");
        fs::write(&old, b"old").unwrap();
        fs::write(&new, b"new").unwrap();
        let before = fs::read(bin.join("Evolve.exe")).unwrap();
        assert!(
            inspect_native_offline_readiness(&root, &stage, &profile.id, &old, &new)
                .unwrap_err()
                .contains("No enabled")
        );
        assert_eq!(fs::read(bin.join("Evolve.exe")).unwrap(), before);
        assert!(inspect_native_offline_readiness(&stage, &root, &profile.id, &old, &new).is_err());
        assert!(!root.join("Game").exists());
    }

    #[test]
    fn recovery_audit_classifies_exact_hashes_without_mutating_game() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        let stage = tmp.path().join("stage");
        fs::create_dir_all(game.join("Game")).unwrap();
        fs::create_dir(&stage).unwrap();
        let installed = game.join("Game/libs.pak");
        let backup = game.join("Game/libs.pak.customkey-original");
        fs::write(&installed, b"original signed content").unwrap();
        let original_hash = crate::staging::digest(&installed).unwrap();
        let replacement = tmp.path().join("replacement.pak");
        fs::write(&replacement, b"replaced offline content").unwrap();
        let replacement_hash = crate::staging::digest(&replacement).unwrap();
        let plan = NativeOfflineSwapPlan {
            schema:1, game_root:game.clone(), stage_root:stage,
            profile_id:"local-test".into(), native_launch_armed:false,
            targets:vec![NativeOfflineTarget {
                relative_path:"Game/libs.pak".into(),
                original_sha256:original_hash,
                replacement_sha256:replacement_hash,
            }],
        };
        let snapshot = fs::read(&installed).unwrap();
        assert_eq!(audit_native_offline_swap(&plan).unwrap()[0].1,
            NativeRecoveryState::OriginalReady);
        assert_eq!(fs::read(&installed).unwrap(), snapshot);
        fs::rename(&installed, &backup).unwrap();
        assert_eq!(audit_native_offline_swap(&plan).unwrap()[0].1,
            NativeRecoveryState::InterruptedBeforeMount);
        fs::copy(&replacement, &installed).unwrap();
        assert_eq!(audit_native_offline_swap(&plan).unwrap()[0].1,
            NativeRecoveryState::OfflineReplacementMounted);
        fs::write(&installed, b"third party changes").unwrap();
        assert!(audit_native_offline_swap(&plan).is_err());
        fs::remove_file(&installed).unwrap();
        fs::copy(&backup, &installed).unwrap();
        assert_eq!(audit_native_offline_swap(&plan).unwrap()[0].1,
            NativeRecoveryState::StrandedOriginalBackup);
        assert_eq!(fs::read(&backup).unwrap(), snapshot);
        fs::remove_file(&backup).unwrap();
        assert_eq!(audit_native_offline_swap(&plan).unwrap()[0].1,
            NativeRecoveryState::OriginalReady);
    }

    #[test]
    fn native_launch_rejects_path_traversal_and_orphaned_backup() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("Game")).unwrap();
        let pak = root.join("Game/libs.pak");
        fs::write(&pak, b"original").unwrap();
        assert!(unlinked_game_member(root, "Game/../libs.pak").is_err());
        assert!(unlinked_game_member(root, "/Game/libs.pak").is_err());
        fs::write(root.join("Game/libs.pak.customkey-original"), b"backup").unwrap();
        assert!(unlinked_game_member(root, "Game/libs.pak").is_err());
        assert_eq!(fs::read(pak).unwrap(), b"original");
    }
}
