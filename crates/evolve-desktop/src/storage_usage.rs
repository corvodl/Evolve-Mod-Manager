//! Read-only, bounded per-user storage breakdown for the Settings screen.
//! No cleanup, migration, or archive modification happens during analysis.
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct StorageUsage {
    pub buckets: [u64; 5],
    pub files: usize,
    pub truncated: bool,
}

const MAX_FILES: usize = 400_000;
const MAX_DEPTH: usize = 32;

fn measure(path: &Path, files: &mut usize, truncated: &mut bool) -> u64 {
    let mut stack = vec![(path.to_path_buf(), 0usize)];
    let mut bytes = 0u64;
    while let Some((next, depth)) = stack.pop() {
        if *files >= MAX_FILES {
            *truncated = true;
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&next) else { continue; };
        if meta.file_type().is_symlink() { continue; }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 { continue; } // reparse point
        }
        if meta.is_file() {
            *files += 1;
            bytes = bytes.saturating_add(meta.len());
        } else if meta.is_dir() {
            if depth >= MAX_DEPTH {
                *truncated = true;
                continue;
            }
            if let Ok(children) = fs::read_dir(&next) {
                for entry in children.flatten() {
                    stack.push((entry.path(), depth + 1));
                }
            }
        }
    }
    bytes
}

/// Bucket order: active setup, old setups, projects, Asset Library, other.
/// Uses installed manager Data/GameSets, never scans entire game installs.
pub fn analyze(data: &Path, games: &Path, active_stage: &Path, projects: &Path, library: &Path) -> StorageUsage {
    let mut report = StorageUsage::default();
    let mut active_setup: Option<PathBuf> = None;
    for ancestor in active_stage.ancestors() {
        if ancestor.parent() == Some(games) && ancestor != games {
            active_setup = Some(ancestor.to_path_buf());
            break;
        }
    }
    if let Ok(entries) = fs::read_dir(games) {
        for entry in entries.flatten() {
            let location = entry.path();
            let value = measure(&location, &mut report.files, &mut report.truncated);
            let bucket = if active_setup.as_deref() == Some(location.as_path()) { 0 } else { 1 };
            report.buckets[bucket] = report.buckets[bucket].saturating_add(value);
        }
    }
    report.buckets[2] = measure(projects, &mut report.files, &mut report.truncated);
    report.buckets[3] = measure(library, &mut report.files, &mut report.truncated);
    if let Ok(entries) = fs::read_dir(data) {
        for entry in entries.flatten() {
            let location = entry.path();
            if location == games || location == projects || location == library {
                continue;
            }
            report.buckets[4] = report.buckets[4].saturating_add(
                measure(&location, &mut report.files, &mut report.truncated)
            );
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_active_old_projects_and_library_without_modifying_files() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("Data");
        let games = data.join("GameSets");
        let projects = data.join("Projects");
        let library = data.join("AssetLibrary");
        for dir in [games.join("Setup_1"), games.join("Setup_2"), projects.clone(), library.clone()] {
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("asset.bin"), b"12345").unwrap();
        }
        let report = analyze(&data, &games, &games.join("Setup_1/staged"), &projects, &library);
        assert_eq!(report.buckets, [5, 5, 5, 5, 0]);
        assert_eq!(report.files, 4);
        assert!(library.join("asset.bin").is_file());
    }
}
