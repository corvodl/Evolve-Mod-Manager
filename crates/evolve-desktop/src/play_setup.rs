//! Installed-game path detection and guarded launch handoff for the established
//! reversible private swap. No game PAK is modified by setup or this module.
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const GAME_HINT: &str = r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail";
const CLIENT_HINT: &str =
    r"C:\Users\billn\AppData\Local\Programs\Modded Evolve\ModdedEvolveLauncher.exe";

fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file() && m.len() > 0)
}

/// Accept the EvolveGame folder or its bin64_SteamRetail folder; persist the
/// root, since the original signing/swap journal records EvolveGame.
pub fn game_root(input: &str) -> Result<PathBuf, String> {
    let path = Path::new(input.trim());
    let root = if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("bin64_SteamRetail"))
    {
        path.parent()
            .ok_or("Game executable folder has no parent")?
    } else {
        path
    };
    if !regular(&root.join("bin64_SteamRetail/Evolve.exe"))
        || !regular(&root.join("bin64_SteamRetail/inject.dll"))
    {
        return Err(
            "Select EvolveGame (or bin64_SteamRetail) containing Evolve.exe and inject.dll".into(),
        );
    }
    fs::canonicalize(root).map_err(|e| format!("Cannot inspect installed game: {e}"))
}

pub fn client_exe(input: &str) -> Result<PathBuf, String> {
    let path = Path::new(input.trim());
    if !path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("ModdedEvolveLauncher.exe"))
        || !regular(path)
    {
        return Err("Select the installed ModdedEvolveLauncher.exe file".into());
    }
    fs::canonicalize(path).map_err(|e| format!("Cannot inspect client executable: {e}"))
}

/// Hints are tried only when there is no explicit saved user selection.
pub fn detect_game() -> Option<String> {
    game_root(GAME_HINT)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn detect_client() -> Option<String> {
    let local_app = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let candidate = local_app
        .as_ref()
        .map(|base| base.join("Programs/Modded Evolve/ModdedEvolveLauncher.exe"));
    candidate
        .as_deref()
        .and_then(|path| client_exe(&path.to_string_lossy()).ok())
        .or_else(|| client_exe(CLIENT_HINT).ok())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn validate_paths(game: &str, client: &str) -> Result<(PathBuf, PathBuf), String> {
    Ok((game_root(game)?, client_exe(client)?))
}

/// Only enable the existing private helper for exactly the selected game and
/// client. Never repoint a signing stage to a different installed game.
pub fn configure_existing_helper(
    game: &Path,
    client: &Path,
    stage: &Path,
    swap: &Path,
) -> Result<(), String> {
    crate::launcher_bridge::games_closed()?;
    let stage = fs::canonicalize(stage).map_err(|e| format!("Signing stage: {e}"))?;
    let swap = fs::canonicalize(swap).map_err(|e| format!("Swap helper folder: {e}"))?;
    if !stage.is_dir() || !swap.is_dir() || stage.starts_with(game) || swap.starts_with(game) {
        return Err("Stage, helper and installed game must be separate directories".into());
    }
    let bytes = fs::read(stage.join("rekey_plan.json"))
        .map_err(|e| format!("Signing stage missing rekey_plan.json: {e}"))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Signing stage plan is oversized".into());
    }
    let plan: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid signing stage plan: {e}"))?;
    let recorded = plan["source_root"]
        .as_str()
        .ok_or("Signing stage does not record an installed game root")?;
    let source = fs::canonicalize(recorded)
        .map_err(|e| format!("Signing stage original game path is missing: {e}"))?;
    if source != game {
        return Err("Signing stage belongs to a different game. Recreate the stage for this installation before playing with mods.".into());
    }
    for helper in [
        "controlled_swap.py",
        "auto_launch_frida.py",
        "reusable_restore.py",
        "auto_restore_watch.py",
    ] {
        if !regular(&swap.join(helper)) {
            return Err(format!(
                "Swap helper {helper} is missing. Select the complete private-manager swap folder."
            ));
        }
    }
    let journal = swap.join("controlled_swap_state.json");
    if journal.exists() {
        let data = fs::read(&journal).map_err(|e| e.to_string())?;
        if data.len() > 4 * 1024 * 1024 {
            return Err("Swap journal is oversized; restore the original game files first".into());
        }
        let state: Value =
            serde_json::from_slice(&data).map_err(|e| format!("Invalid swap journal: {e}"))?;
        if !matches!(state["phase"].as_str(), Some("prepared" | "restored")) {
            return Err("Swap journal is active or interrupted. Restore original game files before changing the configured client.".into());
        }
        let files = state["files"]
            .as_array()
            .ok_or("Swap journal has no file list")?;
        if files.iter().any(|item| {
            item["backup"]
                .as_str()
                .is_none_or(|path| Path::new(path).exists())
        }) {
            return Err("Original backup files exist; restore before changing the client".into());
        }
    }
    let config = swap.join("launcher_config.json");
    let mut previous = Map::new();
    if config.exists() {
        let raw = fs::read(&config).map_err(|e| e.to_string())?;
        if raw.len() > 16 * 1024 {
            return Err("Existing launcher configuration is oversized".into());
        }
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|e| format!("Invalid existing launcher configuration: {e}"))?;
        previous = value
            .as_object()
            .ok_or("Expected launcher configuration object")?
            .clone();
    }
    let new_path = client.to_string_lossy().into_owned();
    if previous.get("launcher").and_then(Value::as_str) == Some(new_path.as_str()) {
        return Ok(());
    }
    // Only this one owned field changes. Tempfile.persist performs a rename;
    // a failed write leaves the old configuration intact.
    previous.insert("launcher".into(), Value::String(new_path));
    let mut staging = tempfile::NamedTempFile::new_in(&swap)
        .map_err(|e| format!("Cannot stage launcher configuration: {e}"))?;
    serde_json::to_writer_pretty(&mut staging, &previous)
        .map_err(|e| format!("Cannot serialize launcher configuration: {e}"))?;
    staging.write_all(b"\n").map_err(|e| e.to_string())?;
    staging.as_file().sync_all().map_err(|e| e.to_string())?;
    staging
        .persist(&config)
        .map_err(|e| format!("Cannot save launcher configuration: {e}"))?;
    Ok(())
}

pub(super) fn audit_backup_files(game: &Path) -> Result<(), String> {
    let mut stack = vec![(game.to_path_buf(), 0usize)];
    let mut checked = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > 20 {
            return Err("Installed game directory nesting exceeds safety limit".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| format!("Cannot audit game files: {e}"))? {
            checked += 1;
            if checked > 120_000 {
                return Err("Cannot verify all game backup paths; too many entries".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Symlinked game asset prevents safe clean-play verification".into());
            }
            if kind.is_dir() {
                stack.push((entry.path(), depth + 1));
            } else if entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".customkey-original")
            {
                return Err(format!(
                    "Original game backup remains at {}; restore before normal play",
                    entry.path().display()
                ));
            }
        }
    }
    Ok(())
}

/// Report recovery concerns without preventing the normal client from
/// opening its repair/verification UI. This function NEVER writes game files.
/// The verified modded swap path still refuses to overwrite original backups.
pub(super) fn recovery_warnings(game: &Path, swap: Option<&Path>) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(issue) = audit_backup_files(game) {
        warnings.push(format!("Original-file recovery may be needed: {issue}"));
    }
    if let Some(swap) = swap {
        let journal = swap.join("controlled_swap_state.json");
        if journal.exists() {
            let status = (|| -> Result<(), String> {
                let raw = fs::read(&journal).map_err(|e| e.to_string())?;
                if raw.len() > 4 * 1024 * 1024 {
                    return Err("Recovery record is too large".into());
                }
                let state: Value = serde_json::from_slice(&raw)
                    .map_err(|e| format!("Recovery record cannot be read: {e}"))?;
                if !matches!(state["phase"].as_str(), Some("prepared" | "restored")) {
                    return Err("Previous modded game session may be unfinished".into());
                }
                Ok(())
            })();
            if let Err(issue) = status {
                warnings.push(format!("Recovery warning: {issue}"));
            }
        }
    }
    warnings
}

/// Launch only the regular client. Recovery warnings must never block access
/// to its built-in repair option. No PAK/DLL swapping occurs in this path.
pub fn launch_clean(
    game: &Path,
    client: &Path,
    swap: Option<&Path>,
) -> Result<(u32, Vec<String>), String> {
    crate::launcher_bridge::games_closed()?;
    let warnings = recovery_warnings(game, swap);
    let mut cmd = Command::new(client);
    cmd.current_dir(
        client
            .parent()
            .ok_or("Client executable has no installation folder")?,
    );
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let process = cmd
        .spawn()
        .map_err(|e| format!("Could not start normal Modded Evolve client: {e}"))?;
    Ok((process.id(), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_folder_normalizes_to_game_root() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("EvolveGame");
        let bin = game.join("bin64_SteamRetail");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("Evolve.exe"), b"game").unwrap();
        fs::write(bin.join("inject.dll"), b"original").unwrap();
        let expected = fs::canonicalize(&game).unwrap();
        assert_eq!(game_root(&game.to_string_lossy()).unwrap(), expected);
        assert_eq!(game_root(&bin.to_string_lossy()).unwrap(), expected);
        assert!(client_exe(&bin.join("Evolve.exe").to_string_lossy()).is_err());
    }

    #[test]
    fn stage_cannot_be_retargeted_to_another_installation() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        let other = temp.path().join("other");
        let stage = temp.path().join("stage");
        let swap = temp.path().join("swap");
        for dir in [&game, &other, &stage, &swap] {
            fs::create_dir(dir).unwrap();
        }
        let game = fs::canonicalize(&game).unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::to_vec(&serde_json::json!({"source_root":other})).unwrap(),
        )
        .unwrap();
        assert!(
            configure_existing_helper(&game, &temp.path().join("client.exe"), &stage, &swap)
                .unwrap_err()
                .contains("different game")
        );
        assert!(!swap.join("launcher_config.json").exists());
    }

    #[test]
    fn live_journal_blocks_client_reconfiguration() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let stage = dir.path().join("stage");
        let swap = dir.path().join("swap");
        for folder in [&game, &stage, &swap] {
            fs::create_dir(folder).unwrap();
        }
        let game = fs::canonicalize(&game).unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::to_vec(&serde_json::json!({"source_root":game})).unwrap(),
        )
        .unwrap();
        for helper in [
            "controlled_swap.py",
            "auto_launch_frida.py",
            "reusable_restore.py",
            "auto_restore_watch.py",
        ] {
            fs::write(swap.join(helper), b"helper").unwrap();
        }
        fs::write(
            swap.join("controlled_swap_state.json"),
            br#"{"phase":"swapped","files":[]}"#,
        )
        .unwrap();
        assert!(
            configure_existing_helper(
                &game,
                &dir.path().join("ModdedEvolveLauncher.exe"),
                &stage,
                &swap
            )
            .unwrap_err()
            .contains("active or interrupted")
        );
        assert!(!swap.join("launcher_config.json").exists());
    }

    #[test]
    fn recovery_warnings_do_not_require_a_valid_swap_journal() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let swap = dir.path().join("swap");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&swap).unwrap();
        fs::write(game.join("ui.pak.customkey-original"), b"original").unwrap();
        fs::write(swap.join("controlled_swap_state.json"), b"not JSON").unwrap();
        let warnings = recovery_warnings(&game, Some(&swap));
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("Original-file recovery"));
        assert!(warnings[1].contains("Recovery warning"));
        assert!(game.join("ui.pak.customkey-original").exists());
    }

    #[test]
    fn clean_play_refuses_orphaned_original_backup() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ui.pak.customkey-original"), b"original").unwrap();
        assert!(audit_backup_files(dir.path()).is_err());
    }
}
