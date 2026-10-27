//! Isolated offline PAK staging with a durable one-at-a-time undo journal.
//! This does not replace the production launch/swap integration. It never
//! creates or modifies a target inside the installed game directory: stages
//! must be initialized as NEW empty workspaces owned by this module.
use crate::archive::safe_member_name;
use crate::signed_pak::inspect_signed_pak;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const MARKER: &str = "workspace.json";
const JOURNAL: &str = "active.json";
const HISTORY: &str = ".evolve-history";
const LIMIT: u64 = 16 * 1024 * 1024 * 1024;
/// Opt-in additions stay at or below the rebuilder's single-batch limit.
const MAX_ADDED_MEMBERS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("Choose a NEW staging directory with an existing safe parent")]
    InvalidWorkspace,
    #[error("Stage has a pending swap; restore it before making another change")]
    Pending,
    #[error("No swap journal exists")]
    NoJournal,
    #[error("Unsafe stage target, path or unexpected symlink: {0}")]
    UnsafePath(String),
    #[error("PAK verification failed: {0}")]
    Verification(String),
    #[error("The original or staged file changed; refusing to discard user data")]
    Changed,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Journal serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageTransaction {
    pub schema: u32,
    pub id: String,
    pub target: String,
    pub before_sha256: String,
    pub after_sha256: String,
}

pub(crate) fn ordinary(path: &Path) -> Result<(), StageError> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(StageError::UnsafePath(path.display().to_string()));
    }
    Ok(())
}

pub(crate) fn digest(path: &Path) -> Result<String, StageError> {
    ordinary(path)?;
    if fs::metadata(path)?.len() > LIMIT {
        return Err(StageError::Verification(
            "PAK exceeds staging byte limit".into(),
        ));
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn copy_checked(
    source: &Path,
    destination: &Path,
    expected: &str,
) -> Result<(), StageError> {
    ordinary(source)?;
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = io::copy(&mut (&mut input).take(LIMIT + 1), &mut output)?;
    output.sync_all()?;
    if copied > LIMIT || digest(destination)? != expected {
        return Err(StageError::Changed);
    }
    Ok(())
}

/// Read-only compatibility with the manager's fully-generated first-run
/// signing stages. Older versions created Setup_*/staged with rekey_plan and
/// stage_status files, but did not initialize the standalone legacy marker.
/// Only accept that exact layout, never an arbitrary existing directory or
/// installed game tree. Profile swaps still verify PAK signatures and keep
/// journaled byte-exact backups before modifying any staged files.
fn generated_setup_workspace(root: &Path) -> Result<(), StageError> {
    let bad = || StageError::InvalidWorkspace;
    if root.file_name().and_then(|v| v.to_str()) != Some("staged") {
        return Err(bad());
    }
    let parent = root.parent().ok_or_else(bad)?;
    if !parent
        .file_name()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.starts_with("Setup_"))
    {
        return Err(bad());
    }
    let real_dir = |path: &Path| {
        fs::symlink_metadata(path)
            .is_ok_and(|m| m.file_type().is_dir() && !m.file_type().is_symlink())
    };
    let real_file = |path: &Path, limit: u64| {
        fs::symlink_metadata(path)
            .is_ok_and(|m| m.file_type().is_file() && m.len() > 0 && m.len() <= limit)
    };
    if !real_dir(&root.join("paks"))
        || !real_dir(&root.join("mykeys"))
        || !real_file(&root.join("mykeys/public_key.bin"), 64 * 1024)
        || !real_file(&root.join("mykeys/private_key.pem"), 64 * 1024)
        || !real_file(&root.join("inject-custom.dll"), 1024 * 1024)
        || !fs::symlink_metadata(parent.join("RSAKeyData.bin"))
            .is_ok_and(|m| m.file_type().is_file() && m.len() == 140)
        || !real_file(&root.join("rekey_plan.json"), 8 * 1024 * 1024)
        || !real_file(&root.join("stage_status.json"), 1024 * 1024)
    {
        return Err(bad());
    }
    let plan: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("rekey_plan.json")).map_err(|_| bad())?)
            .map_err(|_| bad())?;
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("stage_status.json")).map_err(|_| bad())?)
            .map_err(|_| bad())?;
    let source = plan["source_root"].as_str().ok_or_else(bad)?;
    let game = fs::canonicalize(source).map_err(|_| bad())?;
    if !game.is_dir()
        || root.starts_with(&game)
        || game.starts_with(root)
        || status["status"].as_str() != Some("fully-staged-offline")
    {
        return Err(bad());
    }
    let rows = plan["entries"].as_array().ok_or_else(bad)?;
    let signed = rows
        .iter()
        .filter(|v| v["kind"].as_str() == Some("signed-encrypted"))
        .count();
    if signed == 0
        || signed > 4096
        || rows.len() > 4096
        || status["count"].as_u64() != Some(signed as u64)
    {
        return Err(bad());
    }
    // Existing modern stages did not have the legacy history directory.
    // Profile batch swaps use their own .evolve-batches recovery journal.
    if let Ok(meta) = fs::symlink_metadata(root.join(HISTORY))
        && (!meta.file_type().is_dir() || meta.file_type().is_symlink())
    {
        return Err(bad());
    }
    Ok(())
}

pub(crate) fn workspace(root: &Path) -> Result<PathBuf, StageError> {
    let kind = fs::symlink_metadata(root)
        .map_err(|_| StageError::InvalidWorkspace)?
        .file_type();
    if !kind.is_dir() || kind.is_symlink() {
        return Err(StageError::InvalidWorkspace);
    }
    let root = fs::canonicalize(root)?;
    let marker = root.join(MARKER);
    if fs::symlink_metadata(&marker).is_ok() {
        ordinary(&marker).map_err(|_| StageError::InvalidWorkspace)?;
        if fs::metadata(&marker)?.len() > 1024
            || fs::read_to_string(&marker)?
                != "{\"schema\":1,\"kind\":\"isolated-offline-pak-stage\"}\n"
        {
            return Err(StageError::InvalidWorkspace);
        }
        for folder in ["paks", HISTORY] {
            let ty = fs::symlink_metadata(root.join(folder))?.file_type();
            if !ty.is_dir() || ty.is_symlink() {
                return Err(StageError::InvalidWorkspace);
            }
        }
    } else {
        generated_setup_workspace(&root)?;
    }
    Ok(root)
}

pub(crate) fn target_name(raw: &str) -> Result<String, StageError> {
    let clean = safe_member_name(raw).map_err(|e| StageError::UnsafePath(e.to_string()))?;
    if !clean.to_ascii_lowercase().ends_with(".pak") {
        return Err(StageError::UnsafePath(clean));
    }
    Ok(clean)
}

pub(crate) fn stage_target(
    root: &Path,
    name: &str,
    create_parent: bool,
) -> Result<PathBuf, StageError> {
    let mut path = root.join("paks");
    let pieces = name.split('/').collect::<Vec<_>>();
    for component in &pieces[..pieces.len() - 1] {
        // Do not silently follow junctions/symlinks inside a stage.
        // Also reject case-insensitive aliases on case-sensitive hosts.
        let entries = fs::read_dir(&path)?;
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(component)
                && entry.file_name() != *component
            {
                return Err(StageError::UnsafePath(name.into()));
            }
        }
        path.push(component);
        if create_parent && !path.exists() {
            fs::create_dir(&path)?;
        }
        if !fs::symlink_metadata(&path)?.file_type().is_dir() {
            return Err(StageError::UnsafePath(name.into()));
        }
    }
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(pieces[pieces.len() - 1])
            && entry.file_name() != pieces[pieces.len() - 1]
        {
            return Err(StageError::UnsafePath(name.into()));
        }
    }
    Ok(path.join(pieces[pieces.len() - 1]))
}

fn verify(path: &Path, public_key: &Path, basename: &str) -> Result<HashSet<String>, StageError> {
    Ok(member_names(path, public_key, basename)?
        .into_iter()
        .map(|name| name.to_lowercase())
        .collect())
}

/// Authenticated member paths exactly as the signed directory spells them.
fn member_names(path: &Path, public_key: &Path, basename: &str) -> Result<Vec<String>, StageError> {
    let report = inspect_signed_pak(path, public_key, basename)
        .map_err(|e| StageError::Verification(e.to_string()))?;
    Ok(report.entries.into_iter().map(|e| e.path).collect())
}

/// Member-list policy shared by strict replacement and opt-in additions.
///
/// Strict mode keeps the original exact member-list rule. Additions mode
/// requires every original member to survive with identical spelling, rejects
/// case-insensitive aliases or collisions, and permits 1..=64 new members.
fn plan_member_changes(
    original: &[String],
    replacement: &[String],
    allow_additions: bool,
) -> Result<Vec<String>, StageError> {
    let mut old_lower = HashMap::with_capacity(original.len());
    for name in original {
        old_lower.insert(name.to_lowercase(), name.as_str());
    }
    let new_lower = replacement
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<HashSet<_>>();
    if !old_lower.keys().all(|name| new_lower.contains(name)) {
        return Err(StageError::Verification(if allow_additions {
            "Replacement removed a signed PAK member".into()
        } else {
            "PAK member list changed".into()
        }));
    }
    if !allow_additions {
        if old_lower.len() != new_lower.len() {
            return Err(StageError::Verification("PAK member list changed".into()));
        }
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    let mut added = Vec::new();
    for name in replacement {
        let lower = name.to_lowercase();
        // Signed archives reject internal aliases, but never accept a
        // case-insensitive duplicate or a case-only rename from any source.
        if !seen.insert(lower.clone()) {
            return Err(StageError::UnsafePath(format!(
                "Case-insensitive PAK member collision: {name}"
            )));
        }
        match old_lower.get(&lower) {
            Some(existing) if *existing != name.as_str() => {
                return Err(StageError::UnsafePath(format!(
                    "Case-insensitive PAK member collision: {name}"
                )));
            }
            Some(_) => {}
            None => added.push(name.clone()),
        }
    }
    if added.is_empty() {
        return Err(StageError::Verification(
            "Replacement adds no signed PAK members".into(),
        ));
    }
    if added.len() > MAX_ADDED_MEMBERS {
        return Err(StageError::Verification(format!(
            "Replacement adds more than {MAX_ADDED_MEMBERS} signed PAK members"
        )));
    }
    Ok(added)
}

fn active(root: &Path) -> Result<Option<StageTransaction>, StageError> {
    // Single-file transactions cannot overlap with a batch profile swap.
    if fs::symlink_metadata(root.join("bundle-active.json")).is_ok() {
        return Err(StageError::Pending);
    }
    let path = root.join(JOURNAL);
    if fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    ordinary(&path)?;
    if fs::metadata(&path)?.len() > 16384 {
        return Err(StageError::Changed);
    }
    let tx: StageTransaction = serde_json::from_slice(&fs::read(path)?)?;
    target_name(&tx.target)?;
    if tx.schema != 1
        || tx.id.len() != 32
        || !tx.id.bytes().all(|b| b.is_ascii_hexdigit())
        || tx.before_sha256.len() != 64
        || tx.after_sha256.len() != 64
        || !tx.before_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || !tx.after_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(StageError::Changed);
    }
    Ok(Some(tx))
}

fn write_journal(root: &Path, tx: &StageTransaction) -> Result<(), StageError> {
    let data = serde_json::to_vec_pretty(tx)?;
    let mut temp = NamedTempFile::new_in(root)?;
    temp.write_all(&data)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(root.join(JOURNAL))
        .map_err(|e| StageError::Io(e.error))?;
    Ok(())
}

/// Initialize a fresh isolated stage; an existing folder (including a game
/// installation) is never accepted.
pub fn initialize_stage(root: &Path) -> Result<(), StageError> {
    if fs::symlink_metadata(root).is_ok() {
        return Err(StageError::InvalidWorkspace);
    }
    let parent = root.parent().ok_or(StageError::InvalidWorkspace)?;
    if !fs::symlink_metadata(parent)?.file_type().is_dir() {
        return Err(StageError::InvalidWorkspace);
    }
    fs::create_dir(root)?;
    fs::create_dir(root.join("paks"))?;
    fs::create_dir(root.join(HISTORY))?;
    fs::write(
        root.join(MARKER),
        b"{\"schema\":1,\"kind\":\"isolated-offline-pak-stage\"}\n",
    )?;
    Ok(())
}

/// Import a verified signed PAK into an isolated stage, without replacing an
/// existing entry or changing the input. The caller supplies their own key.
pub fn import_stage_pak(
    root: &Path,
    original: &Path,
    relative: &str,
    public_key: &Path,
    basename: &str,
) -> Result<String, StageError> {
    let root = workspace(root)?;
    if active(&root)?.is_some() {
        return Err(StageError::Pending);
    }
    let name = target_name(relative)?;
    ordinary(original)?;
    if original.file_name().and_then(|s| s.to_str())
        != Some(name.rsplit('/').next().unwrap_or_default())
    {
        return Err(StageError::UnsafePath(
            "PAK filename must match target".into(),
        ));
    }
    if fs::canonicalize(original)?.starts_with(&root) {
        return Err(StageError::UnsafePath(
            "Import source is inside workspace".into(),
        ));
    }
    verify(original, public_key, basename)?;
    let expected = digest(original)?;
    let destination = stage_target(&root, &name, true)?;
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(StageError::UnsafePath("PAK target already exists".into()));
    }
    let copied = copy_checked(original, &destination, &expected);
    if copied.is_err() {
        let _ = fs::remove_file(&destination);
        return copied.map(|_| expected);
    }
    Ok(expected)
}

/// Apply ONE verified mod PAK to an isolated stage, keeping the original exact
/// member-list rule. Backup and replacement bytes are durable before
/// publishing the journal. The journal is saved before moving the original,
/// making an interrupted swap recoverable.
pub fn apply_stage_pak(
    root: &Path,
    replacement: &Path,
    relative: &str,
    original_key: &Path,
    replacement_key: &Path,
    basename: &str,
) -> Result<StageTransaction, StageError> {
    apply_stage_pak_mode(
        root,
        replacement,
        relative,
        original_key,
        replacement_key,
        basename,
        false,
    )
}

/// Explicit opt-in addition of signed PAK members to an isolated stage.
///
/// Every original member must survive with exactly the same spelling, and the
/// replacement may add 1..=64 new members. Removals, case-only renames and
/// case-insensitive collisions are rejected. The source must stay outside the
/// workspace; byte-exact backups and the same recovery journal/rollback path
/// as [`apply_stage_pak`] are used, and nothing in an installed game is read or
/// written.
pub fn apply_stage_pak_with_additions(
    root: &Path,
    replacement: &Path,
    relative: &str,
    original_key: &Path,
    replacement_key: &Path,
    basename: &str,
) -> Result<StageTransaction, StageError> {
    apply_stage_pak_mode(
        root,
        replacement,
        relative,
        original_key,
        replacement_key,
        basename,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_stage_pak_mode(
    root: &Path,
    replacement: &Path,
    relative: &str,
    original_key: &Path,
    replacement_key: &Path,
    basename: &str,
    allow_additions: bool,
) -> Result<StageTransaction, StageError> {
    let root = workspace(root)?;
    if active(&root)?.is_some() {
        return Err(StageError::Pending);
    }
    let name = target_name(relative)?;
    ordinary(replacement)?;
    if fs::canonicalize(replacement)?.starts_with(&root) {
        return Err(StageError::UnsafePath(
            "Replacement is inside workspace".into(),
        ));
    }
    if replacement.file_name().and_then(|s| s.to_str())
        != Some(name.rsplit('/').next().unwrap_or_default())
    {
        return Err(StageError::UnsafePath(
            "PAK filename must match target".into(),
        ));
    }
    let target = stage_target(&root, &name, false)?;
    ordinary(&target)?;
    let old_members = member_names(&target, original_key, basename)?;
    let new_members = member_names(replacement, replacement_key, basename)?;
    plan_member_changes(&old_members, &new_members, allow_additions)?;
    let before = digest(&target)?;
    let after = digest(replacement)?;
    if before == after {
        return Err(StageError::Verification(
            "Replacement is identical to original".into(),
        ));
    }
    let id = format!("{:032x}", rand::random::<u128>());
    let history = root.join(HISTORY).join(&id);
    fs::create_dir(&history)?;
    copy_checked(&target, &history.join("before.pak"), &before)?;
    copy_checked(replacement, &history.join("after.pak"), &after)?;
    let tx = StageTransaction {
        schema: 1,
        id,
        target: name,
        before_sha256: before,
        after_sha256: after,
    };
    // All preflight, backup and hash checks finish before the first target edit.
    write_journal(&root, &tx)?;
    fs::rename(&target, history.join("displaced.pak"))?;
    copy_checked(
        &history.join("after.pak"),
        &history.join("incoming.pak"),
        &tx.after_sha256,
    )?;
    fs::rename(history.join("incoming.pak"), &target)?;
    if digest(&target)? != tx.after_sha256 {
        return Err(StageError::Changed);
    }
    Ok(tx)
}

/// Read status without modifying any staged PAKs.
pub fn stage_status(root: &Path) -> Result<Option<StageTransaction>, StageError> {
    active(&workspace(root)?)
}

/// Restore verified original bytes even if the previous process exited after
/// moving the old file but before replacing it. Reject third-party edits.
pub fn restore_stage(root: &Path) -> Result<StageTransaction, StageError> {
    let root = workspace(root)?;
    let tx = active(&root)?.ok_or(StageError::NoJournal)?;
    let history = root.join(HISTORY).join(&tx.id);
    let backup = history.join("before.pak");
    if digest(&backup)? != tx.before_sha256 {
        return Err(StageError::Changed);
    }
    let target = stage_target(&root, &tx.target, false)?;
    if fs::symlink_metadata(&target).is_ok() {
        let current = digest(&target)?;
        if current != tx.before_sha256 && current != tx.after_sha256 {
            return Err(StageError::Changed);
        }
        if current == tx.before_sha256 {
            fs::remove_file(root.join(JOURNAL))?;
            return Ok(tx);
        }
    } else {
        // If missing, a prior atomic move must hold the original. Otherwise
        // this could be arbitrary deletion, which is not safe to silently fix.
        let displaced = history.join("displaced.pak");
        if !displaced.is_file() || digest(&displaced)? != tx.before_sha256 {
            return Err(StageError::Changed);
        }
    }
    let pending = history.join("restoring.pak");
    if pending.exists() {
        if digest(&pending)? != tx.before_sha256 {
            return Err(StageError::Changed);
        }
    } else {
        copy_checked(&backup, &pending, &tx.before_sha256)?;
    }
    if target.exists() {
        fs::rename(&target, history.join("modified.pak"))?;
    }
    fs::rename(&pending, &target)?;
    if digest(&target)? != tx.before_sha256 {
        return Err(StageError::Changed);
    }
    fs::remove_file(root.join(JOURNAL))?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_only_and_reject_fake_signed_archive() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("stage");
        initialize_stage(&root).unwrap();
        assert!(initialize_stage(&root).is_err());
        assert!(stage_status(&root).unwrap().is_none());
        let fake = temp.path().join("test.pak");
        fs::write(&fake, b"not signed").unwrap();
        let key = temp.path().join("key.der");
        fs::write(&key, b"no key").unwrap();
        assert!(import_stage_pak(&root, &fake, "Game/test.pak", &key, "test.pak").is_err());
        assert!(!root.join("paks/Game/test.pak").exists());
        assert!(target_name("../bad.pak").is_err());
        assert!(target_name("test.dll").is_err());
    }

    #[test]
    fn recover_completed_or_interrupted_swap_and_refuse_tamper() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("stage");
        initialize_stage(&root).unwrap();
        let destination = root.join("paks/test.pak");
        let history = root.join(HISTORY).join("00112233445566778899aabbccddeeff");
        fs::create_dir(&history).unwrap();
        fs::write(&destination, b"new").unwrap();
        fs::write(history.join("before.pak"), b"old").unwrap();
        fs::write(history.join("displaced.pak"), b"old").unwrap();
        let tx = StageTransaction {
            schema: 1,
            id: "00112233445566778899aabbccddeeff".into(),
            target: "test.pak".into(),
            before_sha256: digest(&history.join("before.pak")).unwrap(),
            after_sha256: digest(&destination).unwrap(),
        };
        write_journal(&root, &tx).unwrap();
        fs::write(&destination, b"unexpected").unwrap();
        assert!(matches!(restore_stage(&root), Err(StageError::Changed)));
        fs::write(&destination, b"new").unwrap();
        restore_stage(&root).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert!(stage_status(&root).unwrap().is_none());

        fs::remove_file(&destination).unwrap();
        write_journal(&root, &tx).unwrap();
        restore_stage(&root).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    fn generated_first_run_stage_supports_profile_metadata_without_recreation() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("game");
        fs::create_dir(&game).unwrap();
        let setup = temp.path().join("Setup_existing");
        let stage = setup.join("staged");
        fs::create_dir_all(stage.join("paks")).unwrap();
        fs::create_dir(stage.join("mykeys")).unwrap();
        fs::write(setup.join("RSAKeyData.bin"), [0u8; 140]).unwrap();
        fs::write(stage.join("mykeys/public_key.bin"), b"key").unwrap();
        fs::write(stage.join("mykeys/private_key.pem"), b"private").unwrap();
        fs::write(stage.join("inject-custom.dll"), b"hook").unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::json!({
                "source_root": game,
                "entries": [{"path": "Game/UI_Assets.pak", "kind": "signed-encrypted"}]
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            stage.join("stage_status.json"),
            serde_json::json!({"status": "fully-staged-offline", "count": 1}).to_string(),
        )
        .unwrap();
        assert!(stage_status(&stage).unwrap().is_none());
        assert!(crate::batch::batch_stage_status(&stage).unwrap().is_none());
        let profile = crate::profiles::create_mod_profile(&stage, "Main menu offline test")
            .expect("An existing generated stage can store new profiles");
        assert_eq!(
            crate::profiles::list_mod_profiles(&stage).unwrap().profiles[0].id,
            profile.id
        );
        assert!(!stage.join(MARKER).exists());

        // A forged marker never falls back to the compatibility layout.
        fs::write(stage.join(MARKER), b"invalid").unwrap();
        assert!(workspace(&stage).is_err());
        fs::remove_file(stage.join(MARKER)).unwrap();

        // A matching-looking folder inside the game installation is unsafe.
        let other_game = setup.parent().unwrap();
        fs::write(
            stage.join("rekey_plan.json"),
            serde_json::json!({
                "source_root": other_game,
                "entries": [{"path": "Game/UI_Assets.pak", "kind": "signed-encrypted"}]
            })
            .to_string(),
        )
        .unwrap();
        assert!(workspace(&stage).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_targets() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("stage");
        initialize_stage(&root).unwrap();
        symlink(temp.path(), root.join("paks/linked")).unwrap();
        assert!(stage_target(&root, "linked/trap.pak", true).is_err());
    }

    #[test]
    fn additions_policy_requires_a_safe_nonempty_superset() {
        let base = vec!["Game/a.caf".to_string(), "Game/b.caf".to_string()];
        assert!(plan_member_changes(&base, &base, false).unwrap().is_empty());
        assert!(
            plan_member_changes(&base, &base, true).is_err(),
            "an additions swap must actually add a member"
        );
        let mut with_add = base.clone();
        with_add.push("Game/new.dds".into());
        assert!(
            plan_member_changes(&base, &with_add, false).is_err(),
            "strict application keeps the exact member-list rule"
        );
        assert_eq!(
            plan_member_changes(&base, &with_add, true).unwrap(),
            vec!["Game/new.dds".to_string()]
        );
        assert!(
            plan_member_changes(&base, &base[..1], true).is_err(),
            "removing a member is never an addition"
        );
        for (label, replacement) in [
            (
                "case-only rename of an existing member",
                vec![
                    "GAME/A.CAF".to_string(),
                    "Game/b.caf".to_string(),
                    "Game/new.dds".to_string(),
                ],
            ),
            (
                "case-insensitive addition collision",
                vec![
                    "Game/a.caf".to_string(),
                    "Game/b.caf".to_string(),
                    "Game/new.dds".to_string(),
                    "GAME/NEW.DDS".to_string(),
                ],
            ),
        ] {
            assert!(
                matches!(
                    plan_member_changes(&base, &replacement, true),
                    Err(StageError::UnsafePath(_))
                ),
                "{label} must be rejected"
            );
        }
        let mut maximum = base.clone();
        for index in 0..MAX_ADDED_MEMBERS {
            maximum.push(format!("Game/new{index}.dds"));
        }
        assert_eq!(
            plan_member_changes(&base, &maximum, true).unwrap().len(),
            MAX_ADDED_MEMBERS
        );
        maximum.push("Game/one-too-many.dds".into());
        assert!(
            matches!(
                plan_member_changes(&base, &maximum, true),
                Err(StageError::Verification(_))
            ),
            "the 65th addition must be rejected"
        );
    }

    const SYNTHETIC_TRAILER: usize = 2320;
    const SYNTHETIC_RSA: usize = 128;

    fn synthetic_mgf1(seed: &[u8], length: usize) -> Vec<u8> {
        let mut output = Vec::with_capacity(length);
        let mut counter = 0u32;
        while output.len() < length {
            let mut hash = Sha256::new();
            hash.update(seed);
            hash.update(counter.to_be_bytes());
            output.extend_from_slice(&hash.finalize());
            counter += 1;
        }
        output.truncate(length);
        output
    }

    /// Synthetic OAEP-shaped public-key unwrap used only to build local
    /// fixtures; this is not game key material.
    fn synthetic_wrap(secret: &[u8; 16], private: &rsa::RsaPrivateKey) -> [u8; SYNTHETIC_RSA] {
        use rsa::BigUint;
        use rsa::traits::{PrivateKeyParts, PublicKeyParts};
        let mut db = [0u8; 95];
        db[..32].copy_from_slice(&Sha256::digest([]));
        db[78] = 1;
        db[79..].copy_from_slice(secret);
        let mut seed = [7u8; 32];
        let db_mask = synthetic_mgf1(&seed, db.len());
        for (value, mask) in db.iter_mut().zip(db_mask) {
            *value ^= mask;
        }
        let seed_mask = synthetic_mgf1(&db, 32);
        for (value, mask) in seed.iter_mut().zip(seed_mask) {
            *value ^= mask;
        }
        let mut encoded = [0u8; SYNTHETIC_RSA];
        encoded[1..33].copy_from_slice(&seed);
        encoded[33..].copy_from_slice(&db);
        let integer = BigUint::from_bytes_be(&encoded).modpow(private.d(), private.n());
        let bytes = integer.to_bytes_be();
        let mut wrapped = [0u8; SYNTHETIC_RSA];
        wrapped[SYNTHETIC_RSA - bytes.len()..].copy_from_slice(&bytes);
        wrapped
    }

    fn synthetic_ctr(data: &[u8], key: &[u8; 16], iv: [u8; 16]) -> Vec<u8> {
        use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
        use twofish::Twofish;
        let cipher = Twofish::new_from_slice(key).unwrap();
        let mut counter = u128::from_le_bytes(iv);
        let mut output = data.to_vec();
        for chunk in output.chunks_mut(16) {
            let mut block = GenericArray::clone_from_slice(&counter.to_le_bytes());
            cipher.encrypt_block(&mut block);
            for (byte, mask) in chunk.iter_mut().zip(block.iter()) {
                *byte ^= mask;
            }
            counter = counter.wrapping_add(1);
        }
        output
    }

    fn synthetic_entry_iv(crc: u32, compressed: u32, size: u32) -> [u8; 16] {
        let values = [
            size ^ compressed.wrapping_shl(12),
            u32::from(compressed == 0),
            crc ^ compressed.wrapping_shl(12),
            u32::from(size == 0) ^ compressed,
        ];
        let mut bytes = [0u8; 16];
        for (index, value) in values.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// Build a minimal authentic signed PAK (stored method 13) so staging can
    /// be exercised end to end without any installed game files.
    fn synthetic_signed_pak(
        basename: &str,
        members: &[(&str, &[u8])],
        private: &rsa::RsaPrivateKey,
    ) -> Vec<u8> {
        let keys = [[4u8; 16]; 16];
        let iv = [9u8; 16];
        let mut bytes = Vec::new();
        let mut directory = Vec::new();
        for (name, payload) in members {
            let offset = bytes.len() as u32;
            let crc = crc32fast::hash(payload);
            let size = u32::try_from(payload.len()).unwrap();
            let compressed = size;
            let mut local = [0u8; 30];
            local[..4].copy_from_slice(b"PK\x03\x04");
            local[8..10].copy_from_slice(&13u16.to_le_bytes());
            local[14..18].copy_from_slice(&crc.to_le_bytes());
            local[18..22].copy_from_slice(&compressed.to_le_bytes());
            local[22..26].copy_from_slice(&size.to_le_bytes());
            local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&local);
            bytes.extend_from_slice(name.as_bytes());
            let slot = (!(crc >> 2) & 15) as usize;
            bytes.extend_from_slice(&synthetic_ctr(
                payload,
                &keys[slot],
                synthetic_entry_iv(crc, compressed, size),
            ));
            let mut record = [0u8; 46];
            record[..4].copy_from_slice(b"PK\x01\x02");
            record[10..12].copy_from_slice(&13u16.to_le_bytes());
            record[16..20].copy_from_slice(&crc.to_le_bytes());
            record[20..24].copy_from_slice(&compressed.to_le_bytes());
            record[24..28].copy_from_slice(&size.to_le_bytes());
            record[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            record[42..46].copy_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(&record);
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_at = bytes.len() as u32;
        bytes.extend_from_slice(&synthetic_ctr(&directory, &keys[0], iv));
        let mut message = directory.clone();
        message.extend_from_slice(basename.as_bytes());
        let signature = private
            .sign_with_rng(
                &mut rand::rngs::OsRng,
                rsa::Pss::new_with_salt::<Sha256>(0),
                &Sha256::digest(&message),
            )
            .unwrap();
        let mut comment = [0u8; SYNTHETIC_TRAILER];
        comment[..6].copy_from_slice(b"\x06\x00\x00\x00\x01\x03");
        comment[6..10].copy_from_slice(&133u32.to_le_bytes());
        comment[11..139].copy_from_slice(&signature);
        comment[139..143].copy_from_slice(&2181u32.to_le_bytes());
        comment[143..271].copy_from_slice(&synthetic_wrap(&iv, private));
        for (index, key) in keys.iter().enumerate() {
            let at = 272 + index * SYNTHETIC_RSA;
            comment[at..at + SYNTHETIC_RSA].copy_from_slice(&synthetic_wrap(key, private));
        }
        let mut eocd = [0u8; 22];
        eocd[..4].copy_from_slice(b"PK\x05\x06");
        let count = u16::try_from(members.len()).unwrap();
        eocd[8..10].copy_from_slice(&count.to_le_bytes());
        eocd[10..12].copy_from_slice(&count.to_le_bytes());
        eocd[12..16].copy_from_slice(&(directory.len() as u32).to_le_bytes());
        eocd[16..20].copy_from_slice(&directory_at.to_le_bytes());
        eocd[20..22].copy_from_slice(&(SYNTHETIC_TRAILER as u16).to_le_bytes());
        bytes.extend_from_slice(&eocd);
        bytes.extend_from_slice(&comment);
        bytes
    }

    #[test]
    fn additions_apply_requires_opt_in_and_retains_rollback() {
        use rsa::pkcs1::EncodeRsaPublicKey;
        let temp = tempfile::tempdir().unwrap();
        let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 1024).unwrap();
        let public = rsa::RsaPublicKey::from(&private);
        let key = temp.path().join("public.der");
        fs::write(&key, public.to_pkcs1_der().unwrap().as_bytes()).unwrap();
        let root = temp.path().join("stage");
        initialize_stage(&root).unwrap();

        let original_members: [(&str, &[u8]); 2] = [
            ("Game/base.caf", b"base original"),
            ("Game/other.caf", b"other original"),
        ];
        let base = synthetic_signed_pak("sample.pak", &original_members, &private);
        let original_dir = temp.path().join("original");
        fs::create_dir(&original_dir).unwrap();
        let original = original_dir.join("sample.pak");
        fs::write(&original, &base).unwrap();
        import_stage_pak(&root, &original, "Game/sample.pak", &key, "sample.pak").unwrap();
        let staged = root.join("paks/Game/sample.pak");
        assert_eq!(fs::read(&staged).unwrap(), base);

        let fixture = |label: &str, members: &[(&str, &[u8])]| {
            let folder = temp.path().join(label);
            fs::create_dir(&folder).unwrap();
            let path = folder.join("sample.pak");
            let bytes = synthetic_signed_pak("sample.pak", members, &private);
            fs::write(&path, &bytes).unwrap();
            assert!(inspect_signed_pak(&path, &key, "sample.pak").is_ok());
            path
        };
        let added_members: [(&str, &[u8]); 3] = [
            ("Game/base.caf", b"base changed"),
            ("Game/other.caf", b"other original"),
            ("Game/added.dds", b"added payload"),
        ];
        let replacement = fixture("added", &added_members);
        let replacement_bytes = fs::read(&replacement).unwrap();

        assert!(
            matches!(
                apply_stage_pak(
                    &root,
                    &replacement,
                    "Game/sample.pak",
                    &key,
                    &key,
                    "sample.pak"
                ),
                Err(StageError::Verification(_))
            ),
            "the default path must keep the exact member-list rule"
        );
        assert_eq!(fs::read(&staged).unwrap(), base);
        assert!(stage_status(&root).unwrap().is_none());

        let removal_members: [(&str, &[u8]); 2] = [
            ("Game/base.caf", b"base changed"),
            ("Game/added.dds", b"added payload"),
        ];
        let removal = fixture("removal", &removal_members);
        assert!(
            apply_stage_pak_with_additions(
                &root,
                &removal,
                "Game/sample.pak",
                &key,
                &key,
                "sample.pak"
            )
            .is_err()
        );
        assert_eq!(fs::read(&staged).unwrap(), base);
        assert!(stage_status(&root).unwrap().is_none());

        let unchanged_members: [(&str, &[u8]); 2] = [
            ("Game/base.caf", b"base changed"),
            ("Game/other.caf", b"other original"),
        ];
        let unchanged = fixture("unchanged", &unchanged_members);
        assert!(
            apply_stage_pak_with_additions(
                &root,
                &unchanged,
                "Game/sample.pak",
                &key,
                &key,
                "sample.pak"
            )
            .is_err()
        );
        assert_eq!(fs::read(&staged).unwrap(), base);
        assert!(stage_status(&root).unwrap().is_none());

        let renamed_members: [(&str, &[u8]); 3] = [
            ("GAME/BASE.CAF", b"base changed"),
            ("Game/other.caf", b"other original"),
            ("Game/added.dds", b"added payload"),
        ];
        let renamed = fixture("renamed", &renamed_members);
        assert!(matches!(
            apply_stage_pak_with_additions(
                &root,
                &renamed,
                "Game/sample.pak",
                &key,
                &key,
                "sample.pak"
            ),
            Err(StageError::UnsafePath(_))
        ));
        assert_eq!(fs::read(&staged).unwrap(), base);
        assert!(stage_status(&root).unwrap().is_none());

        let tx = apply_stage_pak_with_additions(
            &root,
            &replacement,
            "Game/sample.pak",
            &key,
            &key,
            "sample.pak",
        )
        .unwrap();
        assert_eq!(tx.target, "Game/sample.pak");
        assert_eq!(tx.before_sha256, digest(&original).unwrap());
        assert_eq!(tx.after_sha256, digest(&replacement).unwrap());
        assert_eq!(fs::read(&staged).unwrap(), replacement_bytes);
        assert_eq!(
            inspect_signed_pak(&staged, &key, "sample.pak")
                .unwrap()
                .entries
                .len(),
            3
        );
        assert_eq!(stage_status(&root).unwrap().unwrap().id, tx.id);
        assert_eq!(
            fs::read(root.join(HISTORY).join(&tx.id).join("before.pak")).unwrap(),
            base
        );
        // The existing journal/rollback path restores the pre-addition archive.
        restore_stage(&root).unwrap();
        assert_eq!(fs::read(&staged).unwrap(), base);
        assert!(stage_status(&root).unwrap().is_none());
    }
}
