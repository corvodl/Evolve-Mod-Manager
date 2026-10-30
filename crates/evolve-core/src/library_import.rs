//! Opt-in, format-aware Asset Library importer for ZIP-compatible legacy and signed Stage 2 PAKs.
//! Compatible with the Qt manager's version-one AssetLibrary manifests.
//! No source PAK or previously imported library directory is modified.
use crate::archive::{extract_legacy_pak, inspect_legacy_pak, safe_member_name};
use crate::cryxml::xml_bytes;
use crate::library_catalog::{imported_archive_files, imported_library};
use crate::library_paths::{ordinary_path, source_path_key};
use crate::signed_pak::extract_signed_pak;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const XML_LIMIT: usize = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024 * 1024;
pub const MAX_LIBRARY_COLLECTION_PAKS: usize = 4096;
const MAX_COLLECTION_DEPTH: usize = 16;
const MAX_COLLECTION_ENTRIES: usize = 100_000;

#[derive(Clone, Debug)]
pub struct LibraryImportPlan {
    pub files: Vec<PathBuf>,
    pub packed_bytes: u64,
    pub already_imported: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryImportReport {
    pub id: String,
    pub archive: String,
    pub files: usize,
    pub bytes: u64,
    pub already_imported: bool,
}

/// Results of an explicitly requested bounded multi-PAK import. Each archive
/// is independently verified and published; a failed archive cannot overwrite
/// a previous import. Partial success is always reported to the caller.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryBatchImportReport {
    pub imported: Vec<LibraryImportReport>,
    pub failed: Vec<LibraryImportFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryImportFailure {
    pub archive: String,
    pub reason: String,
}

/// Only the exact original archive basename and SHA-256 establish the ID.
/// Never put an absolute path, username, or a signing key in the directory name.
fn archive_id(name: &str, source_sha: &str) -> String {
    let key = format!("{}|{source_sha}", name.to_ascii_lowercase());
    let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    hash[..20].to_owned()
}

/// The same PAK filename and bytes in two installations must not cause one
/// installation's imported source path to be reused for the other. Preserve
/// legacy name+hash IDs when that exact source was already indexed.
fn source_scoped_id(name: &str, sha: &str, source: &Path) -> String {
    let path = source_path_key(&source.to_string_lossy());
    let fingerprint = format!("{}|{sha}|{path}", name.to_ascii_lowercase());
    format!("{:x}", Sha256::digest(fingerprint.as_bytes()))[..20].to_owned()
}

fn archive_directory(name: &str, id: &str) -> String {
    let slug = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches([' ', '.', '_'])
        .chars()
        .take(96)
        .collect::<String>();
    format!(
        "{}__{id}",
        if slug.is_empty() {
            "Imported_PAK"
        } else {
            &slug
        }
    )
}

fn sha_file(file: &Path) -> Result<String, String> {
    let mut reader = File::open(file).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// A verified signed CryPAK must never be silently retried as an unsigned ZIP.
/// Signature-trailer detection is a routing hint, not authentication; the full
/// RSA check remains mandatory in the signed extraction path.
fn has_stage2_signature_trailer(source: &Path) -> Result<bool, String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() {
        return Err("Select a regular, non-linked PAK file".into());
    }
    if metadata.len() < 22 {
        return Ok(false);
    }
    let len = metadata.len().min(65_557) as usize;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::End(-(len as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0u8; len];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    for offset in (0..=len - 22).rev() {
        if &tail[offset..offset + 4] != b"PK\x05\x06" {
            continue;
        }
        let comment_len = u16::from_le_bytes([tail[offset + 20], tail[offset + 21]]) as usize;
        if offset + 22 + comment_len != len {
            continue;
        }
        let header = tail.get(offset + 22..offset + 28);
        return Ok(comment_len == 2320 || header == Some(b"\x06\x00\x00\x00\x01\x03".as_slice()));
    }
    Ok(false)
}

#[derive(Clone, Copy)]
enum ImportFormat<'a> {
    Signed(&'a Path, &'a str),
    LegacyZip,
}

/// Retained signed-only entrypoint for explicit cryptographic workflows.
pub fn import_signed_library_pak(
    root: &Path,
    source: &Path,
    public_key: &Path,
) -> Result<LibraryImportReport, String> {
    let basename = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Source PAK name is not valid Unicode")?;
    import_library_pak(root, source, ImportFormat::Signed(public_key, basename))
}

fn import_library_pak(
    root: &Path,
    source: &Path,
    format: ImportFormat<'_>,
) -> Result<LibraryImportReport, String> {
    let root_meta = fs::symlink_metadata(root)
        .map_err(|_| "Select an existing local AssetLibrary directory first".to_owned())?;
    if !root_meta.file_type().is_dir() {
        return Err("AssetLibrary cannot be a symlink or ordinary file".into());
    }
    let source_meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !source_meta.file_type().is_file()
        || source_meta.len() == 0
        || source_meta.len() > MAX_SOURCE_BYTES
    {
        return Err("Select a bounded, non-linked, regular PAK file".into());
    }
    let basename = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Source PAK name is not valid Unicode")?;
    if basename.len() > 240 || !basename.to_ascii_lowercase().ends_with(".pak") {
        return Err("Select a .pak file".into());
    }
    let signed_name = match format {
        ImportFormat::Signed(_, name) => name,
        ImportFormat::LegacyZip => basename,
    };
    let public_key = match format {
        ImportFormat::Signed(key, _) => {
            let meta = fs::symlink_metadata(key).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() || meta.len() > 8192 {
                return Err("Use an existing local, regular RSA verification key".into());
            }
            Some(fs::canonicalize(key).map_err(|e| e.to_string())?)
        }
        ImportFormat::LegacyZip => None,
    };
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if source.starts_with(&root) {
        return Err("Cannot import a PAK already inside the Asset Library".into());
    }
    let before = sha_file(&source)?;
    let legacy_id = archive_id(basename, &before);
    let old_folders = [
        root.join(&legacy_id),
        root.join(archive_directory(basename, &legacy_id)),
    ];
    let mut use_legacy_id = false;
    for old in old_folders {
        if !old.exists() {
            continue;
        }
        let doc_path = old.join("asset-library.json");
        let metadata = fs::symlink_metadata(&doc_path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > 32 * 1024 * 1024 {
            return Err("Existing imported archive manifest is unsafe".into());
        }
        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(doc_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if manifest["source"].as_str()
            .is_some_and(|old| source_path_key(old) == source_path_key(&source.to_string_lossy()))
        {
            use_legacy_id = true;
            break;
        }
    }
    // An old import may already exist under a source-scoped ID hashed
    // from the verbatim \\?\ spelling. Reuse only the same source, name and
    // recorded original SHA, with the full extracted-file verification below.
    let catalog = imported_library(&root).map_err(|e| e.to_string())?;
    let matching = catalog.archives.iter().find(|item| {
        if !item.name.eq_ignore_ascii_case(basename) ||
            source_path_key(&item.source) != source_path_key(&source.to_string_lossy()) {
            return false;
        }
        let path = Path::new(&item.directory).join("asset-library.json");
        fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file() && meta.len() <= 32 * 1024 * 1024)
            && File::open(path).ok()
                .and_then(|file| serde_json::from_reader::<_, serde_json::Value>(file).ok())
                .is_some_and(|manifest| manifest["source_sha256"].as_str() == Some(before.as_str()))
    });
    let id = matching.map(|item| item.id.clone()).unwrap_or_else(|| {
        if use_legacy_id { legacy_id } else { source_scoped_id(basename, &before, &source) }
    });
    let name = archive_directory(basename, &id);
    let target = root.join(&name);
    let legacy = root.join(&id);
    let existing = matching.map(|item| PathBuf::from(&item.directory)).or_else(|| {
        if fs::symlink_metadata(&legacy).is_ok() {
            Some(legacy)
        } else if fs::symlink_metadata(&target).is_ok() {
            Some(target.clone())
        } else {
            None
        }
    });
    if let Some(existing) = existing {
        let catalog = imported_library(&root).map_err(|e| e.to_string())?;
        let item = catalog
            .archives
            .iter()
            .find(|a| a.id == id && Path::new(&a.directory) == existing)
            .ok_or("Conflicting or unfinished Asset Library archive exists; inspect manually")?;
        let manifest_file = existing.join("asset-library.json");
        let manifest_meta = fs::symlink_metadata(&manifest_file).map_err(|e| e.to_string())?;
        if !manifest_meta.file_type().is_file() || manifest_meta.len() > 32 * 1024 * 1024 {
            return Err("Existing library manifest is unsafe".into());
        }
        let doc: serde_json::Value =
            serde_json::from_reader(File::open(&manifest_file).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if doc["source_sha256"].as_str() != Some(before.as_str())
            || doc["name"].as_str() != Some(basename)
        {
            return Err("Existing archive does not match the selected source hash".into());
        }
        let inventory = imported_archive_files(item).map_err(|e| e.to_string())?;
        let manifest_rows = doc["entries"]
            .as_array()
            .ok_or("Existing manifest is missing indexed file records")?;
        if inventory.entries.len() != manifest_rows.len() {
            return Err("Existing library manifest's file count is inconsistent".into());
        }
        for (asset, record) in inventory.entries.iter().zip(manifest_rows) {
            let expected = record["sha256"]
                .as_str()
                .ok_or("Existing library file lacks SHA-256")?;
            if expected.len() != 64
                || !expected.bytes().all(|b| b.is_ascii_hexdigit())
                || sha_file(&Path::new(&inventory.root).join(&asset.relative_path))? != expected
            {
                return Err("Existing imported files differ from the recorded SHA-256".into());
            }
        }
        return Ok(LibraryImportReport {
            id,
            archive: basename.into(),
            files: inventory.entries.len(),
            bytes: inventory.total_bytes,
            already_imported: true,
        });
    }

    // All file writes happen under a temporary, private work directory.
    // Failure or panic cleanup is owned by TempDir, leaving root imports intact.
    let work = tempfile::Builder::new()
        .prefix(".evolve-library-import-")
        .tempdir_in(&root)
        .map_err(|e| e.to_string())?;
    let extracted = work.path().join("files");
    let (entries, archive_format, verification): (Vec<(String, u64)>, &str, &str) =
        match public_key.as_deref() {
            Some(key) => {
                let info = extract_signed_pak(&source, key, signed_name, &extracted)
                    .map_err(|e| e.to_string())?;
                (
                    info.entries
                        .into_iter()
                        .map(|entry| (entry.path, entry.size))
                        .collect(),
                    "signed-crypak",
                    "rsa-pss",
                )
            }
            None => {
                let info = extract_legacy_pak(&source, &extracted).map_err(|e| e.to_string())?;
                (
                    info.entries
                        .into_iter()
                        .map(|entry| (entry.path, entry.size))
                        .collect(),
                    "legacy-zip",
                    "zip-crc32",
                )
            }
        };
    if entries.is_empty() {
        return Err("PAK did not contain any supported entries".into());
    }
    let mut records = Vec::with_capacity(entries.len());
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for (member_path, member_size) in &entries {
        let name = safe_member_name(member_path).map_err(|e| e.to_string())?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err("PAK has a case-insensitive duplicate member name".into());
        }
        let file = extracted.join(&name);
        let original_meta = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        if !original_meta.file_type().is_file() || original_meta.len() != *member_size {
            return Err("Extracted PAK member length is incorrect".into());
        }
        let mut converted = false;
        if original_meta.len() >= 8 && original_meta.len() <= XML_LIMIT as u64 {
            let mut head = [0u8; 8];
            File::open(&file)
                .and_then(|mut f| f.read_exact(&mut head))
                .map_err(|e| e.to_string())?;
            if &head == b"CryXmlB\0" {
                let raw = fs::read(&file).map_err(|e| e.to_string())?;
                let xml = xml_bytes(&raw, XML_LIMIT).map_err(|e| e.to_string())?;
                fs::write(&file, xml.as_ref()).map_err(|e| e.to_string())?;
                converted = true;
            }
        } else if original_meta.len() > XML_LIMIT as u64 {
            let mut head = [0u8; 8];
            File::open(&file)
                .and_then(|mut f| f.read_exact(&mut head))
                .map_err(|e| e.to_string())?;
            if &head == b"CryXmlB\0" {
                return Err("Oversized CryXmlB member requires explicit review".into());
            }
        }
        let bytes = fs::metadata(&file).map_err(|e| e.to_string())?.len();
        total = total.checked_add(bytes).ok_or("Extracted size overflow")?;
        records.push(serde_json::json!({
            "path": name, "bytes": bytes,
            "sha256": sha_file(&file)?, "cryxml": converted,
        }));
    }
    if sha_file(&source)? != before {
        return Err("Source PAK changed during import; no library entry was published".into());
    }
    let source_mtime_ns = source_meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let source_mtime_ns = u64::try_from(source_mtime_ns)
        .map_err(|_| "Source modification timestamp exceeds manifest limit")?;
    let imported_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let key_hash = public_key.as_deref().map(sha_file).transpose()?;
    let manifest = serde_json::json!({
        "version": 1, "name": basename, "signed_name": signed_name,
        "source": ordinary_path(&source.to_string_lossy()),
        "source_sha256": before, "source_size": source_meta.len(),
        "source_mtime_ns": source_mtime_ns,
        "public_key_sha256": key_hash,
        "archive_format": archive_format,
        "verification": verification, "imported_at": imported_at,
        "extracted_bytes": total, "warnings": [], "entries": records,
    });
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(work.path().join("asset-library.json"))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, &manifest).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    // Windows does not permit publishing a directory containing a file
    // handle that is still open in this process.
    drop(file);

    // A concurrently created target must not be replaced. Windows rename
    // refuses an existing destination; other platforms need an explicit
    // guard as well. Source and existing import directories remain untouched.
    if fs::symlink_metadata(&target).is_ok() || fs::symlink_metadata(root.join(&id)).is_ok() {
        return Err("Import already exists; refresh the Asset Library".into());
    }
    fs::rename(work.path(), &target).map_err(|e| e.to_string())?;
    Ok(LibraryImportReport {
        id,
        archive: basename.into(),
        files: records.len(),
        bytes: total,
        already_imported: false,
    })
}

/// Auto-route ZIP-compatible legacy archives without RSA; authenticate signed
/// Stage 2 archives using candidate keys. Unsupported formats stay fail-closed.
pub fn import_signed_library_pak_with_keys(
    root: &Path,
    source: &Path,
    public_keys: &[PathBuf],
) -> Result<LibraryImportReport, String> {
    if !has_stage2_signature_trailer(source)? {
        return match inspect_legacy_pak(source) {
            Ok(_) => import_library_pak(root, source, ImportFormat::LegacyZip),
            Err(error) => Err(format!(
                "Unsupported or corrupt PAK (not ZIP-compatible legacy or recognized signed CryPAK): {error}"
            )),
        };
    }
    if public_keys.is_empty() {
        return Err("Signed CryPAK requires an RSA verification key".into());
    }
    let basename = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Source PAK name is not valid Unicode")?;
    // Retail CryPAK signatures authenticate the original filename bytes.
    // The private manager accepts the physical basename and then its lowercase
    // spelling (e.g. Engine.pak versus engine.pak). Do not rename a source
    // archive or weaken the signature check to make either variant pass.
    let lowercase = basename.to_ascii_lowercase();
    let signed_names = if lowercase == basename {
        vec![basename]
    } else {
        vec![basename, lowercase.as_str()]
    };
    let mut errors = Vec::new();
    for key in public_keys {
        let mut variants = Vec::new();
        for signed_name in &signed_names {
            match crate::signed_pak::inspect_signed_pak(source, key, signed_name) {
                Ok(_) => {
                    return import_library_pak(
                        root,
                        source,
                        ImportFormat::Signed(key, signed_name),
                    );
                }
                Err(err) => variants.push(format!("{signed_name}: {err}")),
            }
        }
        errors.push(format!(
            "{}: {}",
            key.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("RSA public key"),
            variants.join(" | ")
        ));
    }
    Err(format!(
        "No matching RSA verification key/signature for {basename} ({} keys, {} signed-name variants attempted): {}",
        public_keys.len(),
        signed_names.len(),
        errors.join("; ")
    ))
}

/// Import one user-selected PAK, or the direct .pak children of one folder.
///
/// Directory selection does not recurse, follow symlinks, or silently treat
/// unrelated files as archives. At most 64 signed archives are attempted per
/// explicit request. An individual bad signature or unsupported legacy format
/// is reported without hiding any successful, independently verified imports.
pub fn import_signed_library_selection(
    root: &Path,
    source: &Path,
    public_key: &Path,
) -> Result<LibraryBatchImportReport, String> {
    import_signed_library_selection_with_keys(root, source, &[public_key.to_path_buf()])
}

pub fn import_signed_library_selection_with_keys(
    root: &Path,
    source: &Path,
    public_keys: &[PathBuf],
) -> Result<LibraryBatchImportReport, String> {
    let source_meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if source_meta.file_type().is_file() {
        return import_signed_library_pak_with_keys(root, source, public_keys).map(|report| {
            LibraryBatchImportReport {
                imported: vec![report],
                failed: Vec::new(),
            }
        });
    }
    if !source_meta.file_type().is_dir() {
        return Err("Select one regular signed PAK or a real folder containing PAKs".into());
    }
    let source_root = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let library_root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if source_root.starts_with(&library_root) {
        return Err("Cannot batch-import PAKs from inside the Asset Library".into());
    }

    let mut sources = Vec::new();
    for (index, item) in fs::read_dir(&source_root)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= 8192 {
            return Err("Source folder has too many entries; select a smaller folder".into());
        }
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pak"))
        {
            sources.push(path);
            if sources.len() > 64 {
                return Err("Batch import is limited to 64 PAKs per request".into());
            }
        }
    }
    if sources.is_empty() {
        return Err("The selected folder has no directly contained .pak files".into());
    }
    sources.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .cmp(
                &b.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            )
            .then_with(|| a.cmp(b))
    });

    let mut report = LibraryBatchImportReport {
        imported: Vec::new(),
        failed: Vec::new(),
    };
    for path in sources {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<invalid filename>".into());
        match import_signed_library_pak_with_keys(root, &path, public_keys) {
            Ok(imported) => report.imported.push(imported),
            Err(reason) => report.failed.push(LibraryImportFailure {
                archive: name,
                reason,
            }),
        }
    }
    Ok(report)
}

/// Preview the exact PAKs selected using native file or folder pickers.
///
/// Unlike the legacy 64-file, top-level-only import, this matches the private
/// manager's opt-in whole-collection scan: up to 4096 source PAKs across at
/// most 16 folder levels, no followed links and no scans within Asset Library.
/// The scanner makes no modifications and returns a deterministic import plan.
pub fn scan_library_import_sources(
    library_root: &Path,
    selections: &[PathBuf],
    recursive: bool,
) -> Result<LibraryImportPlan, String> {
    if selections.is_empty() || selections.len() > MAX_LIBRARY_COLLECTION_PAKS {
        return Err("Select 1–4,096 PAK files or one source directory".into());
    }
    let library = fs::canonicalize(library_root)
        .map_err(|_| "Configure an existing Asset Library folder in Settings → Storage")?;
    if !library.is_dir() {
        return Err("Asset Library root must be a directory".into());
    }
    let mut files = Vec::<PathBuf>::new();
    let mut seen = HashSet::<PathBuf>::new();
    let mut scanned = 0usize;
    for selected in selections {
        let info = fs::symlink_metadata(selected)
            .map_err(|e| format!("Cannot inspect source {}: {e}", selected.display()))?;
        if info.file_type().is_symlink() {
            return Err("A selected source may not be a link".into());
        }
        let chosen = fs::canonicalize(selected).map_err(|e| e.to_string())?;
        if chosen.starts_with(&library) {
            return Err("Cannot import archives from inside the Asset Library".into());
        }
        if info.file_type().is_file() {
            if !chosen
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pak"))
            {
                return Err("Select only .pak files".into());
            }
            if info.len() > MAX_SOURCE_BYTES {
                return Err("Selected PAK exceeds maximum supported archive size".into());
            }
            if seen.insert(chosen.clone()) {
                files.push(chosen);
            }
            continue;
        }
        if !info.file_type().is_dir() || selections.len() != 1 {
            return Err("Select multiple PAK files or one real source directory".into());
        }
        let mut queue = VecDeque::from([(chosen.clone(), 0usize)]);
        while let Some((folder, depth)) = queue.pop_front() {
            let entries = fs::read_dir(&folder).map_err(|e| {
                format!("Cannot scan collection directory {}: {e}", folder.display())
            })?;
            for entry in entries {
                scanned += 1;
                if scanned > MAX_COLLECTION_ENTRIES {
                    return Err("Source collection exceeds safe directory-scan limit".into());
                }
                let entry = entry.map_err(|e| e.to_string())?;
                let location = entry.path();
                let item = fs::symlink_metadata(&location).map_err(|e| e.to_string())?;
                if item.file_type().is_symlink() {
                    continue;
                }
                if item.file_type().is_dir() {
                    if recursive && depth < MAX_COLLECTION_DEPTH {
                        let next = fs::canonicalize(&location).map_err(|e| e.to_string())?;
                        if next.starts_with(&chosen) && !next.starts_with(&library) {
                            queue.push_back((next, depth + 1));
                        }
                    }
                    continue;
                }
                if !item.file_type().is_file()
                    || !location
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak"))
                {
                    continue;
                }
                if item.len() > MAX_SOURCE_BYTES {
                    return Err(format!(
                        "Selected archive exceeds size limit: {}",
                        location.display()
                    ));
                }
                let candidate = fs::canonicalize(&location).map_err(|e| e.to_string())?;
                if !candidate.starts_with(&chosen) || candidate.starts_with(&library) {
                    return Err("Source folder entry escaped its verified root".into());
                }
                if seen.insert(candidate.clone()) {
                    files.push(candidate);
                }
                if files.len() > MAX_LIBRARY_COLLECTION_PAKS {
                    return Err("More than 4,096 PAKs found; choose a narrower folder".into());
                }
            }
        }
    }
    if files.is_empty() {
        return Err("No .pak files found in the selected source".into());
    }
    files.sort_by(|a, b| {
        a.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&b.to_string_lossy().to_ascii_lowercase())
            .then_with(|| a.cmp(b))
    });
    let mut packed_bytes = 0u64;
    for source in &files {
        packed_bytes = packed_bytes
            .checked_add(fs::metadata(source).map_err(|e| e.to_string())?.len())
            .ok_or("Collection size overflow")?;
    }
    let existing = imported_library(&library)
        .map_err(|e| e.to_string())?
        .archives
        .into_iter()
        .filter_map(|archive| fs::canonicalize(archive.source).ok())
        .collect::<HashSet<_>>();
    let already_imported = files.iter().filter(|path| existing.contains(*path)).count();
    Ok(LibraryImportPlan {
        files,
        packed_bytes,
        already_imported,
    })
}

/// Import the displayed, explicit preview selection, preserving completed
/// imported PAKs if the user cancels between archives. The source files and
/// existing imports are never modified; signed files require RSA verification,
/// and ZIP-compatible files require CRC validation.
pub fn import_library_plan_with_keys(
    library_root: &Path,
    plan: &LibraryImportPlan,
    public_keys: &[PathBuf],
    cancelled: &AtomicBool,
    completed: &AtomicUsize,
) -> Result<LibraryBatchImportReport, String> {
    if plan.files.is_empty() || plan.files.len() > MAX_LIBRARY_COLLECTION_PAKS {
        return Err("Invalid or oversized collection import plan".into());
    }
    completed.store(0, Ordering::Relaxed);
    let mut report = LibraryBatchImportReport {
        imported: Vec::new(),
        failed: Vec::new(),
    };
    for path in &plan.files {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<unknown>".into());
        match import_signed_library_pak_with_keys(library_root, path, public_keys) {
            Ok(result) => report.imported.push(result),
            Err(reason) => report.failed.push(LibraryImportFailure {
                archive: name,
                reason,
            }),
        }
        completed.fetch_add(1, Ordering::Relaxed);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zip_compatible_legacy_import_does_not_require_rsa_keys() {
        use std::io::Write as _;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("LegacyGame.pak");
        let library = temp.path().join("library");
        fs::create_dir(&library).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(&source).unwrap());
        zip.start_file(
            "Models/example.cdf",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"<Character/>").unwrap();
        zip.finish().unwrap();
        let original = fs::read(&source).unwrap();

        assert!(!has_stage2_signature_trailer(&source).unwrap());
        let imported = import_signed_library_pak_with_keys(&library, &source, &[]).unwrap();
        assert_eq!(imported.files, 1);
        assert!(!imported.already_imported);
        let snapshot = imported_library(&library).unwrap();
        let archive = snapshot
            .archives
            .iter()
            .find(|a| a.id == imported.id)
            .unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(Path::new(&archive.directory).join("asset-library.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["archive_format"], "legacy-zip");
        assert_eq!(manifest["verification"], "zip-crc32");
        assert!(manifest["public_key_sha256"].is_null());
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(
            import_signed_library_pak_with_keys(&library, &source, &[])
                .unwrap()
                .already_imported
        );
    }

    #[test]
    fn identical_archives_from_two_installations_are_separate_editable_sources() {
        use std::io::Write as _;
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("Library");
        let a = temporary.path().join("LegacyModdedEvolve");
        let b = temporary.path().join("ModdedEvolve");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let first = a.join("UI_Assets.pak");
        let second = b.join("UI_Assets.pak");
        let mut zip = zip::ZipWriter::new(File::create(&first).unwrap());
        zip.start_file(
            "Libs/UI/main_menu.gfx",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"identical").unwrap();
        zip.finish().unwrap();
        fs::copy(&first, &second).unwrap();
        let first_report = import_signed_library_pak_with_keys(&library, &first, &[]).unwrap();
        let second_report = import_signed_library_pak_with_keys(&library, &second, &[]).unwrap();
        assert_ne!(first_report.id, second_report.id);
        assert_eq!(imported_library(&library).unwrap().archives.len(), 2);
        assert!(
            import_signed_library_pak_with_keys(&library, &first, &[])
                .unwrap()
                .already_imported
        );
        assert!(
            import_signed_library_pak_with_keys(&library, &second, &[])
                .unwrap()
                .already_imported
        );
    }

    #[test]
    fn signed_marker_cannot_fallback_to_unsigned_zip_import() {
        use std::io::Write as _;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("SignedCandidate.pak");
        let library = temp.path().join("library");
        fs::create_dir(&library).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(&source).unwrap());
        zip.start_file("foo.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"data").unwrap();
        zip.set_raw_comment(vec![0u8; 2320].into_boxed_slice());
        zip.finish().unwrap();
        assert!(has_stage2_signature_trailer(&source).unwrap());
        assert!(
            import_signed_library_pak_with_keys(&library, &source, &[])
                .unwrap_err()
                .contains("requires an RSA")
        );
        assert!(imported_library(&library).unwrap().archives.is_empty());
    }

    #[test]
    fn recursive_collection_scan_finds_nested_paks_but_not_links_or_other_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let source = temp.path().join("game");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(source.join("nested/more")).unwrap();
        fs::write(source.join("root.pak"), b"root").unwrap();
        fs::write(source.join("nested/more/other.PAK"), b"nested").unwrap();
        fs::write(source.join("nested/text.txt"), b"ignore").unwrap();
        let non_recursive =
            scan_library_import_sources(&library, std::slice::from_ref(&source), false).unwrap();
        assert_eq!(non_recursive.files.len(), 1);
        let recursive =
            scan_library_import_sources(&library, std::slice::from_ref(&source), true).unwrap();
        assert_eq!(recursive.files.len(), 2);
        assert_eq!(recursive.packed_bytes, 10);
        assert_eq!(recursive.already_imported, 0);
        assert_eq!(recursive.files[0].file_name().unwrap(), "other.PAK");
        assert!(
            scan_library_import_sources(&library, std::slice::from_ref(&library), true).is_err()
        );
        assert!(
            scan_library_import_sources(&library, &[source.join("nested/text.txt")], false)
                .is_err()
        );
        assert!(fs::read_dir(&library).unwrap().next().is_none());
    }

    #[test]
    fn bulk_import_is_resumable_and_cancelled_before_first_archive() {
        let temp = tempfile::tempdir().unwrap();
        let lib = temp.path().join("library");
        fs::create_dir(&lib).unwrap();
        let source = temp.path().join("file.pak");
        fs::write(&source, b"unverified").unwrap();
        let plan = scan_library_import_sources(&lib, &[source], false).unwrap();
        let cancelled = AtomicBool::new(true);
        let completed = AtomicUsize::new(12);
        let report =
            import_library_plan_with_keys(&lib, &plan, &[], &cancelled, &completed).unwrap();
        assert!(report.imported.is_empty());
        assert!(report.failed.is_empty());
        assert_eq!(completed.load(Ordering::Relaxed), 0);
        assert!(fs::read_dir(&lib).unwrap().next().is_none());
    }
    #[test]
    fn multiple_key_import_rejects_unverifiable_archive_without_side_effects() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("AssetLibrary");
        fs::create_dir(&library).unwrap();
        let source = root.path().join("original.pak");
        fs::write(&source, b"not a signed PAK").unwrap();
        assert!(import_signed_library_pak_with_keys(&library, &source, &[]).is_err());
        let missing_key = root.path().join("missing.der");
        let result = import_signed_library_pak_with_keys(&library, &source, &[missing_key]);
        assert!(result.unwrap_err().contains("Unsupported or corrupt PAK"));
        assert_eq!(fs::read_dir(&library).unwrap().count(), 0);
    }

    #[test]
    fn folder_import_is_bounded_and_does_not_recurse() {
        let home = tempfile::tempdir().unwrap();
        let library = home.path().join("AssetLibrary");
        let source = home.path().join("PAKs");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&source).unwrap();
        assert!(import_signed_library_selection(&library, &source, Path::new("unused")).is_err());
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join("nested/hidden.pak"), b"fake").unwrap();
        assert!(import_signed_library_selection(&library, &source, Path::new("unused")).is_err());
        for index in 0..65 {
            fs::write(source.join(format!("source_{index:02}.pak")), b"fake").unwrap();
        }
        assert!(
            import_signed_library_selection(&library, &source, Path::new("unused"))
                .unwrap_err()
                .contains("64 PAKs")
        );
        assert_eq!(fs::read_dir(&library).unwrap().count(), 0);
    }

    #[test]
    fn folder_import_reports_individual_failures_without_modifying_sources() {
        let home = tempfile::tempdir().unwrap();
        let library = home.path().join("AssetLibrary");
        let source = home.path().join("PAKs");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("readme.txt"), b"ignored").unwrap();
        fs::write(source.join("invalid.pak"), b"not a signed PAK").unwrap();
        let key = home.path().join("public.der");
        fs::write(&key, b"not a real verification key").unwrap();
        let report = import_signed_library_selection(&library, &source, &key).unwrap();
        assert!(report.imported.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].archive, "invalid.pak");
        assert!(!report.failed[0].reason.is_empty());
        assert_eq!(
            fs::read(source.join("invalid.pak")).unwrap(),
            b"not a signed PAK"
        );
        assert_eq!(fs::read_dir(&library).unwrap().count(), 0);
    }

    #[test]
    fn stable_private_compatible_archive_directory() {
        let source = "Characters_Hunters_Merc_Emet_Data.pak";
        let id = archive_id(source, &"0".repeat(64));
        assert_eq!(id.len(), 20);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(archive_directory(source, &id).ends_with(&format!("__{id}")));
        assert_eq!(
            archive_id("SAMPLE.pak", "abcd"),
            archive_id("sample.pak", "abcd")
        );
    }

    #[test]
    fn rejects_unsafe_sources_without_publishing_a_manifest() {
        let home = tempfile::tempdir().unwrap();
        let library = home.path().join("AssetLibrary");
        fs::create_dir(&library).unwrap();
        let fake = home.path().join("example.pak");
        fs::write(&fake, b"not a signed CryPAK").unwrap();
        let key = home.path().join("public_key.bin");
        fs::write(&key, b"not rsa").unwrap();
        assert!(import_signed_library_pak(&library, &fake, &key).is_err());
        assert_eq!(fs::read_dir(&library).unwrap().count(), 0);
        assert_eq!(fs::read(&fake).unwrap(), b"not a signed CryPAK");
        assert!(import_signed_library_pak(&library, &library.join("inside.pak"), &key).is_err());
    }
}
