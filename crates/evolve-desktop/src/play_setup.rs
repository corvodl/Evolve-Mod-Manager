//! Installed-game path detection and guarded launch handoff for the established
//! reversible private swap. No game PAK is modified by setup or this module.
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

/// Use ordinary Win32 paths when serializing game locations to Python.
/// Canonical filesystem paths remain in use for symlink/containment checks.
pub(crate) fn ordinary_windows_spelling(input: &str) -> String {
    let normalized = input.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if lower.starts_with("//?/unc/") {
        format!("//{}", &normalized[8..])
    } else if let Some(stripped) = normalized.strip_prefix("//?/") {
        stripped.to_owned()
    } else {
        normalized
    }
}

pub(crate) fn ordinary_game_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(ordinary_windows_spelling(&path.to_string_lossy()))
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

// Only replace hashes from published older nightlies. The Python launcher
// imports controlled_swap from its setup, so migrating auto_launch alone is
// insufficient for an existing Test B setup with an extended-path plan.
const PREVIOUS_AUTO_LAUNCH_SHA256: &str =
    "833c057c42846d51793e52fc429c506a6623c4ee59dade9df7e019a59d4ddc07";
const NIGHTLY_56_AUTO_LAUNCH_SHA256: &str =
    "8a7c67a33dadf221749b7d29a469a0df692a6245a72ea949d198f778a1fbd0b3";
const NIGHTLY_69_AUTO_LAUNCH_SHA256: &str =
    "3145d8feb68d0ea61f8bf7c960ed7dfd2bb2a983cb9b56a6bcc7b68f7b5dd09a";
// Exact helper shipped in Nightly .70 before the completion-marker fix.
const NIGHTLY_70_AUTO_LAUNCH_SHA256: &str =
    "4c153b3368bc99766016b56dccf24c5fd1de532a5194cd4ce43fb71767629547";
const PREVIOUS_CONTROLLED_SWAP_SHA256: &str =
    "5229e00859664c13c846de65e346031d4ff05c77f52170212dcd631a111a614d";
// Exact bundled controlled_swap.py shipped through Nightly 60 (PR #79/#80).
// Refresh only recognized published scripts; preserve user-modified helpers.
const NIGHTLY_60_CONTROLLED_SWAP_SHA256: &str =
    "9adbb5e58a8cffdccfb952afec88fa4e2d8c9d22e21002973fe314ca4186596d";
// Exact published helper from repository commit 43ccad26, shipped before
// the official Stage 2 signing upgrade. Existing offline installations may
// still have this helper; migrate it rather than failing modded launch.
const PRE_OFFICIAL_CONTROLLED_SWAP_SHA256: &str =
    "fdabe24e1b5659c04a93c4240e50b4c6fcd482a0da1dff08ac86816cd3227341";
// The full-inventory helper immediately before experimental sparse profile swaps.
// Only auto-migrate this exact published source; never replace user edits.
const PRE_SPARSE_CONTROLLED_SWAP_SHA256: &str =
    "1b40d8a9e17cef3df82ca9a776f134319223219e59e33b117672d91e9219e0a2";
// Helper shipped by PR #107/#108 before orphaned READY cache recovery.
// Migrate only this exact verified source; custom edits still fail closed.
const PRE_STALE_CACHE_CONTROLLED_SWAP_SHA256: &str =
    "2ba534cf790bd66ebe52b60cdbe8003e562f25440942d464cec3f3076ece0053";
const KNOWN_CONTROLLED_SWAP_SHA256: &[&str] = &[
    PRE_STALE_CACHE_CONTROLLED_SWAP_SHA256,
    PRE_SPARSE_CONTROLLED_SWAP_SHA256,
    PREVIOUS_CONTROLLED_SWAP_SHA256,
    NIGHTLY_60_CONTROLLED_SWAP_SHA256,
    PRE_OFFICIAL_CONTROLLED_SWAP_SHA256,
];
// Exact published Frida helper from commit ecf53f3b. Allow it to migrate
// as well, so existing Client Launch setups can upgrade both scripts.
const PRE_TOKEN_AUTO_LAUNCH_SHA256: &str =
    "37ba0586dfbd50ceb791a3156ce2bacb3d01f541b7cebccd21f8d10dc5b99aba";
const CURRENT_AUTO_LAUNCH: &str = include_str!("../launcher_support/auto_launch_frida.py");
const CURRENT_CONTROLLED_SWAP: &str = include_str!("../launcher_support/controlled_swap.py");

fn refresh_verified_helper(
    swap: &Path,
    name: &str,
    current: &str,
    known_sha256: &[&str],
) -> Result<bool, String> {
    let path = swap.join(name);
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > 1024 * 1024 {
        return Err(format!("Existing {name} must be a regular, bounded file"));
    }
    let existing = fs::read(&path).map_err(|e| e.to_string())?;
    if existing == current.as_bytes() {
        return Ok(false);
    }
    // Windows-distributed Python helpers can use CRLF even when the Git
    // source was LF. Recognize that one newline-only transformation, but
    // never accept changed Python code as a known published helper.
    let existing_text = String::from_utf8(existing.clone())
        .map_err(|_| format!("{name} is not a valid UTF-8 Python helper"))?;
    let canonical_digest = Sha256::digest(existing_text.replace("\r\n", "\n").as_bytes());
    let digest = format!("{canonical_digest:x}");
    if !known_sha256.contains(&digest.as_str()) {
        return Err(format!(
            "Unrecognized customized or older {name} (SHA-256 {digest}); cannot safely migrate the offline launcher. The file was not overwritten."
        ));
    }
    let mut temp = tempfile::NamedTempFile::new_in(swap)
        .map_err(|e| format!("Cannot prepare updated {name}: {e}"))?;
    temp.write_all(current.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    if fs::read(&path).map_err(|e| e.to_string())? != existing {
        return Err(format!(
            "{name} changed during migration; refusing replacement"
        ));
    }
    temp.persist(&path)
        .map_err(|e| format!("Cannot update verified legacy {name}: {}", e.error))?;
    Ok(true)
}

/// Only enable the existing private helper for exactly the selected game and
/// client. Never repoint a signing stage to a different installed game.
/// Common path/journal verification; direct mode does not need an installed
/// Modded Evolve launcher or its Frida / standard-user client broker.
fn configure_helper(
    game: &Path,
    client: Option<&Path>,
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
    // Reuse prepared game assets while refreshing both verified bundled helpers.
    refresh_verified_helper(
        &swap,
        "controlled_swap.py",
        CURRENT_CONTROLLED_SWAP,
        KNOWN_CONTROLLED_SWAP_SHA256,
    )?;
    if let Some(client) = client {
        refresh_verified_helper(
            &swap,
            "auto_launch_frida.py",
            CURRENT_AUTO_LAUNCH,
            &[
                PREVIOUS_AUTO_LAUNCH_SHA256,
                NIGHTLY_56_AUTO_LAUNCH_SHA256,
                NIGHTLY_69_AUTO_LAUNCH_SHA256,
                NIGHTLY_70_AUTO_LAUNCH_SHA256,
                PRE_TOKEN_AUTO_LAUNCH_SHA256,
            ],
        )?;
        crate::standard_user_launch::install_into_swap(&swap)?;
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
        let new_path = ordinary_game_path(client).to_string_lossy().into_owned();
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
    } else {
        Ok(())
    }
}

pub fn configure_existing_helper(
    game: &Path,
    client: &Path,
    stage: &Path,
    swap: &Path,
) -> Result<(), String> {
    configure_helper(game, Some(client), stage, swap)
}

/// Direct mode checks the same signed stage and recovery journal, without
/// requiring or overwriting the legacy ModdedEvolveLauncher installation.
pub fn configure_direct_helper(game: &Path, stage: &Path, swap: &Path) -> Result<(), String> {
    configure_helper(game, None, stage, swap)
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
    let mut warnings = recovery_warnings(game, swap);
    // The private manager can leave a persistent Evolve-specific block.
    // Unmodded client access should restore networking even after an
    // interrupted modded game. A failure becomes an actionable warning:
    // the ordinary client's Repair/Verify UI must remain accessible.
    if let Err(error) = crate::online_guard::apply(game, true) {
        warnings.push(format!(
            "Could not remove manager-owned offline firewall rule: {error}"
        ));
    }
    // Launch through the interactive user's verified Explorer token. An
    // elevated Mod Manager must NOT silently elevate the normal client/game.
    let pid = crate::standard_user_launch::launch(client)?;
    Ok((pid, warnings))
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
    fn game_paths_serialize_as_ordinary_drive_and_unc_paths() {
        assert_eq!(
            ordinary_windows_spelling(r"\\?\C:\Games\ModdedEvolve\EvolveGame"),
            "C:/Games/ModdedEvolve/EvolveGame"
        );
        assert_eq!(
            ordinary_windows_spelling(r"C:\Games\ModdedEvolve\EvolveGame"),
            "C:/Games/ModdedEvolve/EvolveGame"
        );
        assert_eq!(
            ordinary_windows_spelling(r"\\?\UNC\server\share\EvolveGame"),
            "//server/share/EvolveGame"
        );
    }

    #[test]
    fn helper_migration_refuses_custom_private_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("auto_launch_frida.py");
        let custom = b"print('private user helper')\n";
        fs::write(&helper, custom).unwrap();
        assert!(
            refresh_verified_helper(
                dir.path(),
                "auto_launch_frida.py",
                CURRENT_AUTO_LAUNCH,
                &[PREVIOUS_AUTO_LAUNCH_SHA256, NIGHTLY_56_AUTO_LAUNCH_SHA256]
            )
            .is_err()
        );
        assert_eq!(fs::read(&helper).unwrap(), custom);
    }

    #[test]
    fn published_pre_official_swap_hash_is_recognized() {
        // The SHA-256 reported by a Nightly user exactly matches the
        // controlled_swap.py published at commit 43ccad26. Never accept
        // arbitrary Python helper content simply to bypass migration.
        assert!(
            KNOWN_CONTROLLED_SWAP_SHA256
                .contains(&"fdabe24e1b5659c04a93c4240e50b4c6fcd482a0da1dff08ac86816cd3227341")
        );
    }

    #[test]
    fn verified_legacy_helper_with_windows_newlines_migrates_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("controlled_swap.py");
        let old = "import sys\nprint('known script')\n";
        let old_windows = old.replace('\n', "\r\n");
        fs::write(&path, old_windows).unwrap();
        let expected = format!("{:x}", Sha256::digest(old.as_bytes()));
        assert!(
            refresh_verified_helper(
                dir.path(),
                "controlled_swap.py",
                CURRENT_CONTROLLED_SWAP,
                &[expected.as_str()]
            )
            .unwrap()
        );
        assert_eq!(fs::read(&path).unwrap(), CURRENT_CONTROLLED_SWAP.as_bytes());
    }

    #[test]
    fn verified_helper_migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auto_launch_frida.py");
        fs::write(&path, CURRENT_AUTO_LAUNCH).unwrap();
        assert!(
            !refresh_verified_helper(
                dir.path(),
                "auto_launch_frida.py",
                CURRENT_AUTO_LAUNCH,
                &[PREVIOUS_AUTO_LAUNCH_SHA256]
            )
            .unwrap()
        );
    }

    #[test]
    fn clean_play_refuses_orphaned_original_backup() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ui.pak.customkey-original"), b"original").unwrap();
        assert!(audit_backup_files(dir.path()).is_err());
    }
}
