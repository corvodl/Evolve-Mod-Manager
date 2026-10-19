//! Opt-in, fail-closed Stage 2 Asset Library importer.
//! Compatible with the Qt manager's version-one AssetLibrary manifests.
//! No source PAK or previously imported library directory is modified.
use crate::archive::safe_member_name;
use crate::cryxml::xml_bytes;
use crate::library_catalog::{imported_archive_files, imported_library};
use crate::signed_pak::extract_signed_pak;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

const XML_LIMIT: usize = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024 * 1024;

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

/// Import a user-selected signed/encrypted Stage 2 PAK. Normal ZIP and legacy
/// PAK imports remain separate until their complete workflow is ported.
pub fn import_signed_library_pak(
    root: &Path,
    source: &Path,
    public_key: &Path,
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
        return Err("Select a Stage 2 .pak file".into());
    }
    let public_meta = fs::symlink_metadata(public_key).map_err(|e| e.to_string())?;
    if !public_meta.file_type().is_file() || public_meta.len() > 8192 {
        return Err("Use an existing local, regular RSA verification key".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if source.starts_with(&root) {
        return Err("Cannot import a PAK already inside the Asset Library".into());
    }
    let public_key = fs::canonicalize(public_key).map_err(|e| e.to_string())?;
    let before = sha_file(&source)?;
    let id = archive_id(basename, &before);
    let name = archive_directory(basename, &id);
    let target = root.join(&name);
    let legacy = root.join(&id);
    let existing = if fs::symlink_metadata(&legacy).is_ok() {
        Some(legacy)
    } else if fs::symlink_metadata(&target).is_ok() {
        Some(target.clone())
    } else {
        None
    };
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
    let info = extract_signed_pak(&source, &public_key, basename, &extracted)
        .map_err(|e| e.to_string())?;
    if info.entries.is_empty() {
        return Err("Signed PAK did not contain any supported entries".into());
    }
    let mut records = Vec::with_capacity(info.entries.len());
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for entry in &info.entries {
        let name = safe_member_name(&entry.path).map_err(|e| e.to_string())?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err("Signed PAK has a case-insensitive duplicate member name".into());
        }
        let file = extracted.join(&name);
        let original_meta = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        if !original_meta.file_type().is_file() || original_meta.len() != entry.size {
            return Err("Extracted signed file length is incorrect".into());
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
    let manifest = serde_json::json!({
        "version": 1, "name": basename, "signed_name": basename,
        "source": source.to_string_lossy(),
        "source_sha256": before, "source_size": source_meta.len(),
        "source_mtime_ns": source_mtime_ns,
        "public_key_sha256": sha_file(&public_key)?,
        "archive_format": "signed-crypak",
        "verification": "rsa-pss", "imported_at": imported_at,
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

/// Import one user-selected PAK, or the direct .pak children of one folder.
///
/// Directory selection does not recurse, follow symlinks, or silently treat
/// unrelated files as archives. At most 64 signed archives are attempted per
/// explicit request. An individual bad signature is reported without hiding
/// any successful, independently verified imports.
pub fn import_signed_library_selection(
    root: &Path,
    source: &Path,
    public_key: &Path,
) -> Result<LibraryBatchImportReport, String> {
    let source_meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if source_meta.file_type().is_file() {
        return import_signed_library_pak(root, source, public_key).map(|report| {
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
        match import_signed_library_pak(root, &path, public_key) {
            Ok(imported) => report.imported.push(imported),
            Err(reason) => report.failed.push(LibraryImportFailure {
                archive: name,
                reason,
            }),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
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
