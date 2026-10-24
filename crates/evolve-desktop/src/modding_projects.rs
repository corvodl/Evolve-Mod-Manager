//! Read-only project dashboard compatible with the private Qt manager's
//! mod-projects.json registry and .evolve-pak-workspace.json manifests.
//! This is a navigation/index, not a bypass for verified build preflight.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_PROJECTS: usize = 4096;
const MAX_DIRECTORIES: usize = 32768;
const MAX_ENTRIES: usize = 60000;
const MAX_HASHED_PER_WORKSPACE: u64 = 256 * 1024 * 1024;
const MAX_XML_BYTES: usize = 48 * 1024 * 1024;

fn editable_format(record: &Value) -> Result<&str, String> {
    match record.get("format").and_then(Value::as_str) {
        Some("raw") => Ok("raw"),
        Some("cryxml") => Ok("cryxml"),
        _ => Err("Unsupported editable member format".into()),
    }
}

fn serialize_project_xml(
    original: &[u8],
    edited: &[u8],
    record: &Value,
) -> Result<Vec<u8>, String> {
    let expected = record
        .get("original_sha256")
        .and_then(Value::as_str)
        .ok_or("Original binary XML digest missing")?;
    if !format!("{:x}", Sha256::digest(original)).eq_ignore_ascii_case(expected) {
        return Err("Original binary XML differs from the project manifest".into());
    }
    evolve_core::cryxml::encode_edited_xml(original, edited, MAX_XML_BYTES)
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub label: String,
    pub target: String,
    pub source_build: String,
    pub status: String,
    pub details: String,
    pub workspace: PathBuf,
    pub modified: Vec<String>,
    pub changed_known: bool,
}

fn regular(path: &Path, max: u64) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file() && m.len() <= max)
}
fn read_json(path: &Path, max: u64) -> Result<Value, String> {
    if !regular(path, max) {
        return Err("Not a safe regular metadata file".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Malformed project metadata: {e}"))
}
fn digest(path: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut reader = File::open(path)?;
    let mut data = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut data)?;
        if n == 0 {
            break;
        }
        hash.update(&data[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn safe_member(raw: &str) -> bool {
    raw.len() <= 2048
        && !raw.is_empty()
        && !raw.contains('\\')
        && !raw.contains(':')
        && raw
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && !raw.starts_with('/')
}
fn inspect_project(
    folder: &Path,
    stage: &Path,
    label: String,
    source_build: String,
    hash_budget: u64,
) -> ProjectRow {
    let mut item = ProjectRow {
        label,
        target: "Unverified source".into(),
        source_build,
        status: "Needs review".into(),
        details: String::new(),
        workspace: folder.to_path_buf(),
        modified: Vec::new(),
        changed_known: false,
    };
    let read = || -> Result<(String, Vec<String>, bool), String> {
        let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 32 * 1024 * 1024)?;
        if manifest.get("version").and_then(Value::as_u64) != Some(1)
            || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
        {
            return Err("Not a supported editable project manifest".into());
        }
        let relative = if manifest["extraction_mode"].as_str() == Some("imported-snapshot") {
            let name = manifest["target_name"]
                .as_str()
                .ok_or("Snapshot has no archive name")?;
            if super::modding_keys::signed_basename(Path::new(name))? != name {
                return Err("Imported snapshot archive name is unsafe".into());
            }
            // No game version, signing-stage, or current-source dependency
            // exists during independent asset editing.
            name.to_owned()
        } else {
            let source = manifest
                .get("source_pak")
                .and_then(Value::as_str)
                .ok_or("Missing project source archive")?;
            let source = Path::new(source);
            if !regular(source, 16 * 1024 * 1024 * 1024) {
                return Err("Original staged archive is unavailable".into());
            }
            let original = fs::canonicalize(source).map_err(|e| e.to_string())?;
            let base = fs::canonicalize(stage.join("paks"))
                .map_err(|_| "Configured signing stage is unavailable")?;
            let relative = original
                .strip_prefix(&base)
                .map_err(|_| "Project is not backed by the configured signed stage")?
                .to_string_lossy()
                .replace('\\', "/");
            if !safe_member(&relative) || !relative.to_ascii_lowercase().ends_with(".pak") {
                return Err("Unsafe staged archive identity".into());
            }
            relative
        };
        let files = fs::canonicalize(folder.join("files"))
            .map_err(|_| "Extracted project files are missing")?;
        let entries = manifest
            .get("entries")
            .and_then(Value::as_array)
            .ok_or("Missing project file records")?;
        if entries.is_empty() || entries.len() > MAX_ENTRIES {
            return Err("Project file index exceeds safe limits".into());
        }
        let mut changed = Vec::new();
        let mut checked = 0u64;
        let mut known = true;
        let mut seen = HashSet::new();
        // Inspect GFX movie edits first. UI PAKs may contain hundreds of MiB
        // of unrelated assets; otherwise a main_menu.gfx edit near the end
        // of the manifest is invisible in the bounded dashboard scan.
        let gfx_member = |entry: &&Value| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| p.to_ascii_lowercase().ends_with(".gfx"))
        };
        for entry in entries
            .iter()
            .filter(|entry| gfx_member(entry))
            .chain(entries.iter().filter(|entry| !gfx_member(entry)))
        {
            let name = entry
                .get("path")
                .and_then(Value::as_str)
                .ok_or("Invalid project entry")?;
            if !safe_member(name) || !seen.insert(name.to_ascii_lowercase()) {
                return Err("Unsafe or duplicate project file name".into());
            }
            let expected = entry
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or("Missing original file digest")?;
            if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("Invalid original file digest".into());
            }
            let path = folder.join("files").join(name);
            if !regular(&path, 16 * 1024 * 1024 * 1024) {
                known = false;
                continue;
            }
            let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if !canonical.starts_with(&files) {
                return Err("Project file escaped its workspace".into());
            }
            let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
            if checked.saturating_add(size) > hash_budget {
                known = false;
                continue;
            }
            checked += size;
            if !digest(&path)
                .map_err(|e| e.to_string())?
                .eq_ignore_ascii_case(expected)
            {
                changed.push(name.to_owned());
            }
        }
        Ok((relative, changed, known))
    };
    match read() {
        Ok((target, modified, known)) => {
            item.target = target;
            item.status = if !modified.is_empty() {
                "Modified"
            } else if !known {
                "Needs review"
            } else {
                "Draft"
            }
            .into();
            item.details = if known {
                format!("{} changed asset(s)", modified.len())
            } else {
                format!(
                    "{} changed asset(s) found; other files exceed fast-scan budget and will be fully verified during Build Mod",
                    modified.len()
                )
            };
            item.modified = modified;
            item.changed_known = known;
        }
        Err(error) => item.details = error,
    }
    item
}

/// Includes registered projects and legacy manifests up to four directories
/// deep; never follows symlinks/junctions or scans into backup/build folders.
pub fn discover_projects(root: &Path, stage: &Path) -> Result<Vec<ProjectRow>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    if fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Projects directory must not be a symbolic link".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let registry = root.join("mod-projects.json");
    let mut registered: HashMap<PathBuf, (String, String)> = HashMap::new();
    if registry.exists() {
        let meta = read_json(&registry, 4 * 1024 * 1024)?;
        if meta.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("Unsupported private-manager project catalog version".into());
        }
        let projects = meta
            .get("projects")
            .and_then(Value::as_object)
            .ok_or("Project catalog is malformed")?;
        if projects.len() > MAX_PROJECTS {
            return Err("Too many registered projects".into());
        }
        for row in projects.values() {
            if let Some(source) = row.get("workspace").and_then(Value::as_str) {
                let path = Path::new(source);
                if !path.is_dir()
                    || fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_dir())
                {
                    continue;
                }
                if let Ok(valid) = fs::canonicalize(path)
                    && valid.starts_with(&root)
                    && valid != root
                {
                    let label = row.get("label").and_then(Value::as_str).unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Project")
                    });
                    let build = row
                        .get("source_build_id")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown build");
                    registered.insert(
                        valid,
                        (
                            label.chars().take(120).collect(),
                            build.chars().take(120).collect(),
                        ),
                    );
                }
            }
        }
    }
    let mut folders: HashSet<PathBuf> = registered.keys().cloned().collect();
    let mut queue = vec![(root.clone(), 0usize)];
    let mut visited = 0usize;
    while let Some((folder, depth)) = queue.pop() {
        visited += 1;
        if visited > MAX_DIRECTORIES {
            return Err("Project scan exceeded directory limit".into());
        }
        if regular(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024) {
            if folder != root {
                folders.insert(folder);
            }
            continue;
        }
        if depth >= 4 {
            continue;
        }
        for item in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            if !item.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let name = item.file_name().to_string_lossy().to_ascii_lowercase();
            if ["built", "backups", "editorbackups", ".build", ".git"].contains(&name.as_str()) {
                continue;
            }
            queue.push((item.path(), depth + 1));
        }
    }
    if folders.len() > MAX_PROJECTS {
        return Err("Project count exceeds safe limit".into());
    }
    let mut projects: Vec<_> = folders
        .into_iter()
        .map(|path| {
            let (label, build) = registered.remove(&path).unwrap_or_else(|| {
                (
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    "Unknown build".into(),
                )
            });
            inspect_project(&path, stage, label, build, MAX_HASHED_PER_WORKSPACE)
        })
        .collect();
    projects.sort_by(|a, b| {
        a.label
            .to_ascii_lowercase()
            .cmp(&b.label.to_ascii_lowercase())
    });
    Ok(projects)
}

/// Editing a previously imported archive never needs the source game PAK
/// or a signing stage. Work from the immutable, verified Asset Library
/// extraction into a new private project; launchability is checked later.
pub fn edit_imported_snapshot(
    pak: &evolve_core::ImportedPak,
    projects: &Path,
) -> Result<(PathBuf, usize), String> {
    let inventory = evolve_core::imported_archive_files(pak).map_err(|e| e.to_string())?;
    if !projects.is_absolute() {
        return Err("Configure an absolute Projects directory before editing".into());
    }
    if !projects.exists() {
        fs::create_dir_all(projects)
            .map_err(|e| format!("Cannot create Projects directory: {e}"))?;
    }
    if !fs::symlink_metadata(projects).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Projects must be a real directory".into());
    }
    let base = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    let imports = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let original = read_json(
        &Path::new(&pak.directory).join("asset-library.json"),
        32 * 1024 * 1024,
    )?;
    let expected = original["entries"].as_array().ok_or("Imported file list missing")?;
    if expected.len() != inventory.entries.len() || expected.is_empty() || expected.len() > MAX_ENTRIES {
        return Err("Imported archive has an invalid file index".into());
    }
    let staging = tempfile::Builder::new()
        .prefix(".editable-snapshot-")
        .tempdir_in(&base)
        .map_err(|e| e.to_string())?;
    let extracted = staging.path().join("files");
    fs::create_dir(&extracted).map_err(|e| e.to_string())?;
    let mut records = Vec::with_capacity(inventory.entries.len());
    for asset in &inventory.entries {
        let rel = asset.relative_path.as_str();
        if !safe_member(rel) {
            return Err("Unsafe imported archive member".into());
        }
        let source = imports.join(rel);
        let actual = fs::canonicalize(&source).map_err(|e| e.to_string())?;
        if !actual.starts_with(&imports) || !regular(&source, 16 * 1024 * 1024 * 1024) {
            return Err("Imported file escaped the Asset Library".into());
        }
        let copied = extracted.join(rel);
        fs::create_dir_all(copied.parent().ok_or("Invalid copy destination")?)
            .map_err(|e| e.to_string())?;
        fs::copy(&source, &copied).map_err(|e| e.to_string())?;
        let checksum = digest(&copied).map_err(|e| e.to_string())?;
        let original_record = expected.iter()
            .find(|row| row["path"].as_str() == Some(rel))
            .ok_or("Imported archive manifest entry missing")?;
        if let Some(recorded) = original_record["sha256"].as_str()
            && !recorded.eq_ignore_ascii_case(&checksum)
        {
            return Err(format!("Imported original was modified: {rel}"));
        }
        records.push(serde_json::json!({
            "path": rel,
            "archive_name": rel,
            "format": if original_record["converted"].as_bool() == Some(true) {
                "cryxml"
            } else {
                "raw"
            },
            "sha256": checksum,
        }));
    }
    let manifest = serde_json::json!({
        "version": 1,
        "extraction_mode": "imported-snapshot",
        "target_name": pak.name,
        "source_pak": null,
        "signed_basename": original["signed_name"].as_str().unwrap_or(&pak.name),
        "imported_source": pak.source,
        "imported_archive_id": pak.id,
        "imported_source_sha256": original["source_sha256"],
        "deployment_mapped": false,
        "entries": records,
    });
    let mut file = fs::OpenOptions::new()
        .write(true).create_new(true)
        .open(staging.path().join(".evolve-pak-workspace.json"))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, &manifest).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let stem: String = pak.name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(70).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?.as_nanos();
    let destination = base.join(format!("{stem}_editable_{stamp:x}"));
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("An editing project already exists at the destination".into());
    }
    fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
    Ok((destination, inventory.entries.len()))
}

/// Extract a selected, verified staged CryPAK into a NEW private-style project.
/// Extract a new project, decoding CryXmlB to editable UTF-8 XML. Both
/// editable and original binary digests are retained; source PAKs are untouched.
pub fn unpack_raw_project(
    projects: &Path,
    stage: &Path,
    source: &Path,
) -> Result<(PathBuf, usize), String> {
    let basename = super::modding_keys::signed_basename(source)?;
    unpack_raw_project_with_signed_name(projects, stage, source, &basename)
}

/// Extract an authenticated staged editing copy even when the original
/// recorded signed spelling differs in ASCII case from its physical name.
pub fn unpack_raw_project_with_signed_name(
    projects: &Path,
    stage: &Path,
    source: &Path,
    signed_name: &str,
) -> Result<(PathBuf, usize), String> {
    let _target = super::modding_keys::staged_target(stage, source)?;
    let basename = super::modding_keys::signed_basename(source)?;
    if signed_name != basename && signed_name != basename.to_ascii_lowercase() {
        return Err("Signed archive name differs from the verified physical basename".into());
    }
    let public = super::modding_keys::public_key(stage)?;
    // A first launch normally has a configured default Projects path but no
    // directory yet. Create that workspace on demand, rather than making the
    // user import PAKs again or manually initialize an empty projects folder.
    if !projects.is_absolute() {
        return Err("Configure an absolute Projects directory before editing".into());
    }
    if fs::symlink_metadata(projects).is_err() {
        fs::create_dir_all(projects)
            .map_err(|e| format!("Cannot create Projects directory: {e}"))?;
    }
    if !fs::symlink_metadata(projects).is_ok_and(|meta| meta.file_type().is_dir()) {
        return Err("Projects must be a real directory, not a symbolic link".into());
    }
    let canonical_projects = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    let before = digest(source).map_err(|e| e.to_string())?;
    let safe_stem: String = basename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(90)
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let output = canonical_projects.join(format!("{}_editable_{stamp:x}", safe_stem));
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let sentinel = output.join(".EXTRACTION-INCOMPLETE.txt");
    fs::write(
        &sentinel,
        b"Extraction is incomplete; do not build from this project.\n",
    )
    .map_err(|e| e.to_string())?;
    // The core's signed extractor uses an atomic temporary directory. It
    // refuses to merge results with pre-existing assets or follow bad paths.
    let info = evolve_core::extract_signed_pak(source, &public, signed_name, &output.join("files"))
        .map_err(|e| e.to_string())?;
    if info.entries.len() > 60_000 {
        return Err("Project exceeds supported manifest entry limit".into());
    }
    let mut records = Vec::with_capacity(info.entries.len());
    for entry in &info.entries {
        if !safe_member(&entry.path) {
            return Err("Invalid extracted archive virtual path".into());
        }
        let asset = output.join("files").join(&entry.path);
        if !fs::symlink_metadata(&asset).is_ok_and(|m| m.file_type().is_file()) {
            return Err("An extracted archive member is missing".into());
        }
        let original_sha256 = digest(&asset).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 8];
        let read = File::open(&asset)
            .and_then(|mut f| f.read(&mut magic))
            .map_err(|e| e.to_string())?;
        let format = if read == magic.len() && &magic == b"CryXmlB\0" {
            if fs::metadata(&asset).map_err(|e| e.to_string())?.len() > MAX_XML_BYTES as u64 {
                return Err("Binary XML exceeds the editable document limit".into());
            }
            let binary = fs::read(&asset).map_err(|e| e.to_string())?;
            let text = evolve_core::cryxml::xml_bytes(&binary, MAX_XML_BYTES)
                .map_err(|e| e.to_string())?;
            // Establish representability before replacing the extracted copy.
            evolve_core::cryxml::encode_edited_xml(&binary, &text, MAX_XML_BYTES)
                .map_err(|e| e.to_string())?;
            let backup = output
                .join("EditorBackups/extracted-original")
                .join(&entry.path);
            fs::create_dir_all(backup.parent().ok_or("Missing backup parent")?)
                .map_err(|e| e.to_string())?;
            fs::write(&backup, &text).map_err(|e| e.to_string())?;
            fs::write(&asset, &text).map_err(|e| e.to_string())?;
            "cryxml"
        } else {
            "raw"
        };
        let sha256 = digest(&asset).map_err(|e| e.to_string())?;
        records.push(serde_json::json!({
            "path":entry.path,
            "archive_name":entry.path,
            "format":format,
            "sha256":sha256,
            "original_sha256":original_sha256,
            "method":entry.method
        }));
    }
    if before != digest(source).map_err(|e| e.to_string())? {
        return Err("Original staged PAK changed during extraction".into());
    }
    let manifest = serde_json::json!({
        "version":1,
        "source_pak":fs::canonicalize(source).map_err(|e| e.to_string())?,
        "source_sha256":before,
        "signed_basename":signed_name,
        "public_key_sha256":digest(&public).map_err(|e| e.to_string())?,
        "archive_entry_count":info.entries.len(),
        "entries":records,
        "extraction_mode":"editable"
    });
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(".evolve-pak-workspace.json"))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut writer, &manifest).map_err(|e| e.to_string())?;
    writer.sync_all().map_err(|e| e.to_string())?;
    fs::remove_file(sentinel).map_err(|e| e.to_string())?;
    Ok((output, info.entries.len()))
}

/// Build changed raw/XML project members into a single signed PAK after
/// validating original payload identity and binary XML semantic round trips.
pub fn build_project_raw(
    project: &ProjectRow,
    projects: &Path,
    stage: &Path,
) -> Result<(String, PathBuf, ProjectRow), String> {
    // The Modding dashboard uses a bounded scan for responsiveness. Never
    // trust its cached change list to build: refresh every member on a blocking
    // worker so large PAKs and newly applied GFX edits cannot be missed.
    let parent = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    if !fs::symlink_metadata(projects).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Projects path must be a real directory".into());
    }
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if !folder.starts_with(&parent) || folder == parent {
        return Err("Selected workspace is outside Projects".into());
    }
    let verified = inspect_project(
        &folder,
        stage,
        project.label.clone(),
        project.source_build.clone(),
        u64::MAX,
    );
    if !verified.changed_known
        || verified.modified.is_empty()
        || verified.modified.len() > 64
        || verified.status != "Modified"
    {
        return Err(format!(
            "Signed build requires 1–64 fully verified changed files; {}. Save GFX edits with Apply to Mod Project before building.",
            verified.details
        ));
    }
    let modified = &verified.modified;
    let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
    let records = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Original manifest has no file records")?;
    for rel in modified {
        if !safe_member(rel) {
            return Err("Unsafe modified member".into());
        }
        let expected = records
            .iter()
            .find(|entry| entry.get("path").and_then(Value::as_str) == Some(rel.as_str()))
            .ok_or("Modified member is absent from the verified original manifest")?;
        editable_format(expected)?;
    }
    let original = stage.join("paks").join(&project.target);
    let relative = super::modding_keys::staged_target(stage, &original)?;
    if relative != project.target {
        return Err("Manifest target and staged original disagree".into());
    }
    let manifest_source = manifest
        .get("source_pak")
        .and_then(Value::as_str)
        .ok_or("Original manifest source path missing")?;
    if fs::canonicalize(manifest_source).map_err(|e| e.to_string())?
        != fs::canonicalize(&original).map_err(|e| e.to_string())?
    {
        return Err("Workspace source archive differs from the selected staged PAK".into());
    }
    let baseline = manifest
        .get("source_sha256")
        .and_then(Value::as_str)
        .ok_or("Source archive identity digest missing")?;
    if !digest(&original)
        .map_err(|e| e.to_string())?
        .eq_ignore_ascii_case(baseline)
    {
        return Err("Source PAK changed since extraction; refusing to build".into());
    }
    let keys = super::modding_keys::signing_keys(stage)?;
    if let Some(hash) = manifest.get("public_key_sha256").and_then(Value::as_str)
        && !digest(&keys.public)
            .map_err(|e| e.to_string())?
            .eq_ignore_ascii_case(hash)
    {
        return Err("Stage signing key no longer matches the original extracted archive".into());
    }
    let basename = super::modding_keys::signed_basename(&original)?;
    let signed_name = manifest
        .get("signed_basename")
        .and_then(Value::as_str)
        .unwrap_or(&basename);
    if signed_name != basename && signed_name != basename.to_ascii_lowercase() {
        return Err("Archive signed basename is not an authenticated filename variant".into());
    }
    let source_files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let original_index =
        evolve_core::open_verified_signed_pak(&original, &keys.public, signed_name)
            .map_err(|e| e.to_string())?;
    let serialized = tempfile::Builder::new()
        .prefix(".project-serialization-")
        .tempdir_in(&folder)
        .map_err(|e| e.to_string())?;
    let mut replacements = Vec::with_capacity(modified.len());
    for rel in modified {
        let replacement = folder.join("files").join(rel);
        let canonical = fs::canonicalize(&replacement).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&source_files)
            || !fs::symlink_metadata(&replacement).is_ok_and(|m| m.file_type().is_file())
        {
            return Err("Replacement is not a regular file inside the selected project".into());
        }
        let record = records
            .iter()
            .find(|entry| entry.get("path").and_then(Value::as_str) == Some(rel.as_str()))
            .ok_or("Modified member is absent from manifest")?;
        if editable_format(record)? == "cryxml" {
            if !regular(&replacement, MAX_XML_BYTES as u64) {
                return Err("Edited XML exceeds document limit".into());
            }
            let prior = original_index
                .read_member(&original, rel, MAX_XML_BYTES as u64)
                .map_err(|e| e.to_string())?;
            let encoded = serialize_project_xml(
                &prior,
                &fs::read(&replacement).map_err(|e| e.to_string())?,
                record,
            )?;
            let staged = serialized
                .path()
                .join(format!("member-{}.bin", replacements.len()));
            fs::write(&staged, encoded).map_err(|e| e.to_string())?;
            replacements.push((rel.clone(), staged));
        } else {
            replacements.push((rel.clone(), replacement));
        }
    }
    let built = parent.join("Built");
    if !built.exists() {
        fs::create_dir(&built).map_err(|e| e.to_string())?;
    }
    if !fs::symlink_metadata(&built).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Built output folder cannot be a symlink".into());
    }
    let number = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let output_folder = built.join(format!("Rust-Raw-{:x}", number));
    fs::create_dir(&output_folder).map_err(|e| e.to_string())?;
    let output = output_folder.join(&basename);
    // The core signs using the authenticated original basename. If retail
    // CryPAK uses ASCII lowercase while the installed file uses mixed case,
    // first publish the signed file under that verified spelling, then move
    // it to its original physical basename within the same new output folder.
    let signed_output = output_folder.join(signed_name);
    let result = evolve_core::rebuild_signed_pak_entries(
        &original,
        &keys.public,
        &keys.private,
        &keys.public,
        signed_name,
        &replacements,
        &signed_output,
    );
    match result {
        Ok(reports) => {
            if signed_output != output {
                fs::rename(&signed_output, &output).map_err(|e| {
                    format!("Cannot restore the original PAK filename after signing: {e}")
                })?;
            }
            Ok((
                format!("{} modified member(s)", reports.len()),
                output,
                verified,
            ))
        }
        Err(error) => {
            // The core uses an atomic tempfile on failure. Remove only our
            // new empty output directory, never an existing user's path.
            let _ = fs::remove_dir(output_folder);
            Err(error.to_string())
        }
    }
}

/// Import explicitly selected bytes/XML over ONE manifest-listed editable
/// member. Never writes source PAKs or imported library originals. Keeps a
/// verified, immutable editor backup for manual or in-app recovery.
pub fn import_raw_member(
    project: &ProjectRow,
    projects_root: &Path,
    member: &str,
    replacement_source: &Path,
) -> Result<bool, String> {
    import_raw_member_checked(project, projects_root, member, replacement_source, None)
}
fn import_raw_member_checked(
    project: &ProjectRow,
    projects_root: &Path,
    member: &str,
    replacement_source: &Path,
    expected_current: Option<&[u8]>,
) -> Result<bool, String> {
    const MAX_BYTES: u64 = 512 * 1024 * 1024;
    if !safe_member(member)
        || !regular(replacement_source, MAX_BYTES)
        || !regular(
            &project.workspace.join(".evolve-pak-workspace.json"),
            8 * 1024 * 1024,
        )
    {
        return Err(
            "Select an existing, bounded replacement and manifest-listed project file".into(),
        );
    }
    let root = fs::canonicalize(projects_root).map_err(|e| e.to_string())?;
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if folder == root || !folder.starts_with(&root) {
        return Err("Selected project is not managed by the Projects root".into());
    }
    let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
    if manifest.get("version").and_then(Value::as_u64) != Some(1)
        || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
    {
        return Err("Project is reference-only or its manifest version is unsupported".into());
    }
    let record = manifest
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|records| {
            records
                .iter()
                .find(|e| e.get("path").and_then(Value::as_str) == Some(member))
        })
        .ok_or("Selected file is absent from the managed manifest")?;
    if editable_format(record)? == "cryxml" {
        if !regular(replacement_source, MAX_XML_BYTES as u64) {
            return Err("Edited XML exceeds document limit".into());
        }
        evolve_core::cryxml::encode_xml(
            &fs::read(replacement_source).map_err(|e| e.to_string())?,
            MAX_XML_BYTES,
        )
        .map_err(|e| e.to_string())?;
    }
    let files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let original = folder.join("files").join(member);
    if !regular(&original, MAX_BYTES) {
        return Err("Selected extracted file is missing or exceeds 512 MiB".into());
    }
    let original = fs::canonicalize(&original).map_err(|e| e.to_string())?;
    if !original.starts_with(&files) {
        return Err("Selected extracted file escaped the workspace".into());
    }
    let source = fs::canonicalize(replacement_source).map_err(|e| e.to_string())?;
    if source == original || source.starts_with(&folder) {
        return Err("Replacement must be an independent file outside this managed project".into());
    }
    let old_digest = digest(&original).map_err(|e| e.to_string())?;
    if expected_current.is_some_and(|bytes| format!("{:x}", Sha256::digest(bytes)) != old_digest) {
        return Err("File changed since opening; no changes were written".into());
    }
    let new_digest = digest(&source).map_err(|e| e.to_string())?;
    if old_digest == new_digest {
        return Ok(false);
    }

    let backup_root = folder.join("EditorBackups");
    if backup_root.exists() {
        if !fs::symlink_metadata(&backup_root).is_ok_and(|meta| meta.file_type().is_dir()) {
            return Err("Refusing linked/non-directory EditorBackups".into());
        }
    } else {
        fs::create_dir(&backup_root).map_err(|e| e.to_string())?;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let mut revision = None;
    for sequence in 0..10 {
        let folder = backup_root.join(format!("edit-{stamp:x}-{sequence}"));
        match fs::create_dir(&folder) {
            Ok(()) => {
                revision = Some(folder);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let revision = revision.ok_or("Unable to allocate unique editor revision")?;
    let backup = revision.join(member);
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&original, &backup).map_err(|e| e.to_string())?;
    if digest(&backup).map_err(|e| e.to_string())? != old_digest {
        return Err("Editor backup verification failed; extracted file untouched".into());
    }
    let parent = original.parent().ok_or("Invalid extracted asset parent")?;
    let mut pending = tempfile::Builder::new()
        .prefix(".evolve-import-")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?;
    let mut input = File::open(&source).map_err(|e| e.to_string())?;
    io::copy(&mut input, pending.as_file_mut()).map_err(|e| e.to_string())?;
    pending
        .as_file_mut()
        .sync_all()
        .map_err(|e| e.to_string())?;
    if digest(pending.path()).map_err(|e| e.to_string())? != new_digest {
        return Err("Staged import digest differs from replacement source".into());
    }
    if digest(&original).map_err(|e| e.to_string())? != old_digest {
        return Err("Selected extracted file changed during editing; backup retained".into());
    }
    pending
        .persist(&original)
        .map_err(|e| e.error.to_string())?;
    if digest(&original).map_err(|e| e.to_string())? != new_digest {
        return Err("Imported file did not verify; original saved under EditorBackups".into());
    }
    Ok(true)
}

/// Restore one original, manifest-verified editable workspace member from a
/// trustworthy EditorBackups revision. Unlike 'reload', this makes a real,
/// reversible change and never modifies the signed source PAK.
pub fn restore_original_raw_member(
    project: &ProjectRow,
    projects_root: &Path,
    member: &str,
) -> Result<bool, String> {
    const MAX_BYTES: u64 = 512 * 1024 * 1024;
    const MAX_REVISIONS: usize = 4096;
    if !safe_member(member) {
        return Err("Invalid extracted member path".into());
    }
    let root = fs::canonicalize(projects_root).map_err(|e| e.to_string())?;
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if folder == root || !folder.starts_with(&root) {
        return Err("Selected project is outside the managed Projects root".into());
    }
    let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
    if manifest.get("version").and_then(Value::as_u64) != Some(1)
        || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
    {
        return Err("Read-only or unsupported project manifest".into());
    }
    let record = manifest
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("path").and_then(Value::as_str) == Some(member))
        })
        .ok_or("The member is not in this project's original manifest")?;
    editable_format(record)?;
    let baseline = record
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or("Original extracted digest missing")?;
    if baseline.len() != 64 || !baseline.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Original extracted digest is invalid".into());
    }
    let files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let target = folder.join("files").join(member);
    if !regular(&target, MAX_BYTES) {
        return Err("Selected extracted file is not a regular bounded file".into());
    }
    let current = fs::canonicalize(&target).map_err(|e| e.to_string())?;
    if !current.starts_with(&files) {
        return Err("Selected extracted file resolves outside the project".into());
    }
    let present_digest = digest(&current).map_err(|e| e.to_string())?;
    if present_digest.eq_ignore_ascii_case(baseline) {
        return Ok(false);
    }
    let backups = folder.join("EditorBackups");
    if !fs::symlink_metadata(&backups).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Verified original backup is unavailable".into());
    }
    let mut revisions = Vec::new();
    for entry in fs::read_dir(&backups).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if revisions.len() >= MAX_REVISIONS {
            return Err("Editor revision count exceeds safety budget".into());
        }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            revisions.push(entry.path());
        }
    }
    revisions.sort();
    let mut found = None;
    for revision in revisions {
        let candidate = revision.join(member);
        if !regular(&candidate, MAX_BYTES) {
            continue;
        }
        let clean = fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
        if !clean.starts_with(&backups) {
            continue;
        }
        if digest(&clean)
            .map_err(|e| e.to_string())?
            .eq_ignore_ascii_case(baseline)
        {
            found = Some(clean);
            break;
        }
    }
    let original = found.ok_or("No SHA-verified extracted original exists in EditorBackups")?;
    // Snapshot the current edited bytes first; a restore is also undoable.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let mut revision = None;
    for sequence in 0..10 {
        let dir = backups.join(format!("restore-{stamp:x}-{sequence}"));
        match fs::create_dir(&dir) {
            Ok(()) => {
                revision = Some(dir);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let revision = revision.ok_or("Could not allocate a unique restore revision")?;
    let saved = revision.join(member);
    if let Some(parent) = saved.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&current, &saved).map_err(|e| e.to_string())?;
    if digest(&saved).map_err(|e| e.to_string())? != present_digest {
        return Err("Edited revision backup did not verify; no restoration performed".into());
    }
    let parent = current.parent().ok_or("Target parent is missing")?;
    let mut pending = tempfile::Builder::new()
        .prefix(".evolve-restore-")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?;
    let mut input = File::open(&original).map_err(|e| e.to_string())?;
    io::copy(&mut input, pending.as_file_mut()).map_err(|e| e.to_string())?;
    pending
        .as_file_mut()
        .sync_all()
        .map_err(|e| e.to_string())?;
    if !digest(pending.path())
        .map_err(|e| e.to_string())?
        .eq_ignore_ascii_case(baseline)
        || digest(&current).map_err(|e| e.to_string())? != present_digest
        || !digest(&original)
            .map_err(|e| e.to_string())?
            .eq_ignore_ascii_case(baseline)
    {
        return Err(
            "Source or target changed during verified restoration; revision retained".into(),
        );
    }
    pending.persist(&current).map_err(|e| e.error.to_string())?;
    if !digest(&current)
        .map_err(|e| e.to_string())?
        .eq_ignore_ascii_case(baseline)
    {
        return Err(
            "Restoration verification failed; edited revision remains in EditorBackups".into(),
        );
    }
    Ok(true)
}

/// Enumerate editable assets by virtual path, without exposing archive internals.
pub fn guided_members(project: &ProjectRow, models: bool) -> Result<Vec<String>, String> {
    let manifest = read_json(
        &project.workspace.join(".evolve-pak-workspace.json"),
        32 * 1024 * 1024,
    )?;
    if manifest.get("version").and_then(Value::as_u64) != Some(1)
        || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
    {
        return Err("Not an editable project".into());
    }
    let entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Missing project entries")?;
    if entries.len() > MAX_ENTRIES {
        return Err("Too many project entries".into());
    }
    let mut paths = Vec::new();
    for e in entries {
        let p = e
            .get("path")
            .and_then(Value::as_str)
            .ok_or("Missing member path")?;
        if !safe_member(p) {
            return Err("Unsafe member path".into());
        }
        if if models {
            evolve_core::mod_safety::model_document(p)
        } else {
            evolve_core::mod_safety::gameplay_document(p)
        } {
            paths.push(p.to_owned());
        }
    }
    paths.sort_by_key(|p| p.to_ascii_lowercase());
    Ok(paths)
}

pub fn read_gameplay_document(project: &ProjectRow, member: &str) -> Result<Vec<u8>, String> {
    if !guided_members(project, false)?.iter().any(|p| p == member) {
        return Err("Not an indexed gameplay document".into());
    }
    let files = fs::canonicalize(project.workspace.join("files")).map_err(|e| e.to_string())?;
    let path = project.workspace.join("files").join(member);
    if !regular(&path, evolve_core::gameplay_values::LIMIT as u64)
        || !fs::canonicalize(&path)
            .map_err(|e| e.to_string())?
            .starts_with(&files)
    {
        return Err(
            "Gameplay document is missing, linked, outside the project, or too large".into(),
        );
    }
    fs::read(path).map_err(|e| e.to_string())
}

pub fn save_gameplay_document(
    project: &ProjectRow,
    projects_root: &Path,
    member: &str,
    snapshot: &[u8],
    fields: &[evolve_core::gameplay_values::ValueField],
) -> Result<bool, String> {
    use std::io::Write;
    if read_gameplay_document(project, member)? != snapshot {
        return Err(
            "File changed since opening. Reopen it before saving; no changes were written.".into(),
        );
    }
    let bytes = evolve_core::gameplay_values::apply(snapshot, fields)?;
    let mut replacement = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    replacement.write_all(&bytes).map_err(|e| e.to_string())?;
    replacement.flush().map_err(|e| e.to_string())?;
    import_raw_member_checked(
        project,
        projects_root,
        member,
        replacement.path(),
        Some(snapshot),
    )
}

/// Read only the bounded imported manifest for guided navigation; creation still
/// goes through the full original-file and signing-stage verification path.
pub fn guided_import_members(
    archive: &evolve_core::library_catalog::ImportedPak,
    models: bool,
) -> Result<Vec<String>, String> {
    let manifest = read_json(
        &Path::new(&archive.directory).join("asset-library.json"),
        32 * 1024 * 1024,
    )?;
    if manifest.get("version").and_then(Value::as_u64) != Some(1)
        || manifest.get("source").and_then(Value::as_str) != Some(archive.source.as_str())
    {
        return Err("Imported source identity changed".into());
    }
    let entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Missing imported members")?;
    if entries.len() > MAX_ENTRIES {
        return Err("Imported member limit".into());
    }
    let mut paths = Vec::new();
    for entry in entries {
        let p = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or("Missing member path")?;
        if !safe_member(p) {
            return Err("Unsafe imported member".into());
        }
        if if models {
            evolve_core::mod_safety::model_document(p)
        } else {
            evolve_core::mod_safety::gameplay_document(p)
        } {
            paths.push(p.to_owned());
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_gfx_is_found_by_full_build_scan_after_dashboard_budget_exceeded() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("staged");
        let original_pak = stage.join("paks/ui_data.pak");
        fs::create_dir_all(original_pak.parent().unwrap()).unwrap();
        fs::write(&original_pak, b"source baseline").unwrap();
        let project = temp.path().join("Projects/Test_B");
        let member = "Libs/UI/main_menu.gfx";
        let asset = project.join("files").join(member);
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(&asset, b"new main menu Test B").unwrap();
        let manifest = serde_json::json!({
            "version": 1,
            "source_pak": original_pak,
            "entries": [{
                "path": member, "format": "raw",
                "sha256": format!("{:x}", Sha256::digest(b"unmodified main menu")),
            }]
        });
        fs::write(
            project.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let limited = inspect_project(&project, &stage, "Test B".into(), "".into(), 8);
        assert!(!limited.changed_known);
        assert!(limited.modified.is_empty());
        let full = inspect_project(&project, &stage, "Test B".into(), "".into(), u64::MAX);
        assert_eq!(full.status, "Modified");
        assert!(full.changed_known);
        assert_eq!(full.modified, vec![member]);
    }

    #[test]
    fn guided_values_save_backup_restore_and_stale_guard() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let workspace = projects.join("Example");
        let member = "libs/Actor/BaseMonster.actorparams";
        let file = workspace.join("files").join(member);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = br#"<Actor><param name="health" value="100|200|300"/></Actor>"#;
        fs::write(&file, original).unwrap();
        fs::write(workspace.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version":1, "entries":[{"path":member,"format":"raw", "sha256":format!("{:x}",Sha256::digest(original))}]
        })).unwrap()).unwrap();
        let project = ProjectRow {
            label: "Test".into(),
            target: "libs.pak".into(),
            source_build: "Test".into(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
        };
        assert_eq!(guided_members(&project, false).unwrap(), vec![member]);
        let snapshot = read_gameplay_document(&project, member).unwrap();
        let mut fields = evolve_core::gameplay_values::fields(&snapshot).unwrap();
        fields[0].value = "110|220|330".into();
        assert!(save_gameplay_document(&project, &projects, member, &snapshot, &fields).unwrap());
        assert!(save_gameplay_document(&project, &projects, member, &snapshot, &fields).is_err());
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(restore_original_raw_member(&project, &projects, member).unwrap());
        assert_eq!(fs::read(file).unwrap(), original);
        assert!(read_gameplay_document(&project, "../outside.xml").is_err());
    }
    #[test]
    fn converted_xml_import_restore_and_binary_serialization_preserve_identity() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let workspace = projects.join("Example");
        fs::create_dir_all(workspace.join("files")).unwrap();
        let original = br#"<Config xmlns:test="urn:test" test:Value="original"></Config>"#;
        let edited = br#"<Config xmlns:test="urn:test" test:Value="edited"></Config>"#;
        let binary = evolve_core::cryxml::encode_xml(original, MAX_XML_BYTES).unwrap();
        let record = serde_json::json!({"path":"config.xml", "format":"cryxml",
            "sha256":format!("{:x}",Sha256::digest(original)),
            "original_sha256":format!("{:x}",Sha256::digest(&binary))});
        fs::write(workspace.join("files/config.xml"), original).unwrap();
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({"version":1,"entries":[record.clone()]}))
                .unwrap(),
        )
        .unwrap();
        let project = ProjectRow {
            label: "Example".into(),
            target: "example.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: Vec::new(),
            changed_known: true,
        };
        let replacement = temp.path().join("replacement.xml");
        fs::write(&replacement, edited).unwrap();
        assert!(import_raw_member(&project, &projects, "config.xml", &replacement).unwrap());
        let serialized = serialize_project_xml(&binary, edited, &record).unwrap();
        assert_eq!(
            evolve_core::cryxml::xml_bytes(&serialized, MAX_XML_BYTES)
                .unwrap()
                .as_ref(),
            edited
        );
        assert!(serialize_project_xml(&binary, b"<Other/>", &record).is_err());
        let mut wrong = record.clone();
        wrong["original_sha256"] = Value::String("0".repeat(64));
        assert!(serialize_project_xml(&binary, edited, &wrong).is_err());
        fs::write(&replacement, b"<Config><x/>lost tail</Config>").unwrap();
        assert!(import_raw_member(&project, &projects, "config.xml", &replacement).is_err());
        assert_eq!(
            fs::read(workspace.join("files/config.xml")).unwrap(),
            edited
        );
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(restore_original_raw_member(&project, &projects, "config.xml").unwrap());
        assert_eq!(
            fs::read(workspace.join("files/config.xml")).unwrap(),
            original
        );
    }
    #[test]
    fn import_over_raw_asset_keeps_sha_verified_original_backup() {
        let temporary = tempfile::tempdir().unwrap();
        let projects = temporary.path().join("Projects");
        let workspace = projects.join("Example");
        fs::create_dir_all(workspace.join("files/hunters")).unwrap();
        fs::write(workspace.join("files/hunters/example.bin"), b"original").unwrap();
        let original_digest = digest(&workspace.join("files/hunters/example.bin")).unwrap();
        let manifest = serde_json::json!({
            "version":1,
            "entries":[{"path":"hunters/example.bin","format":"raw","sha256":original_digest}]
        });
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let external = temporary.path().join("replacement.bin");
        fs::write(&external, b"edited content").unwrap();
        let project = ProjectRow {
            label: "Example".into(),
            target: "example.pak".into(),
            source_build: "Stage 2".into(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: Vec::new(),
            changed_known: true,
        };
        assert!(import_raw_member(&project, &projects, "hunters/example.bin", &external).unwrap());
        assert_eq!(
            fs::read(workspace.join("files/hunters/example.bin")).unwrap(),
            b"edited content"
        );
        let backups = workspace.join("EditorBackups");
        let revisions = fs::read_dir(&backups)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(
            fs::read(revisions[0].path().join("hunters/example.bin")).unwrap(),
            b"original"
        );
        assert!(!import_raw_member(&project, &projects, "hunters/example.bin", &external).unwrap());
        assert!(import_raw_member(&project, &projects, "unlisted.dat", &external).is_err());
        assert_eq!(fs::read(&external).unwrap(), b"edited content");
        assert!(restore_original_raw_member(&project, &projects, "hunters/example.bin").unwrap());
        assert_eq!(
            fs::read(workspace.join("files/hunters/example.bin")).unwrap(),
            b"original"
        );
        assert!(!restore_original_raw_member(&project, &projects, "hunters/example.bin").unwrap());
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 2);
        assert!(restore_original_raw_member(&project, &projects, "../outside").is_err());
        fs::write(
            workspace.join("files/hunters/example.bin"),
            b"bad external edit",
        )
        .unwrap();
        let renamed = backups.with_extension("hidden");
        fs::rename(&backups, &renamed).unwrap();
        assert!(restore_original_raw_member(&project, &projects, "hunters/example.bin").is_err());
        assert_eq!(
            fs::read(workspace.join("files/hunters/example.bin")).unwrap(),
            b"bad external edit"
        );
    }

    #[test]
    fn indexes_private_manifest_and_ignores_unsafe_targets() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        fs::create_dir_all(stage.join("paks")).unwrap();
        let src = stage.join("paks/libs.pak");
        fs::write(&src, b"original").unwrap();
        let projects = temp.path().join("projects");
        let ws = projects.join("Example");
        fs::create_dir_all(ws.join("files")).unwrap();
        fs::write(ws.join("files/data.xml"), b"edited").unwrap();
        let original = format!("{:x}", Sha256::digest(b"original-data"));
        fs::write(
            ws.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1, "source_pak":src, "entries":[{"path":"data.xml","sha256":original}]
            }))
            .unwrap(),
        )
        .unwrap();
        let list = discover_projects(&projects, &stage).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].target, "libs.pak");
        assert_eq!(list[0].modified, vec!["data.xml"]);
        fs::write(
            ws.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1, "source_pak":src, "entries":[{"path":"../oops","sha256":original}]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            discover_projects(&projects, &stage).unwrap()[0].status,
            "Needs review"
        );
    }
}
