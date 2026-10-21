//! Launch a verified application update without asking users to run a setup EXE.
//!
//! After explicit "Update and restart" consent, a copy of the current binary
//! runs outside the install directory. It waits for Iced to exit, applies the
//! verified release and starts the new manager. Game assets are never touched.
use crate::updater::{ReleaseUpdate, UpdateError, VerifiedAsset, check_file_hash, download_verified_asset};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

const HELPER_FLAG: &str = "--apply-verified-evolve-update";
const MAX_ZIP_CONTENTS: u64 = 256 * 1024 * 1024;
const PORTABLE_MEMBERS: [&str; 4] = [
    "evolve-mod-manager.exe",
    "evolve-cli.exe",
    "hunt.ico",
    "README.md",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallationKind {
    Installed,
    Portable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedUpdate {
    pub version: String,
    pub payload: PathBuf,
    pub asset: VerifiedAsset,
    pub executable: PathBuf,
    pub root: PathBuf,
    pub kind: InstallationKind,
}

fn is_manager_binary(exe: &Path) -> bool {
    exe.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("evolve-mod-manager.exe"))
}

fn installation_kind(exe: &Path) -> Result<InstallationKind, UpdateError> {
    if !is_manager_binary(exe) {
        return Err(UpdateError::Invalid("unexpected application executable".into()));
    }
    let folder = exe
        .parent()
        .ok_or_else(|| UpdateError::Invalid("manager executable has no directory".into()))?;
    Ok(if folder.join("unins000.exe").is_file() {
        InstallationKind::Installed
    } else {
        InstallationKind::Portable
    })
}

pub fn prepare_update(update: &ReleaseUpdate, root: &Path) -> Result<PreparedUpdate, UpdateError> {
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let kind = installation_kind(&executable)?;
    let asset = match kind {
        InstallationKind::Installed => update.installer.clone(),
        InstallationKind::Portable => update.portable.clone(),
    };
    let payload = download_verified_asset(&asset, root)?;
    let prepared = PreparedUpdate {
        version: update.version.clone(),
        payload,
        asset,
        executable,
        root: root.to_owned(),
        kind,
    };
    validate_plan(&prepared)?;
    Ok(prepared)
}

fn validate_plan(plan: &PreparedUpdate) -> Result<(), UpdateError> {
    Version::parse(&plan.version)
        .map_err(|_| UpdateError::Invalid("invalid update version".into()))?;
    let expected_name = match plan.kind {
        InstallationKind::Installed => format!("EvolveModManager-Setup-{}.exe", plan.version),
        InstallationKind::Portable => format!("EvolveModManager-Portable-{}.zip", plan.version),
    };
    if !is_manager_binary(&plan.executable)
        || plan.asset.name != expected_name
        || plan.payload.file_name().and_then(|n| n.to_str()) != Some(expected_name.as_str())
        || plan.asset.size == 0
    {
        return Err(UpdateError::Invalid("update does not match the selected release".into()));
    }
    let current_exe = fs::canonicalize(&plan.executable)?;
    if current_exe != plan.executable
        || installation_kind(&current_exe)? != plan.kind
    {
        return Err(UpdateError::Invalid("application install type has changed".into()));
    }
    let updates = fs::canonicalize(plan.root.join("Updates"))?;
    let payload = fs::canonicalize(&plan.payload)?;
    if payload.parent() != Some(updates.as_path()) {
        return Err(UpdateError::Invalid("update is outside managed user data".into()));
    }
    check_file_hash(&payload, &plan.asset)?;
    Ok(())
}

/// Fork a copy of the running manager into user-owned Updates. This leaves the
/// original executable unlocked when the Iced process exits.
pub fn launch_worker(plan: &PreparedUpdate) -> Result<(), UpdateError> {
    validate_plan(plan)?;
    let updates = plan.root.join("Updates");
    let helper = tempfile::Builder::new()
        .prefix(".evolve-update-worker-")
        .suffix(".exe")
        .tempfile_in(&updates)?;
    let (_, worker) = helper.keep().map_err(|e| UpdateError::Io(e.error))?;
    if let Err(err) = fs::copy(&plan.executable, &worker) {
        let _ = fs::remove_file(&worker);
        return Err(UpdateError::Io(err));
    }
    let mut manifest = tempfile::Builder::new()
        .prefix(".evolve-update-plan-")
        .suffix(".json")
        .tempfile_in(&updates)?;
    serde_json::to_writer(manifest.as_file_mut(), plan)
        .map_err(|e| UpdateError::Invalid(format!("cannot prepare update plan: {e}")))?;
    manifest.as_file_mut().sync_all()?;
    let (_, manifest_path) = manifest.keep().map_err(|e| UpdateError::Io(e.error))?;
    if let Err(err) = Command::new(&worker).arg(HELPER_FLAG).arg(&manifest_path).spawn() {
        let _ = fs::remove_file(&worker);
        let _ = fs::remove_file(&manifest_path);
        return Err(UpdateError::Io(err));
    }
    Ok(())
}

/// Only the four expected top-level release artifacts may be written back to
/// a portable directory, never arbitrary ZIP paths, directories or symlinks.
fn verify_portable_contents<R: Read + io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<(), UpdateError> {
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| UpdateError::Invalid(format!("malformed ZIP: {e}")))?;
        if entry.is_dir()
            || !PORTABLE_MEMBERS.contains(&entry.name())
            || !names.insert(entry.name().to_owned())
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(UpdateError::Invalid(
                "portable update contains an unexpected or unsafe entry".into(),
            ));
        }
        total = total.checked_add(entry.size()).ok_or(UpdateError::TooLarge)?;
        if total > MAX_ZIP_CONTENTS {
            return Err(UpdateError::TooLarge);
        }
    }
    if names.len() != PORTABLE_MEMBERS.len() {
        return Err(UpdateError::Invalid("portable release is missing required files".into()));
    }
    Ok(())
}

fn apply_portable_update(plan: &PreparedUpdate) -> Result<(), UpdateError> {
    let folder = plan
        .executable
        .parent()
        .ok_or_else(|| UpdateError::Invalid("portable folder was not found".into()))?;
    let mut archive = zip::ZipArchive::new(File::open(&plan.payload)?)
        .map_err(|e| UpdateError::Invalid(format!("not a valid portable ZIP: {e}")))?;
    verify_portable_contents(&mut archive)?;

    // Same-volume staging ensures that individual renames are atomic. Backups
    // are kept on failed rollback instead of being deleted silently.
    let staging = tempfile::Builder::new()
        .prefix(".evolve-portable-update-")
        .tempdir_in(folder)?;
    for name in PORTABLE_MEMBERS {
        let mut from = archive
            .by_name(name)
            .map_err(|e| UpdateError::Invalid(format!("missing ZIP member: {e}")))?;
        let mut to = File::create(staging.path().join(format!("new-{name}")))?;
        let copied = io::copy(&mut (&mut from).take(MAX_ZIP_CONTENTS + 1), &mut to)?;
        if copied != from.size() || copied > MAX_ZIP_CONTENTS {
            return Err(UpdateError::TooLarge);
        }
        to.sync_all()?;
    }
    let mut replaced: Vec<(&str, bool)> = Vec::new();
    for name in PORTABLE_MEMBERS {
        let destination = folder.join(name);
        let backup = staging.path().join(format!("old-{name}"));
        let replacement = staging.path().join(format!("new-{name}"));
        let previously_existed = match fs::symlink_metadata(&destination) {
            Ok(info) if info.file_type().is_file() => true,
            Ok(_) => return Err(UpdateError::Invalid("unsafe portable target file".into())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(UpdateError::Io(e)),
        };
        if previously_existed {
            fs::rename(&destination, &backup)?;
        }
        if let Err(err) = fs::rename(&replacement, &destination) {
            let mut restore_failed = false;
            if previously_existed {
                restore_failed |= fs::rename(&backup, &destination).is_err();
            }
            for (previous, had_backup) in replaced.iter().rev() {
                let current = folder.join(previous);
                let saved = staging.path().join(format!("old-{previous}"));
                restore_failed |= fs::remove_file(&current).is_err();
                if *had_backup {
                    restore_failed |= fs::rename(&saved, &current).is_err();
                }
            }
            if restore_failed {
                let kept = staging.keep();
                return Err(UpdateError::Invalid(format!(
                    "portable update failed ({err}); recovery backups retained in {}",
                    kept.display()
                )));
            }
            return Err(UpdateError::Io(err));
        }
        replaced.push((name, previously_existed));
    }
    Ok(())
}

fn apply_update_plan(manifest: &Path) -> Result<(), UpdateError> {
    let info = fs::symlink_metadata(manifest)?;
    if !info.file_type().is_file() || info.len() > 16 * 1024 {
        return Err(UpdateError::Invalid("invalid updater manifest".into()));
    }
    let plan: PreparedUpdate = serde_json::from_slice(&fs::read(manifest)?)
        .map_err(|e| UpdateError::Invalid(format!("invalid update plan: {e}")))?;
    let managed_folder = fs::canonicalize(plan.root.join("Updates"))?;
    if fs::canonicalize(manifest)?.parent() != Some(managed_folder.as_path()) {
        return Err(UpdateError::Invalid("untrusted update manifest location".into()));
    }
    validate_plan(&plan)?;

    // Wait for the GUI, model viewer and archive workers to release DLLs/EXEs.
    thread::sleep(Duration::from_secs(3));
    match plan.kind {
        InstallationKind::Installed => {
            let directory = plan.executable.parent().ok_or_else(|| {
                UpdateError::Invalid("installer destination is unavailable".into())
            })?;
            let status = Command::new(&plan.payload)
                .args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/CLOSEAPPLICATIONS",
                ])
                .arg(format!("/DIR={}", directory.display()))
                .status()?;
            if !status.success() {
                return Err(UpdateError::Invalid(format!(
                    "Windows setup failed with exit status {:?}",
                    status.code()
                )));
            }
        }
        InstallationKind::Portable => apply_portable_update(&plan)?,
    }

    Command::new(&plan.executable).spawn()?;
    let _ = fs::remove_file(manifest);
    Ok(())
}

/// Called before the GUI starts, so the copied worker does not create an Iced
/// window. An error log remains in user data when a silent update fails.
pub fn try_run_helper() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(HELPER_FLAG)) {
        return false;
    }
    let plan_path = args.next().map(PathBuf::from);
    let result = match plan_path.as_deref() {
        Some(plan) if args.next().is_none() => apply_update_plan(plan),
        _ => Err(UpdateError::Invalid("missing or extra update helper arguments".into())),
    };
    if let Err(error) = result {
        if let Some(path) = plan_path {
            if let Some(parent) = path.parent() {
                let _ = fs::write(parent.join("last-update-error.txt"), error.to_string());
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_install_detection_never_treats_game_as_target() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("evolve-mod-manager.exe");
        fs::write(&app, b"test binary").unwrap();
        assert_eq!(installation_kind(&app).unwrap(), InstallationKind::Portable);
        fs::write(dir.path().join("unins000.exe"), b"installed").unwrap();
        assert_eq!(installation_kind(&app).unwrap(), InstallationKind::Installed);
        assert!(installation_kind(&dir.path().join("Evolve.exe")).is_err());
    }

    #[test]
    fn rejects_unexpected_zip_content_before_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.zip");
        let mut output = zip::ZipWriter::new(File::create(&file).unwrap());
        output
            .start_file::<_, ()>("../game/libs.pak", zip::write::FileOptions::default())
            .unwrap();
        output.write_all(b"not allowed").unwrap();
        output.finish().unwrap();
        let mut archive = zip::ZipArchive::new(File::open(file).unwrap()).unwrap();
        assert!(verify_portable_contents(&mut archive).is_err());
    }

    #[test]
    fn exact_portable_file_list_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("portable.zip");
        let mut writer = zip::ZipWriter::new(File::create(&file).unwrap());
        for name in PORTABLE_MEMBERS {
            writer
                .start_file::<_, ()>(name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(b"test").unwrap();
        }
        writer.finish().unwrap();
        let mut archive = zip::ZipArchive::new(File::open(file).unwrap()).unwrap();
        assert!(verify_portable_contents(&mut archive).is_ok());
    }
}
