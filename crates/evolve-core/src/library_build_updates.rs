//! Explicit, versioned game-build refreshes for the imported Asset Library.
//! Scan never writes; import publishes only verified changed/new PAKs and a
//! new append-only build snapshot. Originals and prior snapshots are immutable.
use crate::library_catalog::imported_library;
use crate::library_import::{
    LibraryBatchImportReport, LibraryImportFailure, LibraryImportPlan,
    MAX_LIBRARY_COLLECTION_PAKS, import_signed_library_pak_with_keys, scan_library_import_sources,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HISTORY_FILES: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildMember {
    pub sha256: String,
    pub archive_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BuildSnapshot {
    version: u32,
    build_id: String,
    source_root: String,
    members: BTreeMap<String, BuildMember>,
}

#[derive(Clone, Debug)]
pub struct BuildUpdateFile {
    pub path: PathBuf,
    pub relative: String,
    pub sha256: String,
    pub bytes: u64,
    pub was_present: bool,
}

#[derive(Clone, Debug)]
pub struct LibraryBuildUpdatePlan {
    pub source_root: PathBuf,
    pub build_id: String,
    pub changed: Vec<BuildUpdateFile>,
    pub unchanged: usize,
    pub new_count: usize,
    pub changed_count: usize,
    pub removed: Vec<String>,
    pub previous: BTreeMap<String, BuildMember>,
    pub scanned_bytes: u64,
    pub import_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct LibraryBuildUpdateReport {
    pub batch: LibraryBatchImportReport,
    pub build_id: String,
    pub unchanged: usize,
    pub removed: usize,
    pub updated: usize,
    pub added: usize,
    pub snapshot: PathBuf,
}

fn source_sha(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let count = file.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        digest.update(&buf[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn real_directory(root: &Path) -> Result<PathBuf, String> {
    if !fs::symlink_metadata(root).is_ok_and(|meta| meta.file_type().is_dir()) {
        return Err(format!("Folder is missing or linked: {}", root.display()));
    }
    fs::canonicalize(root).map_err(|e| e.to_string())
}

fn build_identity(root: &Path) -> String {
    let text = root.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    format!("{:x}", Sha256::digest(text.as_bytes()))[..24].to_string()
}

fn relative_name(root: &Path, file: &Path) -> Result<String, String> {
    let relative = file.strip_prefix(root).map_err(|_| "PAK is outside its selected build root")?;
    let name = relative.to_str().ok_or("Non-Unicode PAK path")?.replace('\\', "/");
    if name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || !name.to_ascii_lowercase().ends_with(".pak")
    {
        return Err("Unsafe source-relative PAK path".into());
    }
    Ok(name)
}

fn snapshot_dir(library: &Path, id: &str) -> PathBuf {
    library.join("build-update-history").join(id)
}

fn read_snapshot(path: &Path, id: &str, root: &Path) -> Result<BuildSnapshot, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_LEDGER_BYTES {
        return Err("Build snapshot must be a bounded regular file".into());
    }
    let record: BuildSnapshot = serde_json::from_reader(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid build snapshot: {e}"))?;
    if record.version != 1 || record.build_id != id
        || record.source_root != root.to_string_lossy()
        || record.members.len() > MAX_LIBRARY_COLLECTION_PAKS
        || record.members.iter().any(|(rel, member)| {
            rel.starts_with('/') || rel.split('/').any(|v| v.is_empty() || v == ".." || v == ".")
            || member.sha256.len() != 64
            || !member.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || member.archive_id.len() != 20
            || !member.archive_id.bytes().all(|c| c.is_ascii_hexdigit())
        })
    {
        return Err("Build snapshot identity or contents are invalid".into());
    }
    Ok(record)
}

fn latest_snapshot(library: &Path, id: &str, root: &Path) -> Result<Option<BuildSnapshot>, String> {
    let folder = snapshot_dir(library, id);
    if !folder.exists() { return Ok(None); }
    let folder = real_directory(&folder)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&folder).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") && name.starts_with("snapshot-") {
            entries.push(entry.path());
        }
        if entries.len() > MAX_HISTORY_FILES {
            return Err("Build snapshot history exceeds safe limit".into());
        }
    }
    entries.sort_unstable();
    entries.last().map(|file| read_snapshot(file, id, root)).transpose()
}

fn bootstrap_from_library(library: &Path, root: &Path) -> Result<BTreeMap<String, BuildMember>, String> {
    let catalog = imported_library(library).map_err(|e| e.to_string())?;
    let mut found: BTreeMap<String, (u64, BuildMember)> = BTreeMap::new();
    for archive in catalog.archives {
        let source = PathBuf::from(&archive.source);
        let Ok(relative) = source.strip_prefix(root) else { continue; };
        let Ok(rel) = relative_name(root, &root.join(relative)) else { continue; };
        let doc = archive.directory;
        let manifest_path = Path::new(&doc).join("asset-library.json");
        let metadata = fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_LEDGER_BYTES { continue; }
        let doc: serde_json::Value = serde_json::from_reader(
            File::open(manifest_path).map_err(|e| e.to_string())?
        ).map_err(|e| e.to_string())?;
        let Some(sha) = doc["source_sha256"].as_str() else { continue; };
        if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) { continue; }
        let time = doc["imported_at"].as_u64().unwrap_or(0);
        if found.get(&rel).is_none_or(|(old, _)| time >= *old) {
            found.insert(rel, (time, BuildMember {
                sha256: sha.to_ascii_lowercase(), archive_id: archive.id
            }));
        }
    }
    Ok(found.into_iter().map(|(key, (_, value))| (key, value)).collect())
}

/// Full SHA-256 scan of a *chosen* game root. No file is imported or changed.
pub fn scan_library_build_updates(library: &Path, game: &Path) -> Result<LibraryBuildUpdatePlan, String> {
    let library = real_directory(library)?;
    let root = real_directory(game)?;
    if root.starts_with(&library) || library.starts_with(&root) {
        return Err("Build source and Asset Library must be separate directories".into());
    }
    let id = build_identity(&root);
    let prior = latest_snapshot(&library, &id, &root)?;
    let previous = match prior {
        Some(snapshot) => snapshot.members,
        None => bootstrap_from_library(&library, &root)?,
    };
    let scan: LibraryImportPlan = scan_library_import_sources(
        &library, std::slice::from_ref(&root), true
    )?;
    let mut changed = Vec::new();
    let mut scanned = HashSet::new();
    let mut unchanged = 0usize;
    let mut new_count = 0usize;
    let mut changed_count = 0usize;
    let mut import_bytes = 0u64;
    for file in scan.files {
        let rel = relative_name(&root, &file)?;
        if !scanned.insert(rel.to_ascii_lowercase()) {
            return Err("Build contains case-colliding source PAK paths".into());
        }
        let sha = source_sha(&file)?;
        if previous.get(&rel).is_some_and(|entry| entry.sha256.eq_ignore_ascii_case(&sha)) {
            unchanged += 1;
            continue;
        }
        let bytes = fs::metadata(&file).map_err(|e| e.to_string())?.len();
        import_bytes = import_bytes.checked_add(bytes).ok_or("Import size overflow")?;
        let was_present = previous.contains_key(&rel);
        if was_present { changed_count += 1; } else { new_count += 1; }
        changed.push(BuildUpdateFile {path:file, relative:rel, sha256:sha, bytes, was_present});
    }
    let removed = previous.keys()
        .filter(|rel| !scanned.contains(&rel.to_ascii_lowercase()))
        .cloned().collect();
    Ok(LibraryBuildUpdatePlan {
        source_root: root, build_id:id, changed, unchanged, new_count,
        changed_count, removed, previous, scanned_bytes:scan.packed_bytes, import_bytes,
    })
}

/// Import only changed/new signed or legacy PAKs. Existing library versions
/// and original game assets remain untouched. Failed members stay on their
/// prior accepted snapshot; removed source PAKs are never deleted.
pub fn apply_library_build_updates(
    library: &Path,
    plan: &LibraryBuildUpdatePlan,
    keys: &[PathBuf],
    cancelled: &AtomicBool,
    progress: &AtomicUsize,
) -> Result<LibraryBuildUpdateReport, String> {
    let library = real_directory(library)?;
    let root = real_directory(&plan.source_root)?;
    if build_identity(&root) != plan.build_id {
        return Err("Selected build identity changed".into());
    }
    if latest_snapshot(&library, &plan.build_id, &root)?.is_some() {
        // The scan belongs to a specific source baseline. Reject concurrent
        // commits instead of replacing another update's accepted member map.
        let refreshed = scan_library_build_updates(&library, &root)?;
        if refreshed.previous != plan.previous {
            return Err("Another build update finished. Scan this build again.".into());
        }
    }
    let mut members = plan.previous.clone();
    for name in &plan.removed { members.remove(name); }
    let mut report = LibraryBatchImportReport {imported: Vec::new(), failed: Vec::new()};
    progress.store(0, Ordering::Relaxed);
    let mut added = 0usize;
    let mut updated = 0usize;
    for entry in &plan.changed {
        if cancelled.load(Ordering::Relaxed) { break; }
        let result = (|| {
            let metadata = fs::symlink_metadata(&entry.path).map_err(|e| e.to_string())?;
            if !metadata.file_type().is_file()
                || !entry.path.starts_with(&root)
                || fs::canonicalize(&entry.path).map_err(|e| e.to_string())? != entry.path
                || source_sha(&entry.path)? != entry.sha256
            {
                return Err("Source PAK changed since preview; scan again".into());
            }
            import_signed_library_pak_with_keys(&library, &entry.path, keys)
        })();
        match result {
            Ok(imported) => {
                members.insert(entry.relative.clone(), BuildMember {
                    sha256: entry.sha256.clone(), archive_id: imported.id.clone(),
                });
                if entry.was_present { updated += 1; } else { added += 1; }
                report.imported.push(imported);
            }
            Err(reason) => report.failed.push(LibraryImportFailure {
                archive: entry.path.display().to_string(), reason
            }),
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    let history = library.join("build-update-history");
    if !history.exists() { fs::create_dir(&history).map_err(|e| e.to_string())?; }
    let history = real_directory(&history)?;
    let folder = history.join(&plan.build_id);
    if !folder.exists() { fs::create_dir(&folder).map_err(|e| e.to_string())?; }
    let folder = real_directory(&folder)?;
    let serial = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?.as_nanos();
    let path = folder.join(format!("snapshot-{serial:025}.json"));
    let snapshot = BuildSnapshot {
        version:1, build_id: plan.build_id.clone(),
        source_root: root.to_string_lossy().into_owned(), members,
    };
    let mut temp = tempfile::Builder::new().prefix(".build-snapshot-")
        .tempfile_in(&folder).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(temp.as_file_mut(), &snapshot)
        .map_err(|e| e.to_string())?;
    temp.as_file_mut().write_all(b"\n").map_err(|e| e.to_string())?;
    temp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    temp.persist_noclobber(&path).map_err(|e| e.error.to_string())?;
    Ok(LibraryBuildUpdateReport {
        batch:report, build_id:plan.build_id.clone(),
        unchanged:plan.unchanged, removed:plan.removed.len(),
        updated, added, snapshot:path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incremental_zip_build_update_preserves_old_snapshot_and_other_roots() {
        use zip::write::SimpleFileOptions;
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let game_a = dir.path().join("EvolveA");
        let game_b = dir.path().join("EvolveB");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&game_a).unwrap();
        fs::create_dir_all(&game_b).unwrap();
        fn write_pak(path: &Path, body: &[u8]) {
            let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
            zip.start_file("ui/main_menu.gfx", SimpleFileOptions::default()).unwrap();
            zip.write_all(body).unwrap();
            zip.finish().unwrap();
        }
        write_pak(&game_a.join("UI_Assets.pak"), b"old");
        write_pak(&game_b.join("UI_Assets.pak"), b"other installation");
        let first = scan_library_build_updates(&library, &game_a).unwrap();
        assert_eq!((first.new_count, first.unchanged), (1, 0));
        let done = apply_library_build_updates(
            &library, &first, &[], &AtomicBool::new(false), &AtomicUsize::new(0)
        ).unwrap();
        assert_eq!((done.added, done.updated), (1, 0));
        let unchanged = scan_library_build_updates(&library, &game_a).unwrap();
        assert!(unchanged.changed.is_empty());
        assert_eq!(unchanged.unchanged, 1);
        let b = scan_library_build_updates(&library, &game_b).unwrap();
        assert_ne!(b.build_id, unchanged.build_id);
        assert_eq!(b.new_count, 1);
        write_pak(&game_a.join("UI_Assets.pak"), b"updated UI menu");
        let diff = scan_library_build_updates(&library, &game_a).unwrap();
        assert_eq!((diff.changed_count, diff.new_count), (1, 0));
        let applied = apply_library_build_updates(
            &library, &diff, &[], &AtomicBool::new(false), &AtomicUsize::new(0)
        ).unwrap();
        assert_eq!(applied.updated, 1);
        assert_ne!(done.snapshot, applied.snapshot);
        assert!(done.snapshot.is_file());
        assert_eq!(imported_library(&library).unwrap().archives.len(), 2);
        fs::remove_file(game_a.join("UI_Assets.pak")).unwrap();
        // An empty directory is rejected by the shared archive source scanner;
        // keep another PAK in the build to classify a removed member.
        write_pak(&game_a.join("misc.pak"), b"new archive");
        let removed = scan_library_build_updates(&library, &game_a).unwrap();
        assert_eq!(removed.removed, vec!["UI_Assets.pak"]);
    }

    #[test]
    fn copied_source_cannot_replace_a_different_game_root() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("LegacyModdedEvolve");
        let b = dir.path().join("ModdedEvolve");
        assert_ne!(build_identity(&a), build_identity(&b));
    }
}
