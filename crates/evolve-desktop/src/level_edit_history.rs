//! Verified, session-scoped undo/redo for committed Modding-project level edits.
//! Both revisions live in securely-created temporary files (not game PAKs).
//! Reversal uses the same backup-backed, compare-and-swap project importer as
//! normal authoring, and fails closed if the edited source changed externally.

use crate::modding_projects::{self, ProjectRow};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_REVISION_BYTES: usize = 192 * 1024 * 1024;
pub(super) const MAX_HISTORY_STEPS: usize = 8;
pub(super) const MAX_HISTORY_DISK_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct CommittedRevision {
    workspace: PathBuf,
    target: String,
    member: String,
    original: tempfile::TempPath,
    modified: tempfile::TempPath,
    original_sha: [u8; 32],
    modified_sha: [u8; 32],
    disk_bytes: u64,
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn revision_file(bytes: &[u8]) -> Result<tempfile::TempPath, String> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    Ok(file.into_temp_path())
}

impl CommittedRevision {
    /// Capture both ends *before* attempting the native project transaction.
    /// An I/O error here therefore never turns an already-committed save into
    /// a misleading failure. The saved history is deliberately session-scoped.
    pub(super) fn capture(
        project: &ProjectRow,
        member: &str,
        original: &[u8],
        modified: &[u8],
    ) -> Result<Self, String> {
        if original.is_empty()
            || modified.is_empty()
            || original.len() > MAX_REVISION_BYTES
            || modified.len() > MAX_REVISION_BYTES
            || member.len() > 1024
            || member.contains('\\')
            || member.contains(':')
            || member.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || original == modified
        {
            return Err("Unsafe or empty committed level revision".into());
        }
        let original_sha = sha(original);
        let modified_sha = sha(modified);
        let original_file = revision_file(original)?;
        let modified_file = revision_file(modified)?;
        Ok(Self {
            workspace: project.workspace.clone(),
            target: project.target.clone(),
            member: member.into(),
            original: original_file,
            modified: modified_file,
            original_sha,
            modified_sha,
            disk_bytes: (original.len() + modified.len()) as u64,
        })
    }

    pub(super) fn project_matches(&self, project: &ProjectRow) -> bool {
        self.workspace == project.workspace && self.target == project.target
    }

    pub(super) fn disk_bytes(&self) -> u64 {
        self.disk_bytes
    }

    /// Reapply exact archived source bytes. A stale edit is an error, never a
    /// blind overwrite: save_level_member_checked checks the on-disk digest
    /// again immediately before its verified atomic project replacement.
    pub(super) fn apply(
        &self,
        project: &ProjectRow,
        projects_root: &Path,
        undo: bool,
    ) -> Result<(), String> {
        if !self.project_matches(project) {
            return Err("Committed history belongs to another Modding project".into());
        }
        let path = project.workspace.join("files").join(&self.member);
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_REVISION_BYTES as u64 {
            return Err("Edited project member is not a bounded regular file".into());
        }
        let current = fs::read(&path).map_err(|e| e.to_string())?;
        let (expected_sha, replacement_sha, replacement_file) = if undo {
            (&self.modified_sha, &self.original_sha, &self.original)
        } else {
            (&self.original_sha, &self.modified_sha, &self.modified)
        };
        if sha(&current) != *expected_sha {
            return Err(
                "Committed level changed since this revision; reload, do not overwrite it".into(),
            );
        }
        let replacement = fs::read(replacement_file).map_err(|e| e.to_string())?;
        if sha(&replacement) != *replacement_sha {
            return Err("Committed history snapshot failed its SHA-256 check".into());
        }
        let saved = modding_projects::save_level_member_checked(
            project,
            projects_root,
            &self.member,
            &current,
            &replacement,
        )?;
        if !saved {
            return Err("Committed revision made no verified project change".into());
        }
        if sha(&fs::read(&path).map_err(|e| e.to_string())?) != *replacement_sha {
            return Err("Edited project member failed post-commit SHA-256 verification".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_project_edit_can_undo_redo_and_reject_stale_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Projects");
        let workspace = root.join("Example");
        let member = "levels/Tutorial/layers/objects.lyr";
        let path = workspace.join("files").join(member);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = b"<ObjectLayer><LayerObjects><Object Id=\"rock\" Type=\"Brush\" Prefab=\"objects/rock.cgf\" Pos=\"0,0,0\"/></LayerObjects></ObjectLayer>";
        let after = b"<ObjectLayer><LayerObjects><Object Id=\"rock\" Type=\"Brush\" Prefab=\"objects/rock.cgf\" Pos=\"5,0,0\"/></LayerObjects></ObjectLayer>";
        fs::write(&path, before).unwrap();
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "entries": [{
                    "path": member,
                    "format": "raw",
                    "sha256": format!("{:x}", Sha256::digest(before))
                }]
            })).unwrap(),
        ).unwrap();
        let project = ProjectRow {
            label: "Example".into(), target: "levels_tutorial.pak".into(),
            source_build: String::new(), status: "Draft".into(),
            details: String::new(), workspace, modified: vec![],
            changed_known: true, enabled: false,
        };
        let revision = CommittedRevision::capture(&project, member, before, after).unwrap();
        assert!(revision.disk_bytes() > 0);
        assert!(modding_projects::save_level_member_checked(
            &project, &root, member, before, after
        ).unwrap());
        revision.apply(&project, &root, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        revision.apply(&project, &root, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), after);

        fs::write(&path, b"external mutation").unwrap();
        assert!(revision.apply(&project, &root, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external mutation");
        assert!(fs::read_dir(project.workspace.join("EditorBackups")).unwrap().count() >= 3);
    }

    #[test]
    fn committed_history_rejects_cross_project_and_unsafe_targets() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectRow {
            label: "Example".into(), target: "levels_tutorial.pak".into(),
            source_build: String::new(), status: "Draft".into(),
            details: String::new(), workspace: temp.path().to_path_buf(),
            modified: vec![], changed_known: true, enabled: false,
        };
        assert!(CommittedRevision::capture(&project, "../game.pak", b"a", b"b").is_err());
        let revision = CommittedRevision::capture(&project, "levels/a.lyr", b"a", b"b").unwrap();
        let mut wrong = project.clone();
        wrong.workspace.push("other");
        assert!(!revision.project_matches(&wrong));
        assert!(revision.apply(&wrong, temp.path(), true).is_err());
    }
}
