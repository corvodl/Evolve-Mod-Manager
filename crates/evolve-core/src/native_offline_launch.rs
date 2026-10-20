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
        if component.is_empty() || component == "." || component == ".."
            || component.contains(':') || component.contains('\\')
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
        let verified = inspect_signed_pak(&installed, &original_key, basename)
            .map_err(|e| format!("Installed PAK {} failed original signature: {e}", item.target))?;
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
        assert!(inspect_native_offline_readiness(&root, &stage, &profile.id, &old, &new)
            .unwrap_err().contains("No enabled"));
        assert_eq!(fs::read(bin.join("Evolve.exe")).unwrap(), before);
        assert!(inspect_native_offline_readiness(&stage, &root, &profile.id, &old, &new)
            .is_err());
        assert!(!root.join("Game").exists());
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
