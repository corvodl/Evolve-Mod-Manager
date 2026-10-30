//! Explicit, versioned game-build refreshes for the imported Asset Library.
//! Scan never writes; import publishes only verified changed/new PAKs and a
//! new append-only build snapshot. Originals and prior snapshots are immutable.
use crate::library_catalog::imported_library;
use crate::library_paths::{ordinary_path, source_path_key};
use crate::library_import::{
    LibraryBatchImportReport, LibraryImportFailure, LibraryImportPlan, MAX_LIBRARY_COLLECTION_PAKS,
    import_signed_library_pak_with_keys, scan_library_import_sources,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HISTORY_FILES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Only archive extractions created by this managed update process may
    /// be reclaimed. Historical snapshots default to none (fail closed).
    #[serde(default)]
    owned_extractions: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct BuildUpdateFile {
    pub path: PathBuf,
    pub relative: String,
    pub sha256: String,
    pub bytes: u64,
    pub was_present: bool,
    /// Identical PAK bytes already extracted from a different installed build.
    /// Reuse the verified library archive rather than copying it again.
    pub reuse_archive_id: Option<String>,
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
    /// New/changed PAKs that can reuse identical previously imported extraction.
    pub reused_count: usize,
}

#[derive(Clone, Debug)]
pub struct LibraryBuildUpdateReport {
    pub batch: LibraryBatchImportReport,
    pub build_id: String,
    pub unchanged: usize,
    pub removed: usize,
    pub updated: usize,
    pub added: usize,
    pub reused: usize,
    pub snapshot: PathBuf,
}

fn source_sha(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let count = file.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
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

// Preserve the historical raw-path hash only to authenticate old snapshot
// folders. New identities must ignore Windows verbatim transport prefixes.
fn legacy_build_identity(root: &Path) -> String {
    let text = root.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    format!("{:x}", Sha256::digest(text.as_bytes()))[..24].to_owned()
}

fn build_identity(root: &Path) -> String {
    let key = source_path_key(&root.to_string_lossy());
    format!("{:x}", Sha256::digest(key.as_bytes()))[..24].to_owned()
}

fn relative_name(root: &Path, file: &Path) -> Result<String, String> {
    let relative = file
        .strip_prefix(root)
        .map_err(|_| "PAK is outside its selected build root")?;
    let name = relative
        .to_str()
        .ok_or("Non-Unicode PAK path")?
        .replace('\\', "/");
    if name
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
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
    let record: BuildSnapshot =
        serde_json::from_reader(File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid build snapshot: {e}"))?;
    if record.version != 1
        || record.build_id != id
        || record.source_root != root.to_string_lossy()
        || record.members.len() > MAX_LIBRARY_COLLECTION_PAKS
        || record.owned_extractions.len() > MAX_LIBRARY_COLLECTION_PAKS
        || record
            .owned_extractions
            .iter()
            .any(|id| id.len() != 20 || !id.bytes().all(|c| c.is_ascii_hexdigit()))
        || record.members.iter().any(|(rel, member)| {
            rel.starts_with('/')
                || rel
                    .split('/')
                    .any(|v| v.is_empty() || v == ".." || v == ".")
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
    // Read across legacy snapshot directories with alternative spellings of
    // the same Windows path. The newest recorded snapshot remains authoritative.
    if build_identity(root) != id {
        return Err("Selected game build identity changed".into());
    }
    Ok(build_snapshots(library)?.get(id).and_then(|entries| entries.last().cloned()))
}

fn bootstrap_from_library(
    library: &Path,
    root: &Path,
) -> Result<BTreeMap<String, BuildMember>, String> {
    let catalog = imported_library(library).map_err(|e| e.to_string())?;
    let mut found: BTreeMap<String, (u64, BuildMember)> = BTreeMap::new();
    for archive in catalog.archives {
        let source = PathBuf::from(&archive.source);
        let Ok(resolved) = fs::canonicalize(&source) else {
            continue;
        };
        let Ok(relative) = resolved.strip_prefix(root) else {
            continue;
        };
        let Ok(rel) = relative_name(root, &root.join(relative)) else {
            continue;
        };
        let doc = archive.directory;
        let manifest_path = Path::new(&doc).join("asset-library.json");
        let metadata = fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_LEDGER_BYTES {
            continue;
        }
        let doc: serde_json::Value =
            serde_json::from_reader(File::open(manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let Some(sha) = doc["source_sha256"].as_str() else {
            continue;
        };
        if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let time = doc["imported_at"].as_u64().unwrap_or(0);
        if found.get(&rel).is_none_or(|(old, _)| time >= *old) {
            found.insert(
                rel,
                (
                    time,
                    BuildMember {
                        sha256: sha.to_ascii_lowercase(),
                        archive_id: archive.id,
                    },
                ),
            );
        }
    }
    Ok(found
        .into_iter()
        .map(|(key, (_, value))| (key, value))
        .collect())
}

/// Reuse complete, previously authenticated/extracted archives when a game
/// update is installed in a different directory. Archive identity remains
/// content-specific; build roots stay separate in the snapshot ledger.
fn imported_content_index(library: &Path) -> Result<BTreeMap<(String, String), String>, String> {
    let catalog = imported_library(library).map_err(|e| e.to_string())?;
    let mut index = BTreeMap::new();
    for archive in catalog.archives {
        let manifest = Path::new(&archive.directory).join("asset-library.json");
        let Ok(meta) = fs::symlink_metadata(&manifest) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.len() > MAX_LEDGER_BYTES {
            continue;
        }
        let Ok(doc) = serde_json::from_reader::<_, serde_json::Value>(
            File::open(&manifest).map_err(|e| e.to_string())?,
        ) else {
            continue;
        };
        let Some(sha) = doc["source_sha256"].as_str() else {
            continue;
        };
        if sha.len() != 64
            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
            || doc["name"].as_str() != Some(archive.name.as_str())
        {
            continue;
        }
        index
            .entry((archive.name.to_ascii_lowercase(), sha.to_ascii_lowercase()))
            .or_insert(archive.id);
    }
    Ok(index)
}

/// Full SHA-256 scan of a *chosen* game root. No file is imported or changed.
pub fn scan_library_build_updates(
    library: &Path,
    game: &Path,
) -> Result<LibraryBuildUpdatePlan, String> {
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
    let scan: LibraryImportPlan =
        match scan_library_import_sources(&library, std::slice::from_ref(&root), true) {
            Ok(scan) => scan,
            Err(reason) if reason == "No .pak files found in the selected source" => {
                // Removal-only updates are valid, including an emptied PAK folder.
                LibraryImportPlan {
                    files: Vec::new(),
                    packed_bytes: 0,
                    already_imported: 0,
                }
            }
            Err(reason) => return Err(reason),
        };
    let content_index = imported_content_index(&library)?;
    let mut changed = Vec::new();
    let mut reused_count = 0usize;
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
        if previous
            .get(&rel)
            .is_some_and(|entry| entry.sha256.eq_ignore_ascii_case(&sha))
        {
            unchanged += 1;
            continue;
        }
        let bytes = fs::metadata(&file).map_err(|e| e.to_string())?.len();
        let basename = file
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Non-Unicode archive filename")?;
        let reuse_archive_id = content_index
            .get(&(basename.to_ascii_lowercase(), sha.to_ascii_lowercase()))
            .cloned();
        if reuse_archive_id.is_some() {
            reused_count += 1;
        } else {
            import_bytes = import_bytes
                .checked_add(bytes)
                .ok_or("Import size overflow")?;
        }
        let was_present = previous.contains_key(&rel);
        if was_present {
            changed_count += 1;
        } else {
            new_count += 1;
        }
        changed.push(BuildUpdateFile {
            path: file,
            relative: rel,
            sha256: sha,
            bytes,
            was_present,
            reuse_archive_id,
        });
    }
    let removed = previous
        .keys()
        .filter(|rel| !scanned.contains(&rel.to_ascii_lowercase()))
        .cloned()
        .collect();
    Ok(LibraryBuildUpdatePlan {
        source_root: root,
        build_id: id,
        changed,
        unchanged,
        new_count,
        changed_count,
        removed,
        previous,
        scanned_bytes: scan.packed_bytes,
        import_bytes,
        reused_count,
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
    // Compare only the versioned ledger here: rescanning every unchanged
    // source PAK would rehash the entire multi-GiB game a second time.
    // Every changed/new source is separately rehashed before its own import.
    if let Some(snapshot) = latest_snapshot(&library, &plan.build_id, &root)? {
        if snapshot.members != plan.previous {
            return Err("Another build update finished. Scan this build again.".into());
        }
    } else if bootstrap_from_library(&library, &root)? != plan.previous {
        return Err("Asset Library changed after preview. Scan this build again.".into());
    }
    let mut members = plan.previous.clone();
    let mut owned_extractions = latest_snapshot(&library, &plan.build_id, &root)?
        .map_or_else(BTreeSet::new, |snapshot| snapshot.owned_extractions);
    for name in &plan.removed {
        members.remove(name);
    }
    let mut report = LibraryBatchImportReport {
        imported: Vec::new(),
        failed: Vec::new(),
    };
    progress.store(0, Ordering::Relaxed);
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut reused = 0usize;
    let content_index = imported_content_index(&library)?;
    for entry in &plan.changed {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let result = (|| {
            let metadata = fs::symlink_metadata(&entry.path).map_err(|e| e.to_string())?;
            if !metadata.file_type().is_file()
                || !entry.path.starts_with(&root)
                || fs::canonicalize(&entry.path).map_err(|e| e.to_string())? != entry.path
                || source_sha(&entry.path)? != entry.sha256
            {
                return Err("Source PAK changed since preview; scan again".into());
            }
            // A new installation may contain hundreds of unchanged PAKs
            // already extracted by another game build. Recheck their
            // manifest/content identity immediately before reuse; never trust
            // an outdated UI plan or copy the same game twice.
            if let Some(id) = &entry.reuse_archive_id {
                let name = entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("Invalid archive filename")?;
                if content_index
                    .get(&(name.to_ascii_lowercase(), entry.sha256.to_ascii_lowercase()))
                    != Some(id)
                {
                    return Err("Previously imported PAK changed or was removed; rescan".into());
                }
                Ok((id.clone(), true, None))
            } else {
                let imported = import_signed_library_pak_with_keys(&library, &entry.path, keys)?;
                Ok((imported.id.clone(), false, Some(imported)))
            }
        })();
        match result {
            Ok((archive_id, reused_source, imported)) => {
                members.insert(
                    entry.relative.clone(),
                    BuildMember {
                        sha256: entry.sha256.clone(),
                        archive_id,
                    },
                );
                if entry.was_present {
                    updated += 1;
                } else {
                    added += 1;
                }
                if reused_source {
                    reused += 1;
                }
                if let Some(imported) = imported {
                    if !imported.already_imported {
                        owned_extractions.insert(imported.id.clone());
                    }
                    report.imported.push(imported);
                }
            }
            Err(reason) => report.failed.push(LibraryImportFailure {
                archive: entry.path.display().to_string(),
                reason,
            }),
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    let history = library.join("build-update-history");
    if !history.exists() {
        fs::create_dir(&history).map_err(|e| e.to_string())?;
    }
    let history = real_directory(&history)?;
    let folder = history.join(&plan.build_id);
    if !folder.exists() {
        fs::create_dir(&folder).map_err(|e| e.to_string())?;
    }
    let folder = real_directory(&folder)?;
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = folder.join(format!("snapshot-{serial:025}.json"));
    let snapshot = BuildSnapshot {
        version: 1,
        build_id: plan.build_id.clone(),
        source_root: ordinary_path(&root.to_string_lossy()),
        members,
        owned_extractions,
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".build-snapshot-")
        .tempfile_in(&folder)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(temp.as_file_mut(), &snapshot).map_err(|e| e.to_string())?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|e| e.to_string())?;
    temp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    temp.persist_noclobber(&path)
        .map_err(|e| e.error.to_string())?;
    Ok(LibraryBuildUpdateReport {
        batch: report,
        build_id: plan.build_id.clone(),
        unchanged: plan.unchanged,
        removed: plan.removed.len(),
        updated,
        added,
        reused,
        snapshot: path,
    })
}

/// A removable build snapshot group stored in the Asset Library. Game
/// installations and independent Modding projects are always outside scope.
#[derive(Clone, Debug)]
pub struct ManagedLibraryBuild {
    pub id: String,
    pub source_root: String,
    pub display_name: String,
    pub aliases: usize,
    pub pak_count: usize,
    pub extracted_bytes: u64,
    /// Only extraction folders solely referenced by this group are eligible.
    pub reclaimable_paks: usize,
    pub reclaimable_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct LibraryBuildRemovalReport {
    pub source_root: String,
    pub deleted_snapshots: usize,
    pub removed_archives: usize,
    pub reclaimed_bytes: u64,
    pub retained_shared: usize,
    pub failed_cleanup: Vec<String>,
}

fn build_snapshots(library: &Path) -> Result<BTreeMap<String, Vec<BuildSnapshot>>, String> {
    let history = library.join("build-update-history");
    if !history.exists() {
        return Ok(BTreeMap::new());
    }
    let history = real_directory(&history)?;
    let mut builds = BTreeMap::<String, Vec<(String, BuildSnapshot)>>::new();
    let mut folders_seen = 0usize;
    for entry in fs::read_dir(&history).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        folders_seen += 1;
        if folders_seen > 128 {
            return Err("Too many managed game builds".into());
        }
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            return Err("Unexpected entry in build history; inspect manually".into());
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Unexpected build history directory; inspect manually".into());
        }
        let folder = real_directory(&entry.path())?;
        let mut paths = Vec::new();
        for file in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.starts_with("snapshot-")
                || !name.ends_with(".json")
                || !file.file_type().map_err(|e| e.to_string())?.is_file()
            {
                return Err("Unexpected file in build snapshot folder; no deletion allowed".into());
            }
            paths.push(file.path());
            if paths.len() > MAX_HISTORY_FILES {
                return Err("Too many snapshots for a managed game build".into());
            }
        }
        paths.sort_unstable();
        let mut records = Vec::new();
        for file in paths {
            let meta = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() || meta.len() > MAX_LEDGER_BYTES {
                return Err("Unsafe build history snapshot".into());
            }
            let doc: serde_json::Value =
                serde_json::from_reader(File::open(&file).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Cannot read build snapshot: {e}"))?;
            let root = doc["source_root"]
                .as_str()
                .ok_or("Missing build source root")?;
            if legacy_build_identity(Path::new(root)) != id
                && build_identity(Path::new(root)) != id
            {
                return Err("Build snapshot source identity differs from folder".into());
            }
            let mut record = read_snapshot(&file, &id, Path::new(root))?;
            record.build_id = build_identity(Path::new(root));
            record.source_root = ordinary_path(root);
            if records
                .first()
                .is_some_and(|old: &(String, BuildSnapshot)| source_path_key(&old.1.source_root) != source_path_key(&record.source_root))
            {
                return Err("Conflicting build roots within one snapshot history".into());
            }
            records.push((file.file_name().and_then(|v| v.to_str()).unwrap_or_default().to_owned(), record));
        }
        for (stamp, record) in records {
            builds.entry(record.build_id.clone()).or_default().push((stamp, record));
        }
    }
    Ok(builds.into_iter().map(|(id, mut entries)| {
        // Snapshot files use zero-padded nanosecond timestamps, so sorting
        // across old and new identity folders preserves update chronology.
        entries.sort_by(|a,b| a.0.cmp(&b.0));
        (id, entries.into_iter().map(|(_, record)| record).collect())
    }).collect())
}

/// Versioned registry is a non-destructive migration index. Existing
/// snapshot histories and extracted data remain byte-for-byte untouched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct BuildRegistration {
    source_root: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    history_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct BuildRegistry {
    version: u32,
    builds: BTreeMap<String, BuildRegistration>,
}

fn registry_path(library: &Path) -> PathBuf {
    library.join("managed-builds.json")
}

fn read_registry(library: &Path) -> Result<BuildRegistry, String> {
    let path = registry_path(library);
    if !path.exists() {
        return Ok(BuildRegistry { version: 1, builds: BTreeMap::new() });
    }
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > MAX_LEDGER_BYTES {
        return Err("Unsafe managed-build registry".into());
    }
    let data: BuildRegistry = serde_json::from_reader(File::open(&path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Cannot parse managed-build registry: {e}"))?;
    if data.version != 1 || data.builds.len() > 128 ||
        data.builds.iter().any(|(id, build)| {
            id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) ||
            build_identity(Path::new(&build.source_root)) != *id ||
            build.display_name.len() > 120 ||
            build.display_name.chars().any(char::is_control) ||
            build.history_ids.len() > 128 ||
            build.history_ids.iter().any(|alias| alias.len() != 24 || !alias.bytes().all(|b| b.is_ascii_hexdigit()))
        }) {
        return Err("Managed-build registry identity or label is invalid".into());
    }
    Ok(data)
}

fn write_registry(library: &Path, registry: &BuildRegistry) -> Result<(), String> {
    let mut temp = tempfile::Builder::new()
        .prefix(".managed-builds-")
        .tempfile_in(library)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(temp.as_file_mut(), registry).map_err(|e| e.to_string())?;
    temp.as_file_mut().write_all(b"\n").map_err(|e| e.to_string())?;
    temp.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    temp.persist(registry_path(library)).map_err(|e| e.error.to_string())?;
    Ok(())
}

/// Read physical history folder IDs for a logical build. Every source record
/// is revalidated against its original physical ID by read_snapshot.
fn physical_history_ids(library: &Path, logical_id: &str) -> Result<BTreeSet<String>, String> {
    let history = library.join("build-update-history");
    if !history.exists() {
        return Ok(BTreeSet::new());
    }
    let history = real_directory(&history)?;
    let mut result = BTreeSet::new();
    for entry in fs::read_dir(&history).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            return Err("Unexpected build history entry".into());
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid build history directory".into());
        }
        let mut files = fs::read_dir(entry.path()).map_err(|e| e.to_string())?
            .map(|f| f.map(|f| f.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        files.sort();
        if let Some(file) = files.first() {
            let value: serde_json::Value = serde_json::from_reader(
                File::open(file).map_err(|e| e.to_string())?
            ).map_err(|e| e.to_string())?;
            let root = value["source_root"].as_str().ok_or("History without source root")?;
            let _ = read_snapshot(file, &id, Path::new(root))?;
            if build_identity(Path::new(root)) == logical_id {
                result.insert(id);
            }
        }
    }
    Ok(result)
}

/// Persist legacy extended-prefix IDs as aliases, without moving or deleting
/// prior imported data. Called by explicit build-management operations.
fn migrate_registry(library: &Path, all: &BTreeMap<String, Vec<BuildSnapshot>>) -> Result<BuildRegistry, String> {
    let mut registry = read_registry(library)?;
    let before = registry.clone();
    let mut next = BTreeMap::new();
    for (id, records) in all {
        let latest = records.last().ok_or("Empty managed build")?;
        let previous = registry.builds.remove(id);
        next.insert(id.clone(), BuildRegistration {
            source_root: ordinary_path(&latest.source_root),
            display_name: previous.as_ref().map_or(String::new(), |row| row.display_name.clone()),
            history_ids: physical_history_ids(library, id)?,
        });
    }
    let updated = BuildRegistry {version: 1, builds: next};
    if before != updated {
        write_registry(library, &updated)?;
    }
    Ok(updated)
}

/// A rename affects only the manager's friendly label, never the installation
/// path, build identity, PAK provenance, or source files.
pub fn rename_managed_library_build(
    library: &Path,
    id: &str,
    expected_source_root: &str,
    label: &str,
) -> Result<Vec<ManagedLibraryBuild>, String> {
    let label = label.trim();
    if label.is_empty() || label.len() > 80 ||
        label.chars().any(|ch| ch.is_control() || matches!(ch, '/' | '\\')) {
        return Err("Build name must be 1-80 characters without slashes or controls".into());
    }
    let library = real_directory(library)?;
    let all = build_snapshots(&library)?;
    let latest = all.get(id).and_then(|rows| rows.last())
        .ok_or("Selected managed build no longer exists")?;
    if source_path_key(&latest.source_root) != source_path_key(expected_source_root) {
        return Err("Managed build changed; refresh before renaming".into());
    }
    let mut registry = migrate_registry(&library, &all)?;
    let record = registry.builds.get_mut(id).ok_or("Missing managed build registration")?;
    record.display_name = label.to_owned();
    write_registry(&library, &registry)?;
    managed_library_builds(&library)
}

fn all_without_build(all: &BTreeMap<String, Vec<BuildSnapshot>>, id: &str) -> BTreeMap<String, Vec<BuildSnapshot>> {
    let mut next = all.clone();
    next.remove(id);
    next
}

fn legacy_references(library: &Path) -> Result<BTreeSet<String>, String> {
    let file = library.join("build-collections.json");
    if !file.exists() {
        return Ok(BTreeSet::new());
    }
    let meta = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > MAX_LEDGER_BYTES {
        return Err("Legacy build collection file is unsafe".into());
    }
    let doc: serde_json::Value =
        serde_json::from_reader(File::open(&file).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Cannot parse existing build collections: {e}"))?;
    let builds = doc["builds"]
        .as_object()
        .ok_or("Legacy build collections are malformed")?;
    let mut result = BTreeSet::new();
    for build in builds.values() {
        let members = build["members"]
            .as_object()
            .ok_or("Legacy collection members are malformed")?;
        result.extend(members.keys().cloned());
    }
    Ok(result)
}

fn only_owned_extractions(
    library: &Path,
    target: &[BuildSnapshot],
    others: &BTreeMap<String, Vec<BuildSnapshot>>,
) -> Result<(Vec<(PathBuf, u64)>, usize), String> {
    let mut referenced = legacy_references(library)?;
    for history in others.values() {
        for snapshot in history {
            referenced.extend(
                snapshot
                    .members
                    .values()
                    .map(|item| item.archive_id.clone()),
            );
        }
    }
    // An archive may have existed as a standalone manual import before a
    // managed snapshot referenced it. Only delete files with explicit
    // managed-import provenance; old snapshots without this field retain
    // their extracted bytes rather than guessing ownership.
    let owned = target
        .iter()
        .flat_map(|snapshot| snapshot.owned_extractions.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut retained_shared = 0usize;
    let catalog = imported_library(library).map_err(|e| e.to_string())?;
    let source_root = Path::new(&target[0].source_root);
    let mut unused = Vec::new();
    for id in owned {
        if referenced.contains(&id) {
            retained_shared += 1;
            continue;
        }
        let Some(archive) = catalog.archives.iter().find(|item| item.id == id) else {
            continue;
        };
        // Reused imports from another installation do not belong to this
        // deleted build; never delete another build's extraction by mistake.
        if !Path::new(&archive.source).starts_with(source_root) {
            retained_shared += 1;
            continue;
        }
        let folder = Path::new(&archive.directory);
        if folder.parent() != Some(library)
            || !folder
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| name == id || name.ends_with(&format!("__{id}")))
        {
            retained_shared += 1;
            continue;
        }
        let meta = fs::symlink_metadata(folder).map_err(|e| e.to_string())?;
        if !meta.file_type().is_dir()
            || fs::canonicalize(folder).map_err(|e| e.to_string())? != folder
        {
            retained_shared += 1;
            continue;
        }
        let mut valid = true;
        let mut has_manifest = false;
        let mut has_files = false;
        for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if name == "asset-library.json"
                && entry.file_type().map_err(|e| e.to_string())?.is_file()
            {
                has_manifest = true;
            } else if name == "files" && entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                has_files = true;
            } else {
                valid = false;
            }
        }
        if !valid || !has_manifest || !has_files {
            retained_shared += 1;
            continue;
        }
        unused.push((folder.to_path_buf(), archive.extracted_bytes));
    }
    Ok((unused, retained_shared))
}

/// List only managed, incremental build snapshots. Imported game originals,
/// legacy build collections and separate editable mod projects are untouched.
pub fn managed_library_builds(library: &Path) -> Result<Vec<ManagedLibraryBuild>, String> {
    let library = real_directory(library)?;
    let all = build_snapshots(&library)?;
    let registrations = migrate_registry(&library, &all)?;
    let catalog = imported_library(&library).map_err(|e| e.to_string())?;
    let mut builds = Vec::new();
    for (id, snapshots) in &all {
        let latest = snapshots.last().ok_or("Empty build history")?;
        let mut other = all.clone();
        other.remove(id);
        let (reclaim, _) = only_owned_extractions(&library, snapshots, &other)?;
        let member_ids = latest
            .members
            .values()
            .map(|v| v.archive_id.clone())
            .collect::<BTreeSet<_>>();
        let extracted_bytes = catalog
            .archives
            .iter()
            .filter(|a| member_ids.contains(&a.id))
            .fold(0u64, |sum, a| sum.saturating_add(a.extracted_bytes));
        builds.push(ManagedLibraryBuild {
            id: id.clone(),
            source_root: ordinary_path(&latest.source_root),
            display_name: registrations.builds.get(id).map_or(String::new(), |r| r.display_name.clone()),
            aliases: registrations.builds.get(id).map_or(0, |r| r.history_ids.len()),
            pak_count: latest.members.len(),
            extracted_bytes,
            reclaimable_paks: reclaim.len(),
            reclaimable_bytes: reclaim
                .iter()
                .fold(0u64, |sum, (_, bytes)| sum.saturating_add(*bytes)),
        });
    }
    Ok(builds)
}

/// Delete only a confirmed Asset Library snapshot group and its unshared,
/// manager-created extraction folders. Never remove installed PAKs, other
/// snapshots, or independently editable projects.
pub fn remove_managed_library_build(
    library: &Path,
    id: &str,
    expected_source_root: &str,
) -> Result<LibraryBuildRemovalReport, String> {
    let library = real_directory(library)?;
    if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid selected game build".into());
    }
    let mut all = build_snapshots(&library)?;
    let snapshots = all
        .remove(id)
        .ok_or("Managed build is no longer available")?;
    let latest = snapshots.last().ok_or("Selected build has no snapshots")?;
    if source_path_key(&latest.source_root) != source_path_key(expected_source_root) {
        return Err("Selected build changed; refresh build management".into());
    }
    let (unused, retained_shared) = only_owned_extractions(&library, &snapshots, &all)?;
    let aliases = physical_history_ids(&library, id)?;
    if aliases.is_empty() {
        return Err("No source snapshot history for selected build".into());
    }
    // Every physical alias has already been validated by build_snapshots and
    // physical_history_ids. Remove history only, never original game folders.
    for alias in aliases {
        let history = snapshot_dir(&library, &alias);
        if history.parent() != Some(library.join("build-update-history").as_path())
            || !fs::symlink_metadata(&history).map_err(|e| e.to_string())?.file_type().is_dir()
        {
            return Err("Unsafe build history path".into());
        }
        fs::remove_dir_all(&history).map_err(|e| format!("Cannot remove build snapshot: {e}"))?;
    }
    migrate_registry(&library, &all_without_build(&all, id))?;
    let mut removed_archives = 0;
    let mut reclaimed_bytes = 0u64;
    let mut failed_cleanup = Vec::new();
    for (folder, bytes) in unused {
        match fs::remove_dir_all(&folder) {
            Ok(()) => {
                removed_archives += 1;
                reclaimed_bytes = reclaimed_bytes.saturating_add(bytes);
            }
            Err(e) => failed_cleanup.push(format!("{}: {e}", folder.display())),
        }
    }
    Ok(LibraryBuildRemovalReport {
        source_root: expected_source_root.into(),
        deleted_snapshots: snapshots.len(),
        removed_archives,
        reclaimed_bytes,
        retained_shared,
        failed_cleanup,
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
            zip.start_file("ui/main_menu.gfx", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body).unwrap();
            zip.finish().unwrap();
        }
        write_pak(&game_a.join("UI_Assets.pak"), b"old");
        write_pak(&game_b.join("UI_Assets.pak"), b"other installation");
        let first = scan_library_build_updates(&library, &game_a).unwrap();
        assert_eq!((first.new_count, first.unchanged), (1, 0));
        let done = apply_library_build_updates(
            &library,
            &first,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
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
            &library,
            &diff,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
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
    fn removal_only_build_update_keeps_prior_library_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let game = dir.path().join("game");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&game).unwrap();
        let pak = game.join("UI_Assets.pak");
        let mut zip = zip::ZipWriter::new(File::create(&pak).unwrap());
        zip.start_file("main_menu.gfx", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"menu").unwrap();
        zip.finish().unwrap();
        let plan = scan_library_build_updates(&library, &game).unwrap();
        let first = apply_library_build_updates(
            &library,
            &plan,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        fs::remove_file(&pak).unwrap();
        let removed = scan_library_build_updates(&library, &game).unwrap();
        assert!(removed.changed.is_empty());
        assert_eq!(removed.removed, vec!["UI_Assets.pak"]);
        let applied = apply_library_build_updates(
            &library,
            &removed,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        assert_eq!(applied.removed, 1);
        assert!(first.snapshot.exists());
        assert_eq!(imported_library(&library).unwrap().archives.len(), 1);
    }

    #[test]
    fn copied_game_reuses_matching_paks_and_only_extracts_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let old_game = dir.path().join("game-v1");
        let new_game = dir.path().join("game-v2");
        for folder in [&library, &old_game, &new_game] {
            fs::create_dir_all(folder).unwrap();
        }
        fn write_pak(path: &Path, body: &[u8]) {
            let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
            zip.start_file("models/test.cgf", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body).unwrap();
            zip.finish().unwrap();
        }
        write_pak(&old_game.join("A.pak"), b"same contents");
        write_pak(&old_game.join("B.pak"), b"same other contents");
        let initial = scan_library_build_updates(&library, &old_game).unwrap();
        apply_library_build_updates(
            &library,
            &initial,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        fs::copy(old_game.join("A.pak"), new_game.join("A.pak")).unwrap();
        fs::copy(old_game.join("B.pak"), new_game.join("B.pak")).unwrap();
        write_pak(&new_game.join("C.pak"), b"new content");
        let update = scan_library_build_updates(&library, &new_game).unwrap();
        assert_eq!(update.new_count, 3);
        assert_eq!(update.reused_count, 2);
        assert_eq!(
            update.import_bytes,
            fs::metadata(new_game.join("C.pak")).unwrap().len()
        );
        let applied = apply_library_build_updates(
            &library,
            &update,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        assert_eq!(applied.reused, 2);
        assert_eq!(applied.batch.imported.len(), 1);
        assert_eq!(imported_library(&library).unwrap().archives.len(), 3);
        let old_id = initial.changed[0].relative.clone();
        let current = scan_library_build_updates(&library, &old_game).unwrap();
        assert!(current.changed.is_empty());
        assert!(current.previous.contains_key(&old_id));
    }

    #[test]
    fn managed_build_removal_preserves_source_and_shared_imports() {
        let directory = tempfile::tempdir().unwrap();
        let library = directory.path().join("library");
        let old = directory.path().join("EvolveOld");
        let new = directory.path().join("EvolveNew");
        for dir in [&library, &old, &new] {
            fs::create_dir_all(dir).unwrap();
        }
        fn zip(path: &Path, data: &[u8]) {
            let mut archive = zip::ZipWriter::new(File::create(path).unwrap());
            archive
                .start_file("asset.bin", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(data).unwrap();
            archive.finish().unwrap();
        }
        zip(&old.join("shared.pak"), b"same");
        zip(&old.join("retired.pak"), b"retired");
        let initial = scan_library_build_updates(&library, &old).unwrap();
        apply_library_build_updates(
            &library,
            &initial,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        fs::copy(old.join("shared.pak"), new.join("shared.pak")).unwrap();
        let updated = scan_library_build_updates(&library, &new).unwrap();
        assert_eq!(updated.reused_count, 1);
        apply_library_build_updates(
            &library,
            &updated,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        let snapshot = managed_library_builds(&library).unwrap();
        assert_eq!(snapshot.len(), 2);
        let old_build = snapshot
            .iter()
            .find(|entry| entry.source_root == real_directory(&old).unwrap().to_string_lossy())
            .unwrap();
        assert_eq!(old_build.reclaimable_paks, 1);
        assert!(remove_managed_library_build(&library, &old_build.id, "wrong").is_err());
        let removed =
            remove_managed_library_build(&library, &old_build.id, &old_build.source_root).unwrap();
        assert_eq!(removed.removed_archives, 1);
        assert_eq!(removed.retained_shared, 1);
        assert!(old.join("retired.pak").exists());
        assert!(old.join("shared.pak").exists());
        assert_eq!(imported_library(&library).unwrap().archives.len(), 1);
        assert_eq!(managed_library_builds(&library).unwrap().len(), 1);
    }

    #[test]
    fn manual_import_that_is_later_linked_to_build_is_not_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let game = temp.path().join("game");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&game).unwrap();
        let source = game.join("manual.pak");
        let mut archive = zip::ZipWriter::new(File::create(&source).unwrap());
        archive
            .start_file("data.bin", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"existing import").unwrap();
        archive.finish().unwrap();
        let manually_imported =
            import_signed_library_pak_with_keys(&library, &source, &[]).unwrap();
        let plan = scan_library_build_updates(&library, &game).unwrap();
        assert_eq!(plan.unchanged, 1);
        apply_library_build_updates(
            &library,
            &plan,
            &[],
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )
        .unwrap();
        let builds = managed_library_builds(&library).unwrap();
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].reclaimable_paks, 0);
        let removed =
            remove_managed_library_build(&library, &builds[0].id, &builds[0].source_root).unwrap();
        assert_eq!(removed.removed_archives, 0);
        assert!(source.is_file());
        assert!(
            imported_library(&library)
                .unwrap()
                .archives
                .iter()
                .any(|archive| archive.id == manually_imported.id)
        );
    }

    #[test]
    fn copied_source_cannot_replace_a_different_game_root() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("LegacyModdedEvolve");
        let b = dir.path().join("ModdedEvolve");
        assert_ne!(build_identity(&a), build_identity(&b));
    }
}
