//! Compatibility bridge for the existing private offline-only Evolve launcher.
//! The native manager does NOT reproduce Frida instrumentation or install DLLs:
//! it reuses the user-configured, independently recoverable controlled-swap
//! helper after verifying the exact installed originals, staged signatures,
//! matching RSA hook and stage manifests. Never run for normal/online play.
use evolve_core::client_hook::inspect_pak_hook_pair_with_original_name;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
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
    pub compatibility_warnings: usize,
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
fn normalized_pak_name(raw: &str) -> Result<String, String> {
    let normalized = raw.replace('\\', "/");
    safe_pak_name(&normalized)?;
    Ok(normalized)
}

/// Python's pathlib.Path stringifies source-relative PAKs with Windows
/// backslashes, while rekey_plan.json stores virtual paths using '/'.
/// Compare only canonical safe relative paths, not raw separator spelling.
fn normalized_swap_name(raw: &str) -> Result<String, String> {
    let normalized = raw.replace('\\', "/");
    if normalized.eq_ignore_ascii_case("bin64_SteamRetail/inject.dll") {
        return Ok("bin64_SteamRetail/inject.dll".into());
    }
    normalized_pak_name(&normalized)
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
            "An original game backup remains at {}. Close Evolve and its launcher, then use Modding > Advanced Archive Tools > Tools > Restore original PAKs + hook. Do not delete the backup or launch again until recovery succeeds.",
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
pub(super) fn games_closed() -> Result<(), String> {
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
pub(super) fn games_closed() -> Result<(), String> {
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
    if journal["version"].as_u64() != Some(1) || journal["phase"].as_str() != Some("prepared") {
        return Err("Private swap journal must be version 1 in the prepared phase".into());
    }
    let records = journal["files"]
        .as_array()
        .ok_or("Prepared swap has no file ledger")?;
    if records.len() != files.len() {
        return Err("Prepared swap file count differs from the authenticated PAK/hook plan".into());
    }
    let mut seen = std::collections::HashSet::new();
    for (index, (name, target, staged)) in files.iter().enumerate() {
        // The private auto_launch_frida.py compares this exact file ordering
        // before arming CreateProcessW. Name-only search would let Rust report
        // a false PASS for a journal the real launcher will always refuse.
        let record = records
            .get(index)
            .ok_or("Prepared journal entry ordering/count changed")?;
        let actual_name = normalized_swap_name(
            record["name"]
                .as_str()
                .ok_or("Prepared swap record missing its name")?,
        )?;
        if !actual_name.eq_ignore_ascii_case(name) {
            return Err(format!(
                "Prepared swap entry ordering differs from private launcher: {name}"
            ));
        }
        // Canonicalize both ledger expectations as well: Windows can attach
        // the extended-path prefix when resolving an already-open file, while
        // the private controller stores a normal absolute path.
        let target = fs::canonicalize(target).map_err(|e| e.to_string())?;
        let staged = fs::canonicalize(staged).map_err(|e| e.to_string())?;
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
    // The private controlled_swap.fresh() forbids game/stage nesting.
    // Without this check, a future helper could stage into the live game and
    // make the supposed offline-only rollback no longer independent.
    if game.starts_with(&stage)
        || stage.starts_with(&game)
        || game.starts_with(&swap)
        || swap.starts_with(&game)
    {
        return Err("The original game, signing stage and private swap helper must be in separate directory trees".into());
    }
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
    let mut compatibility_warnings = 0usize;
    for entry in signed {
        let path = normalized_pak_name(entry["path"].as_str().ok_or("Missing PAK path")?)?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err("Duplicate signed PAK destination in plan".into());
        }
        let original_pak = within_file(&game, &path)?;
        let custom_pak = within_file(&stage.join("paks"), &path)?;
        no_orphaned_backup(&original_pak)?;
        let basename = Path::new(&path)
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or("Invalid PAK basename")?;
        // Older stages omit signed_name; use their physical basename.
        // New stages record the RSA-authenticated lowercase variant when
        // present, so both the original and replacement can be verified
        // without guessing names or weakening the signature check.
        let signed_name = entry["signed_name"].as_str().unwrap_or(basename);
        if signed_name != basename && signed_name != basename.to_ascii_lowercase() {
            return Err(format!("Invalid authenticated signing name for {path}"));
        }
        let report = inspect_pak_hook_pair_with_original_name(
            &original_pak,
            &custom_pak,
            &original,
            &custom,
            &old_key,
            &new_key,
            (signed_name, basename),
        )
        .map_err(|e| format!("PAK/hook mismatch for {path}: {e}"))?;
        if report.signed_original_entries == 0 || report.signed_replacement_entries == 0 {
            return Err(format!(
                "Signed PAK has no verified archive members: {path}"
            ));
        }
        // Differing archive member inventories indicate an untested game
        // version, not unsafe paths. Continue offline after cryptographic
        // verification; recovery still owns byte-exact original backups.
        if !report.archive_member_paths_match {
            compatibility_warnings += 1;
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
        compatibility_warnings,
        prepared,
    })
}

/// The detached private helper is only READY after the Frida agent has
/// reported that its CreateProcessW hook was installed. Spawning py.exe alone
/// is not proof that the hook exists, and must not be reported as success.
/// Inspect only output written after this specific launch (bounded to 64 KiB).
#[cfg(windows)]
fn inspect_hook_startup_log(path: &Path, offset: u64) -> Result<bool, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() < offset {
        return Err("Offline helper startup log was truncated during handoff".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let output = String::from_utf8_lossy(&bytes);
    for line in output.lines() {
        if line.contains("AUTOMATIC LAUNCH FAILED:")
            || line.contains("AUTO-RESTORE MONITOR FAILED:")
            || line.contains("Could not install CreateProcessW hook:")
            || line.contains("Frida agent error:")
        {
            return Err(format!(
                "Private launcher reported a failed offline hook: {}",
                line.trim()
            ));
        }
    }
    Ok(output
        .lines()
        .any(|line| line.contains("Automatic CreateProcessW hook ready at ")))
}

#[cfg(windows)]
fn check_private_helper_runtime() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let result = Command::new("py")
        .args([
            "-3.11",
            "-c",
            "import sys,frida; assert sys.maxsize > 2**32; assert sys.version_info[:2] == (3,11)",
        ])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("Private launcher requires Python 3.11 x64 + Frida: {e}"))?;
    if !result.success() {
        return Err("The existing private launcher requires Python 3.11 x64 with the Frida module available to that interpreter. No client swap was started.".into());
    }
    Ok(())
}

/// Validate all file paths in an interrupted swap journal *before* asking
/// the trusted private restore helper to use it. Interrupted swaps can be
/// partially renamed, so target/backup FILES need not all exist; their
/// parents, names and exact plan/source mapping must still match.
fn guarded_restore_state(stage: &Path, swap: &Path) -> Result<String, String> {
    let stage = directory(stage)?;
    let swap = directory(swap)?;
    let journal = json(&swap.join("controlled_swap_state.json"))?;
    if journal["version"].as_u64() != Some(1) {
        return Err("Unrecognized private swap recovery journal version".into());
    }
    let phase = journal["phase"].as_str().ok_or("Missing recovery phase")?;
    if !matches!(
        phase,
        "prepared" | "restored" | "swapping" | "swapped" | "interrupted" | "restoring"
    ) {
        return Err("Unrecognized/unsafe private swap recovery phase".into());
    }
    let plan = json(&stage.join("rekey_plan.json"))?;
    let game = directory(Path::new(
        plan["source_root"]
            .as_str()
            .ok_or("Missing original game root")?,
    ))?;
    if game.starts_with(&stage)
        || stage.starts_with(&game)
        || game.starts_with(&swap)
        || swap.starts_with(&game)
    {
        return Err("Game and stage/swap helper directories overlap".into());
    }
    if directory(Path::new(
        journal["game"]
            .as_str()
            .ok_or("Journal game root missing")?,
    ))? != game
        || directory(Path::new(
            journal["stage"]
                .as_str()
                .ok_or("Journal signing stage missing")?,
        ))? != stage
    {
        return Err(
            "Recovery journal was created for another installed game or signing stage".into(),
        );
    }
    let mut expected = Vec::new();
    for entry in plan["entries"]
        .as_array()
        .ok_or("Signing stage plan has no entries")?
    {
        if entry["kind"].as_str() != Some("signed-encrypted") {
            continue;
        }
        let name = normalized_pak_name(
            entry["path"]
                .as_str()
                .ok_or("Missing signed archive path")?,
        )?;
        expected.push((
            name.clone(),
            stage.join("paks").join(&name),
            game.join(&name),
        ));
    }
    if expected.is_empty() || expected.len() > MAX_ARCHIVES {
        return Err("No valid signed archive recovery targets".into());
    }
    expected.push((
        "bin64_SteamRetail/inject.dll".to_owned(),
        stage.join("inject-custom.dll"),
        game.join("bin64_SteamRetail/inject.dll"),
    ));
    let records = journal["files"]
        .as_array()
        .ok_or("Recovery journal missing files")?;
    if records.len() != expected.len() || journal["count"].as_u64() != Some(expected.len() as u64) {
        // Automatically generated first-run setups start with a restored,
        // empty journal. That is legitimate only when none of the installed
        // targets still has an original-backup suffix from an earlier swap.
        if phase == "restored" && records.is_empty() && journal["count"].as_u64() == Some(0) {
            let stranded = expected.iter().any(|(_, _, target)| {
                fs::symlink_metadata(format!("{}.customkey-original", target.display())).is_ok()
            });
            if !stranded {
                return Ok(phase.to_owned());
            }
        }
        return Err(format!(
            "Recovery ledger has {} file entries (declared count {:?}), but the selected signing stage expects {} signed PAK and hook entries. This stage/swap combination cannot safely restore existing game backups. Keep the .customkey-original files untouched; select the original matching signing stage and swap helper from Settings, then use Restore original game files. Never replace a missing journal with a newly generated one.",
            records.len(),
            journal["count"].as_u64(),
            expected.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (record, (name, source, target)) in records.iter().zip(expected) {
        let actual_name = normalized_swap_name(
            record["name"]
                .as_str()
                .ok_or("Recovery journal name missing")?,
        )?;
        if !actual_name.eq_ignore_ascii_case(&name) || !seen.insert(name.to_ascii_lowercase()) {
            return Err(format!("Recovery journal target ordering changed: {name}"));
        }
        let expected_source = plain(&source, 16 * 1024 * 1024 * 1024)?;
        if !expected_source.starts_with(&stage) {
            return Err(format!("Recovery source escapes its signing stage: {name}"));
        }
        let recorded_source = plain(
            Path::new(
                record["source"]
                    .as_str()
                    .ok_or("Journal source path missing")?,
            ),
            16 * 1024 * 1024 * 1024,
        )?;
        if expected_source != recorded_source {
            return Err(format!(
                "Recovery source does not match staged signed asset: {name}"
            ));
        }
        let expected_parent = directory(target.parent().ok_or("Original target parent missing")?)?;
        if !expected_parent.starts_with(&game) {
            return Err(format!(
                "Recovery target escapes its installed game root: {name}"
            ));
        }
        let target_name = target
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or("Original target filename missing")?;
        for (field, suffix) in [
            ("target", ""),
            ("backup", ".customkey-original"),
            ("ready", ".customkey-ready"),
        ] {
            let raw = Path::new(record[field].as_str().ok_or("Recovery file path missing")?);
            let parent = directory(raw.parent().ok_or("Recovery file parent missing")?)?;
            let filename = raw
                .file_name()
                .and_then(|x| x.to_str())
                .ok_or("Recovery filename missing")?;
            if parent != expected_parent
                || !filename.eq_ignore_ascii_case(&format!("{target_name}{suffix}"))
            {
                return Err(format!(
                    "Recovery {field} path does not match installed original: {name}"
                ));
            }
            if fs::symlink_metadata(raw).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                return Err(format!(
                    "Unsafe symlink in private recovery journal: {name}"
                ));
            }
        }
    }
    Ok(phase.to_owned())
}

/// Fallback from the private manager: orphaned originals can be recovered
/// without recreating an obsolete controlled-swap journal.
#[cfg(windows)]
fn recover_orphaned_originals(stage: &Path, swap: &Path) -> Result<(usize, bool), String> {
    use std::os::windows::process::CommandExt;
    let plan = json(&stage.join("rekey_plan.json"))?;
    let game = directory(Path::new(
        plan["source_root"].as_str().ok_or("Signing stage is missing its game path")?,
    ))?;
    if game.starts_with(stage) || stage.starts_with(&game)
        || game.starts_with(swap) || swap.starts_with(&game)
    {
        return Err("Game and recovery folders must be separate.".into());
    }
    plain(&game.join("bin64_SteamRetail/Evolve.exe"), 1024 * 1024 * 1024)?;
    // Use the exact journal-independent implementation from the private
    // manager. It keeps existing modded files under unique recovery names
    // before restoring .customkey-original backups.
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(swap.join("rust-restore-bridge.log"))
        .map_err(|e| e.to_string())?;
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    let status = Command::new("py")
        .args(["-3.11", "-u", "-c", include_str!("../launcher_support/orphaned_swap_recovery.py")])
        .arg("--game")
        .arg(&game)
        .arg("--manifest-dir")
        .arg(swap.join("recovery-logs"))
        .arg("--apply")
        .current_dir(swap)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("Could not start automatic original-file recovery: {e}"))?;
    if !status.success() {
        return Err(
            "Original-file recovery needs attention. No backup was intentionally deleted; see the recovery log in Mod Data."
                .into(),
        );
    }
    Ok((0, false))
}

#[cfg(windows)]
fn lock_restore_watchdog(swap: &Path) -> Result<Option<File>, String> {
    let path = swap.join("auto_restore_watch.lock");
    if !path.exists() {
        return Ok(None);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Cannot inspect offline restore watchdog lock: {e}"))?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
        "The independent private restore watchdog is active. Allow it to finish before attempting manual recovery."
            .to_owned()
    })?;
    Ok(Some(file))
}

/// Explicit recovery shortcut for an interrupted offline-only private swap.
/// The Rust manager NEVER hand-rolls reverse renames: it invokes the existing
/// helper which owns the original journal, then re-verifies the restored
/// original client hook and signed PAKs with the public prelaunch gate.
#[cfg(windows)]
pub fn restore_existing_offline_client(stage: &Path, swap: &Path) -> Result<(usize, bool), String> {
    use std::os::windows::process::CommandExt;
    games_closed()?;
    let stage = directory(stage)?;
    let swap = directory(swap)?;
    // Avoid racing the private launcher watchdog's independent restoration.
    // Hold this handle until the manual helper finishes verifying originals.
    let _watchdog_guard = lock_restore_watchdog(&swap)?;
    let phase = match guarded_restore_state(&stage, &swap) {
        Ok(phase) => phase,
        Err(_) => {
            // A stale, empty, or missing swap ledger must not prevent the
            // original independent backup recovery path from running.
            return recover_orphaned_originals(&stage, &swap);
        }
    };
    if matches!(phase.as_str(), "prepared" | "restored") {
        let journal = json(&swap.join("controlled_swap_state.json"))?;
        let backups_remain = journal["files"].as_array().is_some_and(|files| {
            files.iter().any(|item| {
                item["backup"]
                    .as_str()
                    .is_some_and(|path| fs::symlink_metadata(path).is_ok())
            })
        });
        if !backups_remain {
            let checked = inspect_existing_launcher(&stage, &swap)?;
            return Ok((checked.archive_count, checked.prepared));
        }
        // The journal phase can lag a crash while originals are still
        // backed up. The trusted helper validates the original PAK hashes and
        // injector RSA slot before restoring anything. Never skip recovery
        // merely because a stale journal claims the stage is clean.
    }
    // Recovery must remain available even when Frida is missing or broken.
    // reusable_restore.py needs Python 3.11, but does not import Frida.
    let script = helper(&swap, "reusable_restore.py")?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(swap.join("rust-restore-bridge.log"))
        .map_err(|e| e.to_string())?;
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    let result = Command::new("py")
        .args(["-3.11", "-u"])
        .arg(script)
        .current_dir(&swap)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("Private recovery helper unavailable: {e}"))?;
    if !result.success() {
        return Err(format!(
            "Private recovery helper failed ({result}); inspect rust-restore-bridge.log before launching again"
        ));
    }
    let checked = inspect_existing_launcher(&stage, &swap)?;
    // The private reusable_restore.py intentionally returns to "prepared"
    // when it can recycle all verified temporary custom files, or "restored"
    // when fresh preparation will be required. Both mean originals returned.
    Ok((checked.archive_count, checked.prepared))
}
#[cfg(not(windows))]
pub fn restore_existing_offline_client(_: &Path, _: &Path) -> Result<(usize, bool), String> {
    Err("Offline Evolve launcher recovery requires Windows x64".into())
}

/// Refresh only temporary .customkey-ready copies when enabled mods change.
/// Existing automatic setups keep their old helper scripts, so recovery
/// cache maintenance belongs here rather than relying on a newly added
/// Python subcommand. No installed game target or original backup is written.
pub fn refresh_prepared_offline_cache(stage: &Path, swap: &Path) -> Result<(), String> {
    games_closed()?;
    let swap = directory(swap)?;
    let stage = directory(stage)?;
    let journal_path = swap.join("controlled_swap_state.json");
    let mut journal = json(&journal_path)?;
    let phase = journal["phase"].as_str().ok_or("Missing swap phase")?;
    if !matches!(phase, "prepared" | "restored") {
        return Err("Restore original game files before changing enabled mods.".into());
    }
    let rows = journal["files"]
        .as_array()
        .ok_or("Swap ledger is incomplete")?;
    if phase == "restored" && rows.is_empty() && journal["count"].as_u64() == Some(0) {
        return Ok(());
    }
    guarded_restore_state(&stage, &swap)?;
    let mut ready_paths = Vec::new();
    for item in rows {
        let backup = Path::new(item["backup"].as_str().ok_or("Missing backup path")?);
        if fs::symlink_metadata(backup).is_ok() {
            return Err(
                "An original game backup remains. Restore original game files first.".into(),
            );
        }
        let ready = Path::new(item["ready"].as_str().ok_or("Missing prepared file path")?);
        match fs::symlink_metadata(ready) {
            Ok(meta) => {
                if !meta.file_type().is_file()
                    || item["bytes"].as_u64() != Some(meta.len())
                    || item["sha256"].as_str() != Some(sha(ready)?.as_str())
                {
                    return Err("A prepared mod copy changed unexpectedly. Restore original game files first.".into());
                }
                ready_paths.push(ready.to_path_buf());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    // All paths, ownership and old digests were verified before touching
    // disposable files. A crash partway through can be retried idempotently.
    for ready in ready_paths {
        fs::remove_file(ready).map_err(|e| e.to_string())?;
    }
    journal["phase"] = Value::String("restored".into());
    let mut file = tempfile::NamedTempFile::new_in(&swap).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, &journal).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(&journal_path)
        .map_err(|e| e.error.to_string())?;
    Ok(())
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
    check_private_helper_runtime()?;
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
    let initial_log_len = log.metadata().map_err(|e| e.to_string())?.len();
    let log_path = helper_dir.join("rust-launch-bridge.log");
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new("py")
        .args(["-3.11", "-u"])
        .arg(helper_dir.join("auto_launch_frida.py"))
        .current_dir(helper_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .creation_flags(0x00000008 | 0x00000200 | 0x08000000) // detached/new process group/no window
        .spawn()
        .map_err(|e| format!("Could not start private offline launcher helper: {e}"))?;
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match inspect_hook_startup_log(&log_path, initial_log_len) {
            Ok(true) => return Ok(pid),
            Ok(false) => (),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "The private launcher helper exited ({status}) without confirming a ready CreateProcessW hook. Check rust-launch-bridge.log; no successful PAK swap can be assumed."
            ));
        }
        if std::time::Instant::now() >= deadline {
            // No swaps are allowed until the Frida controller confirms the
            // exact game-process creation pause. Stop an unready controller.
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "Private launch helper did not confirm a ready CreateProcessW hook within 30 seconds; helper stopped. Close the launcher before retrying."
                    .into(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
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
            "version": 1,
            "phase": "prepared",
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
    fn private_windows_journal_names_normalize_without_path_traversal() {
        assert_eq!(
            normalized_swap_name("Game\\libs.pak").unwrap(),
            "Game/libs.pak"
        );
        assert_eq!(
            normalized_pak_name("Game\\libs.pak").unwrap(),
            "Game/libs.pak"
        );
        assert_eq!(
            normalized_swap_name("bin64_SteamRetail\\inject.dll").unwrap(),
            "bin64_SteamRetail/inject.dll"
        );
        for invalid in [
            "../Game/libs.pak",
            "Game\\\\..\\\\libs.pak",
            "Game\\\\\\\\libs.pak",
            "C:\\\\Game\\\\libs.pak",
            "\\\\server\\\\Game\\\\libs.pak",
            "Game/not-an-inject.dll",
        ] {
            assert!(normalized_swap_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn prepared_ledger_must_match_private_launcher_exact_order_and_schema() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let mut expected = Vec::new();
        let mut rows = Vec::new();
        for name in ["Game/first.pak", "Game/second.pak"] {
            let n = name.split('/').next_back().unwrap();
            let target = base.join(n);
            let staged = base.join(format!("staged-{n}"));
            let ready = base.join(format!("{n}.customkey-ready"));
            fs::write(&target, b"unchanged original").unwrap();
            fs::write(&staged, b"custom payload").unwrap();
            fs::write(&ready, b"custom payload").unwrap();
            rows.push(serde_json::json!({
                "name": name,
                "source": staged,
                "target": target,
                "ready": ready,
                "backup": format!("{}.customkey-original", target.display()),
                "sha256": sha(&ready).unwrap(),
                "bytes": fs::metadata(&ready).unwrap().len()
            }));
            expected.push((name.to_owned(), target, staged));
        }
        let mut state = serde_json::json!({"version":1,"phase":"prepared","files":rows});
        assert!(verify_prepared_journal(&state, &expected).is_ok());
        state["files"][0]["name"] = serde_json::json!("Game\\first.pak");
        assert!(verify_prepared_journal(&state, &expected).is_ok());
        state["files"][0]["name"] = serde_json::json!("Game/first.pak");
        state["files"].as_array_mut().unwrap().reverse();
        assert!(
            verify_prepared_journal(&state, &expected)
                .unwrap_err()
                .contains("ordering")
        );
        state["files"].as_array_mut().unwrap().reverse();
        state["version"] = serde_json::json!(2);
        assert!(verify_prepared_journal(&state, &expected).is_err());
        state["version"] = serde_json::json!(1);
        state["phase"] = serde_json::json!("swapped");
        assert!(verify_prepared_journal(&state, &expected).is_err());
    }

    #[test]
    fn rejects_overlapping_private_stage_and_game_roots_without_writing() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("game/stage");
        let swap = root.path().join("swap");
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir(&swap).unwrap();
        fs::write(
            stage.join("stage_status.json"),
            br#"{"status":"fully-staged-offline","count":1}"#,
        )
        .unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::to_vec(&serde_json::json!({
                "source_root": root.path().join("game"),
                "entries":[{"kind":"signed-encrypted","path":"Game/libs.pak"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let err = inspect_existing_launcher(&stage, &swap).unwrap_err();
        assert!(err.contains("separate directory trees"), "{err}");
        assert!(!swap.join("controlled_swap_state.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn helper_handoff_requires_actual_hook_ready_marker_after_new_log_offset() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rust-launch-bridge.log");
        fs::write(&log, b"Automatic CreateProcessW hook ready at old run\n").unwrap();
        let beginning = fs::metadata(&log).unwrap().len();
        assert!(!inspect_hook_startup_log(&log, beginning).unwrap());
        fs::write(
            &log,
            b"Automatic CreateProcessW hook ready at old run\nPRECHECK PASS: staged\nAutomatic CreateProcessW hook ready at 0x1234\n",
        )
        .unwrap();
        assert!(inspect_hook_startup_log(&log, beginning).unwrap());
        fs::write(
            &log,
            b"Automatic CreateProcessW hook ready at old run\nAUTOMATIC LAUNCH FAILED: Frida unavailable\n",
        )
        .unwrap();
        assert!(inspect_hook_startup_log(&log, beginning).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn manual_restore_does_not_race_running_private_watchdog_lock() {
        let temp = tempfile::tempdir().unwrap();
        let lock = temp.path().join("auto_restore_watch.lock");
        fs::write(&lock, b"0").unwrap();
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .unwrap();
        guard.try_lock().unwrap();
        assert!(lock_restore_watchdog(temp.path()).is_err());
        guard.unlock().unwrap();
        let recovered = lock_restore_watchdog(temp.path()).unwrap();
        assert!(recovered.is_some());
    }

    #[test]
    fn restore_guard_rejects_foreign_and_forged_journals_without_mutation() {
        let home = tempfile::tempdir().unwrap();
        let game = home.path().join("game");
        let stage = home.path().join("stage");
        let swap = home.path().join("swap");
        fs::create_dir_all(game.join("bin64_SteamRetail")).unwrap();
        fs::create_dir_all(game.join("Game")).unwrap();
        fs::create_dir_all(stage.join("paks/Game")).unwrap();
        fs::create_dir(&swap).unwrap();
        fs::write(stage.join("paks/Game/libs.pak"), b"signed staged archive").unwrap();
        fs::write(stage.join("inject-custom.dll"), b"stage hook bytes").unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::to_vec(&serde_json::json!({
                "source_root": game,
                "entries": [{"kind": "signed-encrypted", "path": "Game/libs.pak"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let pak_target = game.join("Game/libs.pak");
        let hook_target = game.join("bin64_SteamRetail/inject.dll");
        let source_pak = stage.join("paks/Game/libs.pak");
        let source_hook = stage.join("inject-custom.dll");
        let row = |name: &str, source: &Path, target: &Path| {
            serde_json::json!({
                "name": name,
                "source": source,
                "target": target,
                "ready": format!("{}.customkey-ready", target.display()),
                "backup": format!("{}.customkey-original", target.display()),
                "bytes": 5,
                "sha256": "test-only"
            })
        };
        let mut journal = serde_json::json!({
            "version": 1, "phase": "interrupted",
            "game": game, "stage": stage, "count": 2,
            "files": [
                row("Game/libs.pak", &source_pak, &pak_target),
                row("bin64_SteamRetail/inject.dll", &source_hook, &hook_target)
            ]
        });
        let ledger = swap.join("controlled_swap_state.json");
        fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert_eq!(guarded_restore_state(&stage, &swap).unwrap(), "interrupted");
        // First-run signing setup has an empty, restored swap journal. It
        // should not be reported as corrupt when the original files are clean.
        let new_setup = serde_json::json!({
            "version": 1, "phase": "restored",
            "game": game, "stage": stage, "count": 0, "files": []
        });
        fs::write(&ledger, serde_json::to_vec(&new_setup).unwrap()).unwrap();
        assert_eq!(guarded_restore_state(&stage, &swap).unwrap(), "restored");
        // However, even one stranded backup forbids treating that empty
        // journal as sufficient recovery instructions.
        let hook_backup = PathBuf::from(format!("{}.customkey-original", hook_target.display()));
        fs::write(&hook_backup, b"original backup").unwrap();
        let mismatch = guarded_restore_state(&stage, &swap).unwrap_err();
        assert!(mismatch.contains("0 file entries"), "{mismatch}");
        assert!(
            mismatch.contains("original matching signing stage"),
            "{mismatch}"
        );
        assert_eq!(fs::read(&hook_backup).unwrap(), b"original backup");
        fs::remove_file(&hook_backup).unwrap();
        fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
        for phase in ["swapping", "prepared", "restored"] {
            journal["phase"] = serde_json::json!(phase);
            fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
            assert_eq!(guarded_restore_state(&stage, &swap).unwrap(), phase);
        }
        journal["phase"] = serde_json::json!("interrupted");
        fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
        journal["files"].as_array_mut().unwrap().reverse();
        fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(guarded_restore_state(&stage, &swap).is_err());
        journal["files"].as_array_mut().unwrap().reverse();
        journal["files"][1]["backup"] = serde_json::json!(home.path().join("foreign-original"));
        fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(guarded_restore_state(&stage, &swap).is_err());
        assert!(!pak_target.exists());
        assert!(!hook_target.exists());
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
