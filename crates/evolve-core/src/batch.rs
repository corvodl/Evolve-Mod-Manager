//! Crash-recoverable multi-PAK swaps inside an isolated offline workspace.
//! Never touches an installed game. A journal is written *before* the first
//! staged target is replaced; recovery refuses unexpected file contents.
use crate::profiles::{ProfileError, plan_mod_profile};
use crate::staging::{
    StageError, copy_checked, digest, stage_status, stage_target, target_name, workspace,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const JOURNAL: &str = "bundle-active.json";
const HISTORY: &str = ".evolve-batches";
const ACTIVE_SELECTION: &str = "active_mod_paks.json";
const MAX_SELECTION_BYTES: usize = 60 * 1024;

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
    if journal.schema != 1
        || journal.entries.is_empty()
        || journal.entries.len() > 500
        || journal.id.len() != 32
        || !journal.id.bytes().all(|c| c.is_ascii_hexdigit())
        || journal.profile_id.len() != 32
        || !journal.profile_id.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(BatchError::Invalid("Invalid batch header".into()));
    }
    let mut found = HashSet::new();
    for item in &journal.entries {
        if target_name(&item.target)? != item.target
            || !found.insert(item.target.to_lowercase())
            || item.original_sha256.len() != 64
            || item.replacement_sha256.len() != 64
            || !item.original_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !item
                .replacement_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(BatchError::Invalid(
                "Invalid batch entry or target collision".into(),
            ));
        }
    }
    Ok(())
}
fn history(root: &Path, journal: &BatchJournal) -> Result<PathBuf, BatchError> {
    let dir = root.join(HISTORY).join(&journal.id);
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
        return Err(BatchError::Invalid(
            "Recovery folder is not a real directory".into(),
        ));
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
        return Err(BatchError::Invalid(
            "Recovery journal is not a bounded regular file".into(),
        ));
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
fn selection_bytes(profile_id: &str, targets: &[String]) -> Result<Vec<u8>, BatchError> {
    if profile_id.len() != 32
        || !profile_id.bytes().all(|b| b.is_ascii_hexdigit())
        || targets.is_empty()
        || targets.len() > 500
    {
        return Err(BatchError::Invalid(
            "Invalid active PAK selection header".into(),
        ));
    }
    let mut seen = HashSet::new();
    for target in targets {
        if target.len() > 600
            || target_name(target)? != *target
            || !seen.insert(target.to_ascii_lowercase())
        {
            return Err(BatchError::Invalid(
                "Unsafe or duplicate active PAK target".into(),
            ));
        }
    }
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1,
        "profile_id": profile_id,
        "targets": targets,
    }))?;
    if bytes.len() > MAX_SELECTION_BYTES {
        return Err(BatchError::Invalid(
            "Active PAK selection exceeds launcher limit".into(),
        ));
    }
    Ok(bytes)
}
fn publish_selection(root: &Path, bytes: &[u8]) -> Result<(), BatchError> {
    let path = root.join(ACTIVE_SELECTION);
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(BatchError::Invalid(
                "Active PAK selection is not a regular file".into(),
            ));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => (),
    }
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| BatchError::Io(e.error))?;
    Ok(())
}
fn clear_selection(root: &Path) -> Result<(), BatchError> {
    let path = root.join(ACTIVE_SELECTION);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.file_type().is_file() => Ok(fs::remove_file(path)?),
        Ok(_) => Err(BatchError::Invalid(
            "Active PAK selection is not a regular file".into(),
        )),
        Err(e) => Err(e.into()),
    }
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
    // Reject a selection the client cannot read before creating recovery intent.
    let selection = selection_bytes(
        profile_id,
        &plan
            .mods
            .iter()
            .map(|m| m.target.clone())
            .collect::<Vec<_>>(),
    )?;
    // A blocked destination must fail before swapping any staged PAKs.
    match fs::symlink_metadata(root.join(ACTIVE_SELECTION)) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(BatchError::Invalid(
                "Active PAK selection is not a regular file".into(),
            ));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => (),
    }
    let tx = BatchJournal {
        schema: 1,
        id: format!("{:032x}", rand::random::<u128>()),
        profile_id: profile_id.into(),
        entries: plan
            .mods
            .iter()
            .map(|m| BatchEntry {
                target: m.target.clone(),
                original_sha256: String::new(),
                replacement_sha256: m.sha256.clone(),
            })
            .collect(),
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
        // The prevalidated 'after' file and target are in this same isolated
        // filesystem. Move the already-verified complete mod PAK into place;
        // copying it to a redundant 'incoming' file adds another multi-GB
        // write on every launch. Keep the journal and original backup intact.
        fs::rename(&target, dir.join(format!("displaced-{i}.pak")))?;
        fs::rename(dir.join(format!("after-{i}.pak")), &target)?;
        if digest(&target)? != item.replacement_sha256 {
            return Err(BatchError::Invalid(
                "Staged bytes failed post-write validation".into(),
            ));
        }
    }
    // Keep the recovery journal on publication failure: a stale selection must
    // never be mistaken for a successfully applied manual profile.
    publish_selection(&root, &selection).map_err(|error| {
        BatchError::Invalid(format!(
            "PAKs were applied but the client selection could not be published: {error}. Restore the staged profile before trying again"
        ))
    })?;
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
                return Err(BatchError::Invalid(
                    "Unexpected third-party change to staged PAK".into(),
                ));
            }
        } else {
            // A crash may occur after moving 'displaced' to 'restoring'
            // but before moving 'restoring' to the now-missing target.
            // Either location is an acceptable recovery candidate ONLY
            // when its bytes still match the independently saved original.
            let displaced = dir.join(format!("displaced-{i}.pak"));
            let restoring = dir.join(format!("restoring-{i}.pak"));
            let candidate = if fs::symlink_metadata(&displaced).is_ok() {
                &displaced
            } else {
                &restoring
            };
            if digest(candidate)? != item.original_sha256 {
                return Err(BatchError::Invalid(
                    "Missing or changed original backup during staged restore".into(),
                ));
            }
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
                return Err(BatchError::Invalid(
                    "Interrupted restore copy differs".into(),
                ));
            }
        } else {
            // The original staged PAK was moved to 'displaced' during apply.
            // Reusing that same-volume file avoids copying the entire original
            // again. 'before' remains a verified recovery fallback for older
            // or interrupted transactions which lack 'displaced'.
            let displaced = dir.join(format!("displaced-{i}.pak"));
            if fs::symlink_metadata(&displaced).is_ok() {
                if digest(&displaced)? != item.original_sha256 {
                    return Err(BatchError::Invalid(
                        "Displaced original differs from the recovery journal".into(),
                    ));
                }
                fs::rename(displaced, &restoring)?;
            } else {
                copy_checked(
                    &dir.join(format!("before-{i}.pak")),
                    &restoring,
                    &item.original_sha256,
                )?;
            }
        }
        if target.exists() {
            // Keep a recovered modified PAK for debugging rather than deleting it.
            let replaced = dir.join(format!("modified-{i}.pak"));
            if replaced.exists() {
                return Err(BatchError::Invalid(
                    "Existing modified backup needs inspection".into(),
                ));
            }
            fs::rename(&target, replaced)?;
        }
        fs::rename(restoring, &target)?;
        if digest(&target)? != item.original_sha256 {
            return Err(BatchError::Invalid(
                "Restored original bytes failed verification".into(),
            ));
        }
    }
    clear_selection(&root)?;
    fs::remove_file(root.join(JOURNAL))?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging::initialize_stage;
    #[test]
    fn active_selection_is_bounded_atomic_and_exact() {
        let tmp = tempfile::tempdir().unwrap();
        let stage = tmp.path().join("isolated");
        initialize_stage(&stage).unwrap();
        let root = workspace(&stage).unwrap();
        let id = "a".repeat(32);
        let first = vec!["Game/UI_Data.pak".to_string()];
        publish_selection(&root, &selection_bytes(&id, &first).unwrap()).unwrap();
        let path = root.join(ACTIVE_SELECTION);
        let original = fs::read(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["profile_id"], id);
        assert_eq!(json["targets"], serde_json::json!(first));
        for invalid in [
            vec![],
            vec!["Game/UI_Data.pak".into(), "game/ui_data.PAK".into()],
            vec!["Game/../other.pak".into()],
            vec!["Game\\other.pak".into()],
            vec!["Game/other.txt".into()],
            vec![format!("Game/{}.pak", "a".repeat(600))],
        ] {
            assert!(selection_bytes(&id, &invalid).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        let oversized = (0..500)
            .map(|i| format!("Game/{}{i:03}.pak", "a".repeat(125)))
            .collect::<Vec<_>>();
        assert!(selection_bytes(&id, &oversized).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let next = vec!["Game/UI_Assets.pak".into(), "Game/UI_Data.pak".into()];
        publish_selection(&root, &selection_bytes(&id, &next).unwrap()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(json["targets"], serde_json::json!(next));
    }
    #[test]
    fn interrupted_rename_restore_accepts_verified_restoring_file() {
        let tmp = tempfile::tempdir().unwrap();
        let stage = tmp.path().join("isolated");
        initialize_stage(&stage).unwrap();
        let root = workspace(&stage).unwrap();
        let tx = BatchJournal {
            schema: 1,
            id: "a".repeat(32),
            profile_id: "b".repeat(32),
            entries: vec![BatchEntry {
                target: "one.pak".into(),
                original_sha256: digest_bytes_for_test(b"original"),
                replacement_sha256: digest_bytes_for_test(b"modified"),
            }],
        };
        let dir = root.join(HISTORY).join(&tx.id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("before-0.pak"), b"original").unwrap();
        // Simulate a crash after displaced -> restoring, with no current
        // target. The independent 'before' copy remains journaled.
        fs::write(dir.join("restoring-0.pak"), b"original").unwrap();
        save_journal(&root, &tx).unwrap();
        restore_profile_stage(&stage).unwrap();
        assert_eq!(fs::read(root.join("paks/one.pak")).unwrap(), b"original");
        assert!(batch_stage_status(&stage).unwrap().is_none());
    }
    fn digest_bytes_for_test(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

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
                BatchEntry {
                    target: "first.pak".into(),
                    original_sha256: String::new(),
                    replacement_sha256: String::new(),
                },
                BatchEntry {
                    target: "second.pak".into(),
                    original_sha256: String::new(),
                    replacement_sha256: String::new(),
                },
            ],
        };
        let backup = root.join(HISTORY).join(&tx.id);
        fs::create_dir_all(&backup).unwrap();
        let mut tx = tx;
        for (i, content) in [b"first".as_slice(), b"second".as_slice()]
            .iter()
            .enumerate()
        {
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
        publish_selection(
            &root,
            &selection_bytes(&tx.profile_id, &["first.pak".into()]).unwrap(),
        )
        .unwrap();
        assert!(stage_status(&root).is_err());
        restore_profile_stage(&root).unwrap();
        // The intact displaced original was moved back, not recopied from
        // 'before'; the independent recovery backup remains untouched.
        assert!(!backup.join("displaced-0.pak").exists());
        assert_eq!(fs::read(backup.join("before-0.pak")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("paks/first.pak")).unwrap(), b"first");
        assert_eq!(fs::read(root.join("paks/second.pak")).unwrap(), b"second");
        assert!(batch_stage_status(&root).unwrap().is_none());
        assert!(!root.join(ACTIVE_SELECTION).exists());
    }
}
