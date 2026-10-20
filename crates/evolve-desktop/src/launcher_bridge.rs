//! Compatibility bridge for the existing private offline-only Evolve launcher.
//! The native manager does NOT reproduce Frida instrumentation or install DLLs:
//! it reuses the user-configured, independently recoverable controlled-swap
//! helper after verifying the exact installed originals, staged signatures,
//! matching RSA hook and stage manifests. Never run for normal/online play.
use evolve_core::inspect_pak_hook_pair;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_MANIFEST: u64 = 4 * 1024 * 1024;
const MAX_ARCHIVES: usize = 512;
const KEY_START: usize = 0x870;
const KEY_BYTES: usize = 140;
const HOOK_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub struct LaunchReady {
    pub stage: PathBuf,
    pub swap: PathBuf,
    pub game: PathBuf,
    pub archive_count: usize,
    pub prepared: bool,
}

fn plain(path: &Path, max_size: u64) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > max_size {
        return Err(format!("Unsafe/missing regular file {}", path.display()));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}
fn directory(path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!("Unsafe or symlinked directory {}", path.display()));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}
fn json(path: &Path) -> Result<Value, String> {
    let path = plain(path, MAX_MANIFEST)?;
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid launcher stage manifest: {e}"))
}
fn sha(path: &Path) -> Result<String, String> {
    let mut stream = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 128 * 1024];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&chunk[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn safe_pak_name(raw: &str) -> Result<&str, String> {
    if !raw.to_ascii_lowercase().ends_with(".pak")
        || raw.len() > 600
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains(':')
        || raw
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("Unsafe signed Stage 2 PAK path: {raw}"));
    }
    Ok(raw)
}
fn within_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let candidate = root.join(relative);
    let file = plain(&candidate, 16 * 1024 * 1024 * 1024)?;
    if !file.starts_with(root) {
        return Err(format!("Stage/game file escaped its root: {relative}"));
    }
    Ok(file)
}
fn helper(swap: &Path, name: &str) -> Result<PathBuf, String> {
    let result = plain(&swap.join(name), 1024 * 1024)?;
    if !result.starts_with(swap) {
        return Err("Launcher helper escaped its configured directory".into());
    }
    Ok(result)
}
fn no_orphaned_backup(target: &Path) -> Result<(), String> {
    let backup = PathBuf::from(format!("{}.customkey-original", target.display()));
    if fs::symlink_metadata(&backup).is_ok() {
        return Err(format!(
            "Original backup remains at {}; restore using the existing private launcher before another launch",
            backup.display()
        ));
    }
    Ok(())
}
fn verify_hook(
    original: &Path,
    patched: &Path,
    old_key: &[u8],
    new_key: &[u8],
) -> Result<(), String> {
    let a = fs::read(original).map_err(|e| e.to_string())?;
    let b = fs::read(patched).map_err(|e| e.to_string())?;
    if a.len() != HOOK_BYTES
        || b.len() != HOOK_BYTES
        || old_key.len() != KEY_BYTES
        || new_key.len() != KEY_BYTES
        || a[KEY_START..KEY_START + KEY_BYTES] != *old_key
        || b[KEY_START..KEY_START + KEY_BYTES] != *new_key
        || a[..KEY_START] != b[..KEY_START]
        || a[KEY_START + KEY_BYTES..] != b[KEY_START + KEY_BYTES..]
    {
        return Err(
            "Original/custom client hook mismatch outside the exact RSA public-key region".into(),
        );
    }
    Ok(())
}
#[cfg(windows)]
fn games_closed() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("Cannot query active game tasks: {e}"))?;
    if !output.status.success() {
        return Err("Unable to verify Evolve and private launcher processes are closed".into());
    }
    let value = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    if value.lines().any(|row| {
        row.starts_with("\"evolve.exe\",") || row.starts_with("\"moddedevolvelauncher.exe\",")
    }) {
        return Err(
            "Evolve.exe and ModdedEvolveLauncher.exe must both be closed before preparation".into(),
        );
    }
    Ok(())
}
#[cfg(not(windows))]
fn games_closed() -> Result<(), String> {
    Ok(())
}

/// Locate the installed client hook using the private signing stage's
/// recorded original game root. No broad disk search or guessed DLL path.
pub fn original_hook_for_stage(stage: &Path) -> Result<PathBuf, String> {
    let stage = directory(stage)?;
    let plan = json(&stage.join("rekey_plan.json"))?;
    let game = directory(Path::new(
        plan["source_root"]
            .as_str()
            .ok_or("Signing stage has no original game root")?,
    ))?;
    within_file(&game, "bin64_SteamRetail/inject.dll")
}

/// Do not trust a stale `prepared` journal or size-only launch check.
/// Every ready file must match its exact original/staged target mapping,
/// SHA-256 and byte count, with no original backup present.
fn verify_prepared_journal(
    journal: &Value,
    files: &[(String, PathBuf, PathBuf)],
) -> Result<(), String> {
    let records = journal["files"]
        .as_array()
        .ok_or("Prepared swap has no file ledger")?;
    if records.len() != files.len() {
        return Err("Prepared swap file count differs from the authenticated PAK/hook plan".into());
    }
    let mut seen = std::collections::HashSet::new();
    for (name, target, staged) in files {
        // Canonicalize both ledger expectations as well: Windows can attach
        // the extended-path prefix when resolving an already-open file, while
        // the private controller stores a normal absolute path.
        let target = fs::canonicalize(target).map_err(|e| e.to_string())?;
        let staged = fs::canonicalize(staged).map_err(|e| e.to_string())?;
        let record = records
            .iter()
            .find(|record| record["name"].as_str() == Some(name.as_str()))
            .ok_or_else(|| format!("Prepared swap lacks expected {name}"))?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err("Duplicate prepared target".into());
        }
        let recorded_source =
            Path::new(record["source"].as_str().ok_or("Prepared source missing")?);
        let recorded_target =
            Path::new(record["target"].as_str().ok_or("Prepared target missing")?);
        if fs::canonicalize(recorded_source).map_err(|e| e.to_string())? != staged
            || fs::canonicalize(recorded_target).map_err(|e| e.to_string())? != target
        {
            return Err(format!("Prepared source/target path mismatch: {name}"));
        }
        let ready_text = record["ready"]
            .as_str()
            .ok_or("Prepared ready path missing")?;
        let backup_text = record["backup"]
            .as_str()
            .ok_or("Prepared backup path missing")?;
        let ready_path = Path::new(ready_text);
        let backup_path = Path::new(backup_text);
        let target_name = target
            .file_name()
            .and_then(|part| part.to_str())
            .ok_or("Unsafe original target filename")?;
        let ready_name = format!("{target_name}.customkey-ready");
        let backup_name = format!("{target_name}.customkey-original");
        if !ready_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(&ready_name))
            || !backup_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&backup_name))
            || fs::canonicalize(ready_path.parent().ok_or("Ready path has no parent")?)
                .map_err(|e| e.to_string())?
                != target.parent().ok_or("Target has no parent")?
            || fs::canonicalize(backup_path.parent().ok_or("Backup path has no parent")?)
                .map_err(|e| e.to_string())?
                != target.parent().ok_or("Target has no parent")?
            || fs::symlink_metadata(backup_path).is_ok()
        {
            return Err(format!(
                "Prepared ready/backup location differs from the original target: {name}"
            ));
        }
        no_orphaned_backup(&target)?;
        let ready = plain(ready_path, 16 * 1024 * 1024 * 1024)?;
        let size = fs::metadata(&ready).map_err(|e| e.to_string())?.len();
        let digest = sha(&ready)?;
        if record["bytes"].as_u64() != Some(size)
            || record["sha256"].as_str() != Some(digest.as_str())
            || sha(&staged)? != digest
        {
            return Err(format!(
                "Prepared file changed since signature verification: {name}"
            ));
        }
    }
    Ok(())
}

/// Exactly mirror the private manager's launcher preflight before permitting
/// its existing controlled_swap.py + auto_launch_frida.py lifecycle. This
/// remains a *prelaunch* verification: only a real game session can show that
/// CryEngine mounted all the custom-key archives.
pub fn inspect_existing_launcher(stage: &Path, swap: &Path) -> Result<LaunchReady, String> {
    let stage = directory(stage)?;
    let swap = directory(swap)?;
    let status = json(&stage.join("stage_status.json"))?;
    let plan = json(&stage.join("rekey_plan.json"))?;
    if status["status"].as_str() != Some("fully-staged-offline")
        || plan["blocked"].as_bool() == Some(true)
        || stage.join("REFRESHED-TO.json").exists()
    {
        return Err(
            "Private signing stage is blocked, superseded or not fully staged offline".into(),
        );
    }
    let game_str = plan["source_root"]
        .as_str()
        .ok_or("Missing exact original game root in rekey_plan.json")?;
    let game = directory(Path::new(game_str))?;
    let steam = directory(&game.join("bin64_SteamRetail"))?;
    plain(&steam.join("Evolve.exe"), 1024 * 1024 * 1024)?;
    // The private controller, not the public Rust manager, installs the
    // optional temporary DLL and original-key signed PAKs into this root.
    let old_key = plain(
        &stage
            .parent()
            .ok_or("Stage missing signing parent")?
            .join("RSAKeyData.bin"),
        140,
    )?;
    let new_key = plain(&stage.join("mykeys/public_key.bin"), 140)?;
    if old_key == new_key
        || fs::read(&old_key).map_err(|e| e.to_string())?
            == fs::read(&new_key).map_err(|e| e.to_string())?
    {
        return Err("Original and custom client public keys must be different".into());
    }
    let custom = within_file(&stage, "inject-custom.dll")?;
    let original = within_file(&game, "bin64_SteamRetail/inject.dll")?;
    no_orphaned_backup(&original)?;
    let original_key_bytes = fs::read(&old_key).map_err(|e| e.to_string())?;
    let custom_key_bytes = fs::read(&new_key).map_err(|e| e.to_string())?;
    verify_hook(&original, &custom, &original_key_bytes, &custom_key_bytes)?;
    let expected_shim = status["shim_sha256"]
        .as_str()
        .ok_or("Missing staged injector digest")?;
    if sha(&custom)? != expected_shim.to_ascii_lowercase() {
        return Err("Staged custom injector does not match stage_status.json".into());
    }
    // Never fall back to the original helper's developer-specific default
    // launcher path. The existing private setup must identify the user's
    // actual installed ModdedEvolveLauncher executable explicitly.
    let config = json(&swap.join("launcher_config.json"))?;
    let launcher_path = config["launcher"]
        .as_str()
        .ok_or("Missing launcher executable path in private launcher_config.json")?;
    let launcher = plain(Path::new(launcher_path), 1024 * 1024 * 1024)?;
    if launcher
        .file_name()
        .and_then(|s| s.to_str())
        .is_none_or(|name| !name.eq_ignore_ascii_case("ModdedEvolveLauncher.exe"))
    {
        return Err("Configured executable is not ModdedEvolveLauncher.exe".into());
    }
    for name in [
        "controlled_swap.py",
        "auto_launch_frida.py",
        "auto_restore_watch.py",
        "reusable_restore.py",
    ] {
        helper(&swap, name)?;
    }
    let entries = plan["entries"]
        .as_array()
        .ok_or("No PAK rekey plan entries")?;
    let signed = entries
        .iter()
        .filter(|e| e["kind"].as_str() == Some("signed-encrypted"))
        .collect::<Vec<_>>();
    if signed.is_empty()
        || signed.len() > MAX_ARCHIVES
        || status["count"].as_u64() != Some(signed.len() as u64)
    {
        return Err("Staged signed archive count mismatches signed rekey plan".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut planned = Vec::with_capacity(signed.len() + 1);
    for entry in signed {
        let path = safe_pak_name(entry["path"].as_str().ok_or("Missing PAK path")?)?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err("Duplicate signed PAK destination in plan".into());
        }
        let original_pak = within_file(&game, path)?;
        let custom_pak = within_file(&stage.join("paks"), path)?;
        no_orphaned_backup(&original_pak)?;
        let basename = Path::new(path)
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or("Invalid PAK basename")?;
        let report = inspect_pak_hook_pair(
            &original_pak,
            &custom_pak,
            &original,
            &custom,
            &old_key,
            &new_key,
            basename,
        )
        .map_err(|e| format!("PAK/hook mismatch for {path}: {e}"))?;
        if !report.archive_member_paths_match || report.signed_original_entries == 0 {
            return Err(format!("Signed source member mismatch for {path}"));
        }
        planned.push((path.to_owned(), original_pak, custom_pak));
    }
    planned.push((
        "bin64_SteamRetail/inject.dll".into(),
        original.clone(),
        custom.clone(),
    ));
    let journal = swap.join("controlled_swap_state.json");
    let mut prepared = false;
    if fs::symlink_metadata(&journal).is_ok() {
        let prior = json(&journal)?;
        match prior["phase"].as_str() {
            Some("restored") => (),
            Some("prepared") => {
                if prior["game"].as_str() != Some(game_str)
                    || prior["stage"].as_str() != stage.to_str()
                    || prior["count"].as_u64() != Some((seen.len() + 1) as u64)
                {
                    return Err(
                        "Prepared swap belongs to another game/stage; restore or reset it".into(),
                    );
                }
                verify_prepared_journal(&prior, &planned)?;
                prepared = true;
            }
            _ => return Err(
                "Existing launcher swap is active, interrupted or unsafe: restore originals first"
                    .into(),
            ),
        }
    }
    games_closed()?;
    Ok(LaunchReady {
        stage,
        swap,
        game,
        archive_count: seen.len(),
        prepared,
    })
}

/// Reuse the private launcher's already-tested control flow. It performs
/// same-volume .customkey-ready renames only during its Frida CreateProcessW
/// pause and starts an independent restore watchdog. Nothing here injects,
/// patches or launches the Evolve.exe executable directly.
#[cfg(windows)]
pub fn launch_existing_offline_client(plan: &LaunchReady) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;
    // Windows's Python launcher must be installed with the exact supported
    // Python 3.11 + Frida used by the private helpers.
    let helper_dir = &plan.swap;
    games_closed()?;
    if !plan.prepared {
        let status = Command::new("py")
            .args(["-3.11", "-u"])
            .arg(helper_dir.join("controlled_swap.py"))
            .arg("prepare")
            .arg("--stage-dir")
            .arg(&plan.stage)
            .arg("--game-root")
            .arg(&plan.game)
            .creation_flags(0x08000000)
            .status()
            .map_err(|e| format!("Python 3.11 launcher preparation unavailable: {e}"))?;
        if !status.success() {
            return Err(format!(
                "Private controlled_swap.py prepare failed: {status}"
            ));
        }
    }
    // The prepared journal is now mandatory, but the game can only be shown
    // compatible once the external helper actually pauses/releases it.
    let refreshed = inspect_existing_launcher(&plan.stage, &plan.swap)?;
    if !refreshed.prepared {
        return Err("Private swap helper did not publish a verified prepared journal".into());
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(helper_dir.join("rust-launch-bridge.log"))
        .map_err(|e| e.to_string())?;
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    let child = Command::new("py")
        .args(["-3.11", "-u"])
        .arg(helper_dir.join("auto_launch_frida.py"))
        .current_dir(helper_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .creation_flags(0x00000008 | 0x00000200 | 0x08000000) // detached/new process group/no window
        .spawn()
        .map_err(|e| format!("Could not start private offline launcher helper: {e}"))?;
    Ok(child.id())
}
#[cfg(not(windows))]
pub fn launch_existing_offline_client(_: &LaunchReady) -> Result<u32, String> {
    Err("The existing private launcher swap requires Windows x64".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_hook_lookup_is_bound_to_exact_signed_stage_game_root() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        let stage = temp.path().join("stage");
        fs::create_dir(&game).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::create_dir(game.join("bin64_SteamRetail")).unwrap();
        fs::write(
            game.join("bin64_SteamRetail/inject.dll"),
            [0x55u8; HOOK_BYTES],
        )
        .unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::to_vec(&serde_json::json!({
                "source_root": game
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            original_hook_for_stage(&stage).unwrap(),
            fs::canonicalize(game.join("bin64_SteamRetail/inject.dll")).unwrap()
        );
        fs::write(
            stage.join("rekey_plan.json"),
            br#"{"source_root":"../escape"}"#,
        )
        .unwrap();
        assert!(original_hook_for_stage(&stage).is_err());
    }

    #[test]
    fn rejects_unsafe_game_paths_and_partial_or_missing_stage() {
        for p in [
            "../evil.pak",
            "C:/root.pak",
            "/abs.pak",
            "assets/../foo.pak",
            "bad.pak/",
        ] {
            assert!(safe_pak_name(p).is_err(), "{p}");
        }
        assert_eq!(safe_pak_name("Game/libs.pak").unwrap(), "Game/libs.pak");
        let temp = tempfile::tempdir().unwrap();
        assert!(inspect_existing_launcher(temp.path(), temp.path()).is_err());
    }
    #[test]
    fn prepared_swap_requires_hash_exact_sources_and_never_overwrites_originals() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let original = root.join("original.pak");
        let staged = root.join("staged.pak");
        fs::write(&original, b"original signed archive").unwrap();
        fs::write(&staged, b"custom signed archive").unwrap();
        let ready = root.join("original.pak.customkey-ready");
        fs::write(&ready, b"custom signed archive").unwrap();
        let name = "Game/original.pak";
        let ready_label = ready.to_string_lossy().into_owned();
        let backup_label = format!("{}.customkey-original", original.display());
        let saved = serde_json::json!({
            "files": [{
                "name": name,
                "target": original,
                "source": staged,
                "ready": ready_label,
                "backup": backup_label,
                "sha256": sha(&ready).unwrap(),
                "bytes": fs::metadata(&ready).unwrap().len()
            }]
        });
        let plan = vec![(name.to_owned(), original.clone(), staged.clone())];
        assert!(verify_prepared_journal(&saved, &plan).is_ok());
        fs::write(&ready, b"tampered signed bytes").unwrap();
        assert!(verify_prepared_journal(&saved, &plan).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"original signed archive");
    }

    #[test]
    fn refuses_unverified_custom_injector_bytes_and_stranded_backup() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("original.dll");
        let custom = dir.path().join("inject-custom.dll");
        let key_a = vec![0x21u8; 140];
        let key_b = vec![0x32u8; 140];
        let mut source = vec![0x55u8; HOOK_BYTES];
        source[KEY_START..KEY_START + KEY_BYTES].copy_from_slice(&key_a);
        fs::write(&old, &source).unwrap();
        source[KEY_START..KEY_START + KEY_BYTES].copy_from_slice(&key_b);
        fs::write(&custom, &source).unwrap();
        assert!(verify_hook(&old, &custom, &key_a, &key_b).is_ok());
        source[3] ^= 0xFF;
        fs::write(&custom, &source).unwrap();
        assert!(verify_hook(&old, &custom, &key_a, &key_b).is_err());
        let stranded = dir.path().join("original.dll.customkey-original");
        fs::write(stranded, b"backup").unwrap();
        assert!(no_orphaned_backup(&old).is_err());
    }
}
