//! Read-only compatibility with the private manager's persistent Asset Library.
//! This catalog is an index of imported manifests, not authority to write game
//! files or an assertion that the imported files are still byte-for-byte valid.
use crate::archive::safe_member_name;
use crate::inventory::{AssetEntry, Inventory, classify_path};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_ARCHIVES: usize = 4096;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_BUILD_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MEMBERS: usize = 250_000;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("Select an existing, non-linked Asset Library directory")]
    InvalidRoot,
    #[error("Asset Library limit exceeded")]
    Limit,
    #[error("Imported archive or manifest is missing or unsafe")]
    UnsafeArchive,
    #[error("Imported archive's extracted files are missing or changed")]
    Modified,
    #[error("Asset Library I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Manifest JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedPak {
    pub id: String,
    pub name: String,
    pub source: String,
    pub directory: String,
    pub group: String,
    pub description: String,
    pub entries: usize,
    pub extracted_bytes: u64,
    pub warnings: usize,
    pub builds: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedLibrary {
    pub root: String,
    pub archives: Vec<ImportedPak>,
    pub builds: Vec<String>,
    pub total_bytes: u64,
    pub ignored: usize,
}
#[derive(Deserialize)]
struct ManifestEntry {
    path: String,
    bytes: u64,
}
#[derive(Deserialize)]
struct Manifest {
    version: u32,
    name: String,
    source: String,
    #[serde(default)]
    extracted_bytes: u64,
    #[serde(default)]
    warnings: Vec<serde_json::Value>,
    entries: Vec<ManifestEntry>,
}
#[derive(Deserialize)]
struct BuildCollections {
    version: u32,
    builds: HashMap<String, BuildSnapshot>,
}
#[derive(Deserialize)]
struct BuildSnapshot {
    label: String,
    members: HashMap<String, serde_json::Value>,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, LibraryError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return Err(LibraryError::UnsafeArchive);
    }
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(LibraryError::Limit);
    }
    Ok(bytes)
}

fn valid_directory_id(name: &str) -> Option<&str> {
    let id = name.rsplit("__").next()?;
    if id.len() == 20 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(id)
    } else {
        None
    }
}

fn readable(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// These labels are derived only from the source PAK filename. Neither
/// dependencies nor data-vs-texture content are inferred by inspecting assets.
pub fn describe_archive(filename: &str) -> (String, String) {
    let name = filename.to_ascii_lowercase();
    let stem = name.strip_suffix(".pak").unwrap_or(&name);
    if let Some(parts) = stem.strip_prefix("characters_") {
        let mut chunks = parts.split('_');
        let kind = chunks.next().unwrap_or("Characters");
        let words: Vec<_> = chunks.collect();
        let is_texture = words.last() == Some(&"ts");
        let character = words
            .iter()
            .take(
                words
                    .len()
                    .saturating_sub(usize::from(matches!(words.last(), Some(&"data" | &"ts")))),
            )
            .map(|part| readable(part))
            .collect::<Vec<_>>()
            .join(" ");
        let label = if character.is_empty() {
            readable(kind)
        } else {
            format!("{} • {character}", readable(kind))
        };
        let description = if is_texture {
            format!("{character} Textures")
        } else if words.last() == Some(&"data") {
            format!("{character} Models / Skeletons")
        } else {
            format!("{character} Archive")
        };
        return (label, description);
    }
    if stem.starts_with("animations") {
        return ("Animations".into(), readable(stem));
    }
    if stem.starts_with("levels") || stem.contains("terrain") {
        return ("Levels".into(), readable(stem));
    }
    ("Other PAKs".into(), readable(stem))
}

fn manifests_by_build(root: &Path) -> Result<HashMap<String, Vec<String>>, LibraryError> {
    let path = root.join("build-collections.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let data: BuildCollections = serde_json::from_slice(&read_bounded(&path, MAX_BUILD_BYTES)?)?;
    if data.version != 1 {
        return Err(LibraryError::UnsafeArchive);
    }
    let mut lookup = HashMap::<String, Vec<String>>::new();
    for (id, build) in data.builds {
        if id == "unassigned" {
            continue;
        }
        if build.label.len() > 120 || build.members.len() > MAX_ARCHIVES {
            return Err(LibraryError::Limit);
        }
        for member in build.members.keys() {
            lookup
                .entry(member.clone())
                .or_default()
                .push(build.label.clone());
        }
    }
    for labels in lookup.values_mut() {
        labels.sort_unstable();
        labels.dedup();
    }
    Ok(lookup)
}

/// Inspect manifests from the established private manager's AssetLibrary.
/// No file is decrypted, extracted, written, or opened from the game directory.
pub fn imported_library(root: &Path) -> Result<ImportedLibrary, LibraryError> {
    let meta = fs::symlink_metadata(root).map_err(|_| LibraryError::InvalidRoot)?;
    if !meta.file_type().is_dir() {
        return Err(LibraryError::InvalidRoot);
    }
    let root = fs::canonicalize(root)?;
    let owners = manifests_by_build(&root)?;
    let mut archives = Vec::new();
    let mut ignored = 0usize;
    let mut total_bytes = 0u64;
    let mut dirs: Vec<_> = fs::read_dir(&root)?.collect::<Result<_, _>>()?;
    if dirs.len() > MAX_ARCHIVES + 40 {
        return Err(LibraryError::Limit);
    }
    dirs.sort_by_key(|entry| entry.file_name());
    for entry in dirs {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = valid_directory_id(&file_name) else {
            continue;
        };
        let folder = entry.path();
        let bytes = match read_bounded(&folder.join("asset-library.json"), MAX_MANIFEST_BYTES) {
            Ok(bytes) => bytes,
            Err(_) => {
                ignored += 1;
                continue;
            }
        };
        let manifest: Manifest = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => {
                ignored += 1;
                continue;
            }
        };
        if manifest.version != 1
            || manifest.entries.len() > MAX_MEMBERS
            || manifest.name.len() > 255
            || !manifest.name.to_ascii_lowercase().ends_with(".pak")
            || manifest
                .entries
                .iter()
                .any(|e| e.path.is_empty() || safe_member_name(&e.path).is_err())
        {
            ignored += 1;
            continue;
        }
        let (group, description) = describe_archive(&manifest.name);
        total_bytes = total_bytes.saturating_add(manifest.extracted_bytes);
        archives.push(ImportedPak {
            id: id.into(),
            name: manifest.name,
            source: manifest.source,
            directory: folder.to_string_lossy().into_owned(),
            group,
            description,
            entries: manifest.entries.len(),
            extracted_bytes: manifest.extracted_bytes,
            warnings: manifest.warnings.len(),
            builds: owners.get(id).cloned().unwrap_or_default(),
        });
    }
    archives.sort_by(|a, b| {
        a.group
            .to_lowercase()
            .cmp(&b.group.to_lowercase())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let mut builds = archives
        .iter()
        .flat_map(|item| item.builds.iter().cloned())
        .collect::<Vec<_>>();
    builds.sort_unstable();
    builds.dedup();
    Ok(ImportedLibrary {
        root: root.to_string_lossy().into_owned(),
        archives,
        builds,
        total_bytes,
        ignored,
    })
}

/// Validate every listed asset's location and original byte count before
/// exposing an imported PAK through the read-only editor window.
pub fn imported_archive_files(archive: &ImportedPak) -> Result<Inventory, LibraryError> {
    let base = PathBuf::from(&archive.directory);
    if !fs::symlink_metadata(&base)?.file_type().is_dir() {
        return Err(LibraryError::UnsafeArchive);
    }
    let manifest: Manifest = serde_json::from_slice(&read_bounded(
        &base.join("asset-library.json"),
        MAX_MANIFEST_BYTES,
    )?)?;
    if manifest.version != 1
        || manifest.name != archive.name
        || manifest.entries.len() > MAX_MEMBERS
    {
        return Err(LibraryError::UnsafeArchive);
    }
    let file_root = base.join("files");
    if !fs::symlink_metadata(&file_root)?.file_type().is_dir() {
        return Err(LibraryError::UnsafeArchive);
    }
    let canonical = fs::canonicalize(&file_root)?;
    let mut entries = Vec::with_capacity(manifest.entries.len());
    let mut total = 0u64;
    let mut unique = HashSet::new();
    for item in manifest.entries {
        let name = safe_member_name(&item.path).map_err(|_| LibraryError::UnsafeArchive)?;
        if !unique.insert(name.to_ascii_lowercase()) {
            return Err(LibraryError::UnsafeArchive);
        }
        let candidate = canonical.join(&name);
        let meta = fs::symlink_metadata(&candidate).map_err(|_| LibraryError::Modified)?;
        if !meta.file_type().is_file() || meta.len() != item.bytes {
            return Err(LibraryError::Modified);
        }
        if !fs::canonicalize(&candidate)?.starts_with(&canonical) {
            return Err(LibraryError::UnsafeArchive);
        }
        total = total.saturating_add(item.bytes);
        entries.push(AssetEntry {
            kind: classify_path(Path::new(&name)),
            relative_path: name,
            byte_size: item.bytes,
        });
    }
    Ok(Inventory {
        schema: 1,
        root: canonical.to_string_lossy().into_owned(),
        entries,
        total_bytes: total,
        skipped_symlinks: 0,
    })
}

/// Provenance-aware cross-PAK resolution. This is deliberately exact virtual
/// path matching, NOT basename matching or a closest-looking texture guess.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryReference {
    pub archive_id: String,
    pub archive_name: String,
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LibraryReferenceStatus {
    Missing,
    Found(LibraryReference),
    Ambiguous(Vec<LibraryReference>),
}

/// Read imported manifests on demand in a background job and check every
/// matching candidate against the read-only file location and byte length.
/// Without an explicit build label, multiple archives remain ambiguous.
pub fn resolve_imported_reference(
    root: &Path,
    authored_path: &str,
    build_label: Option<&str>,
) -> Result<LibraryReferenceStatus, LibraryError> {
    let source = safe_member_name(&authored_path.replace('\\', "/"))
        .map_err(|_| LibraryError::UnsafeArchive)?;
    let source = source.to_ascii_lowercase();
    let candidates = if source.ends_with(".dds.0") {
        vec![source]
    } else if source.ends_with(".dds") || source.ends_with(".tif") {
        let base = &source[..source.len() - 4];
        vec![format!("{base}.dds.0"), format!("{base}.dds")]
    } else {
        vec![source]
    };
    let library = imported_library(root)?;
    let mut found: Vec<Vec<LibraryReference>> = vec![Vec::new(); candidates.len()];
    for archive in &library.archives {
        if let Some(label) = build_label
            && !archive.builds.iter().any(|build| build == label)
        {
            continue;
        }
        let inv = imported_archive_files(archive)?;
        for entry in inv.entries {
            let path = entry.relative_path.to_ascii_lowercase();
            for (i, candidate) in candidates.iter().enumerate() {
                if path == *candidate {
                    found[i].push(LibraryReference {
                        archive_id: archive.id.clone(),
                        archive_name: archive.name.clone(),
                        relative_path: entry.relative_path.clone(),
                        absolute_path: PathBuf::from(&inv.root)
                            .join(&entry.relative_path)
                            .to_string_lossy()
                            .into_owned(),
                    });
                }
            }
        }
    }
    // A high-resolution .dds.0 has priority over .dds, but never prefer one
    // archive over another with the same exact relative texture path.
    let mut hits = found
        .into_iter()
        .find(|group| !group.is_empty())
        .unwrap_or_default();
    hits.sort_by(|a, b| a.archive_id.cmp(&b.archive_id));
    match hits.len() {
        0 => Ok(LibraryReferenceStatus::Missing),
        1 => Ok(LibraryReferenceStatus::Found(hits.remove(0))),
        _ => Ok(LibraryReferenceStatus::Ambiguous(hits)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_labels_are_inferred_from_names() {
        let (group, description) = describe_archive("characters_hunters_merc_emet_ts.pak");
        assert_eq!(group, "Hunters • Merc Emet");
        assert_eq!(description, "Merc Emet Textures");
        assert_eq!(describe_archive("animationsdba.pak").0, "Animations");
    }
    #[test]
    fn source_texture_lookup_fails_closed_on_cross_build_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let archive_ids = ["11111111111111111111", "22222222222222222222"];
        for (i, id) in archive_ids.iter().enumerate() {
            let name = format!("textures{i}.pak");
            let folder = root.path().join(format!("{name}__{id}"));
            fs::create_dir_all(folder.join("files/textures")).unwrap();
            fs::write(folder.join("files/textures/body.dds.0"), b"hires").unwrap();
            fs::write(
                folder.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1,"name":name,"source":"/unused",
                    "entries":[{"path":"textures/body.dds.0","bytes":5}]
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(
            root.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "builds":{
                    "stage-two":{"label":"Stage 2","members":{"11111111111111111111":{}}},
                    "legacy":{"label":"Legacy","members":{"22222222222222222222":{}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let global = resolve_imported_reference(root.path(), "textures/body.tif", None).unwrap();
        assert!(matches!(global, LibraryReferenceStatus::Ambiguous(found) if found.len() == 2));
        let scoped =
            resolve_imported_reference(root.path(), "textures/body.tif", Some("Stage 2")).unwrap();
        assert!(matches!(scoped, LibraryReferenceStatus::Found(hit)
            if hit.archive_id == archive_ids[0] && hit.relative_path == "textures/body.dds.0"));
        assert!(matches!(
            resolve_imported_reference(root.path(), "textures/other.dds", None).unwrap(),
            LibraryReferenceStatus::Missing
        ));
        assert!(resolve_imported_reference(root.path(), "../body.tif", None).is_err());
    }

    #[test]
    fn reads_existing_imports_without_changing_them() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp
            .path()
            .join("characters_hunters_merc_emet_ts.pak__1234567890abcdefabcd");
        fs::create_dir_all(folder.join("files/textures")).unwrap();
        fs::write(folder.join("files/textures/body.dds"), b"safe").unwrap();
        let contents = r#"{"version":1,"name":"characters_hunters_merc_emet_ts.pak","source":"C:/Game/emets.pak","extracted_bytes":4,"entries":[{"path":"textures/body.dds","bytes":4}]}"#;
        fs::write(folder.join("asset-library.json"), contents).unwrap();
        let catalog = imported_library(temp.path()).unwrap();
        assert_eq!(catalog.archives.len(), 1);
        assert_eq!(catalog.archives[0].entries, 1);
        assert_eq!(
            imported_archive_files(&catalog.archives[0])
                .unwrap()
                .entries
                .len(),
            1
        );
        fs::write(folder.join("files/textures/body.dds"), b"tampered").unwrap();
        assert!(matches!(
            imported_archive_files(&catalog.archives[0]),
            Err(LibraryError::Modified)
        ));
        assert_eq!(
            fs::read_to_string(folder.join("asset-library.json")).unwrap(),
            contents
        );
    }
}
