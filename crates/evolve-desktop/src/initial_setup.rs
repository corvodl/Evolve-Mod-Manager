//! First-run signing and launch-helper setup for an installed Evolve Stage 2
//! game. New isolated files only: original game PAKs and inject.dll are never
//! modified by setup. A failed setup stays outside the installed game.
use crate::{launcher_bridge, official_stage2_tools, play_setup};
use evolve_core::{
    client_hook::prepare_client_hook,
    signed_pak::{
        generate_local_signing_keys, inspect_signed_pak, resign_signed_pak_with_original_name,
    },
};
use fs2::available_space;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 512;
const MAX_TREE_ENTRIES: usize = 120_000;
const MIN_FREE_RESERVE: u64 = 1024 * 1024 * 1024;
const HELPERS: [(&str, &str); 5] = [
    (
        "orphaned_swap_recovery.py",
        include_str!("../launcher_support/orphaned_swap_recovery.py"),
    ),
    (
        "controlled_swap.py",
        include_str!("../launcher_support/controlled_swap.py"),
    ),
    (
        "auto_launch_frida.py",
        include_str!("../launcher_support/auto_launch_frida.py"),
    ),
    (
        "auto_restore_watch.py",
        include_str!("../launcher_support/auto_restore_watch.py"),
    ),
    (
        "reusable_restore.py",
        include_str!("../launcher_support/reusable_restore.py"),
    ),
];

#[derive(Clone, Debug)]
pub struct InitialSetup {
    pub stage: PathBuf,
    pub swap: PathBuf,
    pub signed_count: usize,
    pub plain_count: usize,
    pub snapshots: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ArchiveEntry {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    signed_name: Option<String>,
    sha256: String,
    bytes: u64,
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_new(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    output.write_all(data).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())
}

fn json_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write_new(path, &bytes)
}

fn relative_paks(game: &Path) -> Result<Vec<(String, PathBuf, u64)>, String> {
    let mut stack = vec![(game.to_path_buf(), 0usize)];
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mut scanned = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > 20 {
            return Err("Installed game folder nesting exceeds the safety limit".into());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            scanned += 1;
            if scanned > MAX_TREE_ENTRIES {
                return Err("Installed game has too many files for a safe setup inventory".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Symlinked game files prevent a safe original-asset inventory".into());
            }
            if kind.is_dir() {
                stack.push((entry.path(), depth + 1));
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pak"))
            {
                let source_path = entry.path();
                let relative = source_path.strip_prefix(game).map_err(|e| e.to_string())?;
                let virtual_path = relative
                    .to_str()
                    .ok_or("Non-UTF8 PAK path")?
                    .replace('\\', "/");
                if virtual_path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                    || !seen.insert(virtual_path.to_ascii_lowercase())
                {
                    return Err(format!("Unsafe or case-colliding PAK path: {virtual_path}"));
                }
                let bytes = entry.metadata().map_err(|e| e.to_string())?.len();
                if bytes == 0 || bytes > 16 * 1024 * 1024 * 1024 {
                    return Err(format!("Unsupported PAK size: {virtual_path}"));
                }
                rows.push((virtual_path, entry.path(), bytes));
                if rows.len() > MAX_ENTRIES {
                    return Err("Installed game exceeds the supported 512-PAK signing limit".into());
                }
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    if rows.is_empty() {
        return Err("No source PAK archives found beneath the selected EvolveGame folder".into());
    }
    Ok(rows)
}

/// A changed game can no longer pass its old signature preflight. Protect
/// recovery by requiring the prior swap to have an empty, restored journal.
/// Never edit the old baseline, even when the new game differs.
pub fn safe_refresh_preflight(stage: &Path, swap: &Path) -> Result<(), String> {
    launcher_bridge::games_closed()?;
    for dir in [stage, swap] {
        if !fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
            return Err("Existing stage and recovery folders must be real directories".into());
        }
    }
    let journal = swap.join("controlled_swap_state.json");
    let metadata =
        fs::symlink_metadata(&journal).map_err(|e| format!("Missing recovery journal: {e}"))?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return Err("Recovery journal is unsafe or oversized".into());
    }
    let value: serde_json::Value =
        serde_json::from_reader(File::open(journal).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid recovery journal: {e}"))?;
    if value["version"].as_u64() != Some(1)
        || value["phase"].as_str() != Some("restored")
        || value["count"].as_u64() != Some(0)
        || value["files"].as_array().is_none_or(|v| !v.is_empty())
    {
        return Err(
            "Previous offline swap has unresolved changes; restore the original game first".into(),
        );
    }
    let plan = stage.join("rekey_plan.json");
    let metadata = fs::symlink_metadata(&plan)
        .map_err(|e| format!("Missing existing signing inventory: {e}"))?;
    if !metadata.file_type().is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("Signing inventory is unsafe".into());
    }
    let inventory: serde_json::Value =
        serde_json::from_reader(File::open(plan).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid signing inventory: {e}"))?;
    if inventory["source_root"].as_str().is_none() || inventory["entries"].as_array().is_none() {
        return Err("Existing signing inventory is incomplete".into());
    }
    Ok(())
}

/// The private launcher consumes a stage created against the *exact*
/// installed game, plus a separate swap folder with a durable restored
/// journal. This builder creates both using native, verified CryPAK signing.
/// It never starts the client and does not install files into the game.
pub fn create(
    game: &Path,
    client: &Path,
    storage: &Path,
    keep_original_snapshots: bool,
) -> Result<InitialSetup, String> {
    launcher_bridge::games_closed()?;
    let game = play_setup::game_root(&game.to_string_lossy())?;
    let client = if client.as_os_str().is_empty() {
        None
    } else {
        Some(play_setup::client_exe(&client.to_string_lossy())?)
    };
    play_setup::audit_backup_files(&game)?;
    let rows = relative_paks(&game)?;
    let launcher_folder = client.as_ref().and_then(|path| path.parent());
    // Check the resolved destination *before* creating directories, so a
    // mistaken Mod Data Location cannot create setup files in the game/client
    // tree, including when its existing parent is a symlink.
    if !storage.is_absolute()
        || storage
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("Mod Data Location must be an absolute path without .. components".into());
    }
    let mut ancestor = storage;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or("Mod Data Location has no existing parent")?;
    }
    let trailing = storage.strip_prefix(ancestor).map_err(|e| e.to_string())?;
    let proposed = fs::canonicalize(ancestor)
        .map_err(|e| format!("Cannot inspect Mod Data Location's parent: {e}"))?
        .join(trailing);
    if proposed.starts_with(&game)
        || game.starts_with(&proposed)
        || launcher_folder
            .is_some_and(|folder| proposed.starts_with(folder) || folder.starts_with(&proposed))
    {
        return Err("Mod Data Location must be separate from the installed game and client".into());
    }
    if !storage.is_dir() {
        fs::create_dir_all(storage).map_err(|e| format!("Cannot create Game Storage: {e}"))?;
    }
    let storage =
        fs::canonicalize(storage).map_err(|e| format!("Cannot inspect Game Storage: {e}"))?;
    let official = official_stage2_tools::discover(&storage)?;
    // The developer-provided injection shim depends on its matching dbghelp.dll.
    // Only support an already-present original file until recovery is proven
    // for installing a DLL where no original file existed.
    if official.is_some() && !game.join("bin64_SteamRetail/dbghelp.dll").is_file() {
        return Err("Official Stage 2 setup requires an existing original bin64_SteamRetail/dbghelp.dll for reversible swapping. Restore/install the normal game client first.".into());
    }
    let original_shim = game.join("bin64_SteamRetail").join("inject.dll");
    let shim = fs::read(&original_shim).map_err(|e| e.to_string())?;
    if shim.len() != 4096 {
        return Err(
            "Installed inject.dll has an unsupported size; no signing setup created".into(),
        );
    }
    let original_key = &shim[0x870..0x870 + 140];
    // RSA decoding and DER re-encoding are checked by prepare_client_hook.
    // Validate the complete installed key before allocating a large stage.
    {
        use rsa::{
            RsaPublicKey,
            pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey},
            traits::PublicKeyParts,
        };
        let key = RsaPublicKey::from_pkcs1_der(original_key)
            .map_err(|_| "Installed inject.dll has no valid original PKCS#1 RSA key".to_string())?;
        if key.size() != 128
            || key.to_pkcs1_der().map_err(|e| e.to_string())?.as_bytes() != original_key
        {
            return Err("Installed game's RSA key is not the expected canonical format".into());
        }
    }
    let installed_shim_sha = sha256(&original_shim)?;
    let total = rows.iter().try_fold(0u64, |total, (_, _, bytes)| {
        total.checked_add(*bytes).ok_or("Archive size overflow")
    })?;
    let required = total
        .checked_mul(if keep_original_snapshots { 2 } else { 1 })
        .and_then(|size| size.checked_add(MIN_FREE_RESERVE))
        .ok_or("Estimated signing-stage disk requirement overflow")?;
    let free =
        available_space(&storage).map_err(|e| format!("Cannot check free disk space: {e}"))?;
    if free < required {
        return Err(format!(
            "First-run signing setup requires {:.1} GiB free including a 1 GiB reserve; {:.1} GiB available. Disable optional original snapshots or choose another Mod Data Location.",
            required as f64 / 1_073_741_824.0,
            free as f64 / 1_073_741_824.0,
        ));
    }
    // A generated unique directory is retained on failure for explicit
    // inspection. Existing setups, journals and signing keys are never touched.
    let output = tempfile::Builder::new()
        .prefix("Setup_")
        .tempdir_in(&storage)
        .map_err(|e| e.to_string())?
        .keep();
    let stage = output.join("staged");
    let swap = output.join("swap");
    let originals = output.join("originals");
    let result = (|| -> Result<InitialSetup, String> {
        // The auto-generated signing stage is also the profile transaction
        // workspace. Initialize its recovery marker/history now so Build &
        // Prepare Offline Test can journal and restore a profile safely.
        evolve_core::initialize_stage(&stage)
            .map_err(|e| format!("Cannot initialize signed stage recovery: {e}"))?;
        fs::create_dir(&swap).map_err(|e| e.to_string())?;
        if keep_original_snapshots {
            fs::create_dir(&originals).map_err(|e| e.to_string())?;
        }
        let original_public = output.join("RSAKeyData.bin");
        write_new(&original_public, original_key)?;
        let keys = stage.join("mykeys");
        if let Some(tools) = &official {
            fs::create_dir(&keys).map_err(|e| e.to_string())?;
            write_new(&keys.join("public_key.bin"), &fs::read(&tools.public_key).map_err(|e| e.to_string())?)?;
        } else {
            generate_local_signing_keys(&keys).map_err(|e| e.to_string())?;
        }
        let private = keys.join("private_key.pem");
        let public = keys.join("public_key.bin");
        let mut entries = Vec::with_capacity(rows.len());
        let mut signed = 0usize;
        let mut plain = 0usize;
        for (relative, source, bytes) in &rows {
            launcher_bridge::games_closed()?;
            let original_sha = sha256(source)?;
            let authored = if keep_original_snapshots {
                let target = originals.join(relative);
                fs::create_dir_all(target.parent().ok_or("Snapshot has no parent")?)
                    .map_err(|e| e.to_string())?;
                if fs::copy(source, &target).map_err(|e| e.to_string())? != *bytes
                    || sha256(&target)? != original_sha
                {
                    return Err(format!(
                        "Original snapshot changed while copying {relative}"
                    ));
                }
                target
            } else {
                source.clone()
            };
            let basename = Path::new(relative)
                .file_name()
                .and_then(|p| p.to_str())
                .ok_or("Invalid original PAK name")?;
            // Retail Stage 2 CryPAK signatures authenticate their basename
            // bytes; some archives (e.g. mixed-case asset names) are signed
            // against an ASCII-lowercase spelling. Use only fully verified
            // filename variants, just as the Asset Library importer does.
            let lowercase = basename.to_ascii_lowercase();
            let signed_name = if inspect_signed_pak(&authored, &original_public, basename).is_ok() {
                Some(basename.to_owned())
            } else if lowercase != basename
                && inspect_signed_pak(&authored, &original_public, &lowercase).is_ok()
            {
                Some(lowercase)
            } else {
                None
            };
            let (kind, signed_name) = if let Some(signed_name) = signed_name {
                let destination = stage.join("paks").join(relative);
                fs::create_dir_all(destination.parent().ok_or("Staged PAK has no parent")?)
                    .map_err(|e| e.to_string())?;
                if official.is_some() {
                    // Copy only into the isolated stage. A single official 'r'
                    // below converts the stage exactly once, not on each launch.
                    if fs::copy(&authored, &destination).map_err(|e| e.to_string())? != *bytes {
                        return Err(format!("Incomplete copy of {relative} into the official stage"));
                    }
                } else {
                    resign_signed_pak_with_original_name(
                        &authored,
                        &original_public,
                        &private,
                        &public,
                        &signed_name,
                        basename,
                        &destination,
                    )
                    .map_err(|e| format!("Cannot re-sign {relative}: {e}"))?;
                }
                signed += 1;
                ("signed-encrypted", Some(signed_name))
            } else {
                // A bad signature must never be treated as an unsigned
                // archive. The bounded standard ZIP parser is the only
                // alternate supported layout; invalid/corrupt files fail.
                let file = File::open(&authored).map_err(|e| e.to_string())?;
                let archive = zip::ZipArchive::new(file)
                    .map_err(|_| format!("PAK has neither a valid original CryPAK signature (physical/lowercase name) nor a supported plain ZIP layout: {relative}. This may be a different game build, an unsupported archive, or a damaged file."))?;
                if !archive.comment().is_empty() || archive.is_empty() {
                    return Err(format!("Unsupported signed/ZIP PAK trailer: {relative}"));
                }
                plain += 1;
                ("plain-zip", None)
            };
            if sha256(source)? != original_sha {
                return Err(format!("Installed PAK changed while signing: {relative}"));
            }
            entries.push(ArchiveEntry {
                path: relative.clone(),
                kind,
                signed_name,
                sha256: original_sha,
                bytes: *bytes,
            });
        }
        if signed == 0 {
            return Err("No signed Stage 2 PAKs were found in the installed game".into());
        }
        if let Some(tools) = &official {
            tools.rekey_stage(&stage.join("paks"))?;
            // The legacy developer tool can report success after per-file
            // errors. Reauthenticate every converted archive before publishing.
            for row in &entries {
                if row.kind == "signed-encrypted" {
                    let basename = Path::new(&row.path).file_name()
                        .and_then(|name| name.to_str()).ok_or("Bad staged PAK name")?;
                    let signed_name = row.signed_name.as_deref().unwrap_or(basename);
                    inspect_signed_pak(&stage.join("paks").join(&row.path), &public, signed_name)
                        .map_err(|e| format!("Official signing conversion failed for {}: {e}", row.path))?;
                }
            }
        }
        let shim_dst = stage.join("inject-custom.dll");
        let shim_sha = if let Some(tools) = &official {
            write_new(&shim_dst, &fs::read(&tools.inject).map_err(|e| e.to_string())?)?;
            let dbghelp = stage.join("dbghelp-custom.dll");
            write_new(&dbghelp, &fs::read(&tools.dbghelp).map_err(|e| e.to_string())?)?;
            sha256(&shim_dst)?
        } else {
            prepare_client_hook(&original_shim, &original_public, &public, &shim_dst)
                .map_err(|e| format!("Custom offline injector preparation failed: {e}"))?
                .custom_sha256
        };
        json_new(
            &stage.join("rekey_plan.json"),
            &serde_json::json!({
                "source_root": play_setup::ordinary_game_path(&game),
                "blocked": [],
                "entries": entries,
            }),
        )?;
        json_new(
            &stage.join("stage_status.json"),
            &serde_json::json!({
                "status": "fully-staged-offline",
                "count": signed,
                "shim_sha256": shim_sha,
                "signer": if official.is_some() { "official-2025" } else { "local-custom" },
                "dbghelp_sha256": if official.is_some() { Some(sha256(&stage.join("dbghelp-custom.dll"))?) } else { None },
                "original_dbghelp_sha256": if official.is_some() { Some(sha256(&game.join("bin64_SteamRetail/dbghelp.dll"))?) } else { None },
            }),
        )?;
        for (name, source) in HELPERS {
            write_new(&swap.join(name), source.as_bytes())?;
        }
        // Direct signing stages may have no client installed. Client mode
        // writes the verified path when explicitly selected in Settings.
        json_new(
            &swap.join("launcher_config.json"),
            &serde_json::json!({
                "launcher": client.as_ref().map(|path| play_setup::ordinary_game_path(path))
            }),
        )?;
        json_new(
            &swap.join("controlled_swap_state.json"),
            &serde_json::json!({
                "version": 1,
                "phase": "restored",
                "game": game,
                "stage": stage,
                "files": [],
                "count": 0,
            }),
        )?;
        launcher_bridge::games_closed()?;
        if sha256(&original_shim)? != installed_shim_sha {
            return Err("Installed original inject.dll changed during signing".into());
        }
        for row in &entries {
            if sha256(&game.join(&row.path))? != row.sha256 {
                return Err(format!("Installed PAK changed during setup: {}", row.path));
            }
        }
        // End-to-end verification against the launch bridge's schema and
        // original/custom PAK signatures. Do not trust a created manifest.
        let verified = if client.is_some() {
            launcher_bridge::inspect_existing_launcher(&stage, &swap)?
        } else {
            launcher_bridge::inspect_existing_direct_game(&stage, &swap)?
        };
        if verified.archive_count != signed || verified.game != game {
            return Err(
                "Generated setup failed final private-launcher compatibility checks".into(),
            );
        }
        json_new(
            &output.join("refresh_report.json"),
            &serde_json::json!({
                "status": "complete",
                "stage": stage,
                "swap": swap,
                "game": game,
                "launcher": client,
                "initial_setup": true,
                "keep_original_snapshots": keep_original_snapshots,
                "signed_count": signed,
                "plain_count": plain,
            }),
        )?;
        Ok(InitialSetup {
            stage: stage.clone(),
            swap: swap.clone(),
            signed_count: signed,
            plain_count: plain,
            snapshots: keep_original_snapshots,
        })
    })();
    if let Err(error) = &result {
        let report = serde_json::json!({
            "status": "incomplete",
            "error": error,
            "game": game,
            "launcher": client,
            "installed_files_changed": false,
        });
        let _ = json_new(&output.join("setup_incomplete.json"), &report);
    }
    result.map_err(|error| {
        format!(
            "{error}. Incomplete local setup retained at {}",
            output.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn included_private_swap_helpers_are_present() {
        assert_eq!(HELPERS.len(), 5);
        for (name, script) in HELPERS {
            assert!(name.ends_with(".py") && script.len() > 500);
            assert!(script.contains("def "));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn setup_inventory_refuses_symlinks_and_case_collisions() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("FOO.pak"), b"plain").unwrap();
        fs::write(dir.path().join("foo.pak"), b"plain").unwrap();
        assert!(relative_paks(dir.path()).is_err());
    }

    #[test]
    fn setup_inventory_keeps_existing_virtual_paths() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Game").join("UI");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("ui_assets.pak"), b"existing").unwrap();
        let list = relative_paks(dir.path()).unwrap();
        assert_eq!(list[0].0, "Game/UI/ui_assets.pak");
    }
}
