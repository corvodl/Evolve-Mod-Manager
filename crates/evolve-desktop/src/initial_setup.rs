//! First-run signing and launch-helper setup for an installed Evolve Stage 2
//! game. New isolated files only: original game PAKs and inject.dll are never
//! modified by setup. A failed setup stays outside the installed game.
use crate::{launcher_bridge, play_setup};
use evolve_core::{
    client_hook::prepare_client_hook,
    signed_pak::{generate_local_signing_keys, inspect_signed_pak, resign_signed_pak},
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
const HELPERS: [(&str, &str); 4] = [
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
                let relative = entry.path().strip_prefix(game).map_err(|e| e.to_string())?;
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
    let (game, client) =
        play_setup::validate_paths(&game.to_string_lossy(), &client.to_string_lossy())?;
    play_setup::audit_backup_files(&game)?;
    let rows = relative_paks(&game)?;
    let launcher_folder = client.parent().ok_or("Client has no containing folder")?;
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
        || proposed.starts_with(launcher_folder)
        || launcher_folder.starts_with(&proposed)
    {
        return Err("Mod Data Location must be separate from the installed game and client".into());
    }
    if !storage.is_dir() {
        fs::create_dir_all(storage).map_err(|e| format!("Cannot create Game Storage: {e}"))?;
    }
    let storage =
        fs::canonicalize(storage).map_err(|e| format!("Cannot inspect Game Storage: {e}"))?;
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
        fs::create_dir(&stage).map_err(|e| e.to_string())?;
        fs::create_dir(&swap).map_err(|e| e.to_string())?;
        if keep_original_snapshots {
            fs::create_dir(&originals).map_err(|e| e.to_string())?;
        }
        let original_public = output.join("RSAKeyData.bin");
        write_new(&original_public, original_key)?;
        let keys = stage.join("mykeys");
        generate_local_signing_keys(&keys).map_err(|e| e.to_string())?;
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
            let kind = if inspect_signed_pak(&authored, &original_public, basename).is_ok() {
                let destination = stage.join("paks").join(relative);
                fs::create_dir_all(destination.parent().ok_or("Staged PAK has no parent")?)
                    .map_err(|e| e.to_string())?;
                resign_signed_pak(
                    &authored,
                    &original_public,
                    &private,
                    &public,
                    basename,
                    &destination,
                )
                .map_err(|e| format!("Cannot re-sign {relative}: {e}"))?;
                signed += 1;
                "signed-encrypted"
            } else {
                let file = File::open(&authored).map_err(|e| e.to_string())?;
                let archive = zip::ZipArchive::new(file)
                    .map_err(|_| format!("PAK has neither a valid original CryPAK signature nor a supported plain ZIP layout: {relative}"))?;
                if !archive.comment().is_empty() || archive.len() == 0 {
                    return Err(format!("Unsupported signed/ZIP PAK trailer: {relative}"));
                }
                plain += 1;
                "plain-zip"
            };
            if sha256(source)? != original_sha {
                return Err(format!("Installed PAK changed while signing: {relative}"));
            }
            entries.push(ArchiveEntry {
                path: relative.clone(),
                kind,
                sha256: original_sha,
                bytes: *bytes,
            });
        }
        if signed == 0 {
            return Err("No signed Stage 2 PAKs were found in the installed game".into());
        }
        let shim_dst = stage.join("inject-custom.dll");
        let shim_report = prepare_client_hook(&original_shim, &original_public, &public, &shim_dst)
            .map_err(|e| format!("Custom offline injector preparation failed: {e}"))?;
        json_new(
            &stage.join("rekey_plan.json"),
            &serde_json::json!({
                "source_root": game,
                "blocked": [],
                "entries": entries,
            }),
        )?;
        json_new(
            &stage.join("stage_status.json"),
            &serde_json::json!({
                "status": "fully-staged-offline",
                "count": signed,
                "shim_sha256": shim_report.custom_sha256,
            }),
        )?;
        for (name, source) in HELPERS {
            write_new(&swap.join(name), source.as_bytes())?;
        }
        json_new(
            &swap.join("launcher_config.json"),
            &serde_json::json!({"launcher": client}),
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
        let verified = launcher_bridge::inspect_existing_launcher(&stage, &swap)?;
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
        assert_eq!(HELPERS.len(), 4);
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
