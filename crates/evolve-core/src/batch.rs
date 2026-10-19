//! Crash-recoverable multi-PAK swaps inside an isolated offline workspace.
//! Never touches an installed game. A journal is written *before* the first
//! staged target is replaced; recovery refuses unexpected file contents.
use crate::profiles::{ProfileError, plan_mod_profile};
use crate::staging::{
    StageError, copy_checked, digest, stage_status, stage_target, target_name, workspace,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const JOURNAL: &str = "bundle-active.json";
const HISTORY: &str = ".evolve-batches";

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("PAK staging safety error: {0}")]
    Stage(#[from] StageError),
    #[error("Mod profile invalid: {0}")]
    Profile(#[from] ProfileError),
    #[error("Invalid or tampered batch transaction: {0}")]
    Invalid(String),
    #[error("A previous batch transaction needs recovery")]
    Pending,
    #[error("No batch recovery journal exists")]
    NoJournal,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Damaged batch journal: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BatchEntry {
    pub target: String,
    pub original_sha256: String,
    pub replacement_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchJournal {
    pub schema: u32,
    pub id: String,
    pub profile_id: String,
    pub entries: Vec<BatchEntry>,
}

fn validate(journal: &BatchJournal) -> Result<(), BatchError> {
    if journal.schema != 1 || journal.entries.is_empty() || journal.entries.len() > 500 ||
        journal.id.len() != 32 || !journal.id.bytes().all(|c| c.is_ascii_hexdigit()) ||
        journal.profile_id.len() != 32 ||
        !journal.profile_id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(BatchError::Invalid("Invalid batch header".into()));
    }
    let mut found = HashSet::new();
    for item in &journal.entries {
        if target_name(&item.target)? != item.target ||
            !found.insert(item.target.to_lowercase()) ||
            item.original_sha256.len() != 64 ||
            item.replacement_sha256.len() != 64 ||
            !item.original_sha256.bytes().all(|b| b.is_ascii_hexdigit()) ||
            !item.replacement_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(BatchError::Invalid("Invalid batch entry or target collision".into()));
        }
    }
    Ok(())
}
fn history(root: &Path, journal: &BatchJournal) -> Result<PathBuf, BatchError> {
    let dir = root.join(HISTORY).join(&journal.id);
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
        return Err(BatchError::Invalid("Recovery folder is not a real directory".into()));
    }
    Ok(dir)
}
fn read_journal(root: &Path) -> Result<Option<BatchJournal>, BatchError> {
    let file = root.join(JOURNAL);
    let meta = match fs::symlink_metadata(&file) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.file_type().is_file() || meta.len() > 128 * 1024 {
        return Err(BatchError::Invalid("Recovery journal is not a bounded regular file".into()));
    }
    let journal: BatchJournal = serde_json::from_slice(&fs::read(file)?)?;
    validate(&journal)?;
    Ok(Some(journal))
}
fn save_journal(root: &Path, tx: &BatchJournal) -> Result<(), BatchError> {
    validate(tx)?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut file, tx)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(root.join(JOURNAL))
        .map_err(|e| BatchError::Io(e.error))?;
    Ok(())
}
pub fn batch_stage_status(stage: &Path) -> Result<Option<BatchJournal>, BatchError> {
    read_journal(&workspace(stage)?)
}

/// Check every source and signature before touching any staged target.
/// Failure after publishing the recovery journal intentionally leaves it
/// available; use restore_profile_stage to repair an interrupted batch.
pub fn apply_profile_stage(
    stage: &Path,
    profile_id: &str,
    original_key: &Path,
    mod_key: &Path,
) -> Result<BatchJournal, BatchError> {
    let root = workspace(stage)?;
    if read_journal(&root)?.is_some() || stage_status(&root)?.is_some() {
        return Err(BatchError::Pending);
    }
    let plan = plan_mod_profile(&root, profile_id, original_key, mod_key)?;
    if plan.mods.is_empty() {
        return Err(BatchError::Invalid("Profile has no enabled PAKs".into()));
    }
    let tx = BatchJournal {
        schema: 1,
        id: format!("{:032x}", rand::random::<u128>()),
        profile_id: profile_id.into(),
        entries: plan.mods.iter().map(|m| BatchEntry {
            target: m.target.clone(),
            original_sha256: String::new(),
            replacement_sha256: m.sha256.clone(),
        }).collect(),
    };
    let batch_root = root.join(HISTORY);
    if !batch_root.exists() {
        fs::create_dir(&batch_root)?;
    }
    let kind = fs::symlink_metadata(&batch_root)?.file_type();
    if !kind.is_dir() || kind.is_symlink() {
        return Err(BatchError::Invalid("Unsafe batch backup root".into()));
    }
    let dir = batch_root.join(&tx.id);
    fs::create_dir(&dir)?;
    let mut tx = tx;
    for (i, mod_file) in plan.mods.iter().enumerate() {
        let target = stage_target(&root, &mod_file.target, false)?;
        let before = digest(&target)?;
        copy_checked(&target, &dir.join(format!("before-{i}.pak")), &before)?;
        copy_checked(
            Path::new(&mod_file.source),
            &dir.join(format!("after-{i}.pak")),
            &mod_file.sha256,
        )?;
        tx.entries[i].original_sha256 = before;
    }
    // All copies are verified before the durable recovery intent is created.
    save_journal(&root, &tx)?;
    for (i, item) in tx.entries.iter().enumerate() {
        let target = stage_target(&root, &item.target, false)?;
        if digest(&target)? != item.original_sha256 {
            return Err(BatchError::Invalid("Original stage bytes changed".into()));
        }
        // Renames stay within the same filesystem. No overwrite is permitted.
        fs::rename(&target, dir.join(format!("displaced-{i}.pak")))?;
        copy_checked(
            &dir.join(format!("after-{i}.pak")),
            &dir.join(format!("incoming-{i}.pak")),
            &item.replacement_sha256,
        )?;
        fs::rename(dir.join(format!("incoming-{i}.pak")), &target)?;
        if digest(&target)? != item.replacement_sha256 {
            return Err(BatchError::Invalid("Staged bytes failed post-write validation".into()));
        }
    }
    Ok(tx)
}

/// Recover a complete or interrupted batch. Unexpected edits never get
/// deleted: the journal remains for manual investigation instead.
pub fn restore_profile_stage(stage: &Path) -> Result<BatchJournal, BatchError> {
    let root = workspace(stage)?;
    let tx = read_journal(&root)?.ok_or(BatchError::NoJournal)?;
    let dir = history(&root, &tx)?;
    // Verify backups and all current target bytes before beginning recovery.
    for (i, item) in tx.entries.iter().enumerate() {
        if digest(&dir.join(format!("before-{i}.pak")))? != item.original_sha256 {
            return Err(BatchError::Invalid("Original backup was changed".into()));
        }
        let target = stage_target(&root, &item.target, false)?;
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if !meta.file_type().is_file() {
                return Err(BatchError::Invalid("Target is not a regular PAK".into()));
            }
            let current = digest(&target)?;
            if current != item.original_sha256 && current != item.replacement_sha256 {
                return Err(BatchError::Invalid("Unexpected third-party change to staged PAK".into()));
            }
        } else if digest(&dir.join(format!("displaced-{i}.pak")))? != item.original_sha256 {
            return Err(BatchError::Invalid("Missing displaced original backup".into()));
        }
    }
    for (i, item) in tx.entries.iter().enumerate() {
        let target = stage_target(&root, &item.target, false)?;
        if target.exists() && digest(&target)? == item.original_sha256 {
            continue;
        }
        let restoring = dir.join(format!("restoring-{i}.pak"));
        if fs::symlink_metadata(&restoring).is_ok() {
            if digest(&restoring)? != item.original_sha256 {
                return Err(BatchError::Invalid("Interrupted restore copy differs".into()));
            }
        } else {
            copy_checked(&dir.join(format!("before-{i}.pak")), &restoring, &item.original_sha256)?;
        }
        if target.exists() {
            // Keep a recovered modified PAK for debugging rather than deleting it.
            let replaced = dir.join(format!("modified-{i}.pak"));
            if replaced.exists() {
                return Err(BatchError::Invalid("Existing modified backup needs inspection".into()));
            }
            fs::rename(&target, replaced)?;
        }
        fs::rename(restoring, &target)?;
        if digest(&target)? != item.original_sha256 {
            return Err(BatchError::Invalid("Restored original bytes failed verification".into()));
        }
    }
    fs::remove_file(root.join(JOURNAL))?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging::initialize_stage;
    #[test]
    fn partial_batch_recovery_keeps_original_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let stage = tmp.path().join("isolated");
        initialize_stage(&stage).unwrap();
        let root = workspace(&stage).unwrap();
        let tx = BatchJournal {
            schema: 1,
            id: "1".repeat(32),
            profile_id: "2".repeat(32),
            entries: vec![
                BatchEntry { target: "first.pak".into(), original_sha256: String::new(), replacement_sha256: String::new() },
                BatchEntry { target: "second.pak".into(), original_sha256: String::new(), replacement_sha256: String::new() },
            ],
        };
        let backup = root.join(HISTORY).join(&tx.id);
        fs::create_dir_all(&backup).unwrap();
        let mut tx = tx;
        for (i, content) in [b"first".as_slice(), b"second".as_slice()].iter().enumerate() {
            let original = root.join("paks").join(&tx.entries[i].target);
            fs::write(&original, content).unwrap();
            tx.entries[i].original_sha256 = digest(&original).unwrap();
            fs::write(backup.join(format!("before-{i}.pak")), content).unwrap();
            tx.entries[i].replacement_sha256 = tx.entries[i].original_sha256.clone();
        }
        // Simulate process death after replacing one target but before second.
        let first = root.join("paks/first.pak");
        fs::rename(&first, backup.join("displaced-0.pak")).unwrap();
        fs::write(&first, b"modified").unwrap();
        tx.entries[0].replacement_sha256 = digest(&first).unwrap();
        save_journal(&root, &tx).unwrap();
        assert!(stage_status(&root).is_err());
        restore_profile_stage(&root).unwrap();
        assert_eq!(fs::read(root.join("paks/first.pak")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("paks/second.pak")).unwrap(), b"second");
        assert!(batch_stage_status(&root).unwrap().is_none());
    }
}
