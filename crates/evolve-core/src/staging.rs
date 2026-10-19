//! Isolated offline PAK staging with a durable one-at-a-time undo journal.
//! This does not replace the production launch/swap integration. It never
//! creates or modifies a target inside the installed game directory: stages
//! must be initialized as NEW empty workspaces owned by this module.
use crate::archive::safe_member_name;
use crate::signed_pak::inspect_signed_pak;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const MARKER: &str = "workspace.json";
const JOURNAL: &str = "active.json";
const HISTORY: &str = ".evolve-history";
const LIMIT: u64 = 16 * 1024 * 1024 * 1024;

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

fn ordinary(path: &Path) -> Result<(), StageError> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(StageError::UnsafePath(path.display().to_string()));
    }
    Ok(())
}

fn digest(path: &Path) -> Result<String, StageError> {
    ordinary(path)?;
    if fs::metadata(path)?.len() > LIMIT {
        return Err(StageError::Verification("PAK exceeds staging byte limit".into()));
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

fn copy_checked(source: &Path, destination: &Path, expected: &str) -> Result<(), StageError> {
    ordinary(source)?;
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(destination)?;
    let copied = io::copy(&mut (&mut input).take(LIMIT + 1), &mut output)?;
    output.sync_all()?;
    if copied > LIMIT || digest(destination)? != expected {
        return Err(StageError::Changed);
    }
    Ok(())
}

fn workspace(root: &Path) -> Result<PathBuf, StageError> {
    let kind = fs::symlink_metadata(root)
        .map_err(|_| StageError::InvalidWorkspace)?
        .file_type();
    if !kind.is_dir() || kind.is_symlink() {
        return Err(StageError::InvalidWorkspace);
    }
    let root = fs::canonicalize(root)?;
    let marker = root.join(MARKER);
    ordinary(&marker).map_err(|_| StageError::InvalidWorkspace)?;
    if fs::metadata(&marker)?.len() > 1024
        || fs::read_to_string(&marker)? != "{\"schema\":1,\"kind\":\"isolated-offline-pak-stage\"}\n"
    {
        return Err(StageError::InvalidWorkspace);
    }
    for folder in ["paks", HISTORY] {
        let ty = fs::symlink_metadata(root.join(folder))?.file_type();
        if !ty.is_dir() || ty.is_symlink() {
            return Err(StageError::InvalidWorkspace);
        }
    }
    Ok(root)
}

fn target_name(raw: &str) -> Result<String, StageError> {
    let clean = safe_member_name(raw).map_err(|e| StageError::UnsafePath(e.to_string()))?;
    if !clean.to_ascii_lowercase().ends_with(".pak") {
        return Err(StageError::UnsafePath(clean));
    }
    Ok(clean)
}

fn stage_target(root: &Path, name: &str, create_parent: bool) -> Result<PathBuf, StageError> {
    let mut path = root.join("paks");
    let pieces = name.split('/').collect::<Vec<_>>();
    for component in &pieces[..pieces.len() - 1] {
        // Do not silently follow junctions/symlinks inside a stage.
        // Also reject case-insensitive aliases on case-sensitive hosts.
        let entries = fs::read_dir(&path)?;
        for entry in entries {
            let entry = entry?;
            if entry.file_name().to_string_lossy().eq_ignore_ascii_case(component)
                && entry.file_name() != component
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
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(pieces[pieces.len() - 1])
            && entry.file_name() != pieces[pieces.len() - 1]
        {
            return Err(StageError::UnsafePath(name.into()));
        }
    }
    Ok(path.join(pieces[pieces.len() - 1]))
}

fn verify(path: &Path, public_key: &Path, basename: &str) -> Result<HashSet<String>, StageError> {
    let report = inspect_signed_pak(path, public_key, basename)
        .map_err(|e| StageError::Verification(e.to_string()))?;
    Ok(report.entries.into_iter().map(|e| e.path.to_lowercase()).collect())
}

fn active(root: &Path) -> Result<Option<StageTransaction>, StageError> {
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
        return Err(StageError::UnsafePath("PAK filename must match target".into()));
    }
    if fs::canonicalize(original)?.starts_with(&root) {
        return Err(StageError::UnsafePath("Import source is inside workspace".into()));
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

/// Apply ONE verified mod PAK to an isolated stage. Backup and replacement
/// bytes are durable before publishing the journal. The journal is saved
/// before moving the original, making an interrupted swap recoverable.
pub fn apply_stage_pak(
    root: &Path,
    replacement: &Path,
    relative: &str,
    original_key: &Path,
    replacement_key: &Path,
    basename: &str,
) -> Result<StageTransaction, StageError> {
    let root = workspace(root)?;
    if active(&root)?.is_some() {
        return Err(StageError::Pending);
    }
    let name = target_name(relative)?;
    ordinary(replacement)?;
    if fs::canonicalize(replacement)?.starts_with(&root) {
        return Err(StageError::UnsafePath("Replacement is inside workspace".into()));
    }
    if replacement.file_name().and_then(|s| s.to_str())
        != Some(name.rsplit('/').next().unwrap_or_default())
    {
        return Err(StageError::UnsafePath("PAK filename must match target".into()));
    }
    let target = stage_target(&root, &name, false)?;
    ordinary(&target)?;
    let old_entries = verify(&target, original_key, basename)?;
    let new_entries = verify(replacement, replacement_key, basename)?;
    if old_entries != new_entries {
        return Err(StageError::Verification("PAK member list changed".into()));
    }
    let before = digest(&target)?;
    let after = digest(replacement)?;
    if before == after {
        return Err(StageError::Verification("Replacement is identical to original".into()));
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
    copy_checked(&history.join("after.pak"), &history.join("incoming.pak"), &tx.after_sha256)?;
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
}
