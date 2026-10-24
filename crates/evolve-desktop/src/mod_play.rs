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
    if let Some(active) = evolve_core::batch_stage_status(&stage)
        .map_err(|e| e.to_string())?
    {
        // The normal profile-list API deliberately refuses access while a
        // batch journal is active. Read this bounded identity metadata only
        // to distinguish our own prior mod set from a manual transaction.
        let catalog_path = stage.join("profiles.json");
        let meta = std::fs::symlink_metadata(&catalog_path)
            .map_err(|e| e.to_string())?;
        if !meta.file_type().is_file() || meta.len() > 1024 * 1024 {
            return Err("Cannot identify the previous mod set safely.".into());
        }
        let catalog: evolve_core::ProfileCatalog = serde_json::from_slice(
            &std::fs::read(catalog_path).map_err(|e| e.to_string())?
        ).map_err(|e| e.to_string())?;
        let ours = catalog.schema == 1 && catalog.profiles.iter().any(|profile| {
            profile.id == active.profile_id && profile.name == AUTO_PROFILE
        });
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

    // Replace only the manager-owned automatic profile, preserving all named
    // user profiles and manual staging transactions.
    for profile in evolve_core::list_mod_profiles(&stage)
        .map_err(|e| e.to_string())?
        .profiles
    {
        if profile.name == AUTO_PROFILE {
            evolve_core::delete_mod_profile(&stage, &profile.id).map_err(|e| e.to_string())?;
        }
    }
    let profile = evolve_core::create_mod_profile(&stage, AUTO_PROFILE)
        .map_err(|e| format!("Could not prepare enabled mods: {e}"))?;
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
