//! One-click offline play for enabled, locally named Mod Projects.
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

pub fn launch_enabled(
    mods: Vec<ProjectRow>,
    projects_root: PathBuf,
    stage: PathBuf,
    swap: PathBuf,
    game: PathBuf,
    client: PathBuf,
    progress: Arc<AtomicUsize>,
) -> Result<(usize, u32, usize), String> {
    if mods.is_empty() {
        return Err("Enable at least one mod before pressing Play with Mods.".into());
    }
    launcher_bridge::games_closed()?;
    play_setup::configure_existing_helper(&game, &client, &stage, &swap)?;
    // Confirm the current game is restored before changing the offline stage.
    let current = launcher_bridge::inspect_existing_launcher(&stage, &swap)
        .map_err(|e| format!("Restore the original game before playing: {e}"))?;
    if current.game != game {
        return Err("Your mod setup belongs to another game installation.".into());
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
    let keys = modding_keys::signing_keys(&stage)?;

    // A previous enabled-mod set can be rolled back automatically; an
    // unrelated manually applied profile is never silently discarded.
    if let Some(active) = evolve_core::batch_stage_status(&stage).map_err(|e| e.to_string())? {
        let ours = owned_profile_id(&stage)?.is_some_and(|id| id == active.profile_id);
        if !ours {
            return Err("A manually staged profile is active. Restore that profile before playing with enabled mods.".into());
        }
        evolve_core::restore_profile_stage(&stage)
            .map_err(|e| format!("Could not restore the previous mod set: {e}"))?;
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
                &keys.original_public,
                &keys.public,
                &basename,
            )
            .map_err(|e| format!("Could not add {target}: {e}"))?;
        }
        progress.store(78, Ordering::Relaxed);
        evolve_core::apply_profile_stage(&stage, &profile.id, &keys.original_public, &keys.public)
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
    launcher_bridge::refresh_prepared_offline_cache(&stage, &swap)?;
    let ready = launcher_bridge::inspect_existing_launcher(&stage, &swap)?;
    progress.store(92, Ordering::Relaxed);
    let id = launcher_bridge::launch_existing_offline_client(&ready)?;
    progress.store(100, Ordering::Relaxed);
    Ok((mods.len(), id, ready.compatibility_warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

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
