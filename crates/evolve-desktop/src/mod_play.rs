//! One-click mod staging for enabled, locally named Mod Projects.
//! Developer mode permits online testing with any project type; the normal
//! client and server retain control of networking and authentication.
//! Signing and recovery remain in the existing isolated stage and helper.
use super::{launcher_bridge, modding_keys, modding_projects, play_setup};
use modding_projects::ProjectRow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const AUTO_PROFILE: &str = "Enabled Mods";
const AUTO_ID_FILE: &str = "enabled-mod-profile.json";

fn owned_profile_id(stage: &std::path::Path) -> Result<Option<String>, String> {
    let path = stage.join(AUTO_ID_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.file_type().is_file() || meta.len() > 256 {
        return Err("Saved enabled-mod profile is invalid.".into());
    }
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let id = document["profile_id"]
        .as_str()
        .ok_or("Saved mod profile ID is missing")?;
    if document["schema"].as_u64() != Some(1)
        || id.len() != 32
        || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Saved enabled-mod profile ID is invalid".into());
    }
    Ok(Some(id.to_owned()))
}

fn save_owned_profile_id(stage: &std::path::Path, id: &str) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(stage).map_err(|e| e.to_string())?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "schema": 1, "profile_id": id
        }),
    )
    .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(stage.join(AUTO_ID_FILE))
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

/// If an old backup or swap record is unresolved, offer the user access
/// to the ordinary client's Repair/Verify UI, but DO NOT install enabled mods.
/// Never silently overwrite original game backups to satisfy a Play click.
fn launch_for_repair(
    game: &std::path::Path,
    client: &std::path::Path,
    swap: &std::path::Path,
    issue: &str,
) -> Result<(usize, u32, usize, Option<String>), String> {
    if client.as_os_str().is_empty() {
        return Err(format!(
            "Direct modded launch stopped safely: {issue}. Restore original game files or repair the signed stage before retrying."
        ));
    }
    let (pid, warnings) = play_setup::launch_clean(game, client, Some(swap))?;
    let mut message = format!(
        "Enabled mods were not installed: {issue}. The regular client opened instead; use its Repair or Verify option if needed."
    );
    if !warnings.is_empty() {
        message.push(' ');
        message.push_str(&warnings.join(" "));
    }
    Ok((0, pid, 0, Some(message)))
}

/// Reconcile any prior isolated stage batch, regardless of whether it was
/// created by the old manual testing action or the new enabled-mod launcher.
/// Never infer ownership from a stale enabled-mod-profile.json marker.
/// The core restores only hashes verified against its original batch backups,
/// retaining profiles and preserving unexpected third-party changes on error.
fn restore_previous_staged_profile(stage: &std::path::Path) -> Result<bool, String> {
    let active = evolve_core::batch_stage_status(stage)
        .map_err(|e| format!("Cannot read the previous stage transaction: {e}"))?;
    if active.is_none() {
        return Ok(false);
    }
    evolve_core::restore_profile_stage(stage)
        .map_err(|e| format!("Cannot restore the previous signed PAKs: {e}"))?;
    Ok(true)
}

/// Publish a single enabled-profile swap manifest after the old prepared
/// cache was validated/reset. This never alters the full signing inventory.
fn publish_active_mod_paks(
    stage: &std::path::Path,
    profile_id: &str,
    targets: &[String],
) -> Result<(), String> {
    if targets.is_empty() || targets.len() > 512 {
        return Err("No supported enabled-profile PAK targets to swap".into());
    }
    let mut seen = HashSet::new();
    for target in targets {
        if target.is_empty()
            || target.starts_with('/')
            || target.contains('\\')
            || target.contains(':')
            || !target.to_ascii_lowercase().ends_with(".pak")
            || target.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || !seen.insert(target.to_ascii_lowercase())
        {
            return Err("Unsafe or duplicate enabled mod PAK target".into());
        }
    }
    let mut file = tempfile::NamedTempFile::new_in(stage).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(
        &mut file,
        &serde_json::json!({
            "schema": 1,
            "profile_id": profile_id,
            "targets": targets
        }),
    )
    .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(stage.join("active_mod_paks.json"))
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

// The launcher accepts a setup bundle, a progress reporter, and a Developer
// intent flag. Keep the flag explicit so the mode is auditable.
#[allow(clippy::too_many_arguments)]
pub fn launch_enabled(
    mods: Vec<ProjectRow>,
    projects_root: PathBuf,
    stage: PathBuf,
    swap: PathBuf,
    game: PathBuf,
    client: PathBuf,
    progress: Arc<AtomicUsize>,
    // Developer testing override accepts all mod kinds, but the client/server
    // still determine actual network access.
    developer_online: bool,
    direct_launch: bool,
) -> Result<(usize, u32, usize, Option<String>), String> {
    if mods.is_empty() {
        return Err("Enable at least one mod before pressing Play with Mods.".into());
    }
    if direct_launch {
        super::direct_runtime::require_ready(&game, &client)?;
    }
    launcher_bridge::games_closed()?;
    // The former Developer toggle only changed status text. This is the real
    // Windows Firewall switch, matching private-manager outbound rules.
    // Perform it before staging/launching so a failed policy transition never
    // silently starts Evolve in the wrong network mode.
    super::online_guard::apply(&game, developer_online)?;
    // Physical original backups are a warning for client access, but an
    // absolute stop for writing another modded swap over those originals.
    if let Err(issue) = play_setup::audit_backup_files(&game) {
        return launch_for_repair(&game, &client, &swap, &issue);
    }
    let setup = if direct_launch {
        play_setup::configure_direct_helper(&game, &stage, &swap)
    } else {
        play_setup::configure_existing_helper(&game, &client, &stage, &swap)
    };
    if let Err(issue) = setup {
        let recovery_problem = ["journal", "backup", "restore", "recovery"]
            .iter()
            .any(|word| issue.to_ascii_lowercase().contains(word));
        if recovery_problem {
            return launch_for_repair(&game, &client, &swap, &issue);
        }
        return Err(issue);
    }
    progress.store(5, Ordering::Relaxed);

    let mut destinations = HashSet::new();
    let mut targets = Vec::with_capacity(mods.len());
    for project in &mods {
        let target = modding_projects::offline_test_target(project, &stage)
            .map_err(|e| format!("{}: {e}", project.label))?;
        if !destinations.insert(target.to_ascii_lowercase()) {
            return Err(format!(
                "Two enabled mods edit the same PAK ({target}). Disable one or combine their changes into one project."
            ));
        }
        targets.push(target);
    }
    // Profile staging needs verification keys, not a locally generated private key.
    // Official stages deliberately have no private PEM on disk.
    let original_public = modding_keys::original_public_key(&stage)?;
    let public = modding_keys::public_key(&stage)?;

    // A manual "Build & Prepare Offline Test" profile may remain staged
    // from before the enabled-mod workflow existed. Its ID legitimately
    // differs from the current automatic profile marker. The journal
    // contains byte-exact backups and a rollback transaction, so restore
    // ANY previously staged profile before applying the new enabled set.
    // This only restores files inside the isolated signing stage. It does
    // not delete that manual profile from profiles.json or touch game files.
    if let Err(issue) = restore_previous_staged_profile(&stage) {
        return launch_for_repair(
            &game,
            &client,
            &swap,
            &format!("Previous mod setup could not be restored: {issue}"),
        );
    }

    let mut built = Vec::with_capacity(mods.len());
    for (index, (project, target)) in mods.iter().zip(targets).enumerate() {
        progress.store(10 + (index * 60 / mods.len()), Ordering::Relaxed);
        let (_, output, _) = modding_projects::build_project_raw(project, &projects_root, &stage)
            .map_err(|e| format!("Could not build {}: {e}", project.label))?;
        built.push((project.label.clone(), target, output));
        progress.store(10 + ((index + 1) * 60 / mods.len()), Ordering::Relaxed);
    }

    // Use an owned stable ID marker. Never delete a manually named profile,
    // even if the user gave it the same "Enabled Mods" label.
    if let Some(id) = owned_profile_id(&stage)? {
        let catalog = evolve_core::list_mod_profiles(&stage).map_err(|e| e.to_string())?;
        if catalog
            .profiles
            .iter()
            .any(|p| p.id == id && p.name.starts_with(AUTO_PROFILE))
        {
            evolve_core::delete_mod_profile(&stage, &id).map_err(|e| e.to_string())?;
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let profile = evolve_core::create_mod_profile(&stage, &format!("{AUTO_PROFILE} {stamp:x}"))
        .map_err(|e| format!("Could not prepare enabled mods: {e}"))?;
    if let Err(error) = save_owned_profile_id(&stage, &profile.id) {
        let _ = evolve_core::delete_mod_profile(&stage, &profile.id);
        return Err(format!("Could not save enabled-mod settings: {error}"));
    }
    let result = (|| -> Result<(), String> {
        for (_, target, output) in &built {
            let basename = modding_keys::signed_basename(output)?;
            evolve_core::add_mod_to_profile(
                &stage,
                &profile.id,
                output,
                target,
                &original_public,
                &public,
                &basename,
            )
            .map_err(|e| format!("Could not add {target}: {e}"))?;
        }
        progress.store(78, Ordering::Relaxed);
        evolve_core::apply_profile_stage(&stage, &profile.id, &original_public, &public)
            .map_err(|e| format!("Could not activate enabled mods: {e}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        // Never delete a profile owning an active recovery transaction.
        if evolve_core::batch_stage_status(&stage)
            .ok()
            .flatten()
            .is_none()
        {
            let _ = evolve_core::delete_mod_profile(&stage, &profile.id);
        }
        return Err(error);
    }

    // Existing prepared files are copies of the previous signed set. Refresh
    // only those known temporary copies (never .customkey-original).
    progress.store(85, Ordering::Relaxed);
    let _preserved_stale_copies =
        match launcher_bridge::refresh_prepared_offline_cache(&stage, &swap) {
            Ok(count) => count,
            Err(issue) => return launch_for_repair(&game, &client, &swap, &issue),
        };
    // A profile swap contains ONLY the modified PAKs (and the signing
    // runtime DLLs), leaving all other installed stock PAKs untouched.
    let active_targets = built.iter().map(|(_, target, _)| target.clone()).collect::<Vec<_>>();
    publish_active_mod_paks(&stage, &profile.id, &active_targets)?;
    // Staging-only self-repair: accept a rebuilt hook ONLY when it hashes
    // exactly to this setup's saved original digest. A changed installed DLL
    // or unresolved game swap still falls back to the regular client.
    if let Err(issue) = launcher_bridge::repair_staged_hook_for_offline_play(&stage, &swap) {
        return launch_for_repair(&game, &client, &swap, &issue);
    }
    let inspected = if direct_launch {
        launcher_bridge::inspect_existing_direct_game(&stage, &swap)
    } else {
        launcher_bridge::inspect_existing_launcher(&stage, &swap)
    };
    let ready = match inspected {
        Ok(ready) => ready,
        Err(issue) => return launch_for_repair(&game, &client, &swap, &issue),
    };
    progress.store(92, Ordering::Relaxed);
    // Client mode preserves the CreateProcessW hook. Direct mode swaps staged
    // files first, starts its restore watchdog, then creates Evolve.exe.
    // Neither mode guarantees profile-server authentication.
    let id = if direct_launch {
        launcher_bridge::launch_direct_offline_game(&ready, &client, &progress)?
    } else {
        launcher_bridge::launch_existing_offline_client(&ready, &progress)?
    };
    if developer_online {
        // Record the chosen mode in the existing local activity logger.
        // "online" here means permitted by the manager, not server-confirmed.
        eprintln!(
            "Developer modded-online test requested (launched PID {id}); no mod-type filter applied; server acceptance unverified"
        );
    }
    progress.store(100, Ordering::Relaxed);
    Ok((mods.len(), id, ready.compatibility_warnings, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_test_profile_is_restored_before_enabled_play_without_deleting_profile() {
        use sha2::{Digest, Sha256};

        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        evolve_core::initialize_stage(&stage).unwrap();
        let manual =
            evolve_core::create_mod_profile(&stage, "Previous manual offline test").unwrap();
        let relative = "Game/UI_Assets.pak";
        let original = b"original signed staging bytes";
        let modified = b"previous manual modded bytes";
        let target = stage.join("paks").join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, modified).unwrap();
        let journal_id = "1".repeat(32);
        let backup_dir = stage.join(".evolve-batches").join(&journal_id);
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("before-0.pak"), original).unwrap();
        std::fs::write(backup_dir.join("displaced-0.pak"), original).unwrap();
        let hash = |data: &[u8]| format!("{:x}", Sha256::digest(data));
        let transaction = serde_json::json!({
            "schema": 1,
            "id": journal_id,
            "profile_id": manual.id,
            "entries": [{
                "target": relative,
                "original_sha256": hash(original),
                "replacement_sha256": hash(modified),
            }],
        });
        std::fs::write(
            stage.join("bundle-active.json"),
            serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();

        // A different profile ID is NOT a conflict: the old manual batch
        // can be rolled back even if an automatic-mod ownership marker
        // already exists and points at some unrelated/stale profile.
        let previous_enabled_id = "3".repeat(32);
        save_owned_profile_id(&stage, &previous_enabled_id).unwrap();
        assert_ne!(
            owned_profile_id(&stage).unwrap().as_deref(),
            Some(manual.id.as_str())
        );
        assert!(restore_previous_staged_profile(&stage).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert!(evolve_core::batch_stage_status(&stage).unwrap().is_none());
        assert!(
            evolve_core::list_mod_profiles(&stage)
                .unwrap()
                .profiles
                .iter()
                .any(|p| p.id == manual.id)
        );
        assert!(!restore_previous_staged_profile(&stage).unwrap());
    }

    #[test]
    fn unexpected_third_party_stage_changes_are_not_overwritten() {
        use sha2::{Digest, Sha256};

        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        evolve_core::initialize_stage(&stage).unwrap();
        let profile = evolve_core::create_mod_profile(&stage, "Manual project").unwrap();
        let target = stage.join("paks/Game/UI_Assets.pak");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"unexpected third party bytes").unwrap();
        let backup_dir = stage.join(".evolve-batches").join("2".repeat(32));
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("before-0.pak"), b"original").unwrap();
        let hash = |data: &[u8]| format!("{:x}", Sha256::digest(data));
        std::fs::write(
            stage.join("bundle-active.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": 1,
                "id": "2".repeat(32),
                "profile_id": profile.id,
                "entries": [{
                    "target": "Game/UI_Assets.pak",
                    "original_sha256": hash(b"original"),
                    "replacement_sha256": hash(b"expected modified"),
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(restore_previous_staged_profile(&stage).is_err());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"unexpected third party bytes"
        );
        assert!(stage.join("bundle-active.json").exists());
    }

    #[test]
    fn auto_profile_marker_is_bounded_and_never_inferred_from_a_name() {
        let temp = tempfile::tempdir().unwrap();
        assert!(owned_profile_id(temp.path()).unwrap().is_none());
        let id = "00112233445566778899aabbccddeeff";
        save_owned_profile_id(temp.path(), id).unwrap();
        assert_eq!(owned_profile_id(temp.path()).unwrap().as_deref(), Some(id));
        std::fs::write(temp.path().join(AUTO_ID_FILE), b"not a profile").unwrap();
        assert!(owned_profile_id(temp.path()).is_err());
    }
}
